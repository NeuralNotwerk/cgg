//! Scala plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ScalaPlugin;

impl LanguagePlugin for ScalaPlugin {
    fn id(&self) -> &'static str {
        "scala"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".scala", ".sc"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["scala"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_scala::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "scala");
        let mut w = ScalaWalker {
            bases: Vec::new(),
            source,
            facts: &mut facts,
            scope: Vec::new(),
            package: String::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct ScalaWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Supertypes of the enclosing class, innermost last.
    ///
    /// Recorded from 0.6.8. Without it every `base_types` rule for this
    /// language was inert — the contract is identical to Java's and
    /// matched nothing.
    bases: Vec<Vec<String>>,
    /// The package in effect (`package a.b` clauses, chained or block).
    package: String,
}

impl<'a> ScalaWalker<'a> {
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
            "package_clause" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).split_whitespace().collect::<String>())
                    .unwrap_or_default();
                let joined = if self.package.is_empty() {
                    name
                } else {
                    format!("{}.{name}", self.package)
                };
                match node.child_by_field_name("body") {
                    // `package a { … }`: in effect inside the block only.
                    Some(_) => {
                        let outer = std::mem::replace(&mut self.package, joined);
                        self.walk_children(node);
                        self.package = outer;
                    }
                    // `package a.b` then `package c`: the rest of the file.
                    None => {
                        self.facts.imports.push(ImportRecord {
                            kind: "scala-package".into(),
                            path: joined.clone(),
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                        self.package = joined;
                    }
                }
                return;
            }
            // `new T { … }`: an anonymous class extending `T`; its members
            // see `T`'s (WS14b).
            "instance_expression" => {
                let mut c = node.walk();
                let has_body = node.children(&mut c).any(|n| n.kind() == "template_body");
                if has_body {
                    let mut c = node.walk();
                    let bases: Vec<String> = node
                        .named_children(&mut c)
                        .filter(|n| {
                            n.kind() != "template_body" && n.kind() != "arguments"
                        })
                        .map(|n| self.text(n).to_string())
                        .collect();
                    self.facts.type_spans.push(cgg_core::TypeSpan {
                        name: String::new(),
                        start_byte: node.start_byte() as u32,
                        end_byte: node.end_byte() as u32,
                        kind: "anon".into(),
                        bases,
                        package: self.package.clone(),
                        ..Default::default()
                    });
                }
                self.walk_children(node);
                return;
            }
            "class_definition" | "object_definition" | "trait_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let class_bases = super::attrs::base_types(node, self.source);
                if !name.is_empty() {
                    // The type by its path in the package (`Outer.Inner`),
                    // with its kind and bases, so a member lookup can walk
                    // the bases the declaring file's imports name (WS14b).
                    let mut path = self.scope.join(".");
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(&name);
                    self.facts.type_spans.push(cgg_core::TypeSpan {
                        name: path,
                        start_byte: node.start_byte() as u32,
                        end_byte: node.end_byte() as u32,
                        kind: node.kind().trim_end_matches("_definition").to_string(),
                        bases: class_bases.clone(),
                        package: self.package.clone(),
                        ..Default::default()
                    });
                }
                if !name.is_empty() {
                    self.scope.push(name);
                    self.bases.push(class_bases.clone());
                    self.walk_children(node);
                    self.bases.pop();
                    self.scope.pop();
                } else {
                    self.bases.push(class_bases.clone());
                    self.walk_children(node);
                    self.bases.pop();
                }
                return;
            }
            "function_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            // `def area: Double` with no body: an abstract member of a
            // trait or abstract class. Recorded so a call typed to the
            // trait binds to the declaration its implementations are
            // dispatched from; tagged `abstract` so duck-typed fan-out
            // prefers an implementation when one survives.
            "function_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() && !self.scope.is_empty() {
                    self.record_def(&name, node, DefVariant::TraitMethod);
                    if let Some(d) = self.facts.definitions.last_mut() {
                        d.attributes.push("abstract".to_string());
                    }
                }
                return;
            }
            "import_declaration" => {
                self.record_import(node);
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

    fn record_def(&mut self, name: &str, node: Node, variant: DefVariant) {
        let qn = self.qn(name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        let bases = self.bases.last().cloned().unwrap_or_default();
        self.facts.definitions.push(DefRecord {
            base_types: bases,
            simple_name: name.to_string(),
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // Annotations are `@FunctionName("X")` here exactly as in
            // Java, and the shared collector reads them the same way.
            // Recorded from 0.6.8; before that no attribute rule could
            // match this language at all.
            attributes: super::attrs::collect_with_preceding(node, self.source),
            ..Default::default()
        });
    }

    fn record_import(&mut self, node: Node) {
        // tree-sitter-scala flattens the import: `import a.b.{X,Y}` =>
        // `import` `a` `.` `b` `.` `namespace_selectors{X,Y}`. The
        // simplest reliable reconstruction is the node's own text with
        // the leading "import" stripped and whitespace collapsed.
        let raw = self.text(node);
        let path = raw
            .trim_start_matches("import")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("");
        if path.is_empty() {
            return;
        }
        self.facts.imports.push(ImportRecord {
            kind: "import".into(),
            path,
            alias: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_call(&mut self, node: Node) {
        let func = node
            .child_by_field_name("function")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if func.is_empty() {
            return;
        }
        // `this(…)` in an auxiliary constructor delegates to a constructor
        // of the enclosing class (the primary one, or another `def this`
        // the arguments select): a construction of that class, never a
        // by-name match of the `this` members (WS14b).
        let func = if func == "this" {
            match self.scope.last() {
                Some(class) => class.clone(),
                None => return,
            }
        } else {
            func
        };

        self.facts.references.push(RefRecord {
            name: func,
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
        p.set_language(&tree_sitter_scala::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        ScalaPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.scala"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn plugin_loads() {
        let plugin = ScalaPlugin;
        assert_eq!(plugin.id(), "scala");
        assert!(plugin.extensions().contains(&".scala"));
        assert!(plugin.shebangs().contains(&"scala"));
    }

    #[test]
    fn extracts_definitions() {
        let src = "class Service { def run() {} }\nobject Main { def main() {} }\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn extracts_references() {
        let src = "def main() { greet() }\n";
        let f = extract(src);
        assert!(!f.references.is_empty(), "should extract references");
    }

    /// WS14: a trait is a scope like a class: its methods are its own.
    #[test]
    fn trait_methods_are_owned_by_the_trait() {
        let o =
            extract("object Errs extends QueryBase with Compilation { def f() = 1 }\n");
        assert_eq!(
            o.definitions[0].base_types,
            vec!["QueryBase".to_string(), "Compilation".to_string()]
        );
        let f = extract("trait Logging { def logInfo(m: String): Unit = () }\n");
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name == "Logging::logInfo"),
            "{:?}",
            f.definitions
        );
    }

    #[test]
    fn abstract_defs_are_bodiless_declarations() {
        let f = extract("trait Shape { def area(): Double }\ndef top(): Int\n");
        let d = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Shape::area")
            .expect("abstract member");
        assert_eq!(d.variant, DefVariant::TraitMethod);
        assert!(d.attributes.iter().any(|a| a == "abstract"));
        // Outside any type there is nothing to own it.
        assert!(f.definitions.iter().all(|d| d.simple_name != "top"));
    }

    /// WS14b: types are recorded with their package path, kind and bases
    /// (a companion object apart from its class), and `this(…)` in an
    /// auxiliary constructor is a construction of the enclosing class,
    /// not a call named `this`.
    #[test]
    fn types_packages_and_constructor_delegation() {
        let f = extract(
            "package org.ex\npackage sql\n\
             trait Q extends Base { def a = 1 }\nobject Q { def a = 2 }\n\
             class C(n: Int) extends Q with T { def this(s: String) = this(s.length) }\n",
        );
        let types: Vec<(&str, &str, &str, Vec<&str>)> = f
            .type_spans
            .iter()
            .map(|t| {
                (
                    t.name.as_str(),
                    t.kind.as_str(),
                    t.package.as_str(),
                    t.bases.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            types,
            vec![
                ("Q", "trait", "org.ex.sql", vec!["Base"]),
                ("Q", "object", "org.ex.sql", vec![]),
                ("C", "class", "org.ex.sql", vec!["Q", "T"]),
            ],
            "{types:?}"
        );
        let names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"C"), "{names:?}");
        assert!(!names.contains(&"this"), "{names:?}");
    }
}
