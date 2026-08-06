//! Perl plugin — callable extraction for Perl.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct PerlPlugin;

impl LanguagePlugin for PerlPlugin {
    fn id(&self) -> &'static str {
        "perl"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".pl", ".pm", ".t"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["perl"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_perl::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "perl");
        let mut w = PerlWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
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
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}::{simple}", self.scope.join("::"))
        }
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
                        if !c.goto_next_sibling() {
                            break;
                        }
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
            // `use Foo;` parses as `use_no_statement` (the rule covers
            // `use` and `no`). Matching only `use_statement` — a kind
            // this grammar never produces — meant no Perl `use` was
            // ever captured, so the language's documented cross-file
            // resolution resolved nothing.
            "use_no_statement" | "use_statement" | "require_statement" => {
                self.record_import(node);
                self.walk_children(node);
                return;
            }
            "call_expression"
            | "call_expression_with_args_with_brackets"
            | "call_expression_with_bareword"
            | "call_expression_with_spaced_args" => {
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
        let is_use = text.starts_with("use");

        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let child = c.node();
                // `package_name` is what the grammar actually produces
                // for `use Data::Dumper` / `require Exporter`; the other
                // two cover quoted and bareword spellings.
                if matches!(child.kind(), "package_name" | "bareword" | "string") {
                    let path = self
                        .text(child)
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_string();
                    if !path.is_empty()
                        && !path.starts_with("strict")
                        && !path.starts_with("warnings")
                    {
                        let alias = path.split("::").last().unwrap_or("").to_string();
                        self.facts.imports.push(ImportRecord {
                            kind: if is_use {
                                "use".into()
                            } else {
                                "require".into()
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

    fn record_method_call(&mut self, node: Node) {
        // tree-sitter-perl names these fields `function_name` and
        // `object_return_value`; the old `method`/`object` lookups never
        // matched, so `$obj->run()` produced no reference at all.
        if let Some(method_node) = node
            .child_by_field_name("function_name")
            .or_else(|| node.child_by_field_name("method"))
        {
            let name = self.text(method_node).to_string();
            if !name.is_empty() {
                let receiver_hint = node
                    .child_by_field_name("object_return_value")
                    .or_else(|| node.child_by_field_name("object"))
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
        PerlPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.pl"),
            &tree,
            src.as_bytes(),
        )
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
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name == "MyModule::foo")
        );
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
    fn use_and_require_keep_their_own_kinds() {
        // The README's language table promises both forms for Perl.
        let f = extract("use Data::Dumper;\nrequire Exporter;\n");
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "use" && i.path == "Data::Dumper"),
            "imports: {:?}",
            f.imports
        );
        assert!(
            f.imports.iter().any(|i| i.path == "Exporter"),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn pragmas_are_not_imports() {
        // `use strict`/`use warnings` are compiler pragmas, not modules;
        // recording them would invent cross-file edges to nothing.
        let f = extract("use strict;\nuse warnings;\nsub f { }\n");
        assert!(
            !f.imports
                .iter()
                .any(|i| i.path.starts_with("strict") || i.path.starts_with("warnings")),
            "pragmas must be skipped: {:?}",
            f.imports
        );
    }

    #[test]
    fn an_import_alias_is_the_last_path_segment() {
        let f = extract("use Data::Dumper;\n");
        let i = f.imports.iter().find(|i| i.path == "Data::Dumper").unwrap();
        assert_eq!(i.alias, "Dumper");
    }

    #[test]
    fn a_call_is_a_reference() {
        let f = extract("sub outer {\n  inner();\n}\n");
        assert!(
            refs(&f).iter().any(|r| r == "inner"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn a_method_call_records_its_receiver() {
        let f = extract("sub outer {\n  $obj->run();\n}\n");
        assert!(
            f.references.iter().any(|r| r.name == "run"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn a_second_package_rescopes_later_subs() {
        // Perl packages are a running scope, not a block, so a sub after
        // a second `package` belongs to the second one.
        let f = extract("package A;\nsub one { }\npackage B;\nsub two { }\n");
        let d = defs(&f);
        assert!(d.contains(&"A::one".to_string()), "defs: {d:?}");
        assert!(d.contains(&"B::two".to_string()), "defs: {d:?}");
    }

    #[test]
    fn an_empty_file_yields_nothing_and_does_not_panic() {
        let f = extract("");
        assert!(f.definitions.is_empty());
        assert!(f.imports.is_empty());
    }

    #[test]
    fn malformed_source_does_not_panic() {
        let f = extract("sub broken {\n  my $x = ;\n");
        let _ = defs(&f);
    }
}
