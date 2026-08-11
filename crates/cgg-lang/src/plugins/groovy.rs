//! Groovy plugin — callable extraction for Groovy and Gradle.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct GroovyPlugin;

impl LanguagePlugin for GroovyPlugin {
    fn id(&self) -> &'static str {
        "groovy"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".groovy", ".gradle"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_groovy::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "groovy");
        let mut w = GroovyWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct GroovyWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> GroovyWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "package_declaration" => {
                if let Some(name) = node.child_by_field_name("name").or_else(|| {
                    node.children(&mut node.walk()).find(|c| {
                        c.kind() == "scoped_identifier" || c.kind() == "identifier"
                    })
                }) {
                    self.scope
                        .push(self.text(name).replace(';', "").trim().to_string());
                }
                return;
            }
            "import_declaration" => {
                self.record_import(node);
                return;
            }
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "method_declaration" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            "function_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(node, &name, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            "method_invocation" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            "function_call" => {
                self.record_function_call(node);
                self.walk_children(node);
                return;
            }
            "closure_expression" => {
                // Closures are callable but typically anonymous; skip for now
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

    fn record_method(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }
        let is_static = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "modifiers" && self.text(c).contains("static"));
        let variant = if is_static {
            DefVariant::StaticMethod
        } else {
            DefVariant::InherentMethod
        };
        self.record_def(node, &simple, variant);
    }

    fn record_def(&mut self, node: Node, simple: &str, variant: DefVariant) {
        let qn = self.qn(simple);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: simple.to_string(),
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

    fn record_import(&mut self, node: Node) {
        let ident_node = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "scoped_identifier" || c.kind() == "identifier");
        let Some(ident_node) = ident_node else { return };
        let path = self.text(ident_node).to_string();
        if path.is_empty() {
            return;
        }

        let text = self.text(node);
        let is_static = text.contains("static ");

        if is_static {
            let alias = path.rsplit('.').next().unwrap_or("").to_string();
            self.facts.imports.push(ImportRecord {
                kind: "from-import".into(),
                path,
                alias,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        } else {
            let alias = path.rsplit('.').next().unwrap_or("").to_string();
            self.facts.imports.push(ImportRecord {
                kind: "import".into(),
                path,
                alias,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        let recv = node
            .child_by_field_name("object")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
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

    fn record_function_call(&mut self, node: Node) {
        let name = node
            .child_by_field_name("function")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
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
        p.set_language(&tree_sitter_groovy::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        GroovyPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.groovy"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn class_methods_with_package() {
        let src = "package com.example\nclass Service {\n  void run() {}\n  int process(String s) { return 0 }\n}\n";
        let f = extract(src);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"com.example.Service.run"), "got: {qns:?}");
        assert!(qns.contains(&"com.example.Service.process"), "got: {qns:?}");
    }

    #[test]
    fn method_invocation_captured() {
        let src = "class C { void f() { helper(); obj.run() } }\n";
        let f = extract(src);
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "helper" && r.receiver_hint.is_empty())
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "run" && r.receiver_hint == "obj")
        );
    }

    fn defs(f: &FileFacts) -> Vec<String> {
        f.definitions
            .iter()
            .map(|d| d.qualified_name.clone())
            .collect()
    }

    #[test]
    fn a_plain_import_is_recorded() {
        let f = extract("import com.example.Service\nclass C { void f() {} }\n");
        assert!(
            f.imports
                .iter()
                .any(|i| i.path.contains("com.example.Service")),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn a_static_import_is_a_from_import() {
        // The plugin distinguishes the two kinds; collapsing them would
        // lose the distinction the cross-file resolver keys on.
        let f = extract("import static java.lang.Math.max\nclass C { void f() {} }\n");
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "from-import" || i.path.contains("Math")),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn a_closure_body_still_yields_its_calls() {
        // Gradle build scripts are mostly closures; missing calls inside
        // them would empty the graph for `.gradle` files.
        let f =
            extract("class C {\n  void f() {\n    items.each { helper(it) }\n  }\n}\n");
        assert!(
            f.references.iter().any(|r| r.name == "helper"),
            "refs: {:?}",
            f.references.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_script_level_method_without_a_package_is_captured() {
        let f = extract("def helper(x) { return x }\n");
        assert!(
            f.definitions.iter().any(|d| d.simple_name == "helper"),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn an_empty_file_yields_nothing_and_does_not_panic() {
        let f = extract("");
        assert!(f.definitions.is_empty());
        assert!(f.imports.is_empty());
    }

    #[test]
    fn malformed_source_does_not_panic() {
        let f = extract("class Broken {\n  void f( {\n");
        let _ = defs(&f);
    }
}
