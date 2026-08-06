//! Attribute / annotation / decorator capture, and base-type capture.
//!
//! Two extraction signals that half the language plugins were missing,
//! shared because the AST shapes differ only in node names.
//!
//! **Attributes** are shape A of the design — a marker sitting on the
//! definition (`@GetMapping("/x")`, `[HttpGet]`, `#[Route]`). It is the
//! highest-evidence shape there is: the framework's own registration
//! syntax is attached to the callable, so only the route string is ever
//! in doubt, never whether this is an entry point.
//!
//! They are stored **verbatim**, never normalized. `python.rs` refines a
//! `DefVariant` from raw decorator text (`@staticmethod`,
//! `@classmethod`, `@property`), and `--ignore-attributes` matches user
//! patterns against what the user actually wrote. Normalizing at storage
//! would break both; the normalizing accessors
//! (`attribute_key`/`attribute_string_arg`) live at the consumer end
//! instead.
//!
//! **Base types** are shape D — a class or interface declares a contract
//! the runtime invokes (`nn.Module.forward`, `IJob.Execute`). They are
//! recorded on each *method* rather than on the type, because cgg's
//! model has no node for a type: the only thing a framework rule can
//! mark is a callable, so the contract has to travel with one.

use tree_sitter::Node;

/// Node kinds that are an attribute/annotation/decorator, across every
/// grammar cgg links. Deliberately one shared list: a plugin author
/// adding a language should not have to rediscover that Java calls it
/// `marker_annotation` and PHP calls it `attribute`.
const ATTR_KINDS: &[&str] = &[
    // Java / Kotlin / Scala / Groovy
    "annotation",
    "marker_annotation",
    // C# / PHP / Rust
    "attribute",
    "attribute_item",
    // TypeScript / JavaScript / Dart / Python
    "decorator",
];

/// Container kinds that hold attributes as children.
const ATTR_CONTAINER_KINDS: &[&str] =
    &["modifiers", "attribute_list", "attributes", "decorators"];

/// Collect the attributes attached to a definition node.
///
/// Looks in the two places most grammars put them: inside a `modifiers`
/// / `attribute_list` child (Java, C#, PHP, Kotlin) or as a direct
/// child of the definition.
///
/// Deliberately does **not** scan preceding siblings — see
/// [`collect_with_preceding`] for why that costs enough to be opt-in.
pub(crate) fn collect(node: Node, source: &[u8]) -> Vec<String> {
    collect_inner(node, source, false)
}

/// As [`collect`], plus preceding-sibling decorators.
///
/// TypeScript and JavaScript put a decorator *beside* the method inside
/// `class_body`, not inside it, so nothing else finds `@Get('/users')`
/// — the entire NestJS routing surface.
///
/// Separate from [`collect`] because `prev_named_sibling` walks from the
/// parent's first child, making it O(position). Calling it once per
/// definition is O(members²) per class, which on Laravel's larger
/// classes measured as ~4.7s of extra extraction across the corpus.
/// Grammars that keep attributes in a child container must not pay it.
pub(crate) fn collect_with_preceding(node: Node, source: &[u8]) -> Vec<String> {
    collect_inner(node, source, true)
}

fn collect_inner(node: Node, source: &[u8], scan_preceding: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |n: Node| {
        if let Ok(t) = n.utf8_text(source) {
            let t = t.trim();
            if !t.is_empty() && t.len() <= 512 && !out.iter().any(|e: &String| e == t) {
                out.push(t.to_string());
            }
        }
    };

    // (a) inside a modifiers / attribute_list container child.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if ATTR_CONTAINER_KINDS.contains(&child.kind()) {
            let mut inner = child.walk();
            let mut found = false;
            for a in child.named_children(&mut inner) {
                if ATTR_KINDS.contains(&a.kind()) {
                    push(a);
                    found = true;
                }
            }
            // C# `attribute_list` is itself the unit when its children
            // are unnamed; keep the whole `[Attr]` text in that case.
            if !found && child.kind() == "attribute_list" {
                push(child);
            }
        } else if ATTR_KINDS.contains(&child.kind()) {
            // (b) a direct child.
            push(child);
        }
    }

    if !scan_preceding {
        return out;
    }

    // (c) preceding siblings, stopping at the first non-attribute so we
    // never pick up the previous declaration's markers.
    let mut sib = node.prev_named_sibling();
    let mut prefix: Vec<Node> = Vec::new();
    while let Some(s) = sib {
        if ATTR_KINDS.contains(&s.kind()) || ATTR_CONTAINER_KINDS.contains(&s.kind()) {
            prefix.push(s);
            sib = s.prev_named_sibling();
        } else {
            break;
        }
    }
    // Source order, not reverse-walk order.
    for s in prefix.into_iter().rev() {
        if ATTR_KINDS.contains(&s.kind()) {
            push(s);
        } else {
            let mut inner = s.walk();
            for a in s.named_children(&mut inner) {
                if ATTR_KINDS.contains(&a.kind()) {
                    push(a);
                }
            }
        }
    }

    out
}

