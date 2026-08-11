//! Java plugin — full callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct JavaPlugin;

impl LanguagePlugin for JavaPlugin {
    fn id(&self) -> &'static str {
        "java"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".java"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            unreachable: true,
            visibility: true,
            attributes: true,
            impls: true,
            value_refs: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_java::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "java");
        let mut w = JavaWalker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        let mut out = facts;
        if ctx.deadcode_signals {
            out.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::JAVA);
        }
        if ctx.deadcode_signals {
            out.dyn_uses = super::dynuse::extract(tree, source, "java");
        }
        out
    }
}

struct JavaWalker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Base types of the enclosing class, innermost last. A method
    /// carries its owner's supertypes because the framework rule marks a
    /// callable, and only the class declares the contract.
    bases: Vec<Vec<String>>,
}

impl<'a> JavaWalker<'a> {
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
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let bases = super::attrs::base_types(node, self.source);
                if !name.is_empty() {
                    self.scope.push(name);
                    self.bases.push(bases);
                    self.walk_children(node);
                    self.bases.pop();
                    self.scope.pop();
                } else {
                    self.bases.push(bases);
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
            "constructor_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(node, &name, DefVariant::Constructor);
                }
                self.walk_children(node);
                return;
            }
            "method_invocation" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            "local_variable_declaration" => {
                self.record_local_type(node);
                self.walk_children(node);
                return;
            }
            "object_creation_expression" => {
                // new Foo(...)
                if let Some(t) = node.child_by_field_name("type") {
                    let name = self.text(t).to_string();
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
            vis: java_vis(&super::extract_signature(self.text(node))),
            // Verbatim: `@GetMapping("/users")` keeps its route, which
            // `attribute_key` would discard and an entry node needs.
            attributes: {
                let mut a = super::attrs::collect(node, self.source);
                // `native` is a modifier, not an annotation, so
                // `attrs::collect` never sees it — which left the JNI
                // family in `classify_ffi` unreachable from Java source
                // even though the docs promise JNI is detected. It is
                // the declaration that says "the body is out of tree".
                if has_native_modifier(node) {
                    a.push("native".to_string());
                }
                a
            },
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_import(&mut self, node: Node) {
        // Get the full import path from the scoped_identifier child.
        let ident_node = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "scoped_identifier" || c.kind() == "identifier");
        let Some(ident_node) = ident_node else { return };
        let path = self.text(ident_node).to_string();
        if path.is_empty() {
            return;
        }

        // Check for `static` modifier
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
        let context = if recv.is_empty() {
            name.clone()
        } else {
            format!("{recv}.{name}")
        };
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
        // Shape B/C: a handler passed in argument position. Inert unless
        // a detected framework's rule names this call.
        let extra = super::registrar::capture(&self.ctx, node, self.source, &context);
        self.facts.references.extend(extra);
    }

    fn record_local_type(&mut self, node: Node) {
        // local_variable_declaration -> type_identifier + variable_declarator
        let type_node = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "type_identifier" || c.kind() == "generic_type");

        let var_node = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "variable_declarator");
        let Some(var_node) = var_node else { return };
        let var_name = var_node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if var_name.is_empty() {
            return;
        }

        // Try explicit type first
        if let Some(type_node) = type_node {
            let type_name = if type_node.kind() == "generic_type" {
                type_node
                    .child(0)
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default()
            } else {
                self.text(type_node).to_string()
            };
            if !type_name.is_empty() && type_name.starts_with(char::is_uppercase) {
                self.facts.local_types.push(cgg_core::LocalType {
                    var_name,
                    type_name,
                    scope_byte: node.start_byte() as u32,
                });
                return;
            }
        }

        // Infer from `new Foo(...)` on RHS (covers `var x = new Foo()`)
        if let Some(value) = var_node.child_by_field_name("value")
            && value.kind() == "object_creation_expression"
            && let Some(t) = value.child_by_field_name("type")
        {
            let type_name = self.text(t).to_string();
            if !type_name.is_empty() && type_name.starts_with(char::is_uppercase) {
                self.facts.local_types.push(cgg_core::LocalType {
                    var_name,
                    type_name,
                    scope_byte: node.start_byte() as u32,
                });
            }
        }
    }
}

