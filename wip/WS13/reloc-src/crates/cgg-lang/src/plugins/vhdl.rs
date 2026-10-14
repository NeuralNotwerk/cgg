//! VHDL plugin — callable extraction.
//!
//! Definitions: `entity_declaration` (interface), `architecture_definition`
//! (implementation tied to an entity), and `subprogram_definition`
//! (procedures and functions). Imports come from `library_clause` and
//! `use_clause`. Call sites for subprograms are not always cleanly
//! distinguishable in this grammar — we capture them best-effort via
//! `function_call_expression` if present.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct VhdlPlugin;

impl LanguagePlugin for VhdlPlugin {
    fn id(&self) -> &'static str {
        "vhdl"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".vhd", ".vhdl"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_vhdl::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "vhdl");
        let mut w = VhdlWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct VhdlWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> VhdlWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.into()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
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
            "entity_declaration" => {
                self.record_named(node, "entity", DefVariant::FreeFunction);
                self.walk_children(node);
                return;
            }
            "architecture_definition" => {
                // architecture_definition: keyword 'architecture' identifier 'of' name(entity) ...
                let name = self
                    .child_kind(node, "identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    let (sl, el) = (
                        (node.start_position().row as u32) + 1,
                        (node.end_position().row as u32) + 1,
                    );
                    self.facts.definitions.push(DefRecord {
                        simple_name: name.clone(),
                        qualified_name: self.qn(&name),
                        variant: DefVariant::FreeFunction,
                        start_line: sl,
                        end_line: el,
                        start_byte: node.start_byte() as u32,
                        end_byte: node.end_byte() as u32,
                        signature_hint: super::extract_signature(self.text(node)),
                        visibility: String::new(),
                        attributes: vec!["architecture".into()],
                        ..Default::default()
                    });
                    // Architecture binds to an entity — record that as a reference.
                    if let Some(of_name) = self.child_kind(node, "name") {
                        let entity = self.text(of_name).to_string();
                        if !entity.is_empty() {
                            self.facts.references.push(RefRecord {
                                name: entity,
                                receiver_hint: String::new(),
                                site_line: (node.start_position().row as u32) + 1,
                                site_byte: node.start_byte() as u32,
                                ..Default::default()
                            });
                        }
                    }
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                    return;
                }
            }
            "subprogram_definition" => {
                self.record_subprogram(node);
                self.walk_children(node);
                return;
            }
            "library_clause" => {
                if let Some(list) = self.child_kind(node, "logical_name_list") {
                    let names: Vec<String> = self
                        .text(list)
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                    for nm in names {
                        self.facts.imports.push(ImportRecord {
                            kind: "library".into(),
                            path: nm,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                return;
            }
            "use_clause" => {
                if let Some(list) = self.child_kind(node, "selected_name_list") {
                    let text = self.text(list).trim().to_string();
                    if !text.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "use".into(),
                            path: text,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
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

    fn record_named(&mut self, node: Node, tag: &str, variant: DefVariant) {
        let name = self
            .child_kind(node, "identifier")
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
            qualified_name: self.qn(&name),
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: vec![tag.into()],
            ..Default::default()
        });
    }

    fn record_subprogram(&mut self, node: Node) {
        // procedure_specification or function_specification
        let spec = self
            .child_kind(node, "procedure_specification")
            .or_else(|| self.child_kind(node, "function_specification"));
        let Some(spec) = spec else { return };
        let name = self
            .child_kind(spec, "identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let is_function = spec.kind() == "function_specification";
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name.clone(),
            qualified_name: self.qn(&name),
            variant: if self.scope.is_empty() {
                DefVariant::FreeFunction
            } else {
                DefVariant::InherentMethod
            },
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: vec![if is_function {
                "function".into()
            } else {
                "procedure".into()
            }],
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
        p.set_language(&tree_sitter_vhdl::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        VhdlPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/x.vhd"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn entity_and_architecture() {
        let src = "entity counter is end entity;\narchitecture rtl of counter is begin end architecture;\n";
        let f = extract(src);
        let names: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"counter"), "got: {names:?}");
        assert!(names.contains(&"rtl"), "got: {names:?}");
        assert!(
            f.references.iter().any(|r| r.name == "counter"),
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn function_and_procedure() {
        let src = "architecture rtl of foo is\n  function inc(a : integer) return integer is begin return a; end function;\n  procedure tick(signal s : inout bit) is begin s <= s; end procedure;\nbegin end architecture;\n";
        let f = extract(src);
        let names: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"inc"), "got: {names:?}");
        assert!(names.contains(&"tick"), "got: {names:?}");
    }

    #[test]
    fn library_and_use_are_imports() {
        let src = "library ieee;\nuse ieee.std_logic_1164.all;\n";
        let f = extract(src);
        assert!(
            f.imports
                .iter()
                .any(|i| i.path == "ieee" && i.kind == "library")
        );
        assert!(
            f.imports
                .iter()
                .any(|i| i.path.contains("std_logic_1164") && i.kind == "use")
        );
    }
}
