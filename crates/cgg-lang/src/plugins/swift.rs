//! Swift plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct SwiftPlugin;

impl LanguagePlugin for SwiftPlugin {
    fn id(&self) -> &'static str {
        "swift"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".swift"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_swift::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "swift");
        let mut w = SwiftWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct SwiftWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Supertypes and protocols of the enclosing type, innermost last
    /// (`class Sq: Base, Shape`, `extension Foo: Shape`).
    bases: Vec<Vec<String>>,
}

impl<'a> SwiftWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_declaration"
            | "struct_declaration"
            | "enum_declaration"
            | "protocol_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                self.bases.push(swift_bases(node, self.source));
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                self.bases.pop();
                return;
            }
            // `protocol P { func m() }`: a requirement with no body.
            // Recorded as P's member so a call typed `P` binds to the
            // declaration its conformers are dispatched from.
            "protocol_function_declaration" => {
                if !self.scope.is_empty() {
                    self.record_requirement(node);
                }
                return;
            }
            "function_declaration" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "init_declaration" => {
                let qn = self.qn("init");
                let (sl, el) = (
                    (node.start_position().row as u32) + 1,
                    (node.end_position().row as u32) + 1,
                );
                self.facts.definitions.push(DefRecord {
                    simple_name: "init".into(),
                    qualified_name: qn,
                    variant: DefVariant::Constructor,
                    start_line: sl,
                    end_line: el,
                    start_byte: node.start_byte() as u32,
                    end_byte: node.end_byte() as u32,
                    signature_hint: super::extract_signature(self.text(node)),
                    visibility: String::new(),
                    attributes: Vec::new(),
                    ..Default::default()
                });
                self.walk_children(node);
                return;
            }
            "import_declaration" => {
                self.record_import(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                self.walk(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_function(&mut self, node: Node) {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let is_static = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "modifiers" && self.text(c).contains("static"));
        let variant = if is_static {
            DefVariant::StaticMethod
        } else if self.scope.is_empty() {
            DefVariant::FreeFunction
        } else {
            DefVariant::InherentMethod
        };
        let qn = self.qn(&name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: super::attrs::with_override_marker(node, Vec::new()),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_requirement(&mut self, node: Node) {
        let Some(name) = node
            .child_by_field_name("name")
            .filter(|n| n.kind() == "simple_identifier")
            .map(|n| self.text(n).to_string())
        else {
            return;
        };
        if name.is_empty() {
            return;
        }
        let qn = self.qn(&name);
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: DefVariant::TraitMethod,
            start_line: (node.start_position().row as u32) + 1,
            end_line: (node.end_position().row as u32) + 1,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // A declaration, not a target: duck-typed fan-out prefers
            // the conformers when one survives.
            attributes: vec!["abstract".to_string()],
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_import(&mut self, node: Node) {
        let path = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if path.is_empty() {
            return;
        }
        self.facts.imports.push(ImportRecord {
            kind: "import".into(),
            path,
            alias: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_call(&mut self, node: Node) {
        // call_expression -> first child is callee (simple_identifier or navigation_expression)
        let callee = node.child(0);
        let Some(callee) = callee else { return };
        let mut typed_receiver = false;
        let (name, recv) = match callee.kind() {
            "simple_identifier" => (self.text(callee).to_string(), String::new()),
            "navigation_expression" => {
                // `(f).m()`, `(o as! Foo).m()`, `Foo().m()`, `f!.m()`,
                // `(try await x).m()`: the value or the type the
                // receiver's shape stands for (`crate::receiver`).
                let target = callee
                    .child_by_field_name("target")
                    .map(|n| {
                        let p = crate::receiver::peel("swift", n, self.source);
                        match p.type_name() {
                            Some(t) => {
                                typed_receiver = true;
                                t
                            }
                            None => self.text(p.node).to_string(),
                        }
                    })
                    .unwrap_or_default();
                let suffix = callee
                    .children(&mut callee.walk())
                    .find(|c| c.kind() == "navigation_suffix")
                    .and_then(|s| {
                        s.children(&mut s.walk())
                            .find(|c| c.kind() == "simple_identifier")
                    })
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (suffix, target)
            }
            // `Wrap<Int>.s()` where the grammar reads `Wrap < Int > .s()`.
            "prefix_expression" => match misparsed_generic_member_call(node, self.source)
            {
                Some(parts) => parts,
                None => return,
            },
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver,
            ..Default::default()
        });
    }
}

/// `Wrap<Int>.s()` in expression position is parsed as two comparisons,
/// `Wrap < (Int > (.s()))`, with `.s()` an implicit-member call. Swift's
/// comparison operators do not associate, so `a < b > c` is never valid
/// source: that nesting, written with no space before `<` or between `>`
/// and `.`, is a generic type's static member call. Returns `(member,
/// type)` with the type's arguments dropped (`("s", "Wrap")`), or `None`
/// for anything else, including a real implicit-member call (`.make()`).
fn misparsed_generic_member_call(call: Node, src: &[u8]) -> Option<(String, String)> {
    let prefix = call.child(0)?;
    if prefix.kind() != "prefix_expression"
        || prefix.child_by_field_name("operation")?.kind() != "."
    {
        return None;
    }
    let member = prefix.child_by_field_name("target")?;
    if member.kind() != "simple_identifier" {
        return None;
    }
    let close = call.parent()?;
    let open = close.parent()?;
    let is_cmp = |n: Node, op: &str, rhs: Node| {
        n.kind() == "comparison_expression"
            && n.child_by_field_name("op").is_some_and(|o| o.kind() == op)
            && n.child_by_field_name("rhs") == Some(rhs)
    };
    if !is_cmp(close, ">", call) || !is_cmp(open, "<", close) {
        return None;
    }
    let ty = open.child_by_field_name("lhs")?;
    let lt = open.child_by_field_name("op")?;
    let gt = close.child_by_field_name("op")?;
    if !matches!(ty.kind(), "simple_identifier" | "navigation_expression")
        || ty.end_byte() != lt.start_byte()
        || gt.end_byte() != prefix.start_byte()
    {
        return None;
    }
    let text = |n: Node| n.utf8_text(src).unwrap_or("").to_string();
    Some((text(member), text(ty)))
}

/// The types a Swift type declaration inherits from or conforms to, as
/// written minus generic arguments (`Base<T>` -> `Base`).
fn swift_bases(node: Node, source: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in node.children(&mut node.walk()) {
        if c.kind() != "inheritance_specifier" {
            continue;
        }
        let Some(t) = c.child_by_field_name("inherits_from") else {
            continue;
        };
        let text = t.utf8_text(source).unwrap_or("");
        let bare = text.split('<').next().unwrap_or(text).trim();
        let bare = bare.rsplit('.').next().unwrap_or(bare);
        if !bare.is_empty() && !out.iter().any(|x| x == bare) {
            out.push(bare.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    // WS20 gap 5: the `override` marker is recorded so dead-code can
    // cap an override of an out-of-tree base at medium.
    #[test]
    fn override_modifier_is_recorded() {
        let f = extract(
            "class A: ExtBase {\n    override func hook() { }\n    func plain() { }\n}\n",
        );
        let marked = |n: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == n)
                .unwrap_or_else(|| panic!("{n} not extracted"))
                .attributes
                .iter()
                .any(|a| a == "override")
        };
        assert!(marked("hook"));
        assert!(!marked("plain"));
    }

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_swift::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        SwiftPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.swift"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn class_methods() {
        let src = "class Service {\n  init(name: String) {}\n  func run() {}\n  static func create() -> Service { return Service(name: \"\") }\n}\n";
        let f = extract(src);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"Service.init"), "got: {qns:?}");
        assert!(qns.contains(&"Service.run"), "got: {qns:?}");
        assert!(qns.contains(&"Service.create"), "got: {qns:?}");
    }

    #[test]
    fn free_function() {
        let src = "func greet(_ name: String) { print(name) }\n";
        let f = extract(src);
        assert!(
            f.definitions.iter().any(
                |d| d.simple_name == "greet" && d.variant == DefVariant::FreeFunction
            )
        );
    }

    #[test]
    fn import_captured() {
        let src = "import Foundation\nfunc f() {}\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path == "Foundation"));
    }

    #[test]
    fn call_expressions() {
        let src = "func f() { greet(\"x\"); obj.run() }\n";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "greet"),
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn protocol_requirements_are_declarations_and_conformances_are_bases() {
        let f = extract(
            "protocol Shape: Base { func area() -> Double }\nclass Sq: Shape { func area() -> Double { return 1 } }\n",
        );
        let req = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Shape.area")
            .expect("protocol requirement");
        assert_eq!(req.variant, DefVariant::TraitMethod);
        assert!(req.attributes.iter().any(|a| a == "abstract"));
        let imp = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Sq.area")
            .expect("conformer");
        assert_eq!(imp.base_types, vec!["Shape"]);
    }

    /// C1: `Wrap<Int>.s()` is parsed as `Wrap < Int > .s()`; it is a
    /// call of `s` on `Wrap`.
    #[test]
    fn generic_type_static_call_misparsed_as_comparison() {
        let f =
            extract("func caller() {\n  Wrap<Int>.s()\n  foo(M.Wrap<String>.t(1))\n}\n");
        let calls: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(calls.contains(&("s", "Wrap")), "{calls:?}");
        assert!(calls.contains(&("t", "M.Wrap")), "{calls:?}");
    }

    /// Near-miss: real comparisons and a real implicit-member call are
    /// not read as generic calls.
    #[test]
    fn comparisons_and_implicit_members_are_not_generic_calls() {
        let f = extract(
            "func caller(a: Int, b: Int, c: Int) {\n  if a < b && c > d.e() {}\n  let x: W = .make()\n}\n",
        );
        let calls: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert_eq!(calls, vec![("e", "d")]);
    }
}