/// Project the declaration's modifier keywords onto the shared
/// vocabulary. The *absent* case is the interesting one and differs per
/// language, which is exactly why this normalization belongs in the
/// plugin: here it is `Vis::Internal`.
/// Does this declaration carry the `native` modifier?
///
/// Matched on the modifier token itself rather than by searching the
/// text, so an annotation such as `@NativeQuery` cannot be mistaken for
/// one.
fn has_native_modifier(node: Node) -> bool {
    node.children(&mut node.walk())
        .filter(|c| c.kind() == "modifiers")
        .any(|m| m.children(&mut m.walk()).any(|t| t.kind() == "native"))
}

fn java_vis(modifiers: &str) -> cgg_core::Vis {
    // Only the tokens *before* the parameter list are modifiers, and
    // they must match whole words: a parameter named `publicId` is not
    // a `public` modifier.
    let head = modifiers.split('(').next().unwrap_or(modifiers);
    let toks: Vec<&str> = head.split_whitespace().collect();
    let m = |k: &str| toks.contains(&k);
    if m("public") {
        cgg_core::Vis::Public
    } else if m("protected") {
        cgg_core::Vis::Protected
    } else if m("private") {
        cgg_core::Vis::Private
    } else {
        // Java has no `internal` keyword — that is Kotlin and C#. An
        // absent modifier means package-private, which is exactly
        // `Vis::Internal`, so the fallback covers it.
        cgg_core::Vis::Internal
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
        p.set_language(&tree_sitter_java::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        JavaPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.java"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn a_native_method_is_marked_native() {
        // `native` is the Java side of a JNI boundary: the body lives
        // outside the tree. It is a modifier rather than an annotation,
        // so it was invisible to attribute collection, which left the
        // documented JNI family in `classify_ffi` unreachable.
        let f = extract("public class X {\n  public native int compute(int x);\n}\n");
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "compute")
            .expect("method captured");
        assert!(
            d.attributes.iter().any(|a| a == "native"),
            "attributes were {:?}",
            d.attributes
        );
    }

    #[test]
    fn an_ordinary_method_is_not_marked_native() {
        let f =
            extract("public class X {\n  public int compute(int x) { return x; }\n}\n");
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "compute")
            .unwrap();
        assert!(
            !d.attributes.iter().any(|a| a == "native"),
            "{:?}",
            d.attributes
        );
    }

    #[test]
    fn an_annotation_containing_native_is_not_the_modifier() {
        // Guards the token match against a substring search.
        let f = extract(
            "public class X {\n  @NativeQuery(\"native\")\n  public int q() { return 1; }\n}\n",
        );
        let d = f.definitions.iter().find(|d| d.simple_name == "q").unwrap();
        assert!(
            !d.attributes.iter().any(|a| a == "native"),
            "an annotation must not be read as the `native` modifier: {:?}",
            d.attributes
        );
    }

    #[test]
    fn class_methods_with_package() {
        let src = "package com.example;\npublic class Service {\n  public void run() {}\n  private int process(String s) { return 0; }\n}\n";
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
    fn constructor_variant() {
        let src = "class Foo { public Foo(int x) {} }\n";
        let f = extract(src);
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name == "Foo" && d.variant == DefVariant::Constructor)
        );
    }

    #[test]
    fn imports_captured() {
        let src =
            "import java.util.List;\nimport static java.lang.Math.abs;\nclass C {}\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path.contains("java.util.List")));
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "from-import" && i.alias == "abs")
        );
    }

    #[test]
    fn method_invocation_captured() {
        let src = "class C { void f() { helper(); obj.run(); } }\n";
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

    #[test]
    fn static_method_variant() {
        let src = "class C { public static void create() {} }\n";
        let f = extract(src);
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name == "create"
                    && d.variant == DefVariant::StaticMethod)
        );
    }
}
