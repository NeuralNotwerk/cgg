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
        };
        w.walk(tree.root_node());
        facts
    }
}

struct FsharpWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
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
                    self.scope.push(name);
                    self.walk_children(node);
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
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_member(&mut self, node: Node) {
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
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        // application_expression: first child is the callee. May be nested
        // application_expression (curried application) — descend leftward.
        let mut cur = node;
        loop {
            if let Some(first) = cur.child(0) {
                if first.kind() == "application_expression" {
                    cur = first;
                    continue;
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
                        self.facts.references.push(RefRecord {
                            name: self.text(inner).to_string(),
                            receiver_hint: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
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
}
