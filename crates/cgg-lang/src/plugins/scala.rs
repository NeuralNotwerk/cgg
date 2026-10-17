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
        crate::closure_types::record("scala", node, self.source, self.facts);
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
                let class_bases =
                    super::attrs::base_types_with_ctor_args(node, self.source);
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
            // A member value of a class, trait or object (`val io = …` in
            // its body, `class A(val io: B)`): in scope throughout the
            // type, where it shadows a package of the same name. Recorded
            // so a receiver headed by it is never read as that package.
            "val_definition" | "var_definition" | "val_declaration"
            | "var_declaration" | "class_parameter"
                if !self.scope.is_empty()
                    && node.parent().is_some_and(|p| {
                        matches!(p.kind(), "template_body" | "class_parameters")
                    }) =>
            {
                let name = node
                    .child_by_field_name("pattern")
                    .or_else(|| node.child_by_field_name("name"))
                    .filter(|n| n.kind() == "identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.facts.class_fields.push(cgg_core::ClassFieldDecl {
                        class_qn: self.scope.join("::"),
                        field_name: name,
                        type_name: node
                            .child_by_field_name("type")
                            .map(|t| self.text(t).to_string())
                            .unwrap_or_default(),
                        line: (node.start_position().row as u32) + 1,
                    });
                }
                self.walk_children(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            // `b.size`, a bare `helper`: a member named with no argument
            // list is a value read or a call to a parameterless `def`.
            // Recorded as a paren-less site; the resolver keeps it only
            // when such a `def` exists by that name.
            "identifier" | "field_expression" if paren_less_position(node) => {
                self.record_paren_less(node);
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    /// A receiver as the resolver reads it: its text with whitespace
    /// removed, except that a construction without a body (`new
    /// Auction(…)`, `new Box[Int]()`) is the type it constructs.
    fn receiver_text(&self, v: Node) -> String {
        if v.kind() == "instance_expression"
            && !v
                .named_children(&mut v.walk())
                .any(|c| c.kind() == "template_body")
            && let Some(ty) = v.named_children(&mut v.walk()).find(|c| {
                matches!(
                    c.kind(),
                    "type_identifier" | "generic_type" | "stable_type_identifier"
                )
            })
        {
            let ty = if ty.kind() == "generic_type" {
                ty.child_by_field_name("type").unwrap_or(ty)
            } else {
                ty
            };
            return self.text(ty).split_whitespace().collect();
        }
        self.text(v).split_whitespace().collect()
    }

    /// A receiver read through its shape (`crate::receiver`): `(f)` is
    /// `f`, `new Foo(…)`, `Foo(…)` and `o.asInstanceOf[Foo]` are a `Foo`
    /// (flagged as a type). An anonymous class (`new Foo { … }`) keeps
    /// its text: its members are its own body's.
    fn shaped_receiver(&self, v: Node) -> (String, bool) {
        let p = crate::receiver::peel("scala", v, self.source);
        let anonymous = p.node.kind() == "instance_expression"
            && p.node
                .named_children(&mut p.node.walk())
                .any(|c| c.kind() == "template_body");
        match p.ty {
            Some(t) if !anonymous => (t.split_whitespace().collect(), true),
            _ => (self.receiver_text(p.node), false),
        }
    }

    fn record_paren_less(&mut self, node: Node) {
        let (name, receiver) = match node.kind() {
            "field_expression" => {
                let Some(field) = node.child_by_field_name("field") else {
                    return;
                };
                let recv = node
                    .child_by_field_name("value")
                    .map(|v| self.shaped_receiver(v))
                    .unwrap_or_default();
                (self.text(field).to_string(), recv)
            }
            _ => (self.text(node).to_string(), (String::new(), false)),
        };
        let (receiver, typed_receiver) = receiver;
        if name.is_empty() || matches!(name.as_str(), "this" | "super" | "_") {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: receiver,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            paren_less: true,
            typed_receiver,
            ..Default::default()
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
            attributes: super::attrs::with_override_marker(
                node,
                super::attrs::collect_with_preceding(node, self.source),
            ),
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
        let Some(mut function) = node.child_by_field_name("function") else {
            return;
        };
        // `f[Int]()`, `b.m[Int]()`: explicit type arguments wrap the
        // callee in a `generic_function`; the call is of what it wraps.
        if function.kind() == "generic_function" {
            let Some(inner) = function.child_by_field_name("function") else {
                return;
            };
            function = inner;
        }
        // `b.m()`, `this.m()`, `a.b.m()`: the member is the field, the
        // receiver everything before it — never the whole dotted text as
        // one name.
        if function.kind() == "field_expression"
            && let Some(field) = function.child_by_field_name("field")
        {
            let name = self.text(field).to_string();
            let (receiver, typed_receiver) = function
                .child_by_field_name("value")
                .map(|v| self.shaped_receiver(v))
                .unwrap_or_default();
            if !name.is_empty() {
                self.facts.references.push(RefRecord {
                    name,
                    receiver_hint: receiver,
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                    typed_receiver,
                    arity: self.call_arity(node),
                    ..Default::default()
                });
            }
            return;
        }
        let func = self.text(function).to_string();
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
            arity: self.call_arity(node),
            ..Default::default()
        });
    }

    /// The argument count of a call written with a parenthesised list
    /// (`f(a, b)`); a block argument (`f { … }`) is left unknown.
    fn call_arity(&self, node: Node) -> Option<u32> {
        node.child_by_field_name("arguments")
            .filter(|a| a.kind() == "arguments")
            .map(super::arg_count)
    }
}

