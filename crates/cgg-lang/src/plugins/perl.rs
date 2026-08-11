//! Perl plugin — callable extraction for Perl.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct PerlPlugin;

impl LanguagePlugin for PerlPlugin {
    fn id(&self) -> &'static str {
        "perl"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".pl", ".pm", ".t"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["perl"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            value_refs: true,
            ..Default::default()
        }
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_perl::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "perl");
        let mut w = PerlWalker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct PerlWalker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> PerlWalker<'a> {
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
        match node.kind() {
            "package_statement" => {
                // package_statement has package_name as a child (not a field)
                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        let child = c.node();
                        if child.kind() == "package_name" {
                            let name = self.text(child).to_string();
                            if !name.is_empty() {
                                self.scope.clear();
                                self.scope.push(name);
                            }
                            break;
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                self.walk_children(node);
                return;
            }
            "function_definition" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let name = self.text(name_node).to_string();
                    if !name.is_empty() {
                        self.record_def(node, &name, DefVariant::FreeFunction);
                    }
                }
                self.walk_children(node);
                return;
            }
            // `use Foo;` parses as `use_no_statement` (the rule covers
            // `use` and `no`). Matching only `use_statement` — a kind
            // this grammar never produces — meant no Perl `use` was
            // ever captured, so the language's documented cross-file
            // resolution resolved nothing.
            "use_no_statement" | "use_statement" | "require_statement" => {
                self.record_import(node);
                self.walk_children(node);
                return;
            }
            "call_expression"
            | "call_expression_with_args_with_brackets"
            | "call_expression_with_bareword"
            | "call_expression_with_spaced_args" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            "method_call" | "method_invocation" => {
                self.record_method_call(node);
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
        let is_use = text.starts_with("use");

        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let child = c.node();
                // `package_name` is what the grammar actually produces
                // for `use Data::Dumper` / `require Exporter`; the other
                // two cover quoted and bareword spellings.
                if matches!(child.kind(), "package_name" | "bareword" | "string") {
                    let path = self
                        .text(child)
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_string();
                    if !path.is_empty()
                        && !path.starts_with("strict")
                        && !path.starts_with("warnings")
                    {
                        let alias = path.split("::").last().unwrap_or("").to_string();
                        self.facts.imports.push(ImportRecord {
                            kind: if is_use {
                                "use".into()
                            } else {
                                "require".into()
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
        if let Some(func_node) = node.child(0) {
            let name = self.text(func_node).to_string();
            if !name.is_empty() {
                self.facts.references.push(RefRecord {
                    name: name.clone(),
                    receiver_hint: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                    ..Default::default()
                });
                // Dancer2 registers with a bare keyword — `get '/x' =>
                // sub { … }` — so the handler sits in argument position
                // of a receiver-less call.
                self.capture_registrar(node, &name);
            }
        }
    }

    fn record_method_call(&mut self, node: Node) {
        // tree-sitter-perl names these fields `function_name` and
        // `object_return_value`; the old `method`/`object` lookups never
        // matched, so `$obj->run()` produced no reference at all.
        if let Some(method_node) = node
            .child_by_field_name("function_name")
            .or_else(|| node.child_by_field_name("method"))
        {
            let name = self.text(method_node).to_string();
            if !name.is_empty() {
                let receiver_hint = node
                    .child_by_field_name("object_return_value")
                    .or_else(|| node.child_by_field_name("object"))
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                self.facts.references.push(RefRecord {
                    name: name.clone(),
                    receiver_hint: receiver_hint.clone(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                    ..Default::default()
                });
                // Mojolicious routes are method calls on a router
                // object: `$r->get('/x' => sub { … })`, `$r->get('/x')
                // ->to('users#list')`. The rule engine matches the last
                // dotted segment, so a receiver too complex to name —
                // the previous link of a `->` chain — is dropped rather
                // than pasted into the context verbatim.
                let context = match receiver_hint.as_str() {
                    r if r.is_empty() || !is_simple_receiver(r) => name,
                    r => format!("{r}.{name}"),
                };
                self.capture_registrar(node, &context);
            }
        }
    }

    /// The argument list of a call, whichever spelling the grammar used.
    ///
    /// tree-sitter-perl hangs three different node kinds off the `args`
    /// field: `foo(…)` wraps the list in `parenthesized_argument`, the
    /// bracketed form uses `array`, and `foo 'a', $b` puts `arguments`
    /// there directly. None of the three is a kind the shared
    /// `registrar::arguments_of` recognises, which is why this plugin
    /// walks the slots itself the way `rust.rs` does.
    fn arg_list<'t>(node: Node<'t>) -> Option<Node<'t>> {
        let args = node.child_by_field_name("args")?;
        if args.kind() == "parenthesized_argument" {
            let mut c = args.walk();
            return args
                .named_children(&mut c)
                .find(|n| n.kind() == "arguments")
                .or(Some(args));
        }
        Some(args)
    }

    /// Text of `n` if it is a string literal, unquoted.
    fn string_literal(&self, n: Node) -> Option<String> {
        if !n.kind().starts_with("string_") {
            return None;
        }
        let v = super::registrar::unquote(self.text(n));
        (!v.is_empty()).then_some(v)
    }

    /// The route identity a registration call carries.
    ///
    /// Its own first string literal, or — for a Mojolicious chain like
    /// `$r->get('/x')->to('users#list')`, where the path is on the
    /// *receiver* rather than on this call — the nearest one upstream.
    /// Without the fallback the `->to` handler is registered at no path
    /// at all, and a list of anonymous routes is not a surface map.
    fn registrar_route(&self, node: Node, args: Node) -> String {
        if let Some(s) = self.first_string(args) {
            return s;
        }
        let mut recv = node.child_by_field_name("object_return_value");
        // Two links is enough for every router chain in the wild and
        // keeps an unrelated string further up from being claimed.
        for _ in 0..2 {
            let Some(r) = recv else { break };
            if r.kind() != "method_invocation" {
                break;
            }
            if let Some(inner) = Self::arg_list(r)
                && let Some(s) = self.first_string(inner)
            {
                return s;
            }
            recv = r.child_by_field_name("object_return_value");
        }
        String::new()
    }

    fn first_string(&self, args: Node) -> Option<String> {
        let mut c = args.walk();
        args.named_children(&mut c)
            .find_map(|n| self.string_literal(n))
    }

    /// Capture the argument slots of a registration-shaped call —
    /// shapes B, C and E of the framework design.
    ///
    /// Deliberately over-eager, like the shared helper it mirrors: the
    /// framework rule engine gates every record on detected imports, so
    /// an unmatched one is inert. The `is_registrar_verb` check keeps
    /// the whole pass off the hot path.
    fn capture_registrar(&mut self, node: Node, context: &str) {
        if !self
            .ctx
            .is_registrar_verb(super::registrar::last_segment(context))
        {
            return;
        }
        let Some(args) = Self::arg_list(node) else {
            return;
        };
        let mut cursor = args.walk();
        let items: Vec<Node> = args.named_children(&mut cursor).collect();
        if items.is_empty() {
            return;
        }
        let route = self.registrar_route(node, args);
        let mut seen_route_string = false;
        let mut out: Vec<RefRecord> = Vec::new();

        for (i, arg) in items.iter().enumerate() {
            let line = (arg.start_position().row as u32) + 1;
            let byte = arg.start_byte() as u32;

            if let Some(s) = self.string_literal(*arg) {
                // The first string is the route itself, already
                // captured. Unless it is the only argument, where the
                // identity and the target are the same string —
                // `->to('users#list')`.
                if !seen_route_string && s == route && items.len() > 1 {
                    seen_route_string = true;
                    continue;
                }
                out.push(RefRecord {
                    name: s,
                    receiver_hint: cgg_core::STRING_REF_HINT.to_string(),
                    site_line: line,
                    site_byte: byte,
                    context: context.to_string(),
                    route: route.clone(),
                });
                continue;
            }

            if let Some(name) = self.value_ref_name(*arg, i, &items) {
                out.push(RefRecord {
                    name,
                    receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                    site_line: line,
                    site_byte: byte,
                    context: context.to_string(),
                    route: route.clone(),
                });
            }
        }
        self.facts.references.extend(out);
        self.extract_inline_handler(&items, &route, context);
    }

    /// The sub a single argument slot names, if it names one.
    fn value_ref_name(&self, arg: Node, idx: usize, items: &[Node]) -> Option<String> {
        let raw = match arg.kind() {
            // `\&handler` — a code reference to a named sub, the one
            // unambiguous way Perl passes a function as a value.
            "unary_expression" => {
                if arg.named_child(0)?.kind() != "to_reference" {
                    return None;
                }
                self.text(arg)
            }
            // `$handler` — a code ref held in a scalar.
            "scalar_variable" => self.text(arg),
            // A bareword sub name. `controller => 'Chat'` is a hash key,
            // not a handler, so a bareword followed by `=>` is skipped:
            // minting an entry for every option name would put the
            // framework's own vocabulary in the surface map.
            "call_expression_with_bareword" | "identifier" => {
                if items.get(idx + 1).is_some_and(|n| n.kind() == "fat_comma") {
                    return None;
                }
                self.text(arg)
            }
            _ => return None,
        };
        let name = super::registrar::last_segment(raw.trim())
            .trim_start_matches(['\\', '&', '$', '@', '*'])
            .trim();
        if name.len() < 2
            || !name.starts_with(|c: char| c.is_alphabetic() || c == '_')
            || !name.chars().all(|c| c.is_alphanumeric() || c == '_')
        {
            return None;
        }
        Some(name.to_string())
    }

    /// Name the `sub { … }` of a route registration so it can be a
    /// handler.
    ///
    /// This is how both Mojolicious and Dancer2 are normally written —
    /// `get '/x' => sub { … }` — so without a callable to mark, the
    /// framework enumerates nothing. Gated on the registrar verb and on
    /// a leading route string, so `$app->hook(around_action => sub { …
    /// })` and every other bare callback mints nothing.
    fn extract_inline_handler(&mut self, items: &[Node], route: &str, context: &str) {
        if route.is_empty() {
            return;
        }
        for arg in items.iter().filter(|n| super::registrar::is_closure(**n)) {
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
                signature_hint: super::extract_signature(self.text(*arg)),
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
                route: route.to_string(),
            });
        }
    }
}

/// Whether a receiver is a plain name worth putting in a registrar
/// context (`$r`, `$app`, `Mojo::IOLoop`) rather than a whole
/// sub-expression.
fn is_simple_receiver(recv: &str) -> bool {
    recv.len() <= 64
        && !recv.is_empty()
        && recv
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_perl::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        PerlPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.pl"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn subroutine_captured() {
        let src = "sub greet {\n  my $name = shift;\n  print \"Hello, $name\\n\";\n}\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "greet"));
    }

    #[test]
    fn package_scope() {
        let src = "package MyModule;\nsub foo { }\n";
        let f = extract(src);
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name == "MyModule::foo")
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
    fn use_and_require_keep_their_own_kinds() {
        // The README's language table promises both forms for Perl.
        let f = extract("use Data::Dumper;\nrequire Exporter;\n");
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "use" && i.path == "Data::Dumper"),
            "imports: {:?}",
            f.imports
        );
        assert!(
            f.imports.iter().any(|i| i.path == "Exporter"),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn pragmas_are_not_imports() {
        // `use strict`/`use warnings` are compiler pragmas, not modules;
        // recording them would invent cross-file edges to nothing.
        let f = extract("use strict;\nuse warnings;\nsub f { }\n");
        assert!(
            !f.imports
                .iter()
                .any(|i| i.path.starts_with("strict") || i.path.starts_with("warnings")),
            "pragmas must be skipped: {:?}",
            f.imports
        );
    }

    #[test]
    fn an_import_alias_is_the_last_path_segment() {
        let f = extract("use Data::Dumper;\n");
        let i = f.imports.iter().find(|i| i.path == "Data::Dumper").unwrap();
        assert_eq!(i.alias, "Dumper");
    }

    #[test]
    fn a_call_is_a_reference() {
        let f = extract("sub outer {\n  inner();\n}\n");
        assert!(
            refs(&f).iter().any(|r| r == "inner"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn a_method_call_records_its_receiver() {
        let f = extract("sub outer {\n  $obj->run();\n}\n");
        assert!(
            f.references.iter().any(|r| r.name == "run"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn a_second_package_rescopes_later_subs() {
        // Perl packages are a running scope, not a block, so a sub after
        // a second `package` belongs to the second one.
        let f = extract("package A;\nsub one { }\npackage B;\nsub two { }\n");
        let d = defs(&f);
        assert!(d.contains(&"A::one".to_string()), "defs: {d:?}");
        assert!(d.contains(&"B::two".to_string()), "defs: {d:?}");
    }

    /// `get`/`post` are built-in registrar verbs, so these exercise the
    /// real gate rather than a test-only one.
    #[test]
    fn a_mojolicious_route_captures_its_handler_and_path() {
        let f = extract(
            "sub startup {\n  my $r = $self->routes;\n  \
             $r->get('/users' => \\&list_users);\n}\nsub list_users { }\n",
        );
        let r = f
            .references
            .iter()
            .find(|r| r.receiver_hint == cgg_core::VALUE_REF_HINT)
            .unwrap_or_else(|| panic!("no value ref: {:?}", f.references));
        assert_eq!(r.name, "list_users");
        assert_eq!(r.route, "/users");
        assert_eq!(r.context, "$r.get");
    }

    #[test]
    fn a_chained_to_string_is_captured_with_the_upstream_path() {
        let f = extract("$r->get('/logout')->to('login#logout');\n");
        let s = f
            .references
            .iter()
            .find(|r| r.receiver_hint == cgg_core::STRING_REF_HINT)
            .unwrap_or_else(|| panic!("no string ref: {:?}", f.references));
        assert_eq!(s.name, "login#logout");
        // The receiver is the previous link of the chain, not a name,
        // so the context is the bare verb.
        assert_eq!(s.context, "to");
    }

    #[test]
    fn an_inline_sub_handler_gets_a_name() {
        // Dancer2's whole routing surface is `get '/x' => sub { … }`.
        let f = extract("get '/res5' => sub { my $c = shift; };\n");
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name.starts_with("handler_at_")
                    && d.variant == DefVariant::NamedClosure),
            "defs: {:?}",
            defs(&f)
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name.starts_with("handler_at_")
                    && r.route == "/res5"
                    && r.context == "get"),
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn an_ordinary_callback_mints_no_handler() {
        // No route string, so nothing here is a registration — naming
        // this closure would invent an entry point.
        let f = extract("$app->hook(around_action => sub { 1 });\n");
        assert!(
            !f.definitions
                .iter()
                .any(|d| d.simple_name.starts_with("handler_at_")),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn a_hash_key_is_not_a_handler() {
        let f = extract("$r->get('/echo')->to(controller => 'Chat');\n");
        assert!(
            !f.references
                .iter()
                .any(|r| r.receiver_hint == cgg_core::VALUE_REF_HINT
                    && r.name == "controller"),
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn a_non_registrar_call_captures_nothing() {
        let f = extract("my $v = $cache->fetch('/users', \\&loader);\n");
        assert!(
            f.references
                .iter()
                .all(|r| r.receiver_hint != cgg_core::VALUE_REF_HINT),
            "refs: {:?}",
            f.references
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
        let f = extract("sub broken {\n  my $x = ;\n");
        let _ = defs(&f);
    }
}