/// Field names holding a supertype list, across grammars.
const BASE_FIELDS: &[&str] =
    &["superclass", "interfaces", "bases", "superclasses", "type"];

/// Node kinds holding a supertype list.
const BASE_CONTAINER_KINDS: &[&str] = &[
    "superclass",
    "super_interfaces",
    "interfaces",
    "type_list",
    "base_list",
    "class_heritage",
    "extends_clause",
    "implements_clause",
    "argument_list",
    "base_clause",
    "class_interface_clause",
    "extends_type_clause",
    "extends_interfaces",
    "delegation_specifier",
    "delegation_specifiers",
];

/// Collect the base classes / implemented interfaces of a type
/// declaration node.
///
/// Generic arguments are kept as written (`IConsumer<OrderPlaced>`); the
/// matcher strips them, because a rule should be able to name either
/// form and the raw text is what a reader would recognise.
pub(crate) fn base_types(node: Node, source: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let consider = |n: Node, out: &mut Vec<String>| {
        let Ok(t) = n.utf8_text(source) else { return };
        for part in split_type_list(t) {
            if is_type_name(&part) && !out.contains(&part) {
                out.push(part);
            }
        }
    };

    for f in BASE_FIELDS {
        if let Some(n) = node.child_by_field_name(f) {
            consider(n, &mut out);
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if BASE_CONTAINER_KINDS.contains(&child.kind()) {
            consider(child, &mut out);
        }
    }
    out
}

/// Split a supertype clause into individual type names.
///
/// Handles `extends A implements B, C`, `: A, B`, `(A, B)` and
/// `< Super`, and skips keyword-argument noise Python allows in a base
/// list (`class C(Base, metaclass=Meta)`).
fn split_type_list(raw: &str) -> Vec<String> {
    let mut s = raw.trim();
    for kw in ["extends", "implements", "public", "private", "protected"] {
        s = s.trim_start_matches(kw).trim_start();
    }
    let s = s
        .trim_start_matches(['(', ':', '<', '['])
        .trim_end_matches([')', ']', '>'])
        .trim();

    let mut parts: Vec<String> = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '<' | '[' | '(' => {
                depth += 1;
                cur.push(c);
            }
            '>' | ']' | ')' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth <= 0 => {
                parts.push(std::mem::take(&mut cur));
            }
            _ => cur.push(c),
        }
    }
    parts.push(cur);

    parts
        .into_iter()
        .map(|p| {
            p.trim()
                .trim_start_matches("extends ")
                .trim_start_matches("implements ")
                .trim()
                .to_string()
        })
        .filter(|p| !p.is_empty())
        .collect()
}

/// Whether a token looks like a type name rather than a keyword
/// argument, a literal, or grammar noise.
fn is_type_name(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    // `metaclass=Meta` and `**kwargs` are not base types.
    if s.contains('=') || s.starts_with('*') || s.contains(' ') {
        return false;
    }
    if matches!(
        s,
        "object" | "Object" | "extends" | "implements" | "class" | "struct" | "where"
    ) {
        return false;
    }
    s.starts_with(|c: char| c.is_alphabetic() || c == '_')
        && s.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '<' | '>' | ',' | '\\')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_lists_split_on_top_level_commas_only() {
        assert_eq!(split_type_list("(nn.Module)"), vec!["nn.Module"]);
        assert_eq!(
            split_type_list("extends A implements B, C"),
            vec!["A implements B", "C"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
        // Generic arguments must not be mistaken for separators.
        assert_eq!(
            split_type_list(": IConsumer<Order, Item>, IJob"),
            vec!["IConsumer<Order, Item>", "IJob"]
        );
    }

    #[test]
    fn python_keyword_bases_are_not_types() {
        assert!(!is_type_name("metaclass=Meta"));
        assert!(!is_type_name("**kwargs"));
        // `object` carries no information; every Python class has it.
        assert!(!is_type_name("object"));
        assert!(is_type_name("nn.Module"));
        assert!(is_type_name("Sidekiq::Job"));
        assert!(is_type_name("IConsumer<OrderPlaced>"));
    }
}
