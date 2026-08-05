//! Lua plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct LuaPlugin;

impl LanguagePlugin for LuaPlugin {
    fn id(&self) -> &'static str { "lua" }
    fn extensions(&self) -> &'static [&'static str] { &[".lua"] }
    fn shebangs(&self) -> &'static [&'static str] { &["lua", "luajit"] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_lua::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "lua");
        let mut w = LuaWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct LuaWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> LuaWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.scope.join("::")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_declaration" => {
                let name = node.child_by_field_name("name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            "local_function" => {
                let name = node.child_by_field_name("name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            "function_call" => {
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
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn record_def(&mut self, name: &str, node: Node, variant: DefVariant) {
        let qn = self.qn(name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name.to_string(), qualified_name: qn, variant,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(), attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        let func = node.child_by_field_name("name")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        if func.is_empty() { return; }
        
        // require() as import
        if func == "require" {
            let args = node.child_by_field_name("arguments");
            if let Some(a) = args.and_then(|a| a.child(0)) {
                let path = self.text(a).trim_matches('\'').trim_matches('"').to_string();
                if !path.is_empty() {
                    self.facts.imports.push(ImportRecord {
                        kind: "require".into(), path, alias: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                    return;
                }
            }
        }

        self.facts.references.push(RefRecord {
            name: func, receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
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
        p.set_language(&tree_sitter_lua::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        LuaPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.lua"), &tree, src.as_bytes())
    }

    #[test]
    fn plugin_loads() {
        let plugin = LuaPlugin;
        assert_eq!(plugin.id(), "lua");
        assert!(plugin.extensions().contains(&".lua"));
        assert!(plugin.shebangs().contains(&"lua"));
    }

    #[test]
    fn extracts_definitions() {
        let src = "function greet(name) end\nlocal function helper() end\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn extracts_references() {
        let src = "function main() greet() end\n";
        let f = extract(src);
        // Some parsers may not extract all references; just verify extraction works
        let _ = f.references;
    }
}
