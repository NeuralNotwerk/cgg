//! Clojure plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ClojurePlugin;

impl LanguagePlugin for ClojurePlugin {
    fn id(&self) -> &'static str {
        "clojure"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".clj", ".cljs", ".cljc", ".edn"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_clojure_orchard::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "clojure");
        let mut w = ClojureWalker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            namespace: String::new(),
            locals: Vec::new(),
            in_quote: 0,
            referred: Vec::new(),
            refer_all: false,
        };
        w.walk(tree.root_node());
        if ctx.reference_sites {
            let (referred, refer_all) = (std::mem::take(&mut w.referred), w.refer_all);
            keep_reachable_values(&mut facts, &referred, refer_all);
        }
        facts
    }
}

struct ClojureWalker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    namespace: String,
    /// Under `reference_sites`: the names bound by the enclosing `let`,
    /// `fn` parameters, `doseq` … (innermost last). Clojure has no locals
    /// spec, so the shadow filter for value references lives here.
    locals: Vec<String>,
    /// Depth of quoted data (`'(a b)`), where a symbol is not a value.
    in_quote: u32,
    /// Names the `ns` form `:refer`s, and whether it refers or `:use`s a
    /// whole namespace (then any bare name may come from it).
    referred: Vec<String>,
    refer_all: bool,
}

