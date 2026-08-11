//! Nix plugin — callable extraction.
//!
//! Nix is lambda-based: a "function" is a `function_expression`
//! (`x: body` or `{ a, b }: body`). Most named functions show up as
//! `binding`s in a `let`/`rec`/attrset where the right-hand side is a
//! `function_expression`. Calls are `apply_expression` (juxtaposition
//! is application; `f x` applies `f` to `x`). The built-in `import`
//! is a regular function — we surface it as an import record.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct NixPlugin;

impl LanguagePlugin for NixPlugin {
    fn id(&self) -> &'static str {
        "nix"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".nix"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_nix::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "nix");
        let mut w = NixWalker {
            source,
            facts: &mut facts,
        };
        w.walk(tree.root_node());
        facts
    }
}

struct NixWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> NixWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
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
            "binding" => {
                if let Some(rhs) = self.binding_value(node)
                    && rhs.kind() == "function_expression"
                {
                    self.record_function_binding(node);
                }
                self.walk_children(node);
                return;
            }
            "apply_expression" => {
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

    fn binding_value<'n>(&self, node: Node<'n>) -> Option<Node<'n>> {
        // binding: attrpath = <value>;
        // Iterate past attrpath and `=` to find the value node.
        let mut c = node.walk();
        let mut seen_eq = false;
        if c.goto_first_child() {
            loop {
                let n = c.node();
                if seen_eq && n.is_named() {
                    return Some(n);
                }
                if n.kind() == "=" {
                    seen_eq = true;
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }

    fn record_function_binding(&mut self, node: Node) {
        let Some(attrpath) = self.child_kind(node, "attrpath") else {
            return;
        };
        let name = self.text(attrpath).to_string();
        if name.is_empty() {
            return;
        }
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name.clone(),
            qualified_name: name,
            variant: DefVariant::FreeFunction,
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
        // Curried application: descend left until callee is a variable/select.
        let mut cur = node;
        let mut depth_guard = 0;
        while depth_guard < 64 {
            let Some(first) = cur.child(0) else { break };
            if first.kind() == "apply_expression" {
                cur = first;
                depth_guard += 1;
                continue;
            }
            let (name, receiver) = match first.kind() {
                "variable_expression" => {
                    let n = self
                        .child_kind(first, "identifier")
                        .map(|n| self.text(n).to_string())
                        .unwrap_or_default();
                    (n, String::new())
                }
                "select_expression" => {
                    // a.b.c: take last identifier as name, prefix as receiver.
                    let text = self.text(first);
                    if let Some(idx) = text.rfind('.') {
                        (text[idx + 1..].to_string(), text[..idx].to_string())
                    } else {
                        (text.to_string(), String::new())
                    }
                }
                _ => return,
            };
            if name.is_empty() {
                return;
            }

            // import is a built-in: surface as import record.
            if name == "import" && receiver.is_empty() {
                let target = node
                    .child(1)
                    .map(|n| self.text(n).trim().to_string())
                    .unwrap_or_default();
                if !target.is_empty() {
                    self.facts.imports.push(ImportRecord {
                        kind: "import".into(),
                        path: target
                            .trim_matches(|c: char| c == '<' || c == '>' || c == '"')
                            .to_string(),
                        alias: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
                return;
            }

            self.facts.references.push(RefRecord {
                name,
                receiver_hint: receiver,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
                ..Default::default()
            });
            return;
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
        p.set_language(&tree_sitter_nix::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        NixPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/x.nix"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn function_bindings_are_defs() {
        let src = "let greet = name: \"Hi ${name}\"; add = a: b: a + b; x = 1; in greet \"x\"\n";
        let f = extract(src);
        let names: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(names.contains(&"add"), "got: {names:?}");
        // `x = 1` is not a function — should NOT be recorded as a callable.
        assert!(!names.contains(&"x"), "got: {names:?}");
    }

    #[test]
    fn apply_is_a_call() {
        let src = "let f = x: x; in f 42\n";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "f"),
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn import_captured() {
        let src = "let pkgs = import <nixpkgs> {}; in pkgs\n";
        let f = extract(src);
        assert!(
            f.imports.iter().any(|i| i.path == "nixpkgs"),
            "imports: {:?}",
            f.imports
        );
    }
}
