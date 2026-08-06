//! HCL/Terraform plugin — dependency graph extraction.
//! HCL doesn't have traditional functions; instead we extract block labels
//! (resource, module, data) as definitions and function_call as references.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct HclPlugin;

impl LanguagePlugin for HclPlugin {
    fn id(&self) -> &'static str { "hcl" }
    fn extensions(&self) -> &'static [&'static str] { &[".tf", ".hcl", ".tfvars"] }
    fn shebangs(&self) -> &'static [&'static str] { &[] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_hcl::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "hcl");
        let mut w = HclWalker { source, facts: &mut facts };
        w.walk(tree.root_node());
        facts
    }
}

struct HclWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> HclWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "block" => {
                self.record_block(node);
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

    fn record_block(&mut self, node: Node) {
        // Extract block type and labels: resource "aws_s3_bucket" "my_bucket" -> "resource::aws_s3_bucket::my_bucket"
        let mut labels = Vec::new();
        let mut child = node.child(0);
        while let Some(c) = child {
            if c.kind() == "identifier" || c.kind() == "string_lit" {
                let text = self.text(c).trim_matches('"').to_string();
                if !text.is_empty() {
                    labels.push(text);
                }
            }
            child = c.next_sibling();
            if child.map(|ch| ch.kind() == "block").unwrap_or(false) { break; }
        }

        if labels.len() >= 2 {
            let qn = labels.join("::");
            let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
            self.facts.definitions.push(DefRecord {
                simple_name: labels.last().unwrap().clone(),
                qualified_name: qn,
                variant: DefVariant::FreeFunction,
                start_line: sl, end_line: el,
                start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(node)),
                visibility: String::new(), attributes: Vec::new(),
                ..Default::default()
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        // function_call has identifier as first child
        if let Some(func_node) = node.child(0) {
            if func_node.kind() == "identifier" {
                let func = self.text(func_node).to_string();
                if !func.is_empty() {
                    self.facts.references.push(RefRecord {
                        name: func, receiver_hint: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                        ..Default::default()
                    });
                }
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
        p.set_language(&tree_sitter_hcl::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        HclPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.tf"), &tree, src.as_bytes())
    }

    #[test]
    fn plugin_loads() {
        let plugin = HclPlugin;
        assert_eq!(plugin.id(), "hcl");
        assert!(plugin.extensions().contains(&".tf"));
    }

    #[test]
    fn extracts_blocks() {
        let src = "resource \"aws_s3_bucket\" \"my_bucket\" { bucket = \"my-bucket\" }\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract block definitions");
    }

    #[test]
    fn extracts_module_blocks() {
        let src = "module \"vpc\" { source = \"./modules/vpc\" }\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract module definitions");
    }
}
