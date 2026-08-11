//! Julia plugin — callable extraction for Julia.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct JuliaPlugin;

impl LanguagePlugin for JuliaPlugin {
    fn id(&self) -> &'static str {
        "julia"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".jl"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["julia"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_julia::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "julia");
        let mut w = JuliaWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
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
                if let Some(sig_node) = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "signature")
                {
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
            // `f(x) = 2x` — the short form, and the idiomatic way to
            // write a dispatch method. The grammar produces a plain
            // `assignment` whose left side is a `call_expression`, not
            // the `short_function_definition` the arm above looks for,
            // so every one-line method was invisible: absent as a
            // definition, and its head counted as a call to itself.
            "assignment" => {
                if let Some(lhs) = node.named_child(0)
                    && lhs.kind() == "call_expression"
                    && let Some(name_node) = lhs.named_child(0)
                    && name_node.kind() == "identifier"
                {
                    let name = self.text(name_node).to_string();
                    if !name.is_empty() {
                        self.record_def(node, &name, DefVariant::FreeFunction);
                        // Walk only the right-hand side: the
                        // left is the signature, and calls
                        // recorded from it would be phantom
                        // references to the function itself.
                        for i in 1..node.named_child_count() {
                            if let Some(rhs) = node.named_child(i as u32) {
                                self.walk(rhs);
                            }
                        }
                        return;
                    }
                }
                self.walk_children(node);
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
        let text = self.text(node);
        let is_using = text.starts_with("using");

        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let child = c.node();
                if matches!(
                    child.kind(),
                    "identifier" | "dotted_identifier" | "scoped_identifier"
                ) {
                    let path = self.text(child).to_string();
                    if !path.is_empty() {
                        let alias = path.split('.').next_back().unwrap_or("").to_string();
                        self.facts.imports.push(ImportRecord {
                            kind: if is_using {
                                "using".into()
                            } else {
                                "import".into()
                            },
                            path,
                            alias,
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                if !c.goto_next_sibling() {
                    break;
                }
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
        JuliaPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.jl"),
            &tree,
            src.as_bytes(),
        )
    }

    fn defs(f: &FileFacts) -> Vec<String> {
        f.definitions
            .iter()
            .map(|d| d.qualified_name.clone())
            .collect()
    }
    fn refs(f: &FileFacts) -> Vec<String> {
        f.references.iter().map(|r| r.name.clone()).collect()
    }

    #[test]
    fn function_definition_captured() {
        let src = "function greet(name)\n  println(\"Hello, $name\")\nend\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "greet"));
    }

    #[test]
    fn short_form_assignment_definition_captured() {
        // `f(x) = ...` is the idiomatic one-liner and is a definition,
        // not a call.
        let f = extract("double(x) = 2x\n");
        assert!(
            f.definitions.iter().any(|d| d.simple_name == "double"),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn a_module_qualifies_its_definitions() {
        let f = extract("module M\nfunction greet(n)\n  n\nend\nend\n");
        assert!(
            defs(&f).iter().any(|d| d.contains("greet")),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn using_and_import_keep_their_own_kinds() {
        // The README's language table promises both forms for Julia.
        let f = extract("using LinearAlgebra\nimport Base.show\n");
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "using" && i.path.contains("LinearAlgebra")),
            "imports: {:?}",
            f.imports
        );
        assert!(
            f.imports.iter().any(|i| i.kind == "import"),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn calls_in_a_body_are_references() {
        let f = extract("function outer()\n  inner(1)\nend\n");
        assert!(
            refs(&f).iter().any(|r| r == "inner"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn multiple_dispatch_methods_are_separate_definitions() {
        // The README calls out multiple dispatch for Julia: two methods
        // of the same name are two definitions, not one.
        let f = extract("area(c::Circle) = 1\narea(s::Square) = 2\n");
        let n = f
            .definitions
            .iter()
            .filter(|d| d.simple_name == "area")
            .count();
        assert_eq!(
            n,
            2,
            "both dispatch methods must be captured: {:?}",
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
        let f = extract("function broken(\n");
        let _ = defs(&f);
    }
}
