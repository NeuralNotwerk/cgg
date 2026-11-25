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
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            value_refs: true,
            ..Default::default()
        }
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_julia::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "julia");
        let mut w = JuliaWalker {
            ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
            in_signature: false,
        };
        w.walk(tree.root_node());
        facts
    }
}

struct JuliaWalker<'a> {
    ctx: &'a crate::ExtractCtx<'a>,
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Inside a function signature: its parameters are no values.
    in_signature: bool,
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
        if self.ctx.reference_sites && !self.in_signature {
            self.capture_value_positions(node);
        }
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
                // The signature `foo(x)` parses as a call to `foo`: walking
                // it as one recorded every function as calling itself.
                // Only its arguments (default values) are walked.
                let mut c = node.walk();
                for child in node.children(&mut c) {
                    if child.kind() == "signature" {
                        self.walk_signature(child);
                    } else {
                        self.walk(child);
                    }
                }
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
                if let Some(lhs) = node.named_child(0).map(short_form_head)
                    && lhs.kind() == "call_expression"
                    && let Some(name_node) = lhs.named_child(0)
                    && (name_node.kind() == "identifier"
                        || (name_node.kind() == "field_expression"
                            && is_dotted_name(name_node)))
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
                if let Some(callee) = node.named_child(0) {
                    let context = self.text(callee).to_string();
                    self.capture_registrar(node, &context);
                }
                self.walk_children(node);
                return;
            }
            // `@get "/x" function(req) … end`, `@get("/y", handler)`: a
            // macro registering a handler (Oxygen's routes, `@cron`).
            "macrocall_expression" => {
                if let Some(m) = node
                    .named_child(0)
                    .filter(|m| m.kind() == "macro_identifier")
                {
                    let context = self.text(m).trim_start_matches('@').to_string();
                    self.capture_registrar(node, &context);
                }
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    /// Functions used as values, when the run records reference sites: a
    /// call's or macro's argument (positional or keyword: `ODEProblem(f!,
    /// u0)`, `solve(p; callback = cb)`), the right side of `=`, a
    /// `return`'s value, vector and tuple elements and a pair's value
    /// (`"a" => f`). Recorded as the call would be — `f` bare, `M.f`
    /// whole — marked `value_ref`. Names a local or parameter binds are
    /// dropped through the Julia locals (`value_refs::prune`).
    fn capture_value_positions(&mut self, node: Node) {
        let mut values: Vec<Node> = Vec::new();
        match node.kind() {
            "argument_list" | "macro_argument_list" => {
                // A do-block's or an anonymous function's parameter list
                // binds names; it holds no values.
                if node.parent().is_some_and(|p| {
                    matches!(
                        p.kind(),
                        "do_clause" | "signature" | "arrow_function_expression"
                    )
                }) {
                    return;
                }
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "named_argument" {
                        let n = ch.named_child_count();
                        if n >= 2
                            && let Some(v) = ch.named_child(n as u32 - 1)
                        {
                            values.push(v);
                        }
                    } else {
                        values.push(ch);
                    }
                }
            }
            "assignment" => {
                // `f(x) = …` defines; only a plain binding's value counts.
                if node
                    .named_child(0)
                    .map(short_form_head)
                    .is_some_and(|l| l.kind() == "call_expression")
                {
                    return;
                }
                let n = node.named_child_count();
                if n >= 2
                    && let Some(v) = node.named_child(n as u32 - 1)
                {
                    values.push(v);
                }
            }
            "return_statement" | "vector_expression" | "tuple_expression" => {
                let mut c = node.walk();
                values.extend(node.named_children(&mut c));
            }
            "binary_expression" => {
                if node
                    .child_by_field_name("operator")
                    .or_else(|| node.child(1))
                    .is_some_and(|o| self.text(o) == "=>")
                    && let Some(v) = node.named_child(node.named_child_count() as u32 - 1)
                {
                    values.push(v);
                }
            }
            _ => return,
        }
        for v in values {
            if let Some(name) = self.value_name(v) {
                self.facts.references.push(RefRecord {
                    name,
                    receiver_hint: String::new(),
                    site_line: (v.start_position().row as u32) + 1,
                    site_byte: v.start_byte() as u32,
                    value_ref: true,
                    ..Default::default()
                });
            }
        }
    }

    /// The function a value position names: a bare identifier or a dotted
    /// path (`M.f`), written as a call to it would be.
    fn value_name(&self, v: Node) -> Option<String> {
        let ok = match v.kind() {
            "identifier" => true,
            "field_expression" => is_dotted_name(v),
            _ => false,
        };
        if !ok {
            return None;
        }
        let t = self.text(v).trim();
        // `nothing`, `true`, `missing` and friends are no functions.
        if t.is_empty() || matches!(t, "nothing" | "missing" | "true" | "false" | "end") {
            return None;
        }
        Some(t.to_string())
    }

    /// Capture the argument slots of a registration-shaped call or macro —
    /// shapes B and C of the framework design: `route("/p", handler)`,
    /// `HTTP.register!(r, "GET", "/p", handler)`, `@get "/x" function(req)
    /// … end`, `route("/w") do … end`. Gated on the registrar verb; the
    /// rule engine gates every record on detected imports.
    fn capture_registrar(&mut self, node: Node, context: &str) {
        let verb = super::registrar::last_segment(context);
        if !self.ctx.is_registrar_verb(verb) {
            return;
        }
        let mut items: Vec<Node> = Vec::new();
        let mut do_clause: Option<Node> = None;
        let mut c = node.walk();
        for ch in node.named_children(&mut c) {
            match ch.kind() {
                "argument_list" | "macro_argument_list" => {
                    let mut c2 = ch.walk();
                    for a in ch.named_children(&mut c2) {
                        if a.kind() == "named_argument" {
                            let n = a.named_child_count();
                            if n >= 2
                                && let Some(v) = a.named_child(n as u32 - 1)
                            {
                                items.push(v);
                            }
                        } else {
                            items.push(a);
                        }
                    }
                }
                "do_clause" => do_clause = Some(ch),
                _ => {}
            }
        }
        // The route: the leading literals, joined (`GET /p`), else the
        // first number (`@repeat 5 …`).
        let strings: Vec<String> = items
            .iter()
            .filter(|n| n.kind() == "string_literal")
            .map(|n| super::registrar::unquote(self.text(*n)))
            .filter(|s| !s.is_empty())
            .take(2)
            .collect();
        let route = if strings.is_empty() {
            items
                .iter()
                .find(|n| matches!(n.kind(), "integer_literal" | "float_literal"))
                .map(|n| self.text(*n).to_string())
                .unwrap_or_default()
        } else {
            strings.join(" ")
        };
        let mut out: Vec<RefRecord> = Vec::new();
        for arg in &items {
            if let Some(name) = self.value_name(*arg) {
                out.push(RefRecord {
                    name,
                    receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                    site_line: (arg.start_position().row as u32) + 1,
                    site_byte: arg.start_byte() as u32,
                    context: context.to_string(),
                    route: route.clone(),
                    ..Default::default()
                });
            }
        }
        self.facts.references.extend(out);
        if route.is_empty() {
            return;
        }
        let closures = items
            .iter()
            .copied()
            .filter(|n| is_anonymous_function(*n))
            .chain(do_clause);
        for arg in closures.collect::<Vec<_>>() {
            let line = (arg.start_position().row as u32) + 1;
            let simple = format!("handler_at_{line}");
            let qn = self.qn(&simple);
            self.facts.definitions.push(DefRecord {
                simple_name: simple.clone(),
                qualified_name: qn,
                variant: DefVariant::NamedClosure,
                start_line: line,
                end_line: (arg.end_position().row as u32) + 1,
                start_byte: arg.start_byte() as u32,
                end_byte: arg.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(arg)),
                visibility: String::new(),
                attributes: vec!["synthetic".to_string()],
                ..Default::default()
            });
            self.facts.references.push(RefRecord {
                name: simple,
                receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                site_line: line,
                site_byte: arg.start_byte() as u32,
                context: context.to_string(),
                route: route.clone(),
                ..Default::default()
            });
        }
    }

    /// Walk a function signature without its head: the outermost call
    /// (`foo(x = g())`) contributes its arguments, never itself.
    fn walk_signature(&mut self, node: Node) {
        let outer = self.in_signature;
        self.in_signature = true;
        self.walk_signature_inner(node);
        self.in_signature = outer;
    }

    fn walk_signature_inner(&mut self, node: Node) {
        if node.kind() == "call_expression" {
            for i in 1..node.child_count() {
                if let Some(ch) = node.child(i as u32) {
                    self.walk(ch);
                }
            }
            return;
        }
        let mut c = node.walk();
        for child in node.children(&mut c) {
            self.walk_signature_inner(child);
        }
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
        if let Some(mut func_node) = node.child(0) {
            // `Foo{Int}(x)`: the callee without its type parameters.
            if func_node.kind() == "parametrized_type_expression"
                && let Some(head) = func_node.named_child(0)
            {
                func_node = head;
            }
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

/// Whether a `field_expression` is a plain dotted name (`M.f`, `A.B.f`),
/// not a field of a call result or an indexed value.
fn is_dotted_name(n: Node) -> bool {
    match n.kind() {
        "identifier" => true,
        "field_expression" => {
            let mut c = n.walk();
            n.named_children(&mut c).all(is_dotted_name)
        }
        _ => false,
    }
}

/// An anonymous function written as an argument: `function(req) … end`,
/// `req -> …`.
fn is_anonymous_function(n: Node) -> bool {
    match n.kind() {
        "arrow_function_expression" => true,
        "function_definition" => !n.named_children(&mut n.walk()).any(|c| {
            c.kind() == "signature"
                && c.named_child(0)
                    .is_some_and(|h| h.kind() != "argument_list")
        }),
        _ => false,
    }
}

/// The signature of a short-form method under its type parameters or
/// return type: `f(x::T) where {T} = …` and `f(x)::Int = …` are the
/// methods `f(x::T)` and `f(x)`. Anything else is returned unchanged.
pub(crate) fn short_form_head(lhs: Node) -> Node {
    let mut n = lhs;
    for _ in 0..4 {
        match n.kind() {
            "where_expression" | "typed_expression" => match n.named_child(0) {
                Some(inner) => n = inner,
                None => break,
            },
            _ => break,
        }
    }
    n
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

    fn extract_with_refs(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_julia::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        JuliaPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(true),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.jl"),
            &tree,
            src.as_bytes(),
        )
    }

    fn value_refs(f: &FileFacts) -> Vec<String> {
        f.references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| r.name.clone())
            .collect()
    }

    #[test]
    fn functions_passed_as_values_are_reference_sites() {
        let src = "function lorenz!(du, u, p, t) end\n\
                   prob = ODEProblem(lorenz!, u0, tspan; callback = cb, f = M.g)\n\
                   x = handler\n\
                   y = [f1, f2]\n\
                   z = (\"a\" => h, k)\n\
                   function mk(q)\n  return q2\nend\n";
        let got = value_refs(&extract_with_refs(src));
        for want in [
            "lorenz!", "cb", "M.g", "handler", "f1", "f2", "h", "k", "q2",
        ] {
            assert!(got.iter().any(|g| g == want), "{want}: {got:?}");
        }
        // Parameters are bindings, not values.
        for not in ["du", "q", "ODEProblem", "mk"] {
            assert!(!got.iter().any(|g| g == not), "{not}: {got:?}");
        }
        assert!(
            value_refs(&extract(src)).is_empty(),
            "off without reference sites"
        );
    }

    #[test]
    fn a_qualified_short_form_method_is_a_definition() {
        let f = extract("Logging.shouldlog(l::MyLogger, args...) = true\n");
        assert_eq!(defs(&f), vec!["Logging.shouldlog"]);
        assert!(refs(&f).is_empty(), "{:?}", refs(&f));
    }

    #[test]
    fn route_registrations_capture_handlers_and_inline_closures() {
        let src = "HTTP.register!(r, \"GET\", \"/p\", serve_asset)\n\
                   route(\"/z\", hz)\n\
                   route(\"/w\") do\n  1\nend\n\
                   @get \"/x\" function(req) 1 end\n\
                   @post(\"/y\", hy)\n";
        let f = extract(src);
        let reg: Vec<(String, String, String)> = f
            .references
            .iter()
            .filter(|r| r.receiver_hint == cgg_core::VALUE_REF_HINT)
            .map(|r| (r.name.clone(), r.context.clone(), r.route.clone()))
            .collect();
        let has = |n: &str, c: &str, rt: &str| {
            reg.iter()
                .any(|(a, b, d)| a.starts_with(n) && b == c && d == rt)
        };
        assert!(has("serve_asset", "HTTP.register!", "GET /p"), "{reg:?}");
        assert!(has("hz", "route", "/z"), "{reg:?}");
        assert!(has("handler_at_3", "route", "/w"), "{reg:?}");
        assert!(has("handler_at_6", "get", "/x"), "{reg:?}");
        assert!(has("hy", "post", "/y"), "{reg:?}");
        assert!(
            f.definitions
                .iter()
                .filter(|d| d.simple_name.starts_with("handler_at_"))
                .count()
                == 2,
            "{:?}",
            defs(&f)
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
    fn a_function_signature_is_not_a_call_to_itself() {
        let f = extract("function foo(x = bar())\n    baz()\nend\n");
        let r = refs(&f);
        assert!(!r.contains(&"foo".to_string()), "{r:?}");
        // Default values and the body are still walked.
        assert!(r.contains(&"bar".to_string()), "{r:?}");
        assert!(r.contains(&"baz".to_string()), "{r:?}");
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

    /// C1: `Foo{Int}(x)` is a call of `Foo`; a short-form method with
    /// type parameters or a return type is a definition, and its head is
    /// not a call to itself.
    #[test]
    fn parametric_calls_and_short_form_methods() {
        let f = extract(
            "f(::Type{T}, x) where {T} = x\nh(x)::Int = x\nfunction g(x)\n    Foo{Int}(x)\n    Mod.Bar{T}(x)\n    f(Int, x)\nend\n",
        );
        let d = defs(&f);
        assert!(d.iter().any(|q| q.ends_with('f')), "{d:?}");
        assert!(d.iter().any(|q| q.ends_with('h')), "{d:?}");
        let r = refs(&f);
        assert!(r.contains(&"Foo".to_string()), "{r:?}");
        assert!(r.contains(&"Mod.Bar".to_string()), "{r:?}");
        assert_eq!(r.iter().filter(|n| *n == "f").count(), 1, "{r:?}");
        assert!(!r.iter().any(|n| n == "h" || n.contains('{')), "{r:?}");
        // Near-miss: a typed variable assignment is not a method.
        let v = extract("x::Int = g(1)\n");
        assert!(defs(&v).is_empty(), "{:?}", defs(&v));
    }
}
