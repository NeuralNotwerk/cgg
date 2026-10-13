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
                    let pkg = self.text(name).replace(';', "").trim().to_string();
                    if !pkg.is_empty() && !self.facts.packages.contains(&pkg) {
                        self.facts.packages.push(pkg.clone());
                    }
                    self.scope.push(pkg);
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
                    // Every class, bodiless ones included, so an
                    // inheritance chain does not end at a class with no
                    // methods — and so a chain that finds nothing proves
                    // the member lives outside the tree (`super.m()`).
                    self.facts.classes.push(cgg_core::ClassDecl {
                        class_qn: self.qn(&name),
                        base_types: bases.clone(),
                        line: (node.start_position().row as u32) + 1,
                    });
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
                // `new Base() { @Override void m() { super.m(); } }`: the
                // body's methods belong to an anonymous subclass of `Base`,
                // so `Base` — not the enclosing class's bases — is what
                // they override and what their `super` names. Their
                // qualified names stay under the enclosing class.
                let anon_base = node
                    .children(&mut node.walk())
                    .any(|c| c.kind() == "class_body")
                    .then(|| node.child_by_field_name("type"))
                    .flatten()
                    .map(|t| self.text(t).to_string())
                    .filter(|t| !t.is_empty());
                match anon_base {
                    Some(b) => {
                        self.bases.push(vec![b]);
                        self.walk_children(node);
                        self.bases.pop();
                    }
                    None => self.walk_children(node),
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
            vis: java_vis(node),
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
            // `import static a.b.C.*;` imports every static member of `C`:
            // recorded as `*`, not as the class name, so the resolver can
            // tell it from `import static a.b.C.m;`.
            let wildcard = node
                .children(&mut node.walk())
                .any(|c| c.kind() == "asterisk");
            let alias = if wildcard {
                "*".to_string()
            } else {
                path.rsplit('.').next().unwrap_or("").to_string()
            };
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

/// Does this declaration's `modifiers` node carry the keyword token
/// `kw` (`native`, `public`, …)?
///
/// Matched on the grammar's keyword token rather than by searching the
/// text, so an annotation such as `@NativeQuery`, or an annotation
/// argument such as `@SuppressWarnings("unchecked")` standing in front
/// of `public`, can neither fake a modifier nor hide one.
fn has_modifier(node: Node, kw: &str) -> bool {
    node.children(&mut node.walk())
        .filter(|c| c.kind() == "modifiers")
        .any(|m| m.children(&mut m.walk()).any(|t| t.kind() == kw))
}

/// Does this declaration carry the `native` modifier?
fn has_native_modifier(node: Node) -> bool {
    has_modifier(node, "native")
}

/// Project the declaration's access modifier onto the shared
/// vocabulary. The *absent* case is the interesting one and differs per
/// language — and, in Java, per enclosing declaration — which is exactly
/// why this normalization belongs in the plugin.
///
/// Read from the `modifiers` node's keyword tokens. It used to be read
/// from the signature text cut at the first `(`, which an annotation
/// argument ends early: `@SuppressWarnings("x") public void f()` read as
/// package-private, and so as corroborated dead code.
fn java_vis(node: Node) -> cgg_core::Vis {
    if has_modifier(node, "public") {
        return cgg_core::Vis::Public;
    }
    if has_modifier(node, "protected") {
        return cgg_core::Vis::Protected;
    }
    if has_modifier(node, "private") {
        return cgg_core::Vis::Private;
    }
    match node.parent().map(|p| p.kind()) {
        // JLS 9.4: a method declared in an interface body without
        // `private` is implicitly public (and `protected` is a compile
        // error there). JLS 9.6.1 says the same of annotation-type
        // elements. Only the *direct* body counts: a class body nested
        // inside an interface — an anonymous class in a default method —
        // follows class rules again.
        Some("interface_body" | "annotation_type_body") => cgg_core::Vis::Public,
        // JLS 8.9.2: an enum constructor with no access modifier is
        // private. Enum *methods* follow class rules (below).
        Some("enum_body_declarations") if node.kind() == "constructor_declaration" => {
            cgg_core::Vis::Private
        }
        // Java has no `internal` keyword — that is Kotlin and C#. An
        // absent modifier in a class, enum or record body means
        // package-private, which is exactly `Vis::Internal`.
        _ => cgg_core::Vis::Internal,
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

    /// `import static a.b.C.*;` is recorded with `*`, apart from the
    /// single-member form, so the resolver can scope a bare call to `C`.
    #[test]
    fn static_wildcard_import_is_recorded_as_star() {
        let f = extract(
            "import static org.junit.Assert.*;\nimport static a.b.C.m;\nclass X {}\n",
        );
        let got: Vec<(&str, &str)> = f
            .imports
            .iter()
            .filter(|i| i.kind == "from-import")
            .map(|i| (i.path.as_str(), i.alias.as_str()))
            .collect();
        assert_eq!(got, vec![("org.junit.Assert", "*"), ("a.b.C.m", "m")]);
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

    #[test]
    fn package_is_recorded() {
        let f = extract("package com.acme.util;\nclass A { void m() {} }\n");
        assert_eq!(f.packages, vec!["com.acme.util"]);
    }

    fn vis_of(f: &FileFacts, name: &str) -> cgg_core::Vis {
        f.definitions
            .iter()
            .find(|d| d.simple_name == name)
            .unwrap_or_else(|| panic!("{name} not extracted"))
            .vis
    }

    #[test]
    fn an_annotation_argument_does_not_hide_the_access_modifier() {
        // FINDINGS B1: the signature was cut at the first `(`, which here
        // is the annotation's, so every modifier after it was lost and
        // the method read as package-private — corroborated dead code.
        let f = extract(
            "class X {\n\
             \x20 @SuppressWarnings(\"unchecked\")\n  public void pub() {}\n\
             \x20 @Deprecated(since = \"1\", forRemoval = false) protected void prot() {}\n\
             \x20 @SuppressWarnings(\"unused\")\n  private void priv() {}\n\
             \x20 @SuppressWarnings(\"unused\")\n  void pkg() {}\n\
             }\n",
        );
        use cgg_core::Vis;
        assert_eq!(vis_of(&f, "pub"), Vis::Public);
        assert_eq!(vis_of(&f, "prot"), Vis::Protected);
        // Near-misses: an explicit `private`, or no modifier at all,
        // behind the same annotation must not change.
        assert_eq!(vis_of(&f, "priv"), Vis::Private);
        assert_eq!(vis_of(&f, "pkg"), Vis::Internal);
    }

    #[test]
    fn modifier_words_outside_the_modifiers_node_are_not_modifiers() {
        use cgg_core::Vis;
        let f = extract(
            "class X {\n\
             \x20 void a(String publicId) {}\n\
             \x20 @Named(\"public\") void b() {}\n\
             }\n",
        );
        assert_eq!(vis_of(&f, "a"), Vis::Internal);
        assert_eq!(vis_of(&f, "b"), Vis::Internal);
    }

    #[test]
    fn interface_members_without_a_modifier_are_implicitly_public() {
        // JLS 9.4 — abstract, default and static interface methods alike.
        use cgg_core::Vis;
        let f = extract(
            "interface Repo {\n\
             \x20 void findAll();\n\
             \x20 @SuppressWarnings(\"x\") void annotated();\n\
             \x20 default void dflt() {}\n\
             \x20 static void stat() {}\n\
             \x20 private void helper() {}\n\
             }\n\
             class Outer { interface Inner { void nested(); } }\n",
        );
        assert_eq!(vis_of(&f, "findAll"), Vis::Public);
        assert_eq!(vis_of(&f, "annotated"), Vis::Public);
        assert_eq!(vis_of(&f, "dflt"), Vis::Public);
        assert_eq!(vis_of(&f, "stat"), Vis::Public);
        assert_eq!(vis_of(&f, "nested"), Vis::Public);
        // Java 9 private interface methods stay private.
        assert_eq!(vis_of(&f, "helper"), Vis::Private);
    }

    #[test]
    fn class_bodies_inside_an_interface_follow_class_rules() {
        // Near-miss: only the interface's *own* body makes members
        // public. An anonymous class in a default method is a class.
        use cgg_core::Vis;
        let f = extract(
            "interface I {\n  default void make() {\n    Object o = new Object() { void anon() {} };\n  }\n}\n",
        );
        assert_eq!(vis_of(&f, "make"), Vis::Public);
        assert_eq!(vis_of(&f, "anon"), Vis::Internal);
    }

    #[test]
    fn enum_and_record_members_follow_class_rules() {
        // Enum and record methods have no implicit-public rule; an enum
        // constructor with no modifier is private (JLS 8.9.2).
        use cgg_core::Vis;
        let f = extract(
            "enum Color {\n  RED(1);\n  Color(int x) {}\n  void m() {}\n  public void p() {}\n}\n\
             record Point(int x) {\n  void r() {}\n}\n\
             class K {\n  K() {}\n}\n",
        );
        assert_eq!(vis_of(&f, "Color"), Vis::Private);
        assert_eq!(vis_of(&f, "m"), Vis::Internal);
        assert_eq!(vis_of(&f, "p"), Vis::Public);
        assert_eq!(vis_of(&f, "r"), Vis::Internal);
        // A class constructor with no modifier is package-private.
        assert_eq!(vis_of(&f, "K"), Vis::Internal);
    }

    #[test]
    fn every_class_is_recorded_with_its_bases_bodiless_ones_included() {
        let f = extract(
            "package p;\nclass Middle extends Base {}\ninterface I extends J, K { void m(); }\nclass Leaf extends Middle implements I { public void m() {} }\n",
        );
        let by: Vec<(&str, Vec<&str>)> = f
            .classes
            .iter()
            .map(|c| {
                (
                    c.class_qn.as_str(),
                    c.base_types.iter().map(|b| b.as_str()).collect(),
                )
            })
            .collect();
        assert!(by.contains(&("p.Middle", vec!["Base"])), "{by:?}");
        assert!(by.contains(&("p.I", vec!["J", "K"])), "{by:?}");
        assert!(by.contains(&("p.Leaf", vec!["Middle", "I"])), "{by:?}");
    }

    #[test]
    fn anonymous_class_methods_carry_the_anonymous_base() {
        let f = extract(
            "package p;\nclass Outer extends Top {\n  void go() {\n    Runnable r = new Task() { @Override public void run() { } };\n  }\n  void after() { }\n}\n",
        );
        let bases = |n: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == n)
                .map(|d| d.base_types.clone())
                .unwrap()
        };
        assert_eq!(bases("run"), vec!["Task"]);
        assert_eq!(bases("after"), vec!["Top"]);
    }
}
