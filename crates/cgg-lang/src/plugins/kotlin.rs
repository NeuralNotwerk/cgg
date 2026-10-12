//! Kotlin plugin — full callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct KotlinPlugin;

impl LanguagePlugin for KotlinPlugin {
    fn id(&self) -> &'static str {
        "kotlin"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".kt", ".kts"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            attributes: true,
            visibility: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_kotlin_sg::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "kotlin");
        let mut w = KtWalker {
            bases: Vec::new(),
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct KtWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Supertypes of the enclosing class, innermost last.
    ///
    /// Kotlin recorded none until 0.6.8, which made every `base_types`
    /// rule for the language inert — `class Handler : RequestHandler<…>`
    /// is the *identical* contract Java's rule matches on, and it
    /// produced nothing.
    bases: Vec<Vec<String>>,
}

impl<'a> KtWalker<'a> {
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
            "package_header" => {
                // package com.example
                let name = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "identifier")
                    .map(|n| self.text(n).replace(" ", "").replace("\n", ""))
                    .unwrap_or_default();
                if !name.is_empty() {
                    if !self.facts.packages.contains(&name) {
                        self.facts.packages.push(name.clone());
                    }
                    // The identifier node already joins its
                    // simple_identifier children with dots, which is the
                    // separator we want, so the text is used as-is.
                    self.scope.push(name);
                }
                return;
            }
            "import_header" => {
                self.record_import(node);
                return;
            }
            "class_declaration" => {
                let name = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "type_identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let class_bases = kotlin_bases(node, self.source);
                if !name.is_empty() {
                    self.record_class(node, &name, &class_bases);
                    // Emit the class as a Constructor callable so that
                    // `Foo()` calls resolve to it.
                    let qn = self.qn(&name);
                    let (sl, el) = (
                        (node.start_position().row as u32) + 1,
                        (node.end_position().row as u32) + 1,
                    );
                    self.facts.definitions.push(cgg_core::DefRecord {
                        simple_name: name.clone(),
                        qualified_name: qn,
                        variant: cgg_core::DefVariant::Constructor,
                        start_line: sl,
                        end_line: el,
                        start_byte: node.start_byte() as u32,
                        end_byte: node.end_byte() as u32,
                        signature_hint: super::extract_signature(self.text(node)),
                        visibility: String::new(),
                        attributes: super::attrs::collect(node, self.source),
                        ..Default::default()
                    });
                    self.scope.push(name);
                    self.bases.push(class_bases);
                    self.walk_children(node);
                    self.bases.pop();
                    self.scope.pop();
                } else {
                    self.bases.push(class_bases);
                    self.walk_children(node);
                    self.bases.pop();
                }
                return;
            }
            "object_declaration" => {
                let name = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "type_identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // An object's methods see the object's own supertypes,
                // not those of a class it happens to be nested in.
                let object_bases = kotlin_bases(node, self.source);
                if !name.is_empty() {
                    self.record_class(node, &name, &object_bases);
                }
                self.bases.push(object_bases);
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                self.bases.pop();
                return;
            }
            "companion_object" => {
                // companion object { ... } — methods are static-like
                self.walk_children(node);
                return;
            }
            "function_declaration" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            "property_declaration" => {
                // val svc: Service = ... / var helper: Helper = ...
                self.record_local_type(node);
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    /// Every class and object with its supertypes, including one with no
    /// methods: an inheritance chain seeded from method records alone ends
    /// at a bodiless `class Mid : Base()`, so `Leaf`'s `super.start()` and
    /// an inherited `leaf.start()` never reached `Base.start`.
    fn record_class(&mut self, node: Node, name: &str, bases: &[String]) {
        self.facts.classes.push(cgg_core::ClassDecl {
            class_qn: self.qn(name),
            base_types: bases.to_vec(),
            line: (node.start_position().row as u32) + 1,
        });
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

    fn record_function(&mut self, node: Node) {
        // function_declaration has simple_identifier as name
        let name = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "simple_identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }

        // Check if it's an extension function (has receiver_type field)
        let is_extension = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "receiver_type");
        let variant = if is_extension {
            DefVariant::InherentMethod
        } else if self.scope.is_empty() {
            DefVariant::FreeFunction
        } else {
            DefVariant::InherentMethod
        };

        let qn = self.qn(&name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        let bases = self.bases.last().cloned().unwrap_or_default();
        self.facts.definitions.push(DefRecord {
            base_types: bases,
            simple_name: name,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            vis: kotlin_vis(node),
            attributes: super::attrs::collect(node, self.source),
            ..Default::default()
        });
    }

    fn record_import(&mut self, node: Node) {
        let ident = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier")
            .map(|n| self.text(n).replace(" ", "").replace("\n", ""))
            .unwrap_or_default();
        if ident.is_empty() {
            return;
        }

        // Check for alias: `import ... as Alias`
        let explicit_alias = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "import_alias")
            .and_then(|a| {
                a.children(&mut a.walk()).find(|c| {
                    c.kind() == "type_identifier" || c.kind() == "simple_identifier"
                })
            })
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();

        // For non-aliased imports, the binding name is the last segment.
        // `import com.example.Helper` -> alias "Helper"
        let alias = if !explicit_alias.is_empty() {
            explicit_alias
        } else {
            ident.rsplit('.').next().unwrap_or("").to_string()
        };

        self.facts.imports.push(ImportRecord {
            kind: "import".into(),
            path: ident,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_local_type(&mut self, node: Node) {
        // property_declaration -> variable_declaration -> simple_identifier + user_type -> type_identifier
        let var_decl = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "variable_declaration");
        let Some(var_decl) = var_decl else { return };
        let var_name = var_decl
            .children(&mut var_decl.walk())
            .find(|c| c.kind() == "simple_identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if var_name.is_empty() {
            return;
        }

        // Try explicit type annotation first
        let type_name = var_decl
            .children(&mut var_decl.walk())
            .find(|c| c.kind() == "user_type")
            .and_then(|ut| {
                ut.children(&mut ut.walk())
                    .find(|c| c.kind() == "type_identifier")
            })
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if !type_name.is_empty() && type_name.starts_with(char::is_uppercase) {
            self.facts.local_types.push(cgg_core::LocalType {
                var_name,
                type_name,
                scope_byte: node.start_byte() as u32,
            });
            return;
        }

        // Infer from RHS constructor: val x = Foo(...) or val x = Foo.create(...)
        // The RHS is a sibling of variable_declaration in property_declaration
        let call = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "call_expression");
        if let Some(call) = call {
            let callee = call.child(0);
            if let Some(callee) = callee {
                let callee_text = self.text(callee);
                // Direct constructor: Foo(...)
                if callee_text.starts_with(char::is_uppercase)
                    && !callee_text.contains('.')
                {
                    self.facts.local_types.push(cgg_core::LocalType {
                        var_name,
                        type_name: callee_text.to_string(),
                        scope_byte: node.start_byte() as u32,
                    });
                }
            }
        }
    }

    fn record_call(&mut self, node: Node) {
        // call_expression -> first child is the callee
        let callee = node.child(0);
        let Some(callee) = callee else { return };
        let (name, recv) = match callee.kind() {
            "simple_identifier" => (self.text(callee).to_string(), String::new()),
            "navigation_expression" => {
                // `obj.method()`: the method is the last navigation
                // suffix, the receiver everything in front of it — a
                // chain (`a.b`, `this.x`), a call (`x.a()`, `Foo()`), a
                // `x!!`, a parenthesised expression. Only the plain
                // identifier used to be kept: every other receiver came
                // out empty and the call was resolved as a bare one, so
                // `other.stream.close()` bound to the enclosing class's
                // own `close` at high (FINDINGS A3).
                let Some(name) = nav_suffix_name(callee, self.source) else {
                    return;
                };
                let Some(base) = callee.named_child(0) else {
                    return;
                };
                let recv = self
                    .constructed_type(base)
                    .unwrap_or_else(|| self.receiver_text(base));
                (name.to_string(), recv)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        // A receiver the text could not be recovered for is still a
        // receiver: the call must not be read as a bare one.
        let dot_call = recv.is_empty() && callee.kind() == "navigation_expression";
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            dot_call,
            ..Default::default()
        });
    }

    /// `Foo(x)` / `pkg.Foo(x)` in receiver position → `Foo` / `pkg.Foo`.
    ///
    /// Kotlin constructs without `new`, so a call whose callee is a type
    /// name (capitalised last segment) builds a value of that type — the
    /// rule `record_local_type` already types `val x = Foo(…)` by. The
    /// receiver is recorded as the type, exactly what `val x = Foo(…);
    /// x.m()` resolves through after type propagation: an owner lookup on
    /// `Foo`, and, when `Foo` is imported from a dependency
    /// (`File(p).readText()`), the external-module refusal.
    fn constructed_type(&self, n: Node) -> Option<String> {
        if n.kind() != "call_expression" {
            return None;
        }
        let callee = n.child(0)?;
        if !matches!(callee.kind(), "simple_identifier" | "navigation_expression") {
            return None;
        }
        let path = self.receiver_text(callee);
        let plain = path
            .split('.')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'));
        let last = path.rsplit('.').next()?;
        (plain && last.starts_with(char::is_uppercase)).then_some(path)
    }

    /// The receiver expression as source text, rebuilt from the tree the
    /// way the other plugins record theirs (`a.b`, `this.x`, `x.a()`),
    /// with whitespace, comments and line breaks inside a chain left out
    /// and argument lists — often whole trailing-lambda bodies — elided
    /// to `()`. A non-null assertion (`x!!`) is its operand: it changes
    /// nullability, not the value. String literals are `""`.
    fn receiver_text(&self, n: Node) -> String {
        match n.kind() {
            "simple_identifier" => self.text(n).to_string(),
            "super_expression" => "super".to_string(),
            // `this@Outer` where `Outer` is an enclosing class or object
            // is that class's instance: recorded as the type, as a
            // constructed receiver is. Any other label (`this@apply`,
            // a lambda's or extension function's) keeps its text.
            "this_expression" => {
                let mut c = n.walk();
                let label = n
                    .children(&mut c)
                    .find(|c| c.kind() == "type_identifier")
                    .map(|l| self.text(l));
                match label {
                    Some(l) if self.scope.iter().any(|s| s == l) => l.to_string(),
                    _ => collapse_ws(self.text(n)),
                }
            }
            "navigation_expression" => {
                match (n.named_child(0), nav_suffix_name(n, self.source)) {
                    (Some(base), Some(name)) => {
                        format!("{}.{name}", self.receiver_text(base))
                    }
                    _ => collapse_ws(self.text(n)),
                }
            }
            "postfix_expression"
                if n.child((n.child_count() as u32).saturating_sub(1))
                    .is_some_and(|c| c.kind() == "!!") =>
            {
                n.named_child(0)
                    .map(|op| self.receiver_text(op))
                    .unwrap_or_default()
            }
            "call_expression" => match n.child(0) {
                Some(callee) => format!("{}()", self.receiver_text(callee)),
                None => collapse_ws(self.text(n)),
            },
            "string_literal" | "line_string_literal" | "multi_line_string_literal" => {
                "\"\"".to_string()
            }
            _ => collapse_ws(self.text(n)),
        }
    }
}

/// Supertypes of a class or object declaration, in declaration order.
///
/// Read from the grammar: each `delegation_specifier`'s type, whether it
/// is written bare (`I`), as a superclass constructor call (`Base()`,
/// `Exception("x")`) or as a delegation (`I by impl`). The shared
/// text-based `attrs::base_types` rejects the last two forms, so a Kotlin
/// class's superclass was never recorded — its inherited methods could not
/// be found and `super.m()` could not be resolved through it.
fn kotlin_bases(node: Node, source: &[u8]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut specs: Vec<Node> = Vec::new();
    let mut c = node.walk();
    for child in node.children(&mut c) {
        match child.kind() {
            "delegation_specifier" => specs.push(child),
            "delegation_specifiers" => {
                let mut cc = child.walk();
                specs.extend(
                    child
                        .children(&mut cc)
                        .filter(|n| n.kind() == "delegation_specifier"),
                );
            }
            _ => {}
        }
    }
    for spec in specs {
        let mut sc = spec.walk();
        let ty = spec.children(&mut sc).find_map(|n| match n.kind() {
            "user_type" => Some(n),
            "constructor_invocation" | "explicit_delegation" => {
                let mut nc = n.walk();
                n.children(&mut nc).find(|t| t.kind() == "user_type")
            }
            _ => None,
        });
        let Some(ty) = ty.and_then(|t| t.utf8_text(source).ok()) else {
            continue;
        };
        let ty = collapse_ws(ty);
        if !ty.is_empty() && !out.contains(&ty) {
            out.push(ty);
        }
    }
    out
}

/// The member a navigation expression selects: `close` in `a.b.close`,
/// `m` in `x?.m`.
fn nav_suffix_name<'s>(nav: Node, source: &'s [u8]) -> Option<&'s str> {
    let mut c = nav.walk();
    let suffix = nav
        .children(&mut c)
        .filter(|c| c.kind() == "navigation_suffix")
        .last()?;
    let mut c = suffix.walk();
    let id = suffix
        .children(&mut c)
        .find(|c| c.kind() == "simple_identifier")?;
    id.utf8_text(source).ok().filter(|s| !s.is_empty())
}

