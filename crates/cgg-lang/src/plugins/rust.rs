//! Rust plugin.
//!
//! Two-phase AST pass over a `tree-sitter-rust` tree:
//!
//! * **Definitions** — `function_item` (plus its `async` / `unsafe` /
//!   `pub` flavors), inherent-`impl` methods, trait-`impl` methods,
//!   trait-default methods, free functions inside `mod` blocks, and
//!   named closures (`let foo = |x| { ... };`).
//! * **References** — every `call_expression` whose callee is an
//!   identifier, a path (`a::b::c()`), or a method (`recv.method()`).
//! * **Imports** — `use_declaration` records, flattened into one entry
//!   per imported path with optional alias.
//!
//! Qualified names are assembled from a scope stack seeded with
//! `"crate"`: each `mod_item` pushes its identifier; each `impl_item`
//! pushes the implemented type (prefixed with `"<trait> for "` for
//! trait impls so the path stays unique). A leaf `function_item` is
//! then emitted as `crate::mod_a::mod_b::Type::method` or
//! `crate::mod_a::free_fn` as appropriate.

use std::path::Path;

use cgg_core::{
    ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
};
use tree_sitter::{Node, Tree, TreeCursor};

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct RustPlugin;

impl LanguagePlugin for RustPlugin {
    fn id(&self) -> &'static str {
        "rust"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".rs"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "rust");
        let mut walker = Walker {
            source,
            facts: &mut facts,
            scope: vec![ScopeSegment::Crate("crate".into())],
        };
        walker.walk(tree.root_node());
        facts
    }
}

/// A scope-stack entry tagged by origin. Joined into qualified names
/// via `::`; the tag lets us classify a leaf function correctly
/// (inherent method vs trait method vs free function).
#[derive(Clone, Debug)]
enum ScopeSegment {
    Crate(String),
    Mod(String),
    InherentImpl(String),
    TraitImpl(String),
    Trait(String),
}

impl ScopeSegment {
    fn display(&self) -> &str {
        match self {
            ScopeSegment::Crate(s)
            | ScopeSegment::Mod(s)
            | ScopeSegment::InherentImpl(s)
            | ScopeSegment::TraitImpl(s)
            | ScopeSegment::Trait(s) => s.as_str(),
        }
    }
}

/// In-progress walker owning a scope stack.
struct Walker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<ScopeSegment>,
}

impl<'a> Walker<'a> {
    fn text(&self, node: Node) -> &str {
        node.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        let kind = node.kind();

        match kind {
            // `mod foo { ... }` / `mod foo;`
            "mod_item" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(ScopeSegment::Mod(name));
                }
                self.walk_children(node);
                if !self.scope.last().map_or(true, |s| matches!(s, ScopeSegment::Crate(_))) {
                    self.scope.pop();
                }
                return;
            }

            // `impl Type { ... }` or `impl Trait for Type { ... }`
            "impl_item" => {
                let type_name = node
                    .child_by_field_name("type")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_else(|| "<unknown>".into());
                let trait_name = node
                    .child_by_field_name("trait")
                    .map(|n| self.text(n).to_string());
                let segment = match &trait_name {
                    Some(t) => {
                        ScopeSegment::TraitImpl(format!("<{type_name} as {t}>"))
                    }
                    None => ScopeSegment::InherentImpl(type_name.clone()),
                };
                self.scope.push(segment);
                self.walk_children(node);
                self.scope.pop();
                return;
            }

