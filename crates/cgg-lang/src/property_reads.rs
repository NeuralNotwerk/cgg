//! `--property-edges` beyond Python: member reads that run a getter.
//!
//! Reading `a.size` runs code wherever the language has getters — a
//! JS/TS `get size()`, a C# property, a Kotlin `get()`, a Swift computed
//! property, a Dart `get size`, a Groovy `getSize()` and an Objective-C
//! dot-syntax message. Python's plugin records its own reads; this walk
//! covers the others (`PROPERTY_READ_LANGS`), and only runs under
//! [`crate::ExtractCtx::property_sites`], so a default run extracts
//! exactly what it did before.
//!
//! It records two things:
//! - getters the plugin does not extract as callables (Kotlin `get()`,
//!   Swift computed properties) or extracts as methods (a Dart getter is
//!   retagged `Property`); C#'s plugin already names getters by their
//!   property, and Groovy/Objective-C getters are ordinary methods the
//!   pass reads by name;
//! - every member read in value position whose receiver is a plain name
//!   or `this`/`self`, as a `RefRecord::property_read` site. Resolvers
//!   skip those; `cgg_resolve::property_edges` binds them only through a
//!   declared receiver type.

use cgg_core::{DefRecord, DefVariant, FileFacts, RefRecord};
use tree_sitter::{Node, Tree};

/// Languages this walk reads.
pub const PROPERTY_READ_LANGS: &[&str] = &[
    "javascript",
    "typescript",
    "csharp",
    "kotlin",
    "swift",
    "dart",
    "groovy",
    "objc",
];

fn text<'s>(n: Node, src: &'s [u8]) -> &'s str {
    std::str::from_utf8(&src[n.byte_range()]).unwrap_or("")
}

fn ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

/// `(receiver, member)` of a member read node.
fn read_parts<'t>(lang: &str, n: Node<'t>) -> Option<(Node<'t>, Node<'t>)> {
    match (lang, n.kind()) {
        ("javascript" | "typescript" | "dart", "member_expression") => Some((
            n.child_by_field_name("object")?,
            n.child_by_field_name("property")?,
        )),
        ("csharp", "member_access_expression") => Some((
            n.child_by_field_name("expression")?,
            n.child_by_field_name("name")?,
        )),
        ("kotlin", "navigation_expression") => {
            let recv = n.named_child(0)?;
            let suffix = n
                .named_child(1)
                .filter(|s| s.kind() == "navigation_suffix")?;
            let name = suffix
                .named_child(0)
                .filter(|s| s.kind() == "simple_identifier")?;
            Some((recv, name))
        }
        ("swift", "navigation_expression") => {
            let target = n.child_by_field_name("target")?;
            let suffix = n.child_by_field_name("suffix")?;
            Some((target, suffix.child_by_field_name("suffix")?))
        }
        ("groovy", "field_access") => Some((
            n.child_by_field_name("object")?,
            n.child_by_field_name("field")?,
        )),
        ("objc", "field_expression") => Some((
            n.child_by_field_name("argument")?,
            n.child_by_field_name("field")?,
        )),
        _ => None,
    }
}

/// Whether `n` is not a read: the callee of a call, or a write target.
fn not_a_read(n: Node) -> bool {
    let Some(p) = n.parent() else {
        return false;
    };
    let first = p.named_child(0).is_some_and(|c| c.id() == n.id());
    let field_is = |f: &str| p.child_by_field_name(f).is_some_and(|c| c.id() == n.id());
    match p.kind() {
        "call_expression" | "invocation_expression" => field_is("function") || first,
        // Groovy: `obj.m()` holds `obj.m` as its `function`, and `obj.m x`
        // as its `receiver`.
        "method_invocation" => field_is("function"),
        "command_chain" => field_is("receiver"),
        "assignment_expression" | "augmented_assignment_expression" | "assignment" => {
            field_is("left") || first
        }
        k if k.contains("update") || k.contains("increment") => true,
        "directly_assignable_expression" | "assignable_expression" => true,
        _ => false,
    }
}

