//! Starlark plugin — callable extraction. Starlark is the language
//! used in Bazel/Buck/Pants `BUILD` and `.bzl` files. Syntactically a
//! Python subset, so the walker tracks `def`, `call`, and the
//! `load("//path:file.bzl", "symbol")` directive.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct StarlarkPlugin;

impl LanguagePlugin for StarlarkPlugin {
    fn id(&self) -> &'static str {
        "starlark"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".bzl", ".star", ".bazel"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_starlark::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "starlark");
        let mut w = StarlarkWalker {
            source,
            facts: &mut facts,
        };
        w.walk(tree.root_node());
        facts
    }
}

struct StarlarkWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> StarlarkWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "call" => {
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
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
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
        let Some(callee) = node.child(0) else { return };
        let (name, receiver) = match callee.kind() {
            "identifier" => (self.text(callee).to_string(), String::new()),
            "attribute" => {
                // attribute: object . identifier
                let object = callee
                    .child(0)
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let last = callee
                    .children(&mut callee.walk())
                    .filter(|c| c.kind() == "identifier")
                    .last();
                let name = last.map(|n| self.text(n).to_string()).unwrap_or_default();
                (name, object)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }

        // `load("//path:file.bzl", "symbol", ...)` — record import for the bzl path.
        if name == "load" && receiver.is_empty() {
            if let Some(args) = node.child(1)
                && let Some(first_str) = args
                    .children(&mut args.walk())
                    .find(|c| c.kind() == "string")
                {
                    let raw = self.text(first_str);
                    let path = raw
                        .trim_matches(|c: char| c == '"' || c == '\'')
                        .to_string();
                    if !path.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "load".into(),
                            path,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
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
        p.set_language(&tree_sitter_starlark::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        StarlarkPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/x.bzl"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn def_and_call() {
        let src =
            "def my_macro(name):\n    java_helper(name = name)\nmy_macro(name = 'foo')\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "my_macro"));
        assert!(f.references.iter().any(|r| r.name == "java_helper"));
        assert!(f.references.iter().any(|r| r.name == "my_macro"));
    }

    #[test]
    fn load_captured_as_import() {
        let src = "load(\"//common:macros.bzl\", \"java_helper\")\n";
        let f = extract(src);
        assert!(
            f.imports
                .iter()
                .any(|i| i.path == "//common:macros.bzl" && i.kind == "load"),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn attribute_call() {
        let src = "native.cc_library(name = 'x')\n";
        let f = extract(src);
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "cc_library" && r.receiver_hint == "native"),
            "refs: {:?}",
            f.references
        );
    }
}
