//! Fortran plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct FortranPlugin;

impl LanguagePlugin for FortranPlugin {
    fn id(&self) -> &'static str { "fortran" }
    fn extensions(&self) -> &'static [&'static str] { &[".f90", ".f95", ".f03", ".f08", ".f", ".for", ".fpp"] }
    fn shebangs(&self) -> &'static [&'static str] { &[] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_fortran::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "fortran");
        let mut w = FortranWalker { source, facts: &mut facts, module: String::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct FortranWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
}

impl<'a> FortranWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.module.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.module) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "module_statement" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            "use_statement" => {
                self.record_import(node);
                self.walk_children(node);
            }
            "function_statement" | "subroutine_statement" | "program_statement" => {
                self.record_callable(node);
                self.walk_children(node);
            }
            "call_expression" | "subroutine_call" => {
                self.record_call(node);
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn extract_module(&mut self, node: Node) {
        // module name ... end module
        // Find the name child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                if child.kind() == "name" {
                    self.module = self.text(child).to_string();
                    break;
                }
            }
        }
    }

    fn record_import(&mut self, node: Node) {
        // use module_name
        // Find the name child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                if child.kind() == "name" {
                    let module_name = self.text(child).to_string();
                    self.facts.imports.push(ImportRecord {
                        kind: "use".to_string(),
                        path: module_name,
                        alias: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                    break;
                }
            }
        }
    }

    fn record_callable(&mut self, node: Node) {
        // function/subroutine/program name(...) ... end
        // Find the name child
        let mut name = String::new();
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                if child.kind() == "name" {
                    name = self.text(child).to_string();
                    break;
                }
            }
        }
        if name.is_empty() { return; }

        let qn = self.qn(&name);
        let variant = DefVariant::FreeFunction;
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
        });
    }

    fn record_call(&mut self, node: Node) {
        // call_expression has "function" field, subroutine_call has "subroutine" field
        let func_node = node.child_by_field_name("function")
            .or_else(|| node.child_by_field_name("subroutine"));
        
        if let Some(fn_node) = func_node {
            let name = self.text(fn_node).to_string();
            if name.is_empty() { return; }
            
            self.facts.references.push(RefRecord {
                name,
                receiver_hint: String::new(),
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
        p.set_language(&tree_sitter_fortran::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        FortranPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.f90"), &tree, src.as_bytes())
    }

    #[test]
    fn module_and_functions() {
        let src = "module math_utils\ncontains\nfunction add(a, b) result(c)\nreal :: a, b, c\nc = a + b\nend function add\nend module math_utils\n";
        let f = extract(src);
        eprintln!("Definitions found: {}", f.definitions.len());
        for def in &f.definitions {
            eprintln!("  - {}: {}", def.simple_name, def.qualified_name);
        }
        assert!(!f.definitions.is_empty(), "Expected definitions, got none");
        assert_eq!(f.definitions[0].simple_name, "add");
    }
}
