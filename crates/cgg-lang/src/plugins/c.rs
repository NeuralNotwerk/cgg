//! C plugin — preprocessor-aware callable extraction.
//!
//! * `function_definition` → callable (DefVariant::FreeFunction).
//! * `declaration` with a `function_declarator` child → prototype
//!   (DefVariant::FreeFunction, zero-length body).
//! * `preproc_include` with a quoted path → ImportRecord kind="include".
//! * `call_expression` → RefRecord.
//! * Macro invocations that look like calls but have no matching def
//!   are resolved downstream as `reason: "macro-call-site"`.

use std::path::Path;

use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct CPlugin;

impl LanguagePlugin for CPlugin {
    fn id(&self) -> &'static str {
        "c"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".c", ".h"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            unreachable: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "c");
        let mut w = CWalker {
            source,
            facts: &mut facts,
        };
        w.walk(tree.root_node());
        let mut out = facts;
        if crate::deadcode_signals() {
            out.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::C_LIKE);
        }
        out
    }
}

struct CWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> CWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => {
                self.record_function_def(node);
                self.walk_children(node);
                return;
            }
            "declaration" => {
                // Prototype: `int foo(int a, int b);`
                self.try_record_prototype(node);
                self.walk_children(node);
                return;
            }
            "preproc_function_def" => {
                // #define FOO(x) ... — record as a callable so calls
                // to FOO resolve intra-file rather than being tagged
                // unresolved.
                self.record_macro_def(node);
                return;
            }
            "preproc_include" => {
                self.record_include(node);
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
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_function_def(&mut self, node: Node) {
        let Some(decl) = node.child_by_field_name("declarator") else {
            return;
        };
        let name = self.fn_name_from_declarator(decl);
        if name.is_empty() {
            return;
        }
        let (sl, el) = line_range(node);
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

    fn try_record_prototype(&mut self, node: Node) {
        // A declaration that contains a function_declarator is a prototype.
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if child.kind() == "function_declarator" {
                let name = self.fn_name_from_declarator(child);
                if !name.is_empty() {
                    let (sl, el) = line_range(node);
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
                return;
            }
        }
    }

    fn fn_name_from_declarator(&self, decl: Node) -> String {
        // function_declarator -> declarator: identifier | pointer_declarator -> ...
        if let Some(d) = decl.child_by_field_name("declarator") {
            match d.kind() {
                "identifier" => return self.text(d).to_string(),
                "pointer_declarator" => {
                    // *fn_ptr — dig for identifier
                    let mut cc = d.walk();
                    for c in d.children(&mut cc) {
                        if c.kind() == "identifier" {
                            return self.text(c).to_string();
                        }
                    }
                }
                "parenthesized_declarator" => {
                    let mut cc = d.walk();
                    for c in d.children(&mut cc) {
                        if c.kind() == "pointer_declarator" || c.kind() == "identifier" {
                            let mut inner = c.walk();
                            for ic in c.children(&mut inner) {
                                if ic.kind() == "identifier" {
                                    return self.text(ic).to_string();
                                }
                            }
                            if c.kind() == "identifier" {
                                return self.text(c).to_string();
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        String::new()
    }

    fn record_macro_def(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }
        let (sl, el) = line_range(node);
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
            attributes: vec!["macro".to_string()],
            ..Default::default()
        });
    }

    fn record_include(&mut self, node: Node) {
        let Some(path_node) = node.child_by_field_name("path") else {
            return;
        };
        let kind = path_node.kind();
        // Only project-local (quoted) includes become import records.
        if kind == "string_literal" || kind == "string_content" {
            let raw = self.text(path_node);
            let path = raw.trim_matches('"').to_string();
            if !path.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: "include".into(),
                    path,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
        }
        // system_lib_string (<stdio.h>) — skip for cross-file resolution.
    }

    fn record_call(&mut self, node: Node) {
        let Some(func) = node.child_by_field_name("function") else {
            return;
        };
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "field_expression" => {
                let arg = func
                    .child_by_field_name("argument")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let field = func
                    .child_by_field_name("field")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (field, arg)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }
}

fn line_range(n: Node) -> (u32, u32) {
    (
        (n.start_position().row as u32) + 1,
        (n.end_position().row as u32) + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        CPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.c"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn function_definitions_extracted() {
        let src = "int add(int a, int b) { return a+b; }\nvoid greet() {}\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"add"), "got: {names:?}");
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(
            f.definitions
                .iter()
                .all(|d| d.variant == DefVariant::FreeFunction)
        );
    }

    #[test]
    fn prototype_recorded() {
        let src = "int add(int a, int b);\nint add(int a, int b) { return a+b; }\n";
        let f = extract(src);
        // Both prototype and definition are recorded (dedup is Task 9).
        let count = f
            .definitions
            .iter()
            .filter(|d| d.simple_name == "add")
            .count();
        assert_eq!(count, 2);
    }

    #[test]
    fn include_directive_captured() {
        let src = "#include \"helpers.h\"\n#include <stdio.h>\nvoid f() {}\n";
        let f = extract(src);
        assert_eq!(f.imports.len(), 1);
        assert_eq!(f.imports[0].kind, "include");
        assert_eq!(f.imports[0].path, "helpers.h");
    }

    #[test]
    fn call_expressions_captured() {
        let src = "void f() { add(1,2); ptr->run(); }\n";
        let f = extract(src);
        let refs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(refs.contains(&("add", "")), "got: {refs:?}");
        // ptr->run() is a field_expression
        assert!(refs.contains(&("run", "ptr")), "got: {refs:?}");
    }

    #[test]
    fn macro_call_looks_like_call() {
        let src = "#define SQUARE(x) ((x)*(x))\nint f() { return SQUARE(3); }\n";
        let f = extract(src);
        // SQUARE(3) is parsed as a call_expression by tree-sitter-c.
        assert!(
            f.references.iter().any(|r| r.name == "SQUARE"),
            "macro call not captured"
        );
    }
}
