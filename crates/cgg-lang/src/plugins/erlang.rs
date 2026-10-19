//! Erlang plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ErlangPlugin;

impl LanguagePlugin for ErlangPlugin {
    fn id(&self) -> &'static str {
        "erlang"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".erl", ".hrl"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["escript"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_erlang::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "erlang");
        let mut w = ErlangWalker {
            reference_sites: ctx.reference_sites,
            source,
            facts: &mut facts,
            module: String::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct ErlangWalker<'a> {
    /// Record function values; see `crate::ExtractCtx::reference_sites`.
    reference_sites: bool,
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
}

impl<'a> ErlangWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.module.is_empty() {
            simple.to_string()
        } else {
            format!("{}:{simple}", self.module)
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "module_attribute" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            "import_attribute" => {
                self.record_import(node);
                self.walk_children(node);
            }
            // `-include_lib("eunit/include/eunit.hrl").` is how an Erlang
            // module pulls in a library's macros — for eunit, the only
            // evidence the module is a test suite.
            "pp_include_lib" | "pp_include" => {
                if let Some(file) = node.child_by_field_name("file") {
                    let path = self.text(file).trim_matches('"').to_string();
                    if !path.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: if node.kind() == "pp_include_lib" {
                                "include_lib"
                            } else {
                                "include"
                            }
                            .to_string(),
                            path,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                self.walk_children(node);
            }
            // `-behaviour(gen_server).` is how Erlang declares that a
            // module implements an OTP contract, and it is the ONLY
            // evidence of it — the callbacks are ordinary exported
            // functions. Without recording it no Erlang framework rule
            // can detect anything, since `-import` is rare in practice.
            "attribute" | "behaviour_attribute" => {
                self.record_behaviour(node);
                self.walk_children(node);
            }
            "function_clause" | "function" => {
                self.record_function(node);
                self.walk_children(node);
            }
            "call" => {
                self.record_call(node);
                self.walk_children(node);
            }
            // `fun f/1`: a local function value, of that arity.
            "internal_fun" if self.reference_sites => {
                if let (Some(f), Some(a)) = (
                    node.child_by_field_name("fun")
                        .filter(|f| f.kind() == "atom"),
                    self.arity_of(node),
                ) {
                    let name = self.text(f).to_string();
                    self.push_value_ref(f, name, String::new(), Some(a));
                }
                self.walk_children(node);
            }
            // `fun m:f/2`: a remote function value.
            "external_fun" if self.reference_sites => {
                let module = node
                    .child_by_field_name("module")
                    .and_then(|m| m.child_by_field_name("name").or(Some(m)))
                    .filter(|m| m.kind() == "atom");
                if let (Some(m), Some(f), Some(a)) = (
                    module,
                    node.child_by_field_name("fun")
                        .filter(|f| f.kind() == "atom"),
                    self.arity_of(node),
                ) {
                    let m = self.text(m).to_string();
                    let name = format!("{m}:{}", self.text(f));
                    self.push_value_ref(f, name, m, Some(a));
                }
                self.walk_children(node);
            }
            // `{M, F}` / `{M, F, A}` of atoms: a callback named by module
            // and function (kept only when `M` is a module of the tree that
            // declares `F`, `value_refs::prune`).
            "tuple" if self.reference_sites => {
                self.record_mfa_tuple(node);
                self.walk_children(node);
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

    fn extract_module(&mut self, node: Node) {
        // -module(name).
        // Find the atom child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && child.kind() == "atom"
            {
                self.module = self.text(child).to_string();
                break;
            }
        }
    }

    fn record_import(&mut self, node: Node) {
        // -import(module, [func/arity, ...]).
        let mut module_name = String::new();
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && child.kind() == "atom"
                && module_name.is_empty()
            {
                module_name = self.text(child).to_string();
                break;
            }
        }
        if !module_name.is_empty() {
            self.facts.imports.push(ImportRecord {
                kind: "import".to_string(),
                path: module_name,
                alias: String::new(),
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    /// `-behaviour(gen_server).` / `-behavior(...)` (both spellings).
    fn record_behaviour(&mut self, node: Node) {
        let text = self.text(node);
        let t = text.trim_start_matches('-').trim_start();
        if !(t.starts_with("behaviour") || t.starts_with("behavior")) {
            return;
        }
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && child.kind() == "atom"
            {
                let name = self.text(child).to_string();
                if name.is_empty() || name.starts_with("behavi") {
                    continue;
                }
                self.facts.imports.push(ImportRecord {
                    kind: "behaviour".to_string(),
                    path: name,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
                return;
            }
        }
    }

    fn record_function(&mut self, node: Node) {
        // function_clause: atom ( ... ) -> ... ;
        // Extract the atom (function name)
        if let Some(name_node) = node.child(0)
            && name_node.kind() == "atom"
        {
            let name = self.text(name_node).to_string();
            let qn = self.qn(&name);
            let (sl, el) = (
                (node.start_position().row as u32) + 1,
                (node.end_position().row as u32) + 1,
            );
            self.facts.definitions.push(DefRecord {
                simple_name: name,
                qualified_name: qn,
                variant: DefVariant::FreeFunction,
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
        // call: atom ( ... ) or module:function ( ... )
        // First child is the function/module reference
        if let Some(func_node) = node.child(0) {
            let mut name = self.text(func_node).to_string();
            if name.is_empty() {
                return;
            }
            // `m:f(…)` parses as `remote(module: m, fun: call(f, …))`: the
            // module is the parent's, and names the callee (`m:f`).
            if let Some(parent) = node.parent()
                && parent.kind() == "remote"
                && parent
                    .child_by_field_name("fun")
                    .is_some_and(|f| f.id() == node.id())
                && let Some(module) = parent
                    .child_by_field_name("module")
                    .and_then(|m| m.child_by_field_name("module").or(Some(m)))
                && module.kind() == "atom"
                && !name.contains(':')
            {
                name = format!("{}:{name}", self.text(module));
            }

            let receiver_hint = if name.contains(':') {
                let parts: Vec<&str> = name.split(':').collect();
                if parts.len() == 2 {
                    parts[0].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            if self.reference_sites {
                self.record_mfa_call(node, &name);
            }
            // `f(A, B)` is `f/2`; the arity is part of the callee's name.
            let arity = node
                .child_by_field_name("args")
                .map(|a| a.named_child_count() as u32);
            self.facts.references.push(RefRecord {
                name,
                receiver_hint,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
                arity,
                ..Default::default()
            });
        }
    }
}

impl ErlangWalker<'_> {
    /// The `/N` of a `fun f/N` / `fun m:f/N`.
    fn arity_of(&self, fun: Node) -> Option<u32> {
        fun.child_by_field_name("arity")
            .and_then(|a| a.child_by_field_name("value"))
            .and_then(|v| self.text(v).parse().ok())
    }

    fn push_value_ref(
        &mut self,
        site: Node,
        name: String,
        recv: String,
        arity: Option<u32>,
    ) {
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (site.start_position().row as u32) + 1,
            site_byte: site.start_byte() as u32,
            arity,
            value_ref: true,
            ..Default::default()
        });
    }

    /// `M`, `F` atoms and, when `A` is a literal list, its length: the
    /// callee `M:F/len(A)` they name.
    fn push_mfa(&mut self, m: Node, f: Node, arity: Option<u32>) {
        if m.kind() != "atom" || f.kind() != "atom" {
            return;
        }
        let module = self.text(m).to_string();
        let name = format!("{module}:{}", self.text(f));
        self.push_value_ref(f, name, module, arity);
    }

    /// A bare `{M, F}` as an argument or a value: a callback as often as
    /// data, so its binding is never `high` (`weak_receiver_type`).
    fn push_mfa_weak(&mut self, m: Node, f: Node) {
        let before = self.facts.references.len();
        self.push_mfa(m, f, None);
        if let Some(r) = self.facts.references.get_mut(before) {
            r.weak_receiver_type = true;
        }
    }

    /// The arity an `A` argument states: a literal list's length, else
    /// unknown.
    fn args_arity(a: Option<Node>) -> Option<u32> {
        a.filter(|a| a.kind() == "list")
            .map(|a| a.named_child_count() as u32)
    }

    /// `{M, F, [A…]}` (a callback with its arguments), `{M, F, N}` (a
    /// function by arity, as `erlang:trace_pattern` takes it) and a bare
    /// `{M, F}` standing as an argument or a value (`push_mfa_weak`).
    fn record_mfa_tuple(&mut self, tuple: Node) {
        if in_pattern(tuple) {
            return;
        }
        let mut c = tuple.walk();
        let items: Vec<Node> = tuple.named_children(&mut c).collect();
        match items.as_slice() {
            [m, f, a] if a.kind() == "list" => {
                self.push_mfa(*m, *f, Self::args_arity(Some(*a)))
            }
            [m, f] if bare_value_position(tuple) => self.push_mfa_weak(*m, *f),
            [m, f, n] if n.kind() == "integer" => {
                let arity = self.text(*n).parse().ok();
                if arity.is_some() {
                    self.push_mfa(*m, *f, arity);
                }
            }
            _ => {}
        }
    }

    /// `spawn(M, F, A)`, `timer:apply_after(T, M, F, A)`, …
    /// (`cgg_core::stdlib::ERLANG_MFA_CALLS`).
    fn record_mfa_call(&mut self, call: Node, callee: &str) {
        let callee = callee
            .strip_prefix(cgg_core::stdlib::ERLANG_BIF_MODULE)
            .and_then(|c| c.strip_prefix(':'))
            .unwrap_or(callee);
        let Some(args) = call.child_by_field_name("args") else {
            return;
        };
        let mut c = args.walk();
        let items: Vec<Node> = args.named_children(&mut c).collect();
        let Some(&(_, _, pos)) = cgg_core::stdlib::ERLANG_MFA_CALLS
            .iter()
            .find(|(f, n, _)| *f == callee && *n == items.len())
        else {
            return;
        };
        if let (Some(m), Some(f)) = (items.get(pos), items.get(pos + 1)) {
            self.push_mfa(*m, *f, Self::args_arity(items.get(pos + 2).copied()));
        }
    }
}

/// Whether a tuple stands as a call argument, the right of `=`, a
/// record field's value or an expression of a clause body — not inside a
/// list, a map, a binary or another tuple, where a pair of atoms is data.
fn bare_value_position(tuple: Node) -> bool {
    let Some(p) = tuple.parent() else {
        return false;
    };
    match p.kind() {
        "expr_args" => p.parent().is_some_and(|g| g.kind() == "call"),
        "match_expr" => field_of(p, tuple) == Some("rhs"),
        "field_expr" => p.parent().is_some_and(|g| g.kind() == "record_field"),
        "clause_body" => true,
        _ => false,
    }
}

/// Whether `node` is (inside) a pattern — a clause head, a `case`/
/// `receive`/`catch` pattern, the left of `=` or of a generator — where a
/// tuple of atoms matches a value rather than naming a function.
fn in_pattern(node: Node) -> bool {
    let mut n = node;
    while let Some(p) = n.parent() {
        match p.kind() {
            "tuple" | "list" | "cons" | "paren_expr" | "map_expr" | "map_field"
            | "record_expr" | "record_field" | "binary" | "bin_element" => n = p,
            "cr_clause" | "catch_clause" => return field_of(p, n) == Some("pat"),
            "match_expr" | "cond_match_expr" | "generator" | "b_generator"
            | "map_generator" => return field_of(p, n) == Some("lhs"),
            "expr_args" => {
                return p.parent().is_some_and(|g| {
                    matches!(g.kind(), "function_clause" | "fun_clause")
                });
            }
            _ => return false,
        }
    }
    false
}

/// The field `child` fills in `parent`.
fn field_of(parent: Node, child: Node) -> Option<&'static str> {
    let mut c = parent.walk();
    let i = parent.children(&mut c).position(|x| x.id() == child.id())?;
    parent.field_name_for_child(i as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_erlang::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        ErlangPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.erl"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn module_and_functions() {
        let src = "-module(mymod).\n\ngreet(Name) ->\n    io:format(\"Hello, ~s~n\", [Name]).\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "Expected definitions, got none");
        assert_eq!(f.definitions[0].simple_name, "greet");
    }

    /// FINDINGS R14: `m:f(…)` names module `m` — the resolver binds it
    /// to `m`'s `f` only, so the module must reach it as the receiver.
    #[test]
    fn a_remote_call_records_its_module_as_the_receiver() {
        let f = extract("-module(a).\nrun(X) -> other_mod:gen(X), local(X).\n");
        let refs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert_eq!(refs, vec![("other_mod:gen", "other_mod"), ("local", "")]);
    }

    fn extract_with_refs(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_erlang::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        ErlangPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(true),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.erl"),
            &tree,
            src.as_bytes(),
        )
    }

    fn value_refs(f: &FileFacts) -> Vec<(&str, &str, Option<u32>)> {
        f.references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str(), r.arity))
            .collect()
    }

    /// WS26: `fun f/N`, `fun m:f/N`, `{M, F, A}` callbacks and the
    /// M/F arguments of `spawn`/`apply`/`timer:apply_after` name functions.
    #[test]
    fn function_values_are_value_references() {
        let src = "-module(a).\n\
            run(X) ->\n\
            \x20 lists:map(fun step/1, X),\n\
            \x20 F = fun b:go/2,\n\
            \x20 S = {worker, start_link, [X]},\n\
            \x20 spawn(c, loop, [X, 1]),\n\
            \x20 erlang:apply(d, h, []),\n\
            \x20 timer:apply_after(10, e, tick, [X]),\n\
            \x20 erlang:trace_pattern({g, init, 1}, true, []),\n\
            \x20 {F, S}.\n";
        let f = extract_with_refs(src);
        assert_eq!(
            value_refs(&f),
            vec![
                ("step", "", Some(1)),
                ("b:go", "b", Some(2)),
                ("worker:start_link", "worker", Some(1)),
                ("c:loop", "c", Some(2)),
                ("d:h", "d", Some(0)),
                ("e:tick", "e", Some(1)),
                ("g:init", "g", Some(1)),
            ]
        );
    }

    /// Data tuples are recorded only as atom pairs (the resolver drops a
    /// module the tree does not declare); a variable M/F names nothing,
    /// and nothing is recorded without `reference_sites`.
    #[test]
    fn non_literal_callbacks_and_plain_runs_record_nothing() {
        let src = "-module(a).\n\
            run(M, F, X) ->\n\
            \x20 spawn(M, F, X),\n\
            \x20 apply(a, F, []),\n\
            \x20 {ok, X},\n\
            \x20 [{worker, start}],\n\
            \x20 {{x, y}, z},\n\
            \x20 #{label => {gen, stop}},\n\
            \x20 case X of {c, h} -> 1; _ -> 2 end,\n\
            \x20 {d, e} = X,\n\
            \x20 [1 || {g, h} <- X],\n\
            \x20 {1, 2, 3}.\n\
            exc({erlang, error}, [E]) -> E.\n";
        assert!(value_refs(&extract_with_refs(src)).is_empty());
        let off =
            extract("-module(a).\nrun() -> fun f/1, {m, f, []}, spawn(m, f, []).\n");
        assert!(value_refs(&off).is_empty());
    }

    /// A bare `{M, F}` as an argument or a value is recorded, but its
    /// binding is never certain.
    #[test]
    fn a_bare_module_function_pair_is_a_weak_value_reference() {
        let f = extract_with_refs(
            "-module(a).\nrun(X) -> register_cb({m, f}), C = {n, g}, #s{cb = {o, h}}, {p, i}.\n",
        );
        let refs: Vec<(&str, bool)> = f
            .references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| (r.name.as_str(), r.weak_receiver_type))
            .collect();
        assert_eq!(
            refs,
            vec![("m:f", true), ("n:g", true), ("o:h", true), ("p:i", true)]
        );
    }

    #[test]
    fn include_lib_is_recorded() {
        let f = extract(
            "-module(m).\n-include_lib(\"eunit/include/eunit.hrl\").\nf() -> ok.\n",
        );
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "include_lib" && i.path == "eunit/include/eunit.hrl"),
            "{:?}",
            f.imports
        );
    }
}
