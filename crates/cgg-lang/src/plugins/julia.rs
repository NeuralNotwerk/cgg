//! Julia plugin — callable extraction for Julia.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct JuliaPlugin;

impl LanguagePlugin for JuliaPlugin {
    fn id(&self) -> &'static str { "julia" }
    fn extensions(&self) -> &'static [&'static str] { &[".jl"] }
    fn shebangs(&self) -> &'static [&'static str] { &["julia"] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_julia::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "julia");
        let mut w = JuliaWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct JuliaWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> JuliaWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}.{simple}", self.scope.join(".")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "module_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    if !name.is_empty() {
                        self.scope.push(name);
                        self.walk_children(node);
                        self.scope.pop();
                        return;
                    }
                }
                self.walk_children(node);
                return;
            }
            "function_definition" => {
                // In tree-sitter-julia, function_definition has a "signature" child
                // The signature text is like "greet(name)" - extract the name before the paren
                if let Some(sig_node) = node.children(&mut node.walk()).find(|c| c.kind() == "signature") {
                    let sig_text = self.text(sig_node);
                    if let Some(paren_pos) = sig_text.find('(') {
                        let name = sig_text[..paren_pos].trim().to_string();
                        if !name.is_empty() {
                            self.record_def(node, &name, DefVariant::FreeFunction);
                        }
                    }
                }
                self.walk_children(node);
                return;
            }
            "short_function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    if !name.is_empty() {
                        self.record_def(node, &name, DefVariant::FreeFunction);
                    }
                }
                self.walk_children(node);
                return;
            }
            "macro_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    if !name.is_empty() {
                        self.record_def(node, &name, DefVariant::FreeFunction);
                    }
                }
                self.walk_children(node);
                return;
            }
            "using_statement" | "import_statement" => {
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
                if !c.goto_next_sibling() { break; }
            }
        }
    }

    fn record_def(&mut self, node: Node, simple: &str, variant: DefVariant) {
        let qn = self.qn(simple);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: simple.to_string(),
            qualified_name: qn,
            variant,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_import(&mut self, node: Node) {
        let text = self.text(node);
        let is_using = text.starts_with("using");
        
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let child = c.node();
                if matches!(child.kind(), "identifier" | "dotted_identifier" | "scoped_identifier") {
                    let path = self.text(child).to_string();
                    if !path.is_empty() {
                        let alias = path.split('.').last().unwrap_or("").to_string();
                        self.facts.imports.push(ImportRecord {
                            kind: if is_using { "using".into() } else { "import".into() },
                            path,
                            alias,
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                if !c.goto_next_sibling() { break; }
            }
        }
    }

    fn record_call(&mut self, node: Node) {
        if let Some(func_node) = node.child(0) {
            let name = self.text(func_node).to_string();
            if !name.is_empty() {
                self.facts.references.push(RefRecord {
                    name,
                    receiver_hint: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
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
        p.set_language(&tree_sitter_julia::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        JuliaPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/X.jl"), &tree, src.as_bytes())
    }

    #[test]
    fn function_definition_captured() {
        let src = "function greet(name)\n  println(\"Hello, $name\")\nend\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "greet"));
    }
}