/// Whether `node` (an identifier or a field access) stands where an
/// expression is evaluated: a block statement, an argument, an operand, a
/// definition's value or body, a branch, the receiver of a further access.
/// Names, patterns, parameters, types and the callee of a call with an
/// argument list are not.
fn paren_less_position(node: Node) -> bool {
    let Some(p) = node.parent() else {
        return false;
    };
    let is = |field: &str| p.child_by_field_name(field) == Some(node);
    match p.kind() {
        "block"
        | "arguments"
        | "parenthesized_expression"
        | "tuple_expression"
        | "return_expression"
        | "interpolation"
        | "prefix_expression" => true,
        "infix_expression" => is("left") || is("right"),
        "field_expression" => is("value"),
        "val_definition" | "var_definition" | "match_expression" => is("value"),
        "function_definition" | "lambda_expression" | "case_clause" => is("body"),
        "if_expression" => is("consequence") || is("alternative"),
        "assignment_expression" => is("right"),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    // WS20 gap 5: the `override` marker is recorded so dead-code can
    // cap an override of an out-of-tree base at medium.
    #[test]
    fn override_modifier_is_recorded() {
        let f = extract(
            "class A extends Ext {\n  override def hook(): Unit = {}\n  def plain(): Unit = {}\n}\n",
        );
        let marked = |n: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == n)
                .unwrap_or_else(|| panic!("{n} not extracted"))
                .attributes
                .iter()
                .any(|a| a == "override")
        };
        assert!(marked("hook"));
        assert!(!marked("plain"));
    }

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
    fn member_calls_split_receiver_and_paren_less_sites_are_marked() {
        // `b.m()` is `m` through `b`, never a name "b.m"; `b.size` and a
        // bare `helper` are paren-less sites; names, params and the
        // callee of a call with arguments are not.
        let f = extract(
            "object Main {\n  def go(b: B): Int = {\n    b.m()\n    b.size\n    helper\n    helper2()\n  }\n}\n",
        );
        let calls: Vec<(&str, &str, bool)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str(), r.paren_less))
            .collect();
        assert!(calls.contains(&("m", "b", false)), "{calls:?}");
        let g = extract("object O { def f() = new Auction(1).behavior(2) }\n");
        assert!(
            g.references
                .iter()
                .any(|r| r.name == "behavior" && r.receiver_hint == "Auction")
        );
        assert!(calls.contains(&("size", "b", true)), "{calls:?}");
        assert!(calls.contains(&("helper", "", true)), "{calls:?}");
        assert!(calls.contains(&("helper2", "", false)), "{calls:?}");
        assert!(!calls.iter().any(|(n, _, _)| n.contains('.')), "{calls:?}");
        // Near-miss: the definition name, the parameter and the callee
        // `helper2` are never paren-less sites.
        assert!(
            !calls
                .iter()
                .any(|(n, _, p)| *p && matches!(*n, "go" | "helper2" | "Main")),
            "{calls:?}"
        );
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

    /// C1: `f[Int]()` / `b.m[Int]()` are calls of `f` and of `m` through
    /// `b`, never names carrying `[Int]`.
    #[test]
    fn explicit_type_argument_calls_strip_the_arguments() {
        let f = extract(
            "object O {\n  def caller(b: B): Unit = {\n    genF[Int](1)\n    b.m[String](2)\n    genF(3)\n  }\n}\n",
        );
        let calls: Vec<(&str, &str)> = f
            .references
            .iter()
            .filter(|r| !r.paren_less)
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert_eq!(calls, vec![("genF", ""), ("m", "b"), ("genF", "")]);
    }

    /// WS16 (f): a supertype written with constructor arguments is still
    /// the class's base; calls carry their argument count (e).
    #[test]
    fn a_base_with_constructor_arguments_is_recorded() {
        let f = extract(
            "class Log(v: Int, p: String)\n  extends Compactible[Entry](v, spark, p) with Mixin {\n  def g = h(1, 2)\n}\n",
        );
        let t = f.type_spans.iter().find(|t| t.name == "Log").unwrap();
        assert_eq!(t.bases, vec!["Compactible[Entry]", "Mixin"]);
        let h = f.references.iter().find(|r| r.name == "h").unwrap();
        assert_eq!(h.arity, Some(2));
    }
}
