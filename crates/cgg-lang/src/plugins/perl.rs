//! Perl plugin — callable extraction for Perl.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct PerlPlugin;

impl LanguagePlugin for PerlPlugin {
    fn id(&self) -> &'static str { "perl" }
    fn extensions(&self) -> &'static [&'static str] { &[".pl", ".pm", ".t"] }
    fn shebangs(&self) -> &'static [&'static str] { &["perl"] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_perl::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "perl");
        let mut w = PerlWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct PerlWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> PerlWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.scope.join("::")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "package_statement" => {
                // package_statement has package_name as a child (not a field)
                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        let child = c.node();
                        if child.kind() == "package_name" {
                            let name = self.text(child).to_string();
                            if !name.is_empty() {
                                self.scope.clear();
                                self.scope.push(name);
                            }
                            break;
                        }
                        if !c.goto_next_sibling() { break; }
                    }
                }
                self.walk_children(node);
                return;
            }
            "function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    if !name.is_empty() {
                        self.record_def(node, &name, DefVariant::FreeFunction);
                    }
                }
                self.walk_children(node);
                return;
            }
            "use_statement" | "require_statement" => {
                self.record_import(node);
                self.walk_children(node);
                return;
            }
            "call_expression" | "call_expression_with_args_with_brackets"
            | "call_expression_with_bareword" | "call_expression_with_spaced_args" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            "method_call" | "method_invocation" => {
                self.record_method_call(node);
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
        let is_use = text.starts_with("use");
        
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let child = c.node();
                if child.kind() == "bareword" || child.kind() == "string" {
                    let path = self.text(child).trim_matches(|c| c == '\'' || c == '"').to_string();
                    if !path.is_empty() && !path.starts_with("strict") && !path.starts_with("warnings") {
                        let alias = path.split("::").last().unwrap_or("").to_string();
                        self.facts.imports.push(ImportRecord {
                            kind: if is_use { "use".into() } else { "require".into() },
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
                    ..Default::default()
                });
            }
        }
    }

    fn record_method_call(&mut self, node: Node) {
        if let Some(method_node) = node.child_by_field_name("method") {
            let name = self.text(method_node).to_string();
            if !name.is_empty() {
                let receiver_hint = node.child_by_field_name("object")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                self.facts.references.push(RefRecord {
                    name,
                    receiver_hint,
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                    ..Default::default()
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
        p.set_language(&tree_sitter_perl::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        PerlPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/X.pl"), &tree, src.as_bytes())
    }

    #[test]
    fn subroutine_captured() {
        let src = "sub greet {\n  my $name = shift;\n  print \"Hello, $name\\n\";\n}\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "greet"));
    }

    #[test]
    fn package_scope() {
        let src = "package MyModule;\nsub foo { }\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.qualified_name == "MyModule::foo"));
    }
}
