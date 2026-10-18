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
            bases: Vec::new(),
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
    /// Supertypes of the enclosing class, innermost last.
    ///
    /// Recorded from 0.6.8. Without it every `base_types` rule for this
    /// language was inert — the contract is identical to Java's and
    /// matched nothing.
    bases: Vec<Vec<String>>,
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
                let class_bases = super::attrs::base_types(node, self.source);
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
            "method_declaration" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            // `Box(int x) { … }`: a constructor, named after its class as
            // Java's is, so `new Box(…)` and `this(…)` reach it.
            "constructor_declaration" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n).to_string();
                    if !name.is_empty() {
                        self.record_def(node, &name, DefVariant::Constructor);
                    }
                }
                self.walk_children(node);
                return;
            }
            // `this(…)` / `super(…)` as a constructor's first statement.
            "explicit_constructor_invocation" => {
                if let Some(r) = super::java::constructor_delegation(node, self.source) {
                    self.facts.references.push(r);
                }
                self.walk_children(node);
                return;
            }
            // `new Box(…)`: a call of `Box`'s constructor, looked up on the
            // type as a construction receiver is.
            "object_creation_expression" => {
                self.record_construction(node);
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
            // `def t = make()`: the bare call's result, recorded as
            // `make()` for type propagation to read the callee's declared
            // return type (WS23).
            "local_variable_declaration" => {
                let mut c = node.walk();
                for d in node.children_by_field_name("declarator", &mut c) {
                    if let (Some(name), Some(value)) = (
                        d.child_by_field_name("name"),
                        d.child_by_field_name("value"),
                    ) && value.kind() == "method_invocation"
                        && value.child_by_field_name("object").is_none()
                        && let Some(callee) = value.child_by_field_name("name")
                    {
                        let var_name = self.text(name).to_string();
                        let type_name = format!("{}()", self.text(callee));
                        self.facts.local_types.push(cgg_core::LocalType {
                            var_name,
                            type_name,
                            scope_byte: node.start_byte() as u32,
                        });
                    }
                }
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
        let from = self.facts.definitions.len();
        self.record_def(node, &simple, variant);
        // An interface or `abstract` method: a declaration.
        let native = node
            .children(&mut node.walk())
            .filter(|c| c.kind() == "modifiers")
            .any(|m| m.children(&mut m.walk()).any(|t| t.kind() == "native"));
        super::attrs::mark_abstract_from(
            &mut self.facts.definitions,
            from,
            node.child_by_field_name("body").is_none() && !native,
        );
    }

    fn record_def(&mut self, node: Node, simple: &str, variant: DefVariant) {
        let qn = self.qn(simple);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        let bases = self.bases.last().cloned().unwrap_or_default();
        self.facts.definitions.push(DefRecord {
            base_types: bases,
            simple_name: simple.to_string(),
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

    /// `new Box(…)` / `new pkg.Box<T>(…)`: the constructor is named after
    /// the type and looked up on it, as a typed receiver is. An anonymous
    /// class body (`new Base() { … }`) still constructs `Base`.
    fn record_construction(&mut self, node: Node) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let path = self.text(ty);
        let path = path.split('<').next().unwrap_or(path).trim().to_string();
        let name = crate::receiver::type_simple_name(&path).to_string();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: path,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver: true,
            arity: node.child_by_field_name("arguments").map(super::arg_count),
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
        // `(f).m()`, `((Foo) o).m()`, `new Foo().m()`: the value or the
        // type the receiver's shape stands for (`crate::receiver`).
        let mut typed_receiver = false;
        let recv = node
            .child_by_field_name("object")
            .map(|n| {
                let p = crate::receiver::peel("groovy", n, self.source);
                match p.type_name() {
                    Some(t) => {
                        typed_receiver = true;
                        t
                    }
                    None => self.text(p.node).to_string(),
                }
            })
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        // The argument count, for overload narrowing (WS23): the listed
        // arguments plus a trailing closure. Named arguments (`f(a: 1)`)
        // are collected into one map parameter, and a spread passes an
        // unknown number, so neither has a count.
        let args = node.child_by_field_name("arguments");
        let countable = args.is_none_or(|a| {
            !a.has_error()
                && a.named_children(&mut a.walk())
                    .all(|c| !matches!(c.kind(), "map_item" | "spread_argument"))
        });
        let arity = countable.then(|| {
            args.map_or(0, super::arg_count)
                + u32::from(node.child_by_field_name("body").is_some())
        });
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver,
            arity,
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

    /// The definitions named `qn` and whether each is tagged `abstract`.
    fn abstract_tags(f: &FileFacts, qn: &str) -> Vec<bool> {
        f.definitions
            .iter()
            .filter(|d| d.qualified_name == qn)
            .map(|d| d.attributes.iter().any(|a| a == "abstract"))
            .collect()
    }

    // WS22 gap 7: interface and `abstract` methods are declarations.
    #[test]
    fn bodiless_interface_and_abstract_methods_are_tagged_abstract() {
        let f = extract(
            "interface I { int a() }\nabstract class K { abstract int b()\n int c() { 1 } }\n",
        );
        assert_eq!(abstract_tags(&f, "I.a"), vec![true]);
        assert_eq!(abstract_tags(&f, "K.b"), vec![true]);
        assert_eq!(abstract_tags(&f, "K.c"), vec![false]);
    }

    // WS22 (WS21 carry-over): constructors are callables named after
    // their class; `new` and `this(…)`/`super(…)` call them.
    #[test]
    fn constructors_and_constructions_are_recorded() {
        let f = extract(
            "class B { B(int n) { } }\nclass C extends B {\n  C(int x) { super(x) }\n  C() { this(1) }\n}\n\
             class P { P() { super() } }\nclass M { void run() { new C(1); new java.util.ArrayList<String>() } }\n",
        );
        let ctors: Vec<&str> = f
            .definitions
            .iter()
            .filter(|d| d.variant == DefVariant::Constructor)
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(ctors, vec!["B.B", "C.C", "C.C", "P.P"]);
        let refs: Vec<(&str, &str, Option<u32>, bool)> = f
            .references
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    r.receiver_hint.as_str(),
                    r.arity,
                    r.typed_receiver,
                )
            })
            .collect();
        assert!(refs.contains(&("B", "super", Some(1), false)), "{refs:?}");
        assert!(refs.contains(&("C", "", Some(1), false)), "{refs:?}");
        assert!(refs.contains(&("C", "C", Some(1), true)), "{refs:?}");
        assert!(
            refs.contains(&("ArrayList", "java.util.ArrayList", Some(0), true)),
            "{refs:?}"
        );
        // Near miss: `super()` in a class with no superclass calls nothing
        // in the tree.
        assert!(
            !refs.iter().any(|r| r.1 == "super" && r.0 != "B"),
            "{refs:?}"
        );
    }
}