impl<'a> ClojureWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    /// A symbol as written, without its metadata (`^:private f` is `f`,
    /// `^String a/b` is `a/b`); any other node's text.
    fn sym_text(&self, n: Node) -> String {
        if n.kind() != "sym_lit" {
            return self.text(n).to_string();
        }
        let Some(name) = n.child_by_field_name("name") else {
            return self.text(n).to_string();
        };
        match n.child_by_field_name("namespace") {
            Some(ns) => format!("{}/{}", self.text(ns), self.text(name)),
            None => self.text(name).to_string(),
        }
    }

    fn qn(&self, simple: &str) -> String {
        if self.namespace.is_empty() {
            simple.to_string()
        } else {
            format!("{}/{simple}", self.namespace)
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "list_lit" => {
                if self.is_ns_form(node) {
                    self.extract_namespace(node);
                } else if self.is_defn_form(node) {
                    self.record_callable(node);
                } else {
                    self.record_call(node);
                }
                if self.ctx.reference_sites {
                    let mark = self.locals.len();
                    let quoted = self.head_name(node) == Some("quote");
                    self.in_quote += u32::from(quoted);
                    self.enter_form(node);
                    self.walk_children(node);
                    self.in_quote -= u32::from(quoted);
                    self.locals.truncate(mark);
                } else {
                    self.walk_children(node);
                }
            }
            "quoting_lit" if self.ctx.reference_sites => {
                self.in_quote += 1;
                self.walk_children(node);
                self.in_quote -= 1;
            }
            // `#'f`, `#'ns/f`: the var itself, never a local.
            "var_quoting_lit" if self.ctx.reference_sites => {
                if let Some(sym) = forms(node).into_iter().find(|n| n.kind() == "sym_lit")
                {
                    self.push_value(sym, false);
                }
            }
            _ => self.walk_children(node),
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

    fn is_ns_form(&self, node: Node) -> bool {
        if let Some(first) = forms(node).first().copied() {
            self.text(first) == "ns"
        } else {
            false
        }
    }

    fn is_defn_form(&self, node: Node) -> bool {
        if let Some(first) = forms(node).first().copied() {
            let text = self.text(first);
            matches!(
                text,
                "defn"
                    | "defn-"
                    | "defmacro"
                    | "defmacro-"
                    | "def"
                    | "defonce"
                    | "defprotocol"
                    | "definterface"
                    | "deftype"
                    | "defrecord"
                    | "defstruct"
                    | "defmulti"
                    | "defmethod"
            )
        } else {
            false
        }
    }

    fn extract_namespace(&mut self, node: Node) {
        // (ns name (:require [foo.bar :as fb] [baz]) (:use [qux]) ...)
        if let Some(name_node) = forms(node).get(1).copied() {
            // `(ns ^{:added "1.0"} a.b …)`: the metadata is not part of
            // the namespace's name.
            let name_node = name_node.child_by_field_name("name").unwrap_or(name_node);
            self.namespace = self.text(name_node).to_string();
            // Each top-level form after the namespace name is its own
            // `list_lit` like `(:require ...)`. Walk siblings, look for
            // those that start with `:require` / `:use` / `:import`.
            let mut c = node.walk();
            if c.goto_first_child() {
                let _ = c.goto_next_sibling(); // past `(`
                let _ = c.goto_next_sibling(); // past `ns`
                let _ = c.goto_next_sibling(); // past name
                loop {
                    let n = c.node();
                    if n.kind() == "list_lit" {
                        // First inner child after `(` is the keyword.
                        let kw = n
                            .named_child(0)
                            .map(|k| self.text(k).to_string())
                            .unwrap_or_default();
                        if kw == ":require" || kw == ":use" || kw == ":import" {
                            // The rest of the form is a sequence of vec_lits / sym_lits.
                            for i in 1..n.named_child_count() {
                                if let Some(entry) = n.named_child(i as u32) {
                                    self.record_require_entry(entry, &kw);
                                }
                            }
                        }
                    }
                    if !c.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }

    fn record_require_entry(&mut self, node: Node, kw: &str) {
        // Entry is either `sym_lit "foo.bar"` or `vec_lit "[foo.bar :as fb]"`.
        let kind = match kw {
            ":use" => "use",
            ":import" => "import",
            _ => "require",
        };
        let (path, alias) = match node.kind() {
            "sym_lit" => (self.text(node).to_string(), String::new()),
            "vec_lit" => {
                let mut path = String::new();
                let mut alias = String::new();
                let mut seen_as = false;
                let mut refers_all = false;
                let mut refer_names: Vec<String> = Vec::new();
                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        let ch = c.node();
                        if ch.kind() == "sym_lit" {
                            let t = self.text(ch).to_string();
                            if seen_as {
                                alias = t;
                                seen_as = false;
                            } else if path.is_empty() {
                                path = t;
                            }
                        } else if ch.kind() == "kwd_lit" && self.text(ch) == ":as" {
                            seen_as = true;
                        } else if ch.kind() == "kwd_lit" && self.text(ch) == ":refer" {
                            // `:refer [a b]` / `:refer :all`: the names the
                            // file may use bare (a value reference's scope).
                            if let Some(next) = ch.next_named_sibling() {
                                match next.kind() {
                                    "vec_lit" => {
                                        for n in forms(next) {
                                            if n.kind() == "sym_lit" {
                                                let name =
                                                    self.sym_parts(n).1.to_string();
                                                refer_names.push(name.clone());
                                                self.referred.push(name);
                                            }
                                        }
                                    }
                                    _ => refers_all = true,
                                }
                            }
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                if refers_all {
                    self.refer_all_from(&path);
                }
                // `:refer [f]` / `:refer :all`: the bare names the file
                // reaches in that namespace (`refer` / `refer-all`).
                let site = node.start_byte() as u32;
                let line = (node.start_position().row as u32) + 1;
                if !path.is_empty() {
                    for name in refer_names {
                        self.facts.imports.push(ImportRecord {
                            kind: "refer".into(),
                            path: path.clone(),
                            alias: name,
                            site_line: line,
                            site_byte: site,
                        });
                    }
                    if refers_all {
                        self.facts.imports.push(ImportRecord {
                            kind: "refer-all".into(),
                            path: path.clone(),
                            alias: String::new(),
                            site_line: line,
                            site_byte: site,
                        });
                    }
                }
                (path, alias)
            }
            _ => return,
        };
        if path.is_empty() {
            return;
        }
        if kind == "use" {
            self.refer_all_from(&path);
        }
        self.facts.imports.push(ImportRecord {
            kind: kind.into(),
            path,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_callable(&mut self, node: Node) {
        // (defn name [...] ...) — `(defn ^:private name …)`, `^{…}`,
        // `#^Tag` and stacked metadata belong to the symbol node but are
        // not part of the name (`sym_text`), nor is metadata on the form.
        if let Some(name_node) = forms(node).get(1).copied() {
            // `(defmulti ^:private ~n …)`: a template's unquoted name.
            if name_node.kind() == "unquoting_lit" {
                return;
            }
            let name = self.sym_text(name_node);
            // `(defn ~n …)` / `(defn ~(symbol …) …)` inside a syntax-quote:
            // a template whose name the macro computes, not a definition.
            if name.is_empty() || name.starts_with('~') {
                return;
            }

            let qn = self.qn(&name);
            let variant = DefVariant::FreeFunction;
            let (sl, el) = (
                (node.start_position().row as u32) + 1,
                (node.end_position().row as u32) + 1,
            );
            self.facts.definitions.push(DefRecord {
                simple_name: name,
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
    }

    fn record_call(&mut self, node: Node) {
        // (func_name ...)
        // `^String (f x)`: the type hint is the form's metadata, not its
        // operator.
        if let Some(func_node) = forms(node).first().copied()
            && func_node.kind() == "sym_lit"
        {
            let name = self.sym_text(func_node);
            if name.is_empty() || name.starts_with(':') {
                return;
            }

            // The rule engine matches `registrars` against the last
            // dotted segment of the context, and it does not split
            // on `/` — so a namespace-qualified operator like
            // `compojure.core/GET` has to be reduced to `GET` here
            // or no rule could ever see it.
            let verb = super::registrar::last_segment(&name).to_string();

            self.facts.references.push(RefRecord {
                name,
                receiver_hint: String::new(),
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
                ..Default::default()
            });

            self.record_registrar(node, &verb);
        }
    }

    /// Capture the argument slots of a Compojure/Ring registration form.
    ///
    /// `(GET "/users" [] list-users)` hands control to `list-users`
    /// without calling it, which is exactly what
    /// `super::registrar::capture` exists to record. That helper cannot
    /// be used here: it locates arguments through `arguments_of`, which
    /// wants a distinct argument-list node, and Clojure has none — a
    /// call is a `list_lit` whose operator and operands are siblings
    /// under the same parent. So the same shapes are read off the
    /// s-expression directly, emitting the same
    /// `VALUE_REF_HINT`/`STRING_REF_HINT` records with the same
    /// `context`/`route` slots.
    ///
    /// Like that helper this is deliberately over-eager and gated only
    /// on the verb; the framework rule engine decides whether a record
    /// means anything, and an unmatched one is inert.
    fn record_registrar(&mut self, node: Node, verb: &str) {
        if !self.ctx.is_registrar_verb(verb) {
            return;
        }
        // Everything after the operator symbol.
        let elems: Vec<Node> = forms(node).into_iter().skip(1).collect();
        if elems.is_empty() {
            return;
        }

        // Only a *leading* string is a route. `(get headers "accept")`
        // shares the verb `get` with every HTTP router in the table, and
        // treating its second argument as an identity would make Clojure's
        // single most common map lookup look like a route registration.
        let route = match elems.first() {
            Some(n) if n.kind() == "str_lit" => super::registrar::unquote(self.text(*n)),
            _ => String::new(),
        };

        // Compojure's arity is `(VERB path binding & body)`. The slot
        // right after the path destructures the request — `req`, `[]`,
        // `{:keys [id]}` — and never names a handler, so claiming it
        // would bind every route to whatever definition happens to
        // share the binding's name.
        let binding_pos = (!route.is_empty()).then_some(1usize);
        // Without a route, a verb that is also a `clojure.core` function
        // is that function: `(get table k)` reads a map, and `table` is no
        // handler. (`(defroutes app handler)` keeps its captures.)
        let core_call = route.is_empty()
            && cgg_core::stdlib::stdlib_names("clojure")
                .is_some_and(|s| s.contains(verb));

        let mut out: Vec<RefRecord> = Vec::new();
        let mut body: Vec<Node> = Vec::new();
        let mut named_target = false;

        for (i, el) in elems.iter().enumerate() {
            let line = (el.start_position().row as u32) + 1;
            let byte = el.start_byte() as u32;
            match el.kind() {
                "str_lit" => {
                    // The leading string is the route itself, already
                    // carried in `route`; a later one may name the
                    // target. A lone string is both at once, so it is
                    // kept — the same rule `registrar::capture` applies.
                    if i == 0 && !route.is_empty() && elems.len() > 1 {
                        continue;
                    }
                    let s = super::registrar::unquote(self.text(*el));
                    if s.is_empty() {
                        continue;
                    }
                    out.push(RefRecord {
                        arg_type: None,
                        from_macro_arg: false,
                        path_receiver: false,
                        dot_call: false,
                        arrow: false,
                        macro_call: false,
                        invocant: false,
                        implicit_receiver: false,
                        not_def: None,
                        arity: None,
                        value_ref: false,
                        paren_less: false,
                        typed_receiver: false,
                        weak_receiver_type: false,
                        name: s,
                        receiver_hint: cgg_core::STRING_REF_HINT.to_string(),
                        site_line: line,
                        site_byte: byte,
                        context: verb.to_string(),
                        route: route.clone(),
                        kwargs: Vec::new(),
                    });
                }
                "sym_lit" => {
                    if Some(i) == binding_pos || core_call {
                        continue;
                    }
                    let t = super::registrar::last_segment(self.text(*el))
                        .trim_start_matches(['\'', '#', '@']);
                    if t.is_empty()
                        || !t.starts_with(|c: char| c.is_alphabetic() || c == '_')
                    {
                        continue;
                    }
                    named_target = true;
                    out.push(RefRecord {
                        arg_type: None,
                        from_macro_arg: false,
                        path_receiver: false,
                        dot_call: false,
                        arrow: false,
                        macro_call: false,
                        invocant: false,
                        implicit_receiver: false,
                        not_def: None,
                        arity: None,
                        value_ref: false,
                        paren_less: false,
                        typed_receiver: false,
                        weak_receiver_type: false,
                        name: t.to_string(),
                        receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                        site_line: line,
                        site_byte: byte,
                        context: verb.to_string(),
                        route: route.clone(),
                        kwargs: Vec::new(),
                    });
                }
                "list_lit" | "anon_fn_lit" if Some(i) != binding_pos => {
                    body.push(*el);
                }
                // A vector or map in a route form is data — the binding
                // vector, a middleware stack, a Reitit route table. None
                // of them is a handler reference.
                _ => {}
            }
        }

        self.synthesize_inline_handler(&route, named_target, &body, verb, &mut out);
        self.facts.references.extend(out);
    }

    /// Name the body of a route registration so it can be a handler.
    ///
    /// Compojure's dominant idiom writes the handler in place —
    /// `(GET "/" [] (home-page))` — so there is no callable for a rule
    /// to mark and the whole framework enumerates nothing. Worse, the
    /// body usually sits inside `(defroutes …)`, which is not a
    /// definition either, so the calls it makes have no owner at all.
    /// Mirrors `ruby.rs::extract_inline_handler` and is gated the same
    /// way — on the registrar verb and a leading route string, and only
    /// when no argument already names a handler — so `(wrap-defaults
    /// app-routes site-defaults)` and every ordinary form mint nothing.
    fn synthesize_inline_handler(
        &mut self,
        route: &str,
        named_target: bool,
        body: &[Node],
        verb: &str,
        out: &mut Vec<RefRecord>,
    ) {
        if route.is_empty() || named_target || body.is_empty() {
            return;
        }
        let first = body[0];
        let last = body[body.len() - 1];
        let line = (first.start_position().row as u32) + 1;
        let simple = format!("handler_at_{line}");
        let qn = self.qn(&simple);
        self.facts.definitions.push(DefRecord {
            simple_name: simple.clone(),
            qualified_name: qn,
            variant: DefVariant::NamedClosure,
            start_line: line,
            end_line: (last.end_position().row as u32) + 1,
            start_byte: first.start_byte() as u32,
            end_byte: last.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(first)),
            visibility: String::new(),
            attributes: vec!["synthetic".to_string()],
            ..Default::default()
        });
        out.push(RefRecord {
            arg_type: None,
            from_macro_arg: false,
            path_receiver: false,
            dot_call: false,
            arrow: false,
            macro_call: false,
            invocant: false,
            implicit_receiver: false,
            not_def: None,
            arity: None,
            value_ref: false,
            paren_less: false,
            typed_receiver: false,
            weak_receiver_type: false,
            name: simple,
            receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
            site_line: line,
            site_byte: first.start_byte() as u32,
            context: verb.to_string(),
            route: route.to_string(),
            kwargs: Vec::new(),
        });
    }
}

/// The forms of a collection or list: its named children, less metadata,
/// comments and `#_` discards.
fn forms(node: Node) -> Vec<Node> {
    let mut c = node.walk();
    node.named_children(&mut c)
        .filter(|n| {
            !matches!(
                n.kind(),
                "meta_lit" | "old_meta_lit" | "comment" | "dis_expr"
            )
        })
        .collect()
}

/// A bare symbol names a var of its own namespace or one the `ns` form
/// refers (`:refer [f]`, `:refer :all`, `:use`); it never reaches
/// another namespace's function by name, so a bare value reference to
/// neither is a local cgg cannot see (a library macro's binding) and is
/// dropped. `alias/f` is kept: it resolves through the alias.
fn keep_reachable_values(facts: &mut FileFacts, referred: &[String], refer_all: bool) {
    if refer_all {
        return;
    }
    let own: std::collections::HashSet<&str> = facts
        .definitions
        .iter()
        .map(|d| d.simple_name.as_str())
        .collect();
    let keep: Vec<bool> = facts
        .references
        .iter()
        .map(|r| {
            !r.value_ref
                || r.name.contains('/') && r.name.len() > 1
                || own.contains(r.name.as_str())
                || referred.contains(&r.name)
        })
        .collect();
    let mut i = 0;
    facts.references.retain(|_| {
        i += 1;
        keep[i - 1]
    });
}

/// WS26: function values. Recorded only under `reference_sites`.
impl ClojureWalker<'_> {
    /// `(:require [x :refer :all])` / `(:use x)`: any bare name may come
    /// from `x` — unless `x` is the standard library (`clojure.test`),
    /// which defines none of the tree's functions.
    fn refer_all_from(&mut self, ns: &str) {
        if !cgg_core::stdlib::stdlib_modules("clojure").contains(&ns) {
            self.refer_all = true;
        }
    }

    /// `(ns, name)` of a symbol, read without its metadata.
    fn sym_parts(&self, sym: Node) -> (&str, &str) {
        let name = sym
            .child_by_field_name("name")
            .map_or_else(|| self.text(sym), |n| self.text(n));
        let ns = sym
            .child_by_field_name("namespace")
            .map_or("", |n| self.text(n));
        (ns, name)
    }

    /// The operator's name, without namespace (`clojure.core/let` is `let`).
    fn head_name(&self, list: Node) -> Option<&str> {
        let head = *forms(list).first()?;
        (head.kind() == "sym_lit").then(|| self.sym_parts(head).1)
    }

    /// A symbol in value position: a reference to the function it names,
    /// unless a local binds the name (`check_locals`).
    fn push_value(&mut self, sym: Node, check_locals: bool) {
        if self.in_quote > 0 {
            return;
        }
        let (ns, name) = self.sym_parts(sym);
        // A Java class or a namespace (`java.util.Date`, `clojure.string`)
        // is not a function; nor are `&`, `_` or a reader literal.
        if name.is_empty()
            || name == "&"
            || name == "_"
            || name.starts_with(['%', '.'])
            || (ns.is_empty() && name.contains('.'))
            || name.ends_with('.')
        {
            return;
        }
        if check_locals && ns.is_empty() && self.locals.iter().any(|l| l == name) {
            return;
        }
        let full = if ns.is_empty() {
            name.to_string()
        } else {
            format!("{ns}/{name}")
        };
        self.facts.references.push(RefRecord {
            name: full,
            site_line: (sym.start_position().row as u32) + 1,
            site_byte: sym.start_byte() as u32,
            value_ref: true,
            ..Default::default()
        });
    }

    /// A form in value position: a symbol, or the elements of a vector or
    /// the values of a map holding one (`{:handler f}`, `[f g]`). Lists
    /// are calls and are walked on their own.
    fn value_form(&mut self, n: Node) {
        match n.kind() {
            "sym_lit" => self.push_value(n, true),
            "vec_lit" => {
                for e in forms(n) {
                    self.value_form(e);
                }
            }
            "map_lit" => {
                for (i, e) in forms(n).into_iter().enumerate() {
                    if i % 2 == 1 {
                        self.value_form(e);
                    }
                }
            }
            _ => {}
        }
    }

    fn value_forms(&mut self, items: &[Node]) {
        for n in items {
            self.value_form(*n);
        }
    }

    /// Every symbol of a destructuring pattern (`x`, `[a & more]`,
    /// `{:keys [f] :as m}`) is a local.
    fn bind_pattern(&mut self, n: Node) {
        if n.kind() == "sym_lit" {
            let name = self.sym_parts(n).1;
            if name != "&" {
                self.locals.push(name.to_string());
            }
            return;
        }
        for e in forms(n) {
            self.bind_pattern(e);
        }
    }

    /// `[pattern init …]`: each init is a value, then its pattern binds.
    /// `:let [..]` in `for`/`doseq` nests another such vector.
    fn bind_pairs(&mut self, vec: Node) {
        let items = forms(vec);
        for pair in items.chunks(2) {
            let pat = pair[0];
            let init = pair.get(1).copied();
            if pat.kind() == "kwd_lit" {
                if self.text(pat) == ":let"
                    && let Some(v) = init.filter(|v| v.kind() == "vec_lit")
                {
                    self.bind_pairs(v);
                }
                continue;
            }
            if let Some(init) = init {
                self.value_form(init);
            }
            self.bind_pattern(pat);
        }
    }

    /// The parameter vector(s) from `items[start..]`: one vector, or the
    /// arity lists `([x] …) ([x y] …)`; a docstring or attribute map before
    /// them is skipped.
    /// Returns the index of the first body form after a single parameter
    /// vector (arity lists carry their own bodies).
    fn bind_params(&mut self, items: &[Node], start: usize) -> Option<usize> {
        for (i, n) in items.iter().enumerate().skip(start) {
            match n.kind() {
                "str_lit" | "map_lit" => continue,
                // A schema annotation (`(mu/defn f :- :int [x] …)`): the
                // keyword and the schema after it.
                "kwd_lit" if self.text(*n) == ":-" => continue,
                _ if i > start
                    && items.get(i - 1).is_some_and(|p| self.text(*p) == ":-") =>
                {
                    continue;
                }
                "vec_lit" => {
                    self.bind_pattern(*n);
                    return Some(i + 1);
                }
                "list_lit" => {
                    if let Some(p) = forms(*n).first().filter(|p| p.kind() == "vec_lit") {
                        self.bind_pattern(*p);
                    } else {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        None
    }

    /// A method or `letfn` body (`(m [this x] …)`) rather than a call.
    fn is_fn_spec(&self, list: Node) -> bool {
        let Some(p) = list.parent() else {
            return false;
        };
        let owner = if p.kind() == "vec_lit" {
            match p.parent() {
                Some(g) if g.kind() == "list_lit" => g,
                _ => return false,
            }
        } else if p.kind() == "list_lit" {
            p
        } else {
            return false;
        };
        let Some(h) = self.head_name(owner) else {
            return false;
        };
        if !cgg_core::stdlib::CLOJURE_METHOD_BODY_FORMS.contains(&h) {
            return false;
        }
        if p.kind() == "vec_lit" && h != "letfn" {
            return false;
        }
        forms(list)
            .get(1)
            .is_some_and(|n| matches!(n.kind(), "vec_lit" | "list_lit"))
    }

    /// Bind what a form binds over its body and record the function
    /// values among its arguments.
    fn enter_form(&mut self, list: Node) {
        use cgg_core::stdlib as std_t;
        let items = forms(list);
        if self.is_fn_spec(list) {
            if let Some(body) = self.bind_params(&items, 1) {
                self.value_forms(&items[body..]);
            }
            return;
        }
        let Some(head) = items.first().copied() else {
            return;
        };
        let h = if head.kind() == "sym_lit" {
            self.sym_parts(head).1.to_string()
        } else {
            String::new()
        };
        let h = h.as_str();
        let first_vec = items.get(1).copied().filter(|v| v.kind() == "vec_lit");
        if let Some(&(_, skip)) = std_t::CLOJURE_FN_FORMS.iter().find(|(f, _)| *f == h) {
            let mut start = 1 + skip;
            // `(fn name [x] …)` binds its own name too.
            if skip == 0 && items.get(1).is_some_and(|n| n.kind() == "sym_lit") {
                self.bind_pattern(items[1]);
                start += 1;
            }
            // The body's forms are values too: a function returning a
            // handler or a route table (`(defn routes [] [["/" {:get h}]])`).
            if let Some(body) = self.bind_params(&items, start) {
                self.value_forms(&items[body..]);
            }
        } else if h == "letfn" {
            if let Some(v) = first_vec {
                for spec in forms(v) {
                    if let Some(name) =
                        forms(spec).first().filter(|n| n.kind() == "sym_lit")
                    {
                        self.bind_pattern(*name);
                    }
                }
            }
        } else if std_t::CLOJURE_BINDING_FORMS.contains(&h) {
            if let Some(v) = first_vec {
                self.bind_pairs(v);
                self.value_forms(&items[2..]);
            }
        } else if let Some(v) = first_vec.filter(|_| {
            std_t::CLOJURE_BINDING_FORM_PREFIXES
                .iter()
                .any(|p| h.starts_with(p))
        }) {
            self.bind_pattern(v);
        } else if std_t::CLOJURE_TYPE_FIELD_FORMS.contains(&h) {
            if let Some(fields) = items.get(2).filter(|v| v.kind() == "vec_lit") {
                self.bind_pattern(*fields);
            }
        } else if std_t::CLOJURE_DEF_FORMS.contains(&h) {
            if items.len() >= 3
                && let Some(last) = items.last()
            {
                self.value_form(*last);
            }
        } else if !(std_t::CLOJURE_NON_VALUE_FORMS.contains(&h)
            || std_t::CLOJURE_METHOD_BODY_FORMS.contains(&h)
            || head.kind() == "kwd_lit")
        {
            // A call, or a threading step (`(-> x f)`): every argument
            // in value position.
            self.value_forms(&items[1..]);
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
        p.set_language(&tree_sitter_clojure_orchard::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        ClojurePlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/x.clj"),
            &tree,
            src.as_bytes(),
        )
    }

    fn defs(f: &FileFacts) -> Vec<String> {
        f.definitions
            .iter()
            .map(|d| d.qualified_name.clone())
            .collect()
    }

    #[test]
    fn plugin_loads() {
        let p = ClojurePlugin;
        assert_eq!(p.id(), "clojure");
        assert_eq!(p.extensions(), &[".clj", ".cljs", ".cljc", ".edn"]);
        assert!(p.shebangs().is_empty());
    }

    #[test]
    fn defn_is_a_callable() {
        let f = extract("(defn greet [n] (str \"hi \" n))\n");
        assert!(
            f.definitions.iter().any(|d| d.simple_name == "greet"),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn every_def_form_is_recognised() {
        // The plugin claims twelve `def*` heads; a head that silently
        // stops matching turns its whole construct invisible.
        let src = concat!(
            "(defn a [] 1)\n",
            "(defn- b [] 1)\n",
            "(defmacro c [] 1)\n",
            "(def d 1)\n",
            "(defonce e 1)\n",
            "(defprotocol f)\n",
            "(definterface g)\n",
            "(deftype h [x])\n",
            "(defrecord i [x])\n",
            "(defstruct j)\n",
            "(defmulti k identity)\n",
            "(defmethod l :x [_] 1)\n",
        );
        let f = extract(src);
        for name in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"] {
            assert!(
                f.definitions.iter().any(|d| d.simple_name == name),
                "`{name}` was not captured; defs: {:?}",
                defs(&f)
            );
        }
    }

    /// A11: a `defn` inside a syntax-quote whose name is unquoted is a
    /// template the macro fills in, not a definition.
    #[test]
    fn unquoted_defn_names_are_not_definitions() {
        let f = extract(
            "(ns app.gen)\n(defmacro defgetter [n]\n  `(defn ~(symbol (str \"get-\" n)) [] ~(str n)))\n(defmacro defh [n & body] `(defn ~n [] ~@body))\n(defmacro defm [n] `(defmulti ^:private ~n identity))\n(defn real [] 1)\n",
        );
        assert_eq!(
            defs(&f),
            vec![
                "app.gen/defgetter",
                "app.gen/defh",
                "app.gen/defm",
                "app.gen/real"
            ]
        );
    }

    #[test]
    fn the_namespace_qualifies_every_definition() {
        let f = extract("(ns my.app)\n(defn handler [] 1)\n");
        assert!(
            defs(&f).contains(&"my.app/handler".to_string()),
            "definitions must be namespace-qualified: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn without_a_namespace_the_bare_name_is_used() {
        let f = extract("(defn handler [] 1)\n");
        assert!(defs(&f).contains(&"handler".to_string()), "{:?}", defs(&f));
    }

    #[test]
    fn require_with_an_alias_records_path_and_alias() {
        let f = extract("(ns my.app\n  (:require [foo.bar :as fb]))\n");
        let i = f
            .imports
            .iter()
            .find(|i| i.path == "foo.bar")
            .unwrap_or_else(|| panic!("imports: {:?}", f.imports));
        assert_eq!(i.kind, "require");
        assert_eq!(i.alias, "fb", "the `:as` alias must survive");
    }

    #[test]
    fn a_bare_require_vector_has_no_alias() {
        let f = extract("(ns my.app\n  (:require [baz]))\n");
        let i = f
            .imports
            .iter()
            .find(|i| i.path == "baz")
            .expect("baz imported");
        assert_eq!(i.kind, "require");
        assert!(i.alias.is_empty());
    }

    #[test]
    fn use_and_import_keep_their_own_kinds() {
        // `:use` and `:import` are different relationships from
        // `:require`; collapsing them would lose that in the audit.
        let f = extract("(ns my.app\n  (:use [qux])\n  (:import [java.util Date]))\n");
        assert!(
            f.imports.iter().any(|i| i.path == "qux" && i.kind == "use"),
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
    fn calls_inside_a_body_are_references() {
        let f = extract("(ns my.app)\n(defn outer [] (inner 1))\n(defn inner [x] x)\n");
        assert!(
            f.references.iter().any(|r| r.name == "inner"),
            "refs: {:?}",
            f.references.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn keywords_in_call_position_are_not_references() {
        // `(:key m)` is a map lookup, not a call to a function named
        // `:key`; recording it would invent a callee that cannot exist.
        let f = extract("(defn f [m] (:some-key m))\n");
        assert!(
            !f.references.iter().any(|r| r.name.starts_with(':')),
            "keyword lookups must not become references: {:?}",
            f.references.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn definition_spans_cover_the_whole_form() {
        // Spans drive `--since` seeding and dead-code line reporting, so
        // a multi-line form has to report its real extent.
        let f = extract("(defn multi\n  [x]\n  (+ x\n     1))\n");
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "multi")
            .unwrap();
        assert_eq!(d.start_line, 1);
        assert!(
            d.end_line >= 4,
            "end_line was {} for a 4-line form",
            d.end_line
        );
        assert!(d.end_byte > d.start_byte);
    }

    #[test]
    fn an_empty_file_yields_nothing_and_does_not_panic() {
        let f = extract("");
        assert!(f.definitions.is_empty());
        assert!(f.references.is_empty());
        assert!(f.imports.is_empty());
    }

    #[test]
    fn unbalanced_source_does_not_panic() {
        // tree-sitter yields an ERROR tree; extraction must survive it.
        let f = extract("(defn broken [x]\n  (+ x\n");
        let _ = defs(&f);
    }

    fn hinted<'a>(f: &'a FileFacts, hint: &str) -> Vec<&'a RefRecord> {
        f.references
            .iter()
            .filter(|r| r.receiver_hint == hint)
            .collect()
    }

    #[test]
    fn a_compojure_route_captures_its_named_handler_and_path() {
        // `(GET "/users" [] list-users)` hands control to `list-users`
        // without calling it; without the capture the framework rule
        // engine has nothing to match and the route enumerates nothing.
        let f = extract("(ns app)\n(GET \"/users\" [] list-users)\n");
        let v = hinted(&f, cgg_core::VALUE_REF_HINT);
        let r = v
            .iter()
            .find(|r| r.name == "list-users")
            .unwrap_or_else(|| panic!("value refs: {v:?}"));
        assert_eq!(r.context, "GET", "the operator symbol is the context");
        assert_eq!(r.route, "/users", "the leading string is the route");
    }

    #[test]
    fn the_request_binding_slot_is_not_a_handler() {
        // Compojure is `(VERB path binding & body)`. Claiming `req`
        // would bind the route to any definition sharing that name.
        let f = extract("(ns app)\n(POST \"/users\" req (create-user req))\n");
        assert!(
            !hinted(&f, cgg_core::VALUE_REF_HINT)
                .iter()
                .any(|r| r.name == "req"),
            "refs: {:?}",
            hinted(&f, cgg_core::VALUE_REF_HINT)
        );
    }

    #[test]
    fn an_inline_route_body_becomes_a_named_handler() {
        // The dominant Compojure idiom writes the handler in place, and
        // it usually sits inside `(defroutes …)` — not a definition —
        // so without this the body has no callable at all.
        let f = extract("(ns app)\n(GET \"/\" [] (home-page))\n");
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name.starts_with("handler_at_"))
            .unwrap_or_else(|| panic!("defs: {:?}", defs(&f)));
        assert_eq!(d.variant, DefVariant::NamedClosure);
        assert!(d.attributes.iter().any(|a| a == "synthetic"));
        assert!(
            hinted(&f, cgg_core::VALUE_REF_HINT)
                .iter()
                .any(|r| r.name == d.simple_name && r.route == "/"),
            "the synthetic handler needs a reference naming it"
        );
    }

    #[test]
    fn a_named_handler_suppresses_the_synthetic_one() {
        let f = extract("(ns app)\n(GET \"/\" [] home-handler)\n");
        assert!(
            !f.definitions
                .iter()
                .any(|d| d.simple_name.starts_with("handler_at_")),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn a_non_registrar_form_captures_nothing() {
        // The gate is what keeps this off the hot path: an ordinary
        // call must not pay for an argument scan or contribute records.
        let f = extract("(ns app)\n(defn f [] (my-helper a b \"lit\"))\n");
        assert!(
            f.references.iter().all(|r| r.receiver_hint.is_empty()),
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn a_map_lookup_does_not_mint_a_handler() {
        // `get` is a route verb in every HTTP rule table and also
        // Clojure's most common function. Only a *leading* string is a
        // route, so `(get (headers r) "accept")` stays a map lookup
        // instead of becoming a route with a synthesized handler.
        let f = extract("(ns app)\n(defn f [r] (get (headers r) \"accept\"))\n");
        assert!(
            !f.definitions
                .iter()
                .any(|d| d.simple_name.starts_with("handler_at_")),
            "defs: {:?}",
            defs(&f)
        );
    }

    /// `(get table k)` without a route is `clojure.core/get`: `table` is a
    /// map, not a handler.
    #[test]
    fn a_core_get_without_a_route_captures_no_handler() {
        let f = extract("(ns app)\n(defn f [k] (get table k))\n");
        assert!(
            hinted(&f, cgg_core::VALUE_REF_HINT).is_empty(),
            "refs: {:?}",
            hinted(&f, cgg_core::VALUE_REF_HINT)
        );
    }

    #[test]
    fn a_namespaced_operator_reduces_to_its_verb() {
        // The rule engine splits the context on `.`/`:`/`-`/`>` and not
        // on `/`, so `compojure.core/GET` has to arrive as `GET`.
        let f = extract("(ns app)\n(compojure.core/GET \"/x\" [] show)\n");
        assert!(
            hinted(&f, cgg_core::VALUE_REF_HINT)
                .iter()
                .any(|r| r.name == "show" && r.context == "GET"),
            "refs: {:?}",
            hinted(&f, cgg_core::VALUE_REF_HINT)
        );
    }

    fn extract_with_refs(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_clojure_orchard::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        ClojurePlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(true),
            FileId::new(0),
            &PathBuf::from("/tmp/x.clj"),
            &tree,
            src.as_bytes(),
        )
    }

    fn value_names(f: &FileFacts) -> Vec<&str> {
        f.references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| r.name.as_str())
            .collect()
    }

    /// WS26: symbols passed to a function, threaded, var-quoted or held
    /// in a map or vector name functions.
    #[test]
    fn function_values_are_value_references() {
        let f = extract_with_refs(
            "(ns app (:require [a.util :as u :refer [step norm]]))\n\
             (def state (atom 0))\n(defn update-fn [x] x)\n(defn wrap [x] x)\n(defn f1 [x] x)\n\
             (defn f2 [x] x)\n(defn key-fn [x] x)\n(defn handler [x] x)\n(defn on-req [x] x)\n\
             (defn r1 [x] x)\n(defn r2 [x] x)\n(defn a-fn [x] x)\n\
             (defn run [xs]\n\
             \x20 (map step xs)\n\
             \x20 (reduce u/merge {} xs)\n\
             \x20 (swap! state update-fn)\n\
             \x20 (partial wrap 1)\n\
             \x20 (comp f1 f2)\n\
             \x20 (-> xs norm (sort-by key-fn))\n\
             \x20 #'handler\n\
             \x20 #'u/hook\n\
             \x20 {:handler on-req :routes [r1 r2]})\n\
             (def table [a-fn])\n",
        );
        let mut got = value_names(&f);
        got.sort_unstable();
        let mut want = vec![
            "step",
            "xs",
            "u/merge",
            "xs",
            "state",
            "update-fn",
            "wrap",
            "f1",
            "f2",
            "xs",
            "norm",
            "key-fn",
            "handler",
            "u/hook",
            "on-req",
            "r1",
            "r2",
            "a-fn",
        ];
        want.retain(|n| *n != "xs");
        want.sort_unstable();
        assert_eq!(got, want);
    }

    /// Locals bound by `let`, `fn`/`defn` parameters, `loop`, `doseq`,
    /// `for` (with `:let`), `letfn`, destructuring and `with-*` vectors
    /// are not functions; quoted data and `case` constants are not
    /// values; and nothing is recorded without `reference_sites`.
    #[test]
    fn locals_and_data_are_not_function_values() {
        let f = extract_with_refs(
            "(defn g [{:keys [a]} [b & more] ^String c]\n\
             \x20 (let [d (inc a) {e :e} a] (map d [a b c e more]))\n\
             \x20 (fn self [h] (map h (map self [])))\n\
             \x20 (loop [i 0] (recur (inc i)))\n\
             \x20 (doseq [x xs :let [y x]] (run! y x))\n\
             \x20 (letfn [(k [z] (map z []))] (map k []))\n\
             \x20 (with-temp [Model m {:a 1}] (use-it m))\n\
             \x20 (case v foo 1 bar 2)\n\
             \x20 (quote (map nope []))\n\
             \x20 '(map nope2 []))\n\
             (deftype T [field] P (meth [this q] (map q [field this])))\n\
             (mu/defn sch :- [:map {:x 1}] [p :- :int] (map p []))\n(defn p [x] x)\n\
             (def xs [])\n(defn a [x] x)\n(defn d [x] x)\n(defn h [x] x)\n(defn field [x] x)\n\
             (defn run! [f x] x)\n(defn k [x] x)\n(defn m [x] x)\n(defn nope [x] x)\n\
             (defn self [x] x)\n(defn y [x] x)\n(defn q [x] x)\n(defn z [x] x)\n(defn more [x] x)\n",
        );
        assert_eq!(value_names(&f), vec!["xs"]);
        // A bare name neither defined here nor referred is a local of a
        // macro cgg cannot read, never another namespace's function.
        let f = extract_with_refs(
            "(ns b (:require [c :as c]))\n(defn g [] (map elsewhere []))\n",
        );
        assert!(value_names(&f).is_empty());
        let off = extract("(defn r [] (map step []) #'h)\n");
        assert!(value_names(&off).is_empty());
    }

    /// Metadata on a definition's name (`^:private`, `^{…}`, `#^Tag`,
    /// stacked) is not part of the name, for every def form.
    #[test]
    fn name_metadata_is_not_part_of_any_definition_name() {
        let cases = [
            ("(defn ^:private f [x] x)", "f"),
            ("(defn- ^:deprecated f [x] x)", "f"),
            ("(defn ^{:arglists '([x])} f [x] x)", "f"),
            ("(defn #^String f [x] x)", "f"),
            ("(defn ^:private ^:deprecated ^String f [x] x)", "f"),
            ("(defmacro ^:private m [x] x)", "m"),
            ("(defmulti ^:private mm (fn [x] x))", "mm"),
            ("(defmethod ^:private mm :a [x] x)", "mm"),
            ("(defmethod ^:private other/mm :a [x] x)", "other/mm"),
            ("(defprotocol ^:private P (pm [this]))", "P"),
            ("(definterface ^:private I (im []))", "I"),
            ("(defrecord ^:private R [a])", "R"),
            ("(deftype ^{:private true} T [a])", "T"),
            ("(defstruct ^:private S :a)", "S"),
            ("(def ^:private d 1)", "d"),
            ("(def ^{:doc \"x\" :private true} d 1)", "d"),
            ("(defonce ^:private o 1)", "o"),
            ("^:x (defn k [] 1)", "k"),
        ];
        for (src, want) in cases {
            let f = extract(&format!("(ns a.b)\n{src}\n"));
            assert_eq!(
                f.definitions
                    .iter()
                    .map(|d| d.simple_name.as_str())
                    .collect::<Vec<_>>(),
                vec![want],
                "{src}"
            );
            assert_eq!(
                f.definitions[0].qualified_name,
                format!("a.b/{want}"),
                "{src}"
            );
        }
    }

    /// A type hint on a call form (`^String (f x)`) is its metadata, not
    /// its operator.
    #[test]
    fn a_hinted_call_is_still_a_call() {
        let f = extract("(defn g [s] ^String (f s) #^Foo (h s) (^:once k s))\n");
        let names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["f", "h", "k"]);
    }

    /// WS16: namespace metadata is not part of the namespace's name.
    #[test]
    fn namespace_metadata_is_not_part_of_the_name() {
        let f = extract("(ns ^{:added \"0.46.0\"} a.util.sql-2)\n(defn format [x] x)\n");
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(qns, vec!["a.util.sql-2/format"]);
    }
}