            // `trait T { fn default_m() { ... } }`
            "trait_item" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let pushed = !name.is_empty();
                if pushed {
                    self.scope.push(ScopeSegment::Trait(name));
                }
                self.walk_children(node);
                if pushed {
                    self.scope.pop();
                }
                return;
            }

            // `fn foo(...) { ... }`
            "function_item" => {
                self.record_function(node, /* is_trait_default */ false);
                self.walk_children(node);
                return;
            }

            // `fn foo(...);` — trait method signatures without bodies
            // appear inside trait_item as function_signature_item.
            "function_signature_item" => {
                self.record_function(node, /* sig only */ false);
                // No body to descend into.
                return;
            }

            // `use a::b::c;`, `use a::b::c as d;`
            "use_declaration" => {
                self.record_use(node);
                // Still walk children for completeness.
                self.walk_children(node);
                return;
            }

            // `let foo = |x| { ... };` -> named closure.
            "let_declaration" => {
                if let Some(rec) = self.named_closure(node) {
                    self.facts.definitions.push(rec);
                }
                self.walk_children(node);
                return;
            }

            // References.
            "call_expression" => {
                if let Some(r) = self.ref_from_call(node) {
                    self.facts.references.push(r);
                }
                self.walk_children(node);
                return;
            }

            _ => {}
        }

        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut cursor: TreeCursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                self.walk(cursor.node());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_function(&mut self, node: Node, _sig_only: bool) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }

        let visibility = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "visibility_modifier")
            .map(|c| self.text(c).to_string())
            .unwrap_or_default();

        // Detect async and unsafe markers for the signature hint.
        let mut is_async = false;
        {
            let mut c = node.walk();
            for child in node.children(&mut c) {
                match child.kind() {
                    "async" => is_async = true,
                    "function_modifiers" => {
                        let mut mc = child.walk();
                        for m in child.children(&mut mc) {
                            if m.kind() == "async" {
                                is_async = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Classify variant from scope context.
        let variant = if is_async {
            DefVariant::AsyncFunction
        } else {
            match self.scope.last() {
                Some(ScopeSegment::TraitImpl(_)) => DefVariant::TraitMethod,
                Some(ScopeSegment::InherentImpl(_)) => DefVariant::InherentMethod,
                Some(ScopeSegment::Trait(_)) => DefVariant::TraitDefaultMethod,
                _ => DefVariant::FreeFunction,
            }
        };

        let qn = qualified_name(&self.scope, &simple);
        let (sl, el) = line_range(node);
        let signature = single_line(self.text(node));

        let attributes = collect_attributes(node, self.source);

        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: signature,
            visibility,
            attributes,
        });
    }

    /// Detect `let NAME = |..| {..};` and treat the binding as a
    /// named closure definition.
    fn named_closure(&mut self, node: Node) -> Option<DefRecord> {
        let pattern = node.child_by_field_name("pattern")?;
        if pattern.kind() != "identifier" {
            return None;
        }
        let value = node.child_by_field_name("value")?;
        if value.kind() != "closure_expression" {
            return None;
        }
        let simple = self.text(pattern).to_string();
        if simple.is_empty() {
            return None;
        }
        let qn = qualified_name(&self.scope, &simple);
        let (sl, el) = line_range(node);
        Some(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant: DefVariant::NamedClosure,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: single_line(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
        })
    }

    fn record_use(&mut self, node: Node) {
        // Strip `use` and `;`, serialize the tree text, and record the
        // raw path string. Aliases are detected via `as` inside the
        // path. This is intentionally loose — Task 6 parses paths
        // structurally for resolution.
        let text = self.text(node);
        let core = text
            .trim_start_matches("use")
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();
        let (path, alias) = if let Some(idx) = core.rfind(" as ") {
            (
                core[..idx].to_string(),
                core[idx + 4..].to_string(),
            )
        } else {
            (core, String::new())
        };
        let start_line = (node.start_position().row as u32) + 1;
        self.facts.imports.push(ImportRecord {
            kind: "use".into(),
            path,
            alias,
            site_line: start_line,
            site_byte: node.start_byte() as u32,
        });
    }

    fn ref_from_call(&mut self, node: Node) -> Option<RefRecord> {
        let callee = node.child_by_field_name("function")?;
        let (name, receiver_hint) = match callee.kind() {
            "identifier" => (self.text(callee).to_string(), String::new()),
            "field_expression" => {
                // recv.method
                let field = callee.child_by_field_name("field")?;
                let recv = callee
                    .child_by_field_name("value")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (self.text(field).to_string(), recv)
            }
            "scoped_identifier" => {
                // `a::b::c`
                let name = self.text(callee).to_string();
                // Last segment is the simple name; remainder is receiver path.
                let (receiver, simple) = match name.rfind("::") {
                    Some(idx) => (name[..idx].to_string(), name[idx + 2..].to_string()),
                    None => (String::new(), name.clone()),
                };
                (simple, receiver)
            }
            _ => return None,
        };
        if name.is_empty() {
            return None;
        }
        let start_line = (node.start_position().row as u32) + 1;
        Some(RefRecord {
            name,
            receiver_hint,
            site_line: start_line,
            site_byte: node.start_byte() as u32,
        })
    }
}

fn qualified_name(scope: &[ScopeSegment], simple: &str) -> String {
    let mut parts: Vec<&str> = scope.iter().map(|s| s.display()).collect();
    parts.push(simple);
    parts.join("::")
}

fn line_range(node: Node) -> (u32, u32) {
    let start = (node.start_position().row as u32) + 1;
    let end = (node.end_position().row as u32) + 1;
    (start, end)
}

fn single_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn collect_attributes(node: Node, source: &[u8]) -> Vec<String> {
    // Attributes appear as preceding siblings of `function_item` in
    // `declaration_list`; they look like `#[attr]` or `#![attr]`.
    let mut out = Vec::new();
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "attribute_item" | "inner_attribute_item" | "line_comment" | "block_comment" => {
                if s.kind().contains("attribute") {
                    out.push(s.utf8_text(source).unwrap_or("").trim().to_string());
                }
                sib = s.prev_sibling();
            }
            _ => break,
        }
    }
    out.reverse();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        RustPlugin.extract(FileId::new(0), &PathBuf::from("x.rs"), &tree, src.as_bytes())
    }

    #[test]
    fn free_function() {
        let f = extract("fn foo() { bar(); }\nfn bar() {}\n");
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"crate::foo"), "got: {names:?}");
        assert!(names.contains(&"crate::bar"), "got: {names:?}");
        let ref_names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"bar"));
    }

    #[test]
    fn method_in_impl() {
        let src = r#"
struct T;
impl T {
    fn m(&self) { self.n(); }
    fn n(&self) {}
}
"#;
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.qualified_name.clone(), d.clone()))
            .collect();
        assert!(by.contains_key("crate::T::m"), "names: {by:?}");
        assert!(by.contains_key("crate::T::n"), "names: {by:?}");
        assert_eq!(
            by["crate::T::m"].variant,
            DefVariant::InherentMethod,
            "inherent-impl methods must be classified InherentMethod"
        );
        assert_eq!(by["crate::T::n"].variant, DefVariant::InherentMethod);
    }

    #[test]
    fn trait_impl_method() {
        let src = r#"
trait R { fn r(&self); }
struct S;
impl R for S { fn r(&self) {} }
"#;
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("<S as R>::r")),
            "got: {names:?}"
        );
    }

    #[test]
    fn mod_prefix() {
        let f = extract("mod a { mod b { fn c() {} } }");
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(names, vec!["crate::a::b::c"]);
    }

    #[test]
    fn named_closure_is_callable() {
        let src = "fn wrap() {\n    let inc = |x: i32| x + 1;\n    inc(1);\n}\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"crate::wrap"));
        assert!(names.iter().any(|n| n.ends_with("inc")), "got: {names:?}");
        let ref_names: Vec<&str> =
            f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"inc"));
    }

    #[test]
    fn call_expressions_captured() {
        let src = r#"
fn main() {
    std::println!("hi");
    helper();
    self.m();
    a::b::c();
}
fn helper() {}
"#;
        let f = extract(src);
        let names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        // Expect at minimum: helper, m, c.
        assert!(names.contains(&"helper"), "got: {names:?}");
        assert!(names.contains(&"m"), "got: {names:?}");
        assert!(names.contains(&"c"), "got: {names:?}");
    }

    #[test]
    fn use_imports_captured() {
        let src = "use a::b::c;\nuse x::y as z;\nfn main() {}\n";
        let f = extract(src);
        assert_eq!(f.imports.len(), 2);
        assert_eq!(f.imports[0].kind, "use");
        assert_eq!(f.imports[0].path, "a::b::c");
        assert_eq!(f.imports[0].alias, "");
        assert_eq!(f.imports[1].path, "x::y");
        assert_eq!(f.imports[1].alias, "z");
    }

    #[test]
    fn visibility_and_async_captured() {
        let src = "pub async fn f() {}\nasync fn g() {}\npub(crate) fn h() {}\n";
        let f = extract(src);
        let by_name: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.clone()))
            .collect();
        assert_eq!(by_name["f"].visibility, "pub");
        assert_eq!(by_name["f"].variant, DefVariant::AsyncFunction);
        assert_eq!(by_name["g"].variant, DefVariant::AsyncFunction);
        assert!(by_name["h"].visibility.starts_with("pub"));
    }
}
