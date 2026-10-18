//! F# plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct FsharpPlugin;

impl LanguagePlugin for FsharpPlugin {
    fn id(&self) -> &'static str {
        "fsharp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".fs", ".fsi", ".fsx"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_fsharp::LANGUAGE_FSHARP.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "fsharp");
        let mut w = FsharpWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct FsharpWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// The type an enclosing `inherit Base(…)` names, innermost last.
    bases: Vec<Vec<String>>,
}

impl<'a> FsharpWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.into()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
    }
    fn child_kind<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                if c.node().kind() == kind {
                    return Some(c.node());
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "anon_type_defn" => {
                let name = self
                    .child_kind(node, "type_name")
                    .and_then(|n| self.child_kind(n, "identifier"))
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    let inherits = self.child_kind(node, "class_inherits_decl");
                    let base = inherits
                        .and_then(|i| self.child_kind(i, "simple_type"))
                        .map(|t| self.text(t).trim().to_string())
                        .filter(|t| !t.is_empty());
                    self.scope.push(name);
                    self.bases.push(base.into_iter().collect());
                    self.record_primary_constructor(node, inherits);
                    self.walk_children(node);
                    self.bases.pop();
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "function_or_value_defn" => {
                self.record_let(node);
                self.walk_children(node);
                return;
            }
            "member_defn" => {
                self.record_member(node);
                self.walk_children(node);
                return;
            }
            "import_decl" => {
                if let Some(li) = self.child_kind(node, "long_identifier") {
                    let path = self.text(li).to_string();
                    if !path.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "open".into(),
                            path,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                return;
            }
            "application_expression" => {
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

    fn record_let(&mut self, node: Node) {
        // Only `let name args = ...` (function_declaration_left) is a callable.
        let Some(decl) = self.child_kind(node, "function_declaration_left") else {
            return;
        };
        let Some(ident) = self.child_kind(decl, "identifier") else {
            return;
        };
        let name = self.text(ident).to_string();
        if name.is_empty() {
            return;
        }
        let qn = self.qn(&name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: if self.scope.is_empty() {
                DefVariant::FreeFunction
            } else {
                DefVariant::InherentMethod
            },
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // F# attributes are `[<FunctionName("X")>]`, which the
            // shared collector reads like any other. Recorded from
            // 0.6.8; before that F# could not match an attribute rule
            // at all, so Azure Functions — a first-party .NET runtime —
            // had nothing to key on.
            attributes: super::attrs::collect_with_preceding(node, self.source),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_member(&mut self, node: Node) {
        // `new(…) = Box(…)`: an additional constructor, named after its
        // type as the primary one is.
        if self.child_kind(node, "additional_constr_defn").is_some() {
            if let Some(ty) = self.scope.last().cloned() {
                self.push_constructor(
                    &ty,
                    node,
                    node.end_byte(),
                    node.end_position().row,
                );
            }
            return;
        }
        let Some(method) = self.child_kind(node, "method_or_prop_defn") else {
            return;
        };
        let Some(prop) = self.child_kind(method, "property_or_ident") else {
            return;
        };
        // property_or_ident contains identifier(s); we want the last one (after `this.`)
        let mut last_ident: Option<Node> = None;
        let mut c = prop.walk();
        if c.goto_first_child() {
            loop {
                if c.node().kind() == "identifier" {
                    last_ident = Some(c.node());
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        let Some(name_node) = last_ident else { return };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }
        let qn = self.qn(&name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: DefVariant::InherentMethod,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // F# attributes are `[<FunctionName("X")>]`, which the
            // shared collector reads like any other. Recorded from
            // 0.6.8; before that F# could not match an attribute rule
            // at all, so Azure Functions — a first-party .NET runtime —
            // had nothing to key on.
            attributes: super::attrs::collect_with_preceding(node, self.source),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    /// `type Box(x: int) = inherit Base(x) …`: the primary constructor,
    /// named after the type and spanning its header — the parameters and
    /// the `inherit` clause, whose `Base(x)` is the `super` call it makes
    /// — but not the members, which are callables of their own. A type
    /// with no parameter list (a record, a union, a module-like type) has
    /// none.
    fn record_primary_constructor(&mut self, node: Node, inherits: Option<Node>) {
        let Some(args) = self.child_kind(node, "primary_constr_args") else {
            return;
        };
        let Some(ty) = self.scope.last().cloned() else {
            return;
        };
        let last = inherits.unwrap_or(args);
        self.push_constructor(&ty, node, last.end_byte(), last.end_position().row);
        let Some(inherits) = inherits else { return };
        let Some(base) = self.child_kind(inherits, "simple_type") else {
            return;
        };
        // `inherit Base` with no argument list calls no constructor here.
        if base.next_named_sibling().is_none() {
            return;
        }
        let name = crate::receiver::type_simple_name(self.text(base))
            .trim()
            .to_string();
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: "super".to_string(),
            site_line: (base.start_position().row as u32) + 1,
            site_byte: base.start_byte() as u32,
            not_def: Some(node.start_byte() as u32),
            ..Default::default()
        });
    }

    /// A constructor of type `ty` spanning `node`'s start to `end`.
    fn push_constructor(&mut self, ty: &str, node: Node, end: usize, end_row: usize) {
        let text =
            std::str::from_utf8(&self.source[node.start_byte()..end]).unwrap_or("");
        self.facts.definitions.push(DefRecord {
            simple_name: ty.to_string(),
            qualified_name: self.qn(ty),
            variant: DefVariant::Constructor,
            start_line: (node.start_position().row as u32) + 1,
            end_line: (end_row as u32) + 1,
            start_byte: node.start_byte() as u32,
            end_byte: end as u32,
            signature_hint: super::extract_signature(text),
            visibility: String::new(),
            attributes: super::attrs::collect_with_preceding(node, self.source),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        // application_expression: first child is the callee. May be nested
        // application_expression (curried application) — descend leftward.
        let mut cur = node;
        loop {
            if let Some(mut first) = cur.child(0) {
                if first.kind() == "application_expression" {
                    cur = first;
                    continue;
                }
                // `f<int> x`: a type application (`typed_expression`)
                // calls what it applies type arguments to.
                if first.kind() == "typed_expression"
                    && let Some(applied) = first.named_child(0)
                {
                    first = applied;
                }
                let inner = if first.kind() == "long_identifier_or_op" {
                    first.child(0).unwrap_or(first)
                } else {
                    first
                };
                match inner.kind() {
                    "long_identifier" => {
                        // Dotted: take last identifier as name, prefix as receiver.
                        let mut idents: Vec<String> = Vec::new();
                        let mut c = inner.walk();
                        if c.goto_first_child() {
                            loop {
                                if c.node().kind() == "identifier" {
                                    idents.push(self.text(c.node()).to_string());
                                }
                                if !c.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                        if let Some(name) = idents.pop() {
                            let receiver = idents.join(".");
                            self.facts.references.push(RefRecord {
                                name,
                                receiver_hint: receiver,
                                site_line: (node.start_position().row as u32) + 1,
                                site_byte: node.start_byte() as u32,
                                ..Default::default()
                            });
                        }
                    }
                    "identifier" => {
                        let name = self.text(inner).to_string();
                        // `new() = Box(1)` delegates to another constructor
                        // of `Box`, never to itself.
                        let not_def = (self.scope.last() == Some(&name))
                            .then(|| {
                                std::iter::successors(node.parent(), |n| n.parent())
                                    .find(|n| n.kind() == "additional_constr_defn")
                                    .and_then(|a| a.parent())
                                    .map(|m| m.start_byte() as u32)
                            })
                            .flatten();
                        self.facts.references.push(RefRecord {
                            name,
                            receiver_hint: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                            not_def,
                            ..Default::default()
                        });
                    }
                    _ => {}
                }
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        FsharpPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/x.fs"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn let_binding_with_args() {
        let src = "module M\nlet greet name = printfn \"Hi\"\nlet add a b = a + b\n";
        let f = extract(src);
        let names: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(names.contains(&"add"), "got: {names:?}");
    }

    #[test]
    fn type_members() {
        let src = "type Service(name: string) =\n    member this.Run() = printfn \"x\"\n    member this.Stop() = ()\n";
        let f = extract(src);
        let qns: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.iter().any(|q| q.ends_with("Run")), "got: {qns:?}");
        assert!(qns.iter().any(|q| q.ends_with("Stop")), "got: {qns:?}");
    }

    #[test]
    fn open_is_import() {
        let src = "open System\nopen System.IO\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path == "System"));
        assert!(f.imports.iter().any(|i| i.path == "System.IO"));
    }

    #[test]
    fn application_captured() {
        let src = "let go () = greet \"x\"\nlet h () = s.Run()\n";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "greet"),
            "refs: {:?}",
            f.references
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "Run" && r.receiver_hint == "s"),
            "refs: {:?}",
            f.references
        );
    }

    /// C1: `f<int> 1` / `M.f<int> 1` are calls of `f`, never dropped.
    #[test]
    fn type_application_calls_are_recorded() {
        let f = extract(
            "module M\nlet g () =\n    f<int> 1\n    N.h<string> \"a\"\n    f 2\n",
        );
        let calls: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert_eq!(
            calls.iter().filter(|c| **c == ("f", "")).count(),
            2,
            "{calls:?}"
        );
        assert!(calls.contains(&("h", "N")), "{calls:?}");
    }

    // WS22 (WS21 carry-over): the primary and additional constructors are
    // callables named after their type; `inherit Base(x)` calls the base's.
    #[test]
    fn constructors_are_recorded() {
        let f = extract(
            "module M\ntype B(n: int) =\n  member this.N = n\ntype C(x: int) =\n  inherit B(x)\n  new() = C(1)\n  member this.X = x\ntype R = { A: int }\n",
        );
        let ctors: Vec<(&str, u32)> = f
            .definitions
            .iter()
            .filter(|d| d.variant == DefVariant::Constructor)
            .map(|d| (d.qualified_name.as_str(), d.start_line))
            .collect();
        assert_eq!(ctors, vec![("B.B", 2), ("C.C", 4), ("C.C", 6)]);
        let x = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "C.X")
            .unwrap();
        assert_eq!(x.base_types, vec!["B"]);
        let refs: Vec<(&str, &str, bool)> = f
            .references
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    r.receiver_hint.as_str(),
                    r.not_def.is_some(),
                )
            })
            .collect();
        assert!(refs.contains(&("B", "super", true)), "{refs:?}");
        assert!(refs.contains(&("C", "", true)), "{refs:?}");
        // Near miss: a record type has no constructor node.
        assert!(!f.definitions.iter().any(|d| d.qualified_name == "R.R"));
    }
}
