//! Elixir plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct ElixirPlugin;

impl LanguagePlugin for ElixirPlugin {
    fn id(&self) -> &'static str { "elixir" }
    fn extensions(&self) -> &'static [&'static str] { &[".ex", ".exs"] }
    fn shebangs(&self) -> &'static [&'static str] { &["elixir"] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_elixir::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "elixir");
        let mut w = ElixirWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct ElixirWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> ElixirWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.scope.join("::")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "call" => {
                // Get the first child which should be the function name
                let func_name = node.child(0).map(|c| self.text(c).to_string()).unwrap_or_default();
                
                match func_name.as_str() {
                    "defmodule" => {
                        if let Some(module_name) = self.extract_defmodule_name(node) {
                            self.scope.push(module_name);
                            self.walk_children(node);
                            self.scope.pop();
                        } else {
                            self.walk_children(node);
                        }
                        return;
                    }
                    "def" | "defp" | "defmacro" => {
                        self.record_function(node);
                        self.walk_children(node);
                        return;
                    }
                    "alias" | "import" | "use" | "require" => {
                        self.record_import(node, &func_name);
                        self.walk_children(node);
                        return;
                    }
                    _ => {
                        self.record_call(node);
                        self.walk_children(node);
                        return;
                    }
                }
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn extract_defmodule_name(&self, node: Node) -> Option<String> {
        // defmodule name do ... end
        // The name is typically the second child (after "defmodule")
        node.child(1).map(|n| self.text(n).to_string()).filter(|s| !s.is_empty())
    }

    fn record_function(&mut self, node: Node) {
        // def name(...) do ... end
        // The name is typically the second child (after "def")
        if let Some(name_node) = node.child(1) {
            let head_text = self.text(name_node);
            // Extract function name from head (e.g., "foo" or "foo(a, b)")
            let name = head_text.split('(').next().unwrap_or("").trim().to_string();
            if name.is_empty() { return; }
            
            let qn = self.qn(&name);
            let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
            self.facts.definitions.push(DefRecord {
                simple_name: name, qualified_name: qn, variant: DefVariant::FreeFunction,
                start_line: sl, end_line: el,
                start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(node)),
                visibility: String::new(), attributes: Vec::new(),
            });
        }
    }

    fn record_import(&mut self, node: Node, import_type: &str) {
        // alias/import/use/require Module
        // The module is typically the second child
        if let Some(module_node) = node.child(1) {
            let path = self.text(module_node).trim_matches(|c| c == '\'' || c == '"').to_string();
            if !path.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: import_type.to_string(),
                    path,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
        }
    }

    fn record_call(&mut self, node: Node) {
        if let Some(func_node) = node.child(0) {
            let name = self.text(func_node).to_string();
            if name.is_empty() || name.starts_with("def") { return; }
            
            let receiver_hint = if name.contains('.') {
                let parts: Vec<&str> = name.split('.').collect();
                if parts.len() == 2 {
                    parts[0].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            
            self.facts.references.push(RefRecord {
                name,
                receiver_hint,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
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
        p.set_language(&tree_sitter_elixir::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ElixirPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.ex"), &tree, src.as_bytes())
    }

    #[test]
    fn defmodule_and_functions() {
        let src = "defmodule MyModule do\n  def greet(name) do\n    IO.puts(\"Hello, #{name}\")\n  end\nend\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "Expected definitions, got none");
    }
}
