//! Fortran plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct FortranPlugin;

impl LanguagePlugin for FortranPlugin {
    fn id(&self) -> &'static str {
        "fortran"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".f90", ".f95", ".f03", ".f08", ".f", ".for", ".fpp"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_fortran::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "fortran");
        let mut w = FortranWalker {
            source,
            facts: &mut facts,
            module: String::new(),
        };
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
            "module_statement" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            "use_statement" => {
                self.record_import(node);
                self.walk_children(node);
            }
            // Match the enclosing form so the byte range covers the
            // body, not just the header line — needed for intra-file
            // edges to attribute call sites to their containing routine.
            "function" | "subroutine" | "program" => {
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
        // module name ... end module
        // Find the name child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && child.kind() == "name" {
                    self.module = self.text(child).to_string();
                    break;
                }
        }
    }

    fn record_import(&mut self, node: Node) {
        // use module_name
        // Find the name child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && matches!(child.kind(), "name" | "module_name") {
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

    fn record_callable(&mut self, node: Node) {
        // The header (`subroutine_statement` / `function_statement` /
        // `program_statement`) is a direct child of the enclosing form;
        // the routine's name lives inside that header.
        let header_kind = match node.kind() {
            "subroutine" => "subroutine_statement",
            "function" => "function_statement",
            "program" => "program_statement",
            _ => return,
        };
        let header = node
            .children(&mut node.walk())
            .find(|c| c.kind() == header_kind);
        let Some(header) = header else { return };
        let name = header
            .children(&mut header.walk())
            .find(|c| c.kind() == "name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }

        let qn = self.qn(&name);
        let variant = DefVariant::FreeFunction;
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
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
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        // tree-sitter-fortran doesn't expose field names on call_expression /
        // subroutine_call. The callee is the first named `identifier` child.
        let mut c = node.walk();
        if !c.goto_first_child() {
            return;
        }
        loop {
            let n = c.node();
            if n.kind() == "identifier" {
                let name = self.text(n).to_string();
                if !name.is_empty() {
                    self.facts.references.push(RefRecord {
                        name,
                        receiver_hint: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                        ..Default::default()
                    });
                }
                return;
            }
            if !c.goto_next_sibling() {
                return;
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
        p.set_language(&tree_sitter_fortran::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        FortranPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.f90"),
            &tree,
            src.as_bytes(),
        )
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
