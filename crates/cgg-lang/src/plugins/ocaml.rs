//! OCaml plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct OcamlPlugin;

impl LanguagePlugin for OcamlPlugin {
    fn id(&self) -> &'static str {
        "ocaml"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".ml", ".mli"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_ocaml::LANGUAGE_OCAML.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "ocaml");
        let mut w = OcamlWalker {
            source,
            facts: &mut facts,
            module: String::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct OcamlWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
}

impl<'a> OcamlWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.module.is_empty() {
            simple.to_string()
        } else {
            format!("{}::{simple}", self.module)
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "module_definition" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            "open_statement" | "open_module" => {
                self.extract_import(node);
                self.walk_children(node);
            }
            "let_binding" | "value_definition" => {
                self.extract_function(node);
                self.walk_children(node);
            }
            "application_expression" => {
                self.record_call(node);
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
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

    fn extract_module(&mut self, node: Node) {
        // module Name = struct ... end
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && child.kind() == "module_name"
            {
                self.module = self.text(child).to_string();
                break;
            }
        }
    }

    fn extract_import(&mut self, node: Node) {
        // open Module
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && (child.kind() == "module_path" || child.kind() == "module_name")
            {
                let module_name = self.text(child).to_string();
                if !module_name.is_empty() {
                    self.facts.imports.push(ImportRecord {
                        kind: "open".to_string(),
                        path: module_name,
                        alias: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
                break;
            }
        }
    }

    fn extract_function(&mut self, node: Node) {
        // let name args = expr or let name : type = expr
        let mut name = String::new();

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && (child.kind() == "value_name" || child.kind() == "identifier")
            {
                name = self.text(child).to_string();
                break;
            }
        }

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
        // application: function applied to arguments
        if let Some(func_node) = node.child(0)
            && (func_node.kind() == "value_name"
                || func_node.kind() == "value_path"
                || func_node.kind() == "identifier")
        {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_ocaml::LANGUAGE_OCAML.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        OcamlPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.ml"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn plugin_loads() {
        let plugin = OcamlPlugin;
        assert_eq!(plugin.id(), "ocaml");
        assert!(plugin.extensions().contains(&".ml"));
        assert!(plugin.extensions().contains(&".mli"));
    }

    #[test]
    fn let_bindings_and_call() {
        // No enclosing `module M = struct ... end`, so `OcamlWalker::module`
        // stays empty and qualified names are the bare simple names.
        let src = "let helper x = x + 1\n\nlet main () = helper 41\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"helper"), "got: {names:?}");
        assert!(names.contains(&"main"), "got: {names:?}");
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"helper"), "got: {refs:?}");
    }

    #[test]
    fn open_is_recorded() {
        let f = extract("open Printf\n\nlet go () = ()\n");
        let paths: Vec<&str> = f.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"Printf"), "got: {paths:?}");
    }
}
