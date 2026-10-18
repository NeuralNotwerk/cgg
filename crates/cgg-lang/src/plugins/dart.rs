//! Dart plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct DartPlugin;

impl LanguagePlugin for DartPlugin {
    fn id(&self) -> &'static str {
        "dart"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".dart"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_dart::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "dart");
        let mut w = DartWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        // The package this file belongs to (`pubspec.yaml` `name:`), and
        // the directory it is rooted at: `package:<name>/x.dart` is
        // `<root>/lib/x.dart`.
        if let Some((name, root)) = super::manifest::dart_package(path) {
            facts.packages.push(name);
            facts.imports.push(ImportRecord {
                kind: "package-root".into(),
                path: root.to_string_lossy().into_owned(),
                alias: String::new(),
                site_line: 0,
                site_byte: 0,
            });
        }
        facts
    }
}

struct DartWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Supertypes of the enclosing class, innermost last.
    bases: Vec<Vec<String>>,
}

impl<'a> DartWalker<'a> {
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
        crate::closure_types::record("dart", node, self.source, self.facts);
        match node.kind() {
            // A mixin owns its members as a class does.
            "class_declaration" | "mixin_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // `extends A with M implements I`: superclass, mixins and
                // interfaces, which a call typed to any of them reaches.
                self.bases.push(super::attrs::base_types(node, self.source));
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
            // `double area();` in a class body: an abstract member (a
            // `declaration` holding a bare signature, no body). Recorded
            // so a call typed to the class binds to the declaration its
            // implementations are dispatched from; tagged `abstract` so
            // duck-typed fan-out prefers an implementation.
            "declaration"
                if !self.scope.is_empty()
                    && node.parent().is_some_and(|p| p.kind() == "class_member") =>
            {
                if let Some(sig) = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "function_signature")
                    && let Some(name) = sig
                        .child_by_field_name("name")
                        .map(|n| self.text(n).to_string())
                    && !name.is_empty()
                {
                    self.record_def(&name, node, DefVariant::TraitMethod);
                    if let Some(d) = self.facts.definitions.last_mut() {
                        d.attributes.push("abstract".to_string());
                    }
                    return;
                }
                self.walk_children(node);
                return;
            }
            // `var t = make()` / `final t = make()`: the bare call's result,
            // recorded as `make()` for type propagation to read the
            // callee's declared return type (WS23).
            "local_variable_declaration" => {
                let mut c = node.walk();
                for d in node.named_children(&mut c) {
                    if d.kind() != "initialized_variable_definition" {
                        continue;
                    }
                    if let (Some(name), Some(value)) = (
                        d.child_by_field_name("name"),
                        d.child_by_field_name("value"),
                    ) && value.kind() == "call_expression"
                        && let Some(callee) = value
                            .child_by_field_name("function")
                            .filter(|f| f.kind() == "identifier")
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
            "function_declaration" | "local_function_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .or_else(|| {
                        node.child_by_field_name("signature")
                            .and_then(|sig| sig.child_by_field_name("name"))
                    })
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            "method_declaration" => {
                // method_declaration -> signature: method_signature ->
                // function/getter/setter/operator/constructor signature,
                // then `body`. The callable spans the whole declaration,
                // so the calls in its body have it as their source.
                let name = node
                    .child_by_field_name("signature")
                    .and_then(|sig| self.member_signature_name(sig))
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::InherentMethod);
                }
                self.walk_children(node);
                return;
            }
            "function_signature" | "method_signature" => {
                // Only record if not inside a declaration (avoid
                // double-counting): neither directly, nor as the inner
                // signature of a `method_declaration`'s `method_signature`.
                let in_declaration = |p: Node| p.kind().ends_with("declaration");
                let inside = |p: Node| {
                    in_declaration(p)
                        || (p.kind() == "method_signature"
                            && p.parent().is_some_and(in_declaration))
                };
                if !node.parent().is_some_and(inside) {
                    let name = node
                        .child_by_field_name("name")
                        .or_else(|| {
                            (0..node.child_count())
                                .filter_map(|i| node.child(i as u32))
                                .find(|ch| ch.kind() == "identifier")
                        })
                        .map(|n| self.text(n).to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        let variant = if node.kind() == "method_signature" {
                            DefVariant::InherentMethod
                        } else {
                            DefVariant::FreeFunction
                        };
                        self.record_def(&name, node, variant);
                    }
                }
                self.walk_children(node);
                return;
            }
            "import_or_export" => {
                self.record_import(node);
                self.walk_children(node);
                return;
            }
            "part_directive" | "part_of_directive" | "library_name" => {
                self.record_directive(node);
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

    /// The member name a `method_signature` declares: the `name` of its
    /// function, getter or setter signature; the class name for an
    /// unnamed constructor and the suffix for a named one (`K.named`);
    /// `operator+` for an operator.
    fn member_signature_name(&self, sig: Node) -> Option<String> {
        let mut c = sig.walk();
        let inner: Vec<Node> = sig.named_children(&mut c).collect();
        for n in inner {
            match n.kind() {
                "function_signature" | "getter_signature" | "setter_signature" => {
                    return n
                        .child_by_field_name("name")
                        .map(|x| self.text(x).to_string());
                }
                "constructor_signature" | "factory_constructor_signature" => {
                    let mut c = n.walk();
                    return n
                        .children_by_field_name("name", &mut c)
                        .last()
                        .map(|x| self.text(x).to_string());
                }
                "operator_signature" => {
                    return n
                        .child_by_field_name("operator")
                        .map(|x| format!("operator{}", self.text(x)));
                }
                _ => {}
            }
        }
        None
    }

    /// `["@override"]` when a member carries Dart's `@override`
    /// annotation: a child of the `method_declaration`, or of the
    /// `class_member` holding a bodiless `declaration`. Only that marker
    /// is recorded; Dart does not declare attribute capture.
    fn override_marker(&self, node: Node) -> Vec<String> {
        let holders = [
            Some(node),
            node.parent().filter(|p| p.kind() == "class_member"),
        ];
        let marked = holders.into_iter().flatten().any(|h| {
            h.children(&mut h.walk())
                .any(|c| c.kind() == "annotation" && self.text(c).trim() == "@override")
        });
        if marked {
            vec!["@override".to_string()]
        } else {
            Vec::new()
        }
    }

    fn record_def(&mut self, name: &str, node: Node, variant: DefVariant) {
        let qn = self.qn(name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name.to_string(),
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: self.override_marker(node),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_import(&mut self, node: Node) {
        // tree-sitter-dart: import_or_export -> library_import ->
        // import_specification -> configurable_uri -> uri -> string_literal.
        // The literal text we want sits inside template_chars_* under
        // string_literal_*. Descend to find it.
        fn find_uri_text<'a>(n: tree_sitter::Node<'a>, src: &[u8]) -> Option<String> {
            let mut stack = vec![n];
            while let Some(cur) = stack.pop() {
                let kind = cur.kind();
                if kind.starts_with("template_chars_") {
                    return Some(cur.utf8_text(src).ok()?.to_string());
                }
                if kind == "uri" {
                    // Grab text and strip outer quotes/braces.
                    let raw = cur.utf8_text(src).ok()?.trim();
                    let stripped = raw
                        .trim_matches(|c: char| c == '\'' || c == '"' || c == '`')
                        .to_string();
                    if !stripped.is_empty() {
                        return Some(stripped);
                    }
                }
                let mut c = cur.walk();
                if c.goto_first_child() {
                    loop {
                        stack.push(c.node());
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            None
        }
        // Find an `as <ident>` sibling for the alias.
        fn find_alias<'a>(n: tree_sitter::Node<'a>, src: &[u8]) -> String {
            let mut stack = vec![n];
            while let Some(cur) = stack.pop() {
                let mut c = cur.walk();
                if !c.goto_first_child() {
                    continue;
                }
                let mut saw_as = false;
                loop {
                    let ch = c.node();
                    if ch.kind() == "as" {
                        saw_as = true;
                    } else if saw_as && ch.kind() == "identifier" {
                        return ch.utf8_text(src).unwrap_or("").to_string();
                    } else {
                        stack.push(ch);
                    }
                    if !c.goto_next_sibling() {
                        break;
                    }
                }
            }
            String::new()
        }

        let uri = find_uri_text(node, self.source).unwrap_or_default();
        if uri.is_empty() {
            return;
        }
        let alias = find_alias(node, self.source);
        // `export 'x.dart'` re-exports a library's names; it brings none
        // into this file's own scope.
        let is_export = node
            .named_child(0)
            .is_some_and(|c| c.kind() == "library_export");
        let site_line = (node.start_position().row as u32) + 1;
        let site_byte = node.start_byte() as u32;
        self.facts.imports.push(ImportRecord {
            kind: if is_export { "export" } else { "import" }.into(),
            path: uri.clone(),
            alias,
            site_line,
            site_byte,
        });
        // `show a, b` / `hide c`: one record each, keyed to the directive
        // by its URI and position.
        let mut stack = vec![node];
        let mut combinators = Vec::new();
        while let Some(cur) = stack.pop() {
            if cur.kind() == "combinator" {
                combinators.push(cur);
                continue;
            }
            let mut c = cur.walk();
            for ch in cur.children(&mut c) {
                stack.push(ch);
            }
        }
        combinators.sort_by_key(|c| c.start_byte());
        for comb in combinators {
            let mut c = comb.walk();
            let kids: Vec<Node> = comb.children(&mut c).collect();
            let Some(kw) = kids.first().map(|k| k.kind()) else {
                continue;
            };
            if kw != "show" && kw != "hide" {
                continue;
            }
            let names: Vec<&str> = kids
                .iter()
                .filter(|k| k.kind() == "identifier")
                .map(|k| self.text(*k))
                .collect();
            self.facts.imports.push(ImportRecord {
                kind: kw.into(),
                path: uri.clone(),
                alias: names.join(","),
                site_line,
                site_byte,
            });
        }
    }

    /// `part 'x.dart';`, `part of 'lib.dart';` / `part of a.b;`,
    /// `library a.b;`: which files form one library, whose top-level
    /// names every part sees.
    fn record_directive(&mut self, node: Node) {
        let mut uri = String::new();
        let mut dotted = String::new();
        let mut stack = vec![node];
        while let Some(cur) = stack.pop() {
            match cur.kind() {
                k if k.starts_with("template_chars_") => {
                    uri = self.text(cur).to_string();
                    continue;
                }
                "dotted_identifier_list" => {
                    dotted = self.text(cur).split_whitespace().collect();
                    continue;
                }
                _ => {}
            }
            let mut c = cur.walk();
            for ch in cur.children(&mut c) {
                stack.push(ch);
            }
        }
        let (kind, path) = match node.kind() {
            "part_directive" => ("part", uri),
            "part_of_directive" if !uri.is_empty() => ("part-of", uri),
            "part_of_directive" => ("part-of-library", dotted),
            _ => ("library", dotted),
        };
        if path.is_empty() {
            return;
        }
        self.facts.imports.push(ImportRecord {
            kind: kind.into(),
            path,
            alias: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_call(&mut self, node: Node) {
        // call_expression layout:
        //   call_expression -> [identifier | member_expression] arguments
        //   member_expression -> [identifier | super | call_expression] . identifier
        let callee = node.child(0);
        let Some(mut callee) = callee else { return };
        // `f<int>()` / `b.m<T>()`: explicit type arguments wrap the callee
        // in an `instantiation_expression`; the call is of what it wraps.
        if callee.kind() == "instantiation_expression" {
            let Some(inner) = callee.child_by_field_name("function") else {
                return;
            };
            callee = inner;
        }
        let mut typed_receiver = false;
        let (name, receiver) = match callee.kind() {
            "identifier" => (self.text(callee).to_string(), String::new()),
            // `x?.m()` is `x.m()` when `x` is not null.
            "member_expression" | "null_aware_member_expression" => {
                // Last identifier child is the called name; preceding text is the receiver.
                let mut last_ident: Option<Node> = None;
                let mut first_recv: Option<Node> = None;
                let mut c = callee.walk();
                if c.goto_first_child() {
                    loop {
                        let n = c.node();
                        if n.kind() == "identifier" {
                            if first_recv.is_none() && last_ident.is_none() {
                                // identifier in receiver position
                            }
                            last_ident = Some(n);
                        } else if matches!(n.kind(), "super" | "this" | "call_expression")
                            && first_recv.is_none()
                        {
                            first_recv = Some(n);
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                let name_node = last_ident;
                let recv_node = first_recv
                    .or_else(|| callee.child(0))
                    .map(closure_body_receiver);
                let name = name_node
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // `(f).m()`, `(o as Foo).m()`, `Foo().m()`, `(await f).m()`:
                // the value or the type the receiver's shape stands for
                // (`crate::receiver`).
                let mut receiver = recv_node
                    .map(|n| {
                        let p = crate::receiver::peel("dart", n, self.source);
                        match p.type_name() {
                            Some(t) => {
                                typed_receiver = true;
                                t
                            }
                            None => self.text(p.node).to_string(),
                        }
                    })
                    .unwrap_or_default();
                // If receiver == name (only one identifier was found), receiver is empty.
                if name == receiver {
                    receiver.clear();
                }
                (name, receiver)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }

        self.facts.references.push(RefRecord {
            name,
            receiver_hint: receiver,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver,
            ..Default::default()
        });
    }
}

/// The grammar reads `(x) => x.run()` as `((x) => x).run()`: the member
/// access binds to the closure, whose expression body is just `x`. The
/// call is made on that body expression, inside the closure — where `x`
/// is the closure's parameter. Any other receiver is itself.
fn closure_body_receiver(recv: Node) -> Node {
    if recv.kind() == "function_expression"
        && let Some(body) = recv.child_by_field_name("body")
        && body.kind() == "function_expression_body"
        && let Some(expr) = body.named_child(0)
        && expr.kind() != "block"
    {
        return expr;
    }
    recv
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
            "class A extends Ext {\n  @override\n  void hook() {}\n  @override\n  int get g => 1;\n  void plain() {}\n}\n",
        );
        let marked = |n: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == n)
                .unwrap_or_else(|| panic!("{n} not extracted"))
                .attributes
                .iter()
                .any(|a| a == "@override")
        };
        assert!(marked("hook"));
        assert!(marked("g"), "an overriding getter");
        assert!(!marked("plain"));
    }

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_dart::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        DartPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.dart"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn plugin_loads() {
        let plugin = DartPlugin;
        assert_eq!(plugin.id(), "dart");
        assert!(plugin.extensions().contains(&".dart"));
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
    fn extracts_definitions() {
        let src = "class Service {\n  void run() {}\n}\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn a_method_is_qualified_by_its_class() {
        // Bare `run` would collide with every other `run` in the tree;
        // the class scope is what makes the graph addressable.
        let f = extract("class Service {\n  void run() {}\n}\n");
        assert!(
            defs(&f)
                .iter()
                .any(|d| d.contains("Service") && d.ends_with("run")),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn a_method_spans_its_body_and_is_recorded_once() {
        // The method's name sits one level down (method_signature ->
        // function_signature); recording only the inner signature left
        // every call in the body without a source.
        let src = "class K {\n  K(this.x) {\n    a();\n  }\n  int get v => b();\n  void run() {\n    c();\n  }\n  int x;\n}\n";
        let f = extract(src);
        let run: Vec<_> = f
            .definitions
            .iter()
            .filter(|d| d.qualified_name == "K::run")
            .collect();
        assert_eq!(run.len(), 1, "{:?}", defs(&f));
        assert_eq!((run[0].start_line, run[0].end_line), (6, 8));
        // Constructor and getter bodies are callables too.
        assert!(defs(&f).contains(&"K::K".to_string()), "{:?}", defs(&f));
        assert!(defs(&f).contains(&"K::v".to_string()), "{:?}", defs(&f));
        // A bodiless abstract member still records once, as before.
        let f = extract("abstract class A {\n  void m();\n}\n");
        assert_eq!(defs(&f), vec!["A::m".to_string()]);
    }

    #[test]
    fn a_top_level_function_is_captured() {
        let f = extract("void main() {}\n");
        assert!(
            f.definitions.iter().any(|d| d.simple_name == "main"),
            "{:?}",
            defs(&f)
        );
    }

    #[test]
    fn extracts_references() {
        // Was a no-op assertion (`let _ = f.references`), so a plugin
        // that stopped recording calls entirely still passed.
        let f = extract("void main() { greet(); }\n");
        assert!(
            refs(&f).iter().any(|r| r == "greet"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn a_method_call_on_a_receiver_is_recorded() {
        let f = extract("void main() { svc.run(); }\n");
        assert!(
            refs(&f).iter().any(|r| r == "run" || r == "svc.run"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn a_call_in_an_expression_bodied_closure_is_on_the_body_value() {
        let f = extract(
            "void f(List<Foo> xs) {\n  xs.forEach((x) => x.run());\n  xs.forEach((Foo y) => y!.stop());\n  z!.go();\n  ((x) => x).call();\n}\n",
        );
        let pairs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(pairs.contains(&("run", "x")), "{pairs:?}");
        assert!(pairs.contains(&("stop", "y")), "{pairs:?}");
        assert!(pairs.contains(&("go", "z")), "{pairs:?}");
        assert!(
            f.scoped_types
                .iter()
                .any(|s| s.name == "y" && s.type_name == "Foo"),
            "{:?}",
            f.scoped_types
        );
    }

    #[test]
    fn an_import_records_its_uri() {
        let f = extract("import 'package:foo/bar.dart';\nvoid main() {}\n");
        assert!(
            f.imports.iter().any(|i| i.path.contains("bar.dart")),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn an_aliased_import_keeps_the_alias() {
        let f = extract("import 'package:foo/bar.dart' as fb;\nvoid main() {}\n");
        let i = f
            .imports
            .iter()
            .find(|i| i.path.contains("bar.dart"))
            .unwrap_or_else(|| panic!("imports: {:?}", f.imports));
        assert_eq!(
            i.alias, "fb",
            "the `as` alias must survive: {:?}",
            f.imports
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
        let f = extract("class Broken {\n  void run( {\n");
        let _ = defs(&f);
    }

    /// WS14: an `export` is not an import, `show`/`hide` follow their
    /// directive, `part`/`part of` tie a library together.
    #[test]
    fn dart_directives_are_told_apart() {
        let f = extract(
            "library a;\nimport 'x.dart' as p show f, g hide h;\nexport 'y.dart' show k;\n\
             part 'z.dart';\n",
        );
        let got: Vec<(&str, &str, &str)> = f
            .imports
            .iter()
            .filter(|i| i.kind != "package-root")
            .map(|i| (i.kind.as_str(), i.path.as_str(), i.alias.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("library", "a", ""),
                ("import", "x.dart", "p"),
                ("show", "x.dart", "f,g"),
                ("hide", "x.dart", "h"),
                ("export", "y.dart", ""),
                ("show", "y.dart", "k"),
                ("part", "z.dart", ""),
            ]
        );
        let part = extract("part of 'lib.dart';\n");
        assert!(
            part.imports
                .iter()
                .any(|i| i.kind == "part-of" && i.path == "lib.dart")
        );
    }

    #[test]
    fn abstract_members_are_declarations_and_supertypes_are_bases() {
        let f = extract(
            "abstract class Shape { double area(); final int n = 1; }\nclass Sq extends Shape with M implements I { double area() => 1; }\nmixin M { void mm() {} }\n",
        );
        let decl = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Shape::area")
            .expect("abstract member");
        assert_eq!(decl.variant, DefVariant::TraitMethod);
        assert!(decl.attributes.iter().any(|a| a == "abstract"));
        let imp = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "Sq::area")
            .expect("implementation");
        assert_eq!(imp.base_types, vec!["Shape", "M", "I"]);
        assert!(f.definitions.iter().any(|d| d.qualified_name == "M::mm"));
        assert!(f.definitions.iter().all(|d| d.simple_name != "n"));
    }

    /// C1: `f<int>()` / `b.m<T>()` wrap the callee in an
    /// `instantiation_expression`; they are calls of `f` and `b.m`.
    #[test]
    fn explicit_type_argument_calls_are_recorded() {
        let f = extract(
            "void caller(B b) {\n  f<int>(1);\n  b.m<String>('a');\n  g(2);\n}\n",
        );
        let calls: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(calls.contains(&("f", "")), "{calls:?}");
        assert!(calls.contains(&("m", "b")), "{calls:?}");
        assert!(calls.contains(&("g", "")), "{calls:?}");
        assert!(!calls.iter().any(|(n, _)| n.contains('<')), "{calls:?}");
    }
}