/// The names of the classes around `n`, outermost first.
fn class_path(n: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = n.parent();
    while let Some(p) = cur {
        if matches!(p.kind(), "class_declaration" | "object_declaration") {
            let name = p.child_by_field_name("name").or_else(|| {
                let mut c = p.walk();
                p.named_children(&mut c)
                    .find(|c| matches!(c.kind(), "type_identifier" | "simple_identifier"))
            });
            if let Some(name) = name {
                out.push(text(name, src).to_string());
            }
        }
        cur = p.parent();
    }
    out.reverse();
    out
}

/// A Kotlin `get()` or Swift computed property, as `(name node)`.
fn getter_name<'t>(lang: &str, n: Node<'t>) -> Option<Node<'t>> {
    if n.kind() != "property_declaration" {
        return None;
    }
    let mut c = n.walk();
    match lang {
        "kotlin" => {
            let kids: Vec<Node> = n.named_children(&mut c).collect();
            kids.iter().any(|k| k.kind() == "getter").then_some(())?;
            let decl = kids.iter().find(|k| k.kind() == "variable_declaration")?;
            decl.named_child(0)
                .filter(|x| x.kind() == "simple_identifier")
        }
        "swift" => {
            n.child_by_field_name("computed_value")?;
            n.child_by_field_name("name")?
                .child_by_field_name("bound_identifier")
        }
        _ => None,
    }
}

/// Record getters and property reads for one file.
pub fn collect(lang: &str, tree: &Tree, src: &[u8], facts: &mut FileFacts) {
    if !PROPERTY_READ_LANGS.contains(&lang) {
        return;
    }
    let package = if lang == "kotlin" {
        facts.packages.first().cloned()
    } else {
        None
    };
    let mut cursor = tree.walk();
    let mut done = false;
    let mut dart_getters: Vec<(u32, u32, String)> = Vec::new();
    while !done {
        let n = cursor.node();
        if !cursor.goto_first_child() {
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    done = true;
                    break;
                }
            }
        }
        if let Some(name) = getter_name(lang, n) {
            let classes = class_path(n, src);
            let simple = text(name, src).to_string();
            if !classes.is_empty()
                && ident(&simple)
                && !facts
                    .definitions
                    .iter()
                    .any(|d| d.start_byte == n.start_byte() as u32)
            {
                let mut parts: Vec<String> = package.iter().cloned().collect();
                parts.extend(classes);
                parts.push(simple.clone());
                facts.definitions.push(DefRecord {
                    simple_name: simple,
                    qualified_name: parts.join("."),
                    variant: DefVariant::Property,
                    start_line: n.start_position().row as u32 + 1,
                    end_line: n.end_position().row as u32 + 1,
                    start_byte: n.start_byte() as u32,
                    end_byte: n.end_byte() as u32,
                    ..Default::default()
                });
            }
            continue;
        }
        if lang == "dart" && n.kind() == "getter_signature" {
            if let Some(name) = n.child_by_field_name("name") {
                dart_getters.push((
                    n.start_byte() as u32,
                    n.end_byte() as u32,
                    text(name, src).to_string(),
                ));
            }
            continue;
        }
        let Some((recv, member)) = read_parts(lang, n) else {
            continue;
        };
        if not_a_read(n) {
            continue;
        }
        let (r, m) = (text(recv, src), text(member, src));
        if !ident(m) || !(ident(r) || r == "this" || r == "self") {
            continue;
        }
        facts.references.push(RefRecord {
            name: m.to_string(),
            receiver_hint: r.to_string(),
            site_line: member.start_position().row as u32 + 1,
            site_byte: member.start_byte() as u32,
            property_read: true,
            ..Default::default()
        });
    }
    for d in &mut facts.definitions {
        if dart_getters.iter().any(|(s, e, name)| {
            d.start_byte <= *s && *e <= d.end_byte && d.simple_name == *name
        }) {
            d.variant = DefVariant::Property;
        }
    }
}