/// Runs of whitespace (line breaks and indentation of a wrapped
/// expression) as one space.
fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Project the declaration's visibility modifier onto the shared
/// vocabulary. The *absent* case is the interesting one and differs per
/// language, which is exactly why this normalization belongs in the
/// plugin: here it is `Vis::Public`.
///
/// Read from the grammar's `visibility_modifier` token, as WS2 did for
/// Java and C#. It used to be read from the signature text cut at the
/// first `(`, which an annotation argument ends early: `@Suppress("x")
/// private fun f()` read as public.
fn kotlin_vis(node: Node) -> cgg_core::Vis {
    let mut c = node.walk();
    for m in node.children(&mut c).filter(|c| c.kind() == "modifiers") {
        let mut mc = m.walk();
        for v in m
            .children(&mut mc)
            .filter(|c| c.kind() == "visibility_modifier")
        {
            let Some(tok) = v.child(0) else { continue };
            match tok.kind() {
                "public" => return cgg_core::Vis::Public,
                "protected" => return cgg_core::Vis::Protected,
                "private" => return cgg_core::Vis::Private,
                "internal" => return cgg_core::Vis::Internal,
                _ => {}
            }
        }
    }
    cgg_core::Vis::Public
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_kotlin_sg::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        KotlinPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.kt"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn class_methods_with_package() {
        let src = "package com.example\nclass Service {\n  fun run() {}\n  fun process(s: String): Int = s.length\n}\n";
        let f = extract(src);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(
            qns.iter().any(|q| q.ends_with("Service.run")),
            "got: {qns:?}"
        );
        assert!(
            qns.iter().any(|q| q.ends_with("Service.process")),
            "got: {qns:?}"
        );
    }

    #[test]
    fn top_level_function() {
        let src = "fun topLevel() {}\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(
            |d| d.simple_name == "topLevel" && d.variant == DefVariant::FreeFunction
        ));
    }

    #[test]
    fn extension_function() {
        let src = "fun String.greet() { println(this) }\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "greet"));
    }

    #[test]
    fn imports_captured() {
        let src =
            "import com.example.Helper\nimport com.example.format as fmt\nfun f() {}\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path.contains("Helper")));
        assert!(f.imports.iter().any(|i| i.alias == "fmt"));
    }

    #[test]
    fn call_expressions() {
        let src = "fun f() { helper(); obj.run() }\n";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "helper"),
            "refs: {:?}",
            f.references
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "run" && r.receiver_hint == "obj"),
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn package_is_recorded() {
        let f = extract("package com.acme.util\n\nfun helper() {}\n");
        assert_eq!(f.packages, vec!["com.acme.util"]);
    }

    /// `(name, receiver_hint, dot_call)` of every call named `name`, in
    /// source order.
    fn calls<'f>(f: &'f FileFacts, name: &str) -> Vec<(&'f str, bool)> {
        f.references
            .iter()
            .filter(|r| r.name == name)
            .map(|r| (r.receiver_hint.as_str(), r.dot_call))
            .collect()
    }

    #[test]
    fn a_chained_receiver_is_kept_not_dropped() {
        // FINDINGS A3: only a plain-identifier receiver was kept; every
        // other one came out empty and the call read as a bare one, so
        // `other.stream.close()` bound to the enclosing class's `close`.
        let f = extract(
            "class Conn(val stream: S) {\n\
             \x20 fun close() {}\n\
             \x20 fun shutdown(other: Conn) {\n\
             \x20   other.stream.close()\n\
             \x20   this.stream.close()\n\
             \x20   a.b.c.close()\n\
             \x20   x.a().close()\n\
             \x20   x!!.close()\n\
             \x20   x!!.y!!.close()\n\
             \x20   x?.y?.close()\n\
             \x20   list.map { it }.filter { it }.close()\n\
             \x20   \"s\".close()\n\
             \x20   val z = (y).close()\n\
             \x20 }\n}\n",
        );
        let got: Vec<&str> = calls(&f, "close").iter().map(|(r, _)| *r).collect();
        assert_eq!(
            got,
            vec![
                "other.stream",
                "this.stream",
                "a.b.c",
                "x.a()",
                "x",
                "x.y",
                "x.y",
                "list.map().filter()",
                "\"\"",
                "(y)",
            ],
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn a_receiver_chain_is_rebuilt_without_its_line_breaks() {
        let f = extract("fun f() {\n  obj\n    .one()\n    // note\n    .two()\n}\n");
        assert_eq!(calls(&f, "two"), vec![("obj.one()", false)]);
        assert_eq!(calls(&f, "one"), vec![("obj", false)]);
    }

    #[test]
    fn this_and_super_receivers_are_kept() {
        let f = extract(
            "class A : B() {\n  override fun m() { super.m(); this.n(); super<B>.m() }\n  fun n() {}\n}\n",
        );
        assert_eq!(calls(&f, "m"), vec![("super", false), ("super", false)]);
        assert_eq!(calls(&f, "n"), vec![("this", false)]);
    }

    #[test]
    fn a_this_labelled_with_an_enclosing_class_is_that_class() {
        let f = extract(
            "class Outer {\n  fun close() {}\n  inner class In {\n    fun f() {\n      \
             this@Outer.close()\n      this@In.close()\n      run { this@run.close() }\n    }\n  }\n}\n",
        );
        assert_eq!(
            calls(&f, "close"),
            vec![("Outer", false), ("In", false), ("this@run", false)]
        );
    }

    #[test]
    fn a_constructed_receiver_is_its_type() {
        // Kotlin constructs without `new`: `Foo(x).m()` calls `m` on a
        // `Foo`, the rule `val x = Foo(…)` is typed by. An import from a
        // dependency (`File`) then reaches the external-module refusal,
        // and a workspace type its owner lookup.
        let f = extract(
            "fun f() {\n  File(p).readText()\n  java.io.File(p).delete()\n  \
             Thread { }.start()\n  Foo<Int>(1).m()\n  make(1).m()\n  \
             Registry.lookup(k).m()\n}\n",
        );
        assert_eq!(calls(&f, "readText"), vec![("File", false)]);
        assert_eq!(calls(&f, "delete"), vec![("java.io.File", false)]);
        assert_eq!(calls(&f, "start"), vec![("Thread", false)]);
        // A function's result, or a method's, is not a type.
        assert_eq!(
            calls(&f, "m"),
            vec![
                ("Foo", false),
                ("make()", false),
                ("Registry.lookup()", false)
            ]
        );
    }

    #[test]
    fn a_package_qualified_call_keeps_its_package() {
        // `kotlinx.coroutines.runBlocking { }` named only `runBlocking`
        // with no receiver, so it bound to a workspace `runBlocking`.
        let f = extract(
            "fun f() {\n  kotlinx.coroutines.runBlocking { }\n  runBlocking { }\n}\n",
        );
        assert_eq!(
            calls(&f, "runBlocking"),
            vec![("kotlinx.coroutines", false), ("", false)]
        );
    }

    #[test]
    fn plain_receivers_and_bare_calls_are_unchanged() {
        // Near miss: the shapes the plugin always handled.
        let f =
            extract("fun f() {\n  helper()\n  obj.run()\n  Obj.run()\n  x?.run()\n}\n");
        assert_eq!(calls(&f, "helper"), vec![("", false)]);
        assert_eq!(
            calls(&f, "run"),
            vec![("obj", false), ("Obj", false), ("x", false)]
        );
    }

    #[test]
    fn a_superclass_written_as_a_constructor_call_is_a_base() {
        // `: Base()` and `I by x` were dropped by the text-based reader,
        // so a class's superclass was never recorded.
        let f = extract(
            "class A : Base(1), I, J by x, pkg.Gen<T>() {\n  fun m() {}\n}\n\
             object O : Handler<String, Int> {\n  fun h() {}\n}\n\
             class P {\n  fun p() {}\n}\n",
        );
        let bases = |name: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == name)
                .map(|d| d.base_types.clone())
                .unwrap_or_default()
        };
        assert_eq!(bases("m"), vec!["Base", "I", "J", "pkg.Gen<T>"]);
        let class_bases = |qn: &str| {
            f.classes
                .iter()
                .find(|c| c.class_qn == qn)
                .map(|c| c.base_types.clone())
        };
        assert_eq!(
            class_bases("A"),
            Some(vec![
                "Base".into(),
                "I".into(),
                "J".into(),
                "pkg.Gen<T>".into()
            ])
        );
        assert_eq!(class_bases("O"), Some(vec!["Handler<String, Int>".into()]));
        assert_eq!(class_bases("P"), Some(vec![]));
        assert_eq!(bases("h"), vec!["Handler<String, Int>"]);
        // Near miss: a class with no supertypes has none.
        assert!(bases("p").is_empty());
    }

    fn vis_of(f: &FileFacts, name: &str) -> cgg_core::Vis {
        f.definitions
            .iter()
            .find(|d| d.simple_name == name)
            .unwrap_or_else(|| panic!("{name} not extracted"))
            .vis
    }

    #[test]
    fn an_annotation_argument_does_not_hide_the_visibility_modifier() {
        // The signature used to be cut at the first `(` — here the
        // annotation's — so `private` was lost and the default applied.
        let f = extract(
            "class A {\n\
             \x20 @Suppress(\"x\") private fun p() {}\n\
             \x20 @Deprecated(\"y\") protected fun q() {}\n\
             \x20 @JvmName(\"z\") internal fun r() {}\n\
             \x20 private\n  fun s() {}\n\
             }\n",
        );
        assert_eq!(vis_of(&f, "p"), cgg_core::Vis::Private);
        assert_eq!(vis_of(&f, "q"), cgg_core::Vis::Protected);
        assert_eq!(vis_of(&f, "r"), cgg_core::Vis::Internal);
        assert_eq!(vis_of(&f, "s"), cgg_core::Vis::Private);
    }

    #[test]
    fn visibility_defaults_to_public_and_ignores_parameter_names() {
        // Near miss: Kotlin's default is public, and a parameter named
        // like a modifier is not one.
        let f = extract(
            "class A {\n  fun d(privateKey: String) {}\n  @Suppress(\"x\") fun e() {}\n  public fun g() {}\n}\n",
        );
        assert_eq!(vis_of(&f, "d"), cgg_core::Vis::Public);
        assert_eq!(vis_of(&f, "e"), cgg_core::Vis::Public);
        assert_eq!(vis_of(&f, "g"), cgg_core::Vis::Public);
    }
}
