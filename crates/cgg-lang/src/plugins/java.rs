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
            dyn_uses: true,
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
            // `this(…)` / `super(…)` as a constructor's first statement.
            "explicit_constructor_invocation" => {
                self.record_constructor_delegation(node);
                self.walk_children(node);
                return;
            }
            "method_invocation" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            // `this::m`, `Type::m`, `Type::new`, `super::m`: a method
            // named as a value. Only when the run keeps reference sites.
            "method_reference" if self.ctx.reference_sites => {
                self.record_method_reference(node);
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
                            arity: node
                                .child_by_field_name("arguments")
                                .map(super::arg_count),
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

    fn record_constructor_delegation(&mut self, node: Node) {
        if let Some(r) = constructor_delegation(node, self.source) {
            self.facts.references.push(r);
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
        // The `static` keyword token among the modifiers — not the text,
        // which an annotation argument (`@SuppressWarnings("static-access")`)
        // or name (`@StaticResource`) also contains.
        let is_static = node
            .children(&mut node.walk())
            .filter(|c| c.kind() == "modifiers")
            .any(|m| m.children(&mut m.walk()).any(|t| t.kind() == "static"));
        let variant = if is_static {
            DefVariant::StaticMethod
        } else {
            DefVariant::InherentMethod
        };
        let from = self.facts.definitions.len();
        self.record_def(node, &simple, variant);
        // An interface method without a `default` body, or an `abstract`
        // one: a declaration. A `native` method's body is out of tree,
        // not absent.
        super::attrs::mark_abstract_from(
            &mut self.facts.definitions,
            from,
            node.child_by_field_name("body").is_none() && !has_native_modifier(node),
        );
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

        // `import static …`: the keyword token, not a substring of the path.
        let is_static = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "static");

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
        let obj = node.child_by_field_name("object");
        let recv = obj.map(|n| self.text(n).to_string()).unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let context = if recv.is_empty() {
            name.clone()
        } else {
            format!("{recv}.{name}")
        };
        // `(f).m()`, `((Foo) o).m()`, `new Foo().m()`: the value or the
        // type the receiver's shape stands for (`crate::receiver`).
        let (recv, typed_receiver) = match obj {
            Some(o) => {
                let p = crate::receiver::peel("java", o, self.source);
                match p.type_name() {
                    Some(t) => (t, true),
                    None => (self.text(p.node).to_string(), false),
                }
            }
            None => (recv, false),
        };
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver,
            arity: node.child_by_field_name("arguments").map(super::arg_count),
            ..Default::default()
        });
        // Shape B/C: a handler passed in argument position. Inert unless
        // a detected framework's rule names this call.
        let extra = super::registrar::capture(&self.ctx, node, self.source, &context);
        self.facts.references.extend(extra);
    }

    /// A method reference names a method without calling it. It is
    /// recorded the way the call it stands for is — `this::m` as `m`
    /// through `this`, `h::consume` through the variable `h`, `Type::new`
    /// as `new Type()` — and marked [`RefRecord::value_ref`] so whatever
    /// it resolves to is a reference edge, not a call.
    ///
    /// The site is the method name's own byte (the `new` token for a
    /// constructor reference), which no call expression starts at, so the
    /// reference never shares a site with a call in the same expression.
    fn record_method_reference(&mut self, node: Node) {
        let mut c = node.walk();
        let children: Vec<Node> = node.children(&mut c).collect();
        let Some(recv_node) = children
            .iter()
            .copied()
            .find(|n| n.is_named() && n.kind() != "type_arguments")
        else {
            return;
        };
        let site_line = (node.start_position().row as u32) + 1;
        if let Some(new_tok) =
            children.iter().find(|n| !n.is_named() && n.kind() == "new")
        {
            // `int[]::new` constructs an array: no callable.
            if recv_node.kind() == "array_type" {
                return;
            }
            let ty = self.text(recv_node);
            let ty = ty.split('<').next().unwrap_or("").trim().to_string();
            if ty.is_empty() {
                return;
            }
            self.facts.references.push(RefRecord {
                name: ty,
                site_line,
                site_byte: new_tok.start_byte() as u32,
                value_ref: true,
                ..Default::default()
            });
            return;
        }
        let Some(name_node) = children
            .iter()
            .rev()
            .copied()
            .find(|n| n.is_named() && n.kind() == "identifier")
        else {
            return;
        };
        if name_node.id() == recv_node.id() {
            return;
        }
        let recv = if recv_node.kind() == "super" {
            "super".to_string()
        } else {
            self.text(recv_node).to_string()
        };
        self.facts.references.push(RefRecord {
            name: self.text(name_node).to_string(),
            receiver_hint: recv,
            site_line,
            site_byte: name_node.start_byte() as u32,
            value_ref: true,
            ..Default::default()
        });
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
            return;
        }
        // `var t = make()`: the bare call's result, recorded as `make()`
        // for type propagation to read the callee's declared return type.
        if let Some(value) = var_node.child_by_field_name("value")
            && value.kind() == "method_invocation"
            && value.child_by_field_name("object").is_none()
            && let Some(callee) = value.child_by_field_name("name")
        {
            self.facts.local_types.push(cgg_core::LocalType {
                var_name,
                type_name: format!("{}()", self.text(callee)),
                scope_byte: node.start_byte() as u32,
            });
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

/// `this(…)` calls one of the class's own constructors, named after
/// the class as `new Box(…)` is; `super(…)` one of the superclass's,
/// recorded as the `super` call it is so the base chain resolves it.
/// Neither is ever the constructor it is written in. Shared with the
/// Groovy plugin, whose grammar writes these nodes as Java's does.
pub(super) fn constructor_delegation(node: Node, src: &[u8]) -> Option<RefRecord> {
    let text = |n: Node| n.utf8_text(src).unwrap_or("");
    let kw = node.child_by_field_name("constructor")?;
    let ctor = std::iter::successors(node.parent(), |n| n.parent())
        .find(|n| n.kind() == "constructor_declaration")?;
    let class = ctor.parent().and_then(|b| b.parent()).filter(|c| {
        matches!(
            c.kind(),
            "class_declaration" | "enum_declaration" | "record_declaration"
        )
    })?;
    let (name, receiver_hint) = match kw.kind() {
        "this" => (
            text(class.child_by_field_name("name")?).to_string(),
            String::new(),
        ),
        "super" => {
            let sup = class
                .child_by_field_name("superclass")
                .and_then(|s| s.named_child(0))?;
            let name = crate::receiver::type_simple_name(text(sup)).to_string();
            (name, "super".to_string())
        }
        _ => return None,
    };
    if name.is_empty() {
        return None;
    }
    Some(RefRecord {
        name,
        receiver_hint,
        site_line: (node.start_position().row as u32) + 1,
        site_byte: node.start_byte() as u32,
        arity: node.child_by_field_name("arguments").map(super::arg_count),
        not_def: Some(ctor.start_byte() as u32),
        ..Default::default()
    })
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

    fn extract_refs(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_java::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        JavaPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(true),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.java"),
            &tree,
            src.as_bytes(),
        )
    }

    fn value_refs(f: &FileFacts) -> Vec<(String, String)> {
        f.references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| (r.receiver_hint.clone(), r.name.clone()))
            .collect()
    }

    /// B3: a method reference names its method through its receiver, the
    /// way the call it stands for would; `Type::new` is `new Type()`.
    #[test]
    fn static_is_the_keyword_not_a_substring() {
        let f = extract(
            "import static a.b.C.m;\nimport a.staticx.D;\nclass K {\n  @SuppressWarnings(\"static-access\") void inst() {}\n  static void stat() {}\n}\n",
        );
        let variant = |n: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == n)
                .unwrap()
                .variant
        };
        assert_eq!(variant("inst"), DefVariant::InherentMethod);
        assert_eq!(variant("stat"), DefVariant::StaticMethod);
        let kinds: Vec<&str> = f.imports.iter().map(|i| i.kind.as_str()).collect();
        assert_eq!(kinds, vec!["from-import", "import"], "{kinds:?}");
    }

    #[test]
    fn method_references_are_value_refs_through_their_receiver() {
        let src = "class A extends B { void f(java.util.List<String> xs, H h) {\n\
            xs.forEach(this::g); xs.map(A::s); new Thread(this::w);\n\
            java.util.function.Supplier<H> n = H::new; xs.forEach(h::c);\n\
            Runnable r = super::hook; java.util.function.IntFunction<int[]> a = int[]::new;\n\
            java.util.function.Supplier<java.util.List<String>> l = java.util.ArrayList<String>::new; } }";
        let f = extract_refs(src);
        let got = value_refs(&f);
        let want: Vec<(String, String)> = [
            ("this", "g"),
            ("A", "s"),
            ("this", "w"),
            ("", "H"),
            ("h", "c"),
            ("super", "hook"),
            ("", "java.util.ArrayList"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        assert_eq!(got, want);
        // The site is the name's own token, never a call's start byte.
        let call_sites: Vec<u32> = f
            .references
            .iter()
            .filter(|r| !r.value_ref)
            .map(|r| r.site_byte)
            .collect();
        assert!(
            f.references
                .iter()
                .filter(|r| r.value_ref)
                .all(|r| !call_sites.contains(&r.site_byte))
        );
    }

    /// An ordinary run records no reference sites: its graph drops every
    /// reference edge anyway.
    #[test]
    fn method_references_need_reference_sites() {
        let f = extract(
            "class A { void f(java.util.List<String> xs) { xs.forEach(this::g); } }",
        );
        assert!(f.references.iter().all(|r| !r.value_ref));
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

    /// WS16 (e): calls and constructions carry their argument count.
    #[test]
    fn calls_record_their_argument_count() {
        let f = extract_refs(
            "class A { void m() { f(); g(1, \"x\" /* c */); new B(a, b, c); o.h(x); } }",
        );
        let got: Vec<(&str, Option<u32>)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.arity))
            .collect();
        assert!(got.contains(&("f", Some(0))), "{got:?}");
        assert!(got.contains(&("g", Some(2))), "{got:?}");
        assert!(got.contains(&("B", Some(3))), "{got:?}");
        assert!(got.contains(&("h", Some(1))), "{got:?}");
    }

    #[test]
    fn constructor_delegation_is_a_call() {
        let f = extract(
            "class B { B(int x) {} } class C extends B implements I { \
             C() { this(1); } C(int y) { super(y); } } \
             class D implements I { D() { super(); } }",
        );
        let refs: Vec<(&str, &str, Option<u32>, bool)> = f
            .references
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    r.receiver_hint.as_str(),
                    r.arity,
                    r.not_def.is_some(),
                )
            })
            .collect();
        assert!(refs.contains(&("C", "", Some(1), true)), "{refs:?}");
        assert!(refs.contains(&("B", "super", Some(1), true)), "{refs:?}");
        // Near miss: no superclass, so `super()` is `Object`'s.
        assert_eq!(refs.len(), 2, "{refs:?}");
    }

    /// The definitions named `qn` and whether each is tagged `abstract`.
    fn abstract_tags(f: &FileFacts, qn: &str) -> Vec<bool> {
        f.definitions
            .iter()
            .filter(|d| d.qualified_name == qn)
            .map(|d| d.attributes.iter().any(|a| a == "abstract"))
            .collect()
    }

    // WS22 gap 7: a bodiless interface or `abstract` method is a
    // declaration; a `default`, `native` or concrete one is not.
    #[test]
    fn bodiless_interface_and_abstract_methods_are_tagged_abstract() {
        let f = extract(
            "interface I { void a(); default void b() {} }\n\
             abstract class K { abstract void c(); native void d(); void e() {} }\n",
        );
        assert_eq!(abstract_tags(&f, "I.a"), vec![true]);
        assert_eq!(abstract_tags(&f, "I.b"), vec![false]);
        assert_eq!(abstract_tags(&f, "K.c"), vec![true]);
        assert_eq!(abstract_tags(&f, "K.d"), vec![false]);
        assert_eq!(abstract_tags(&f, "K.e"), vec![false]);
    }
}
