//! Elixir plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ElixirPlugin;

impl LanguagePlugin for ElixirPlugin {
    fn id(&self) -> &'static str {
        "elixir"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".ex", ".exs"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["elixir"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            value_refs: true,
            ..Default::default()
        }
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_elixir::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "elixir");
        let mut w = ElixirWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
            suppress_call_at: None,
        };
        w.walk(tree.root_node());
        facts
    }
}

struct ElixirWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Start offset of the head of the `def` currently being walked, if
    /// any. A `call` node beginning exactly there is the definition's
    /// own head re-read as a call, not a call site.
    suppress_call_at: Option<usize>,
}

impl<'a> ElixirWalker<'a> {
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
        if node.kind() == "call" {
            // Get the first child which should be the function name
            let func_name = node
                .child(0)
                .map(|c| self.text(c).to_string())
                .unwrap_or_default();

            match func_name.as_str() {
                "defmodule" => {
                    if let Some(module_name) = self.extract_defmodule_name(node) {
                        self.scope.push(module_name);
                        self.walk_children(node);
                        self.scope.pop();
                    } else {
                        self.walk_children(node);
                    }
                    return;
                }
                "def" | "defp" | "defmacro" => {
                    self.record_function(node);
                    // `def run(x) do … end` parses its head, `run(x)`,
                    // as a nested `call` node. Walking it like any
                    // other call made every parenthesised definition
                    // emit a call to itself: 730 of phoenix's 2982
                    // Elixir edges — 24% — were these phantom
                    // self-loops, and each one also made its function
                    // look reachable to `--dead-code`.
                    //
                    // The head is suppressed by start offset rather
                    // than by skipping the subtree, because the head
                    // can be `run(x \\ default()) when guard(x)`,
                    // whose default arguments and guards are real
                    // call sites that start later than the head does.
                    let prev = self.suppress_call_at;
                    self.suppress_call_at = node.child(1).map(|h| h.start_byte());
                    self.walk_children(node);
                    self.suppress_call_at = prev;
                    return;
                }
                "alias" | "import" | "use" | "require" => {
                    self.record_import(node, &func_name);
                    self.walk_children(node);
                    return;
                }
                _ => {
                    self.record_call(node);
                    self.walk_children(node);
                    return;
                }
            }
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

    fn extract_defmodule_name(&self, node: Node) -> Option<String> {
        // defmodule name do ... end
        // The name is typically the second child (after "defmodule")
        node.child(1)
            .map(|n| self.text(n).to_string())
            .filter(|s| !s.is_empty())
    }

    fn record_function(&mut self, node: Node) {
        // def name(...) do ... end
        // The name is typically the second child (after "def")
        if let Some(name_node) = node.child(1) {
            let head_text = self.text(name_node);
            // Extract function name from head (e.g., "foo" or "foo(a, b)")
            let name = head_text.split('(').next().unwrap_or("").trim().to_string();
            if name.is_empty() {
                return;
            }

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

    fn record_import(&mut self, node: Node, import_type: &str) {
        // alias/import/use/require Module
        // The module is typically the second child
        if let Some(module_node) = node.child(1) {
            let path = self
                .text(module_node)
                .trim_matches(|c| c == '\'' || c == '"')
                .to_string();
            if !path.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: import_type.to_string(),
                    path,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
        }
    }

    fn record_call(&mut self, node: Node) {
        if self.suppress_call_at == Some(node.start_byte()) {
            return;
        }
        if let Some(func_node) = node.child(0) {
            let name = self.text(func_node).to_string();
            if name.is_empty() || name.starts_with("def") {
                return;
            }

            let receiver_hint = if name.contains('.') {
                let parts: Vec<&str> = name.split('.').collect();
                if parts.len() == 2 {
                    parts[0].to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let context = name.clone();
            self.facts.references.push(RefRecord {
                name,
                receiver_hint,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
                ..Default::default()
            });
            self.record_registrar(node, &context);
        }
    }

    /// Route macros — the Phoenix/Plug hand-off.
    ///
    /// Phoenix declares its whole HTTP surface with calls whose
    /// *arguments* name the handler: `get "/", PageController, :home`,
    /// `live "/dash", DashLive`, `forward "/api", to: MyApp.Router`.
    /// The shared registrar pass cannot read them on its own — Elixir
    /// spells a module as an `alias` node and an action as an `atom`,
    /// neither of which is a reference kind any other grammar has — so
    /// this adds the Elixir spellings on top of it.
    ///
    /// Gated on a leading string literal as well as on the verb, so
    /// `Map.get(m, :key)`, whose verb is also a registered one,
    /// contributes nothing. As everywhere else in this pass the records
    /// are inert until a framework rule claims them.
    fn record_registrar(&mut self, node: Node, context: &str) {
        if !crate::is_registrar_verb(super::registrar::last_segment(context)) {
            return;
        }
        // The path is the gate, not an extra. Every Elixir route macro
        // names it *first*, and requiring that costs nothing while
        // keeping the pass off ordinary code that happens to share a
        // verb: `Map.get(m, :key)` has no string at all, and
        // `Map.put(params, :url, "https://…")` has one in the wrong
        // slot. Measured on plausible, accepting a string anywhere in
        // the call turned that second form into two "routes" bound to a
        // function named `url`.
        let Some(args) = super::registrar::arguments_of(node) else {
            return;
        };
        let mut cursor = args.walk();
        if args.named_children(&mut cursor).next().map(|n| n.kind()) != Some("string") {
            return;
        }
        let route = super::registrar::route_of(node, self.source);
        if route.is_empty() {
            return;
        }
        let extra = super::registrar::capture(node, self.source, context);
        self.facts.references.extend(extra);

        let mut cursor = args.walk();
        let mut owner: Option<String> = None;
        for arg in args.named_children(&mut cursor) {
            self.registrar_arg(arg, context, &route, &mut owner, 0);
        }
        self.extract_inline_handler(node, context, &route);
    }

    /// One argument slot of a route macro.
    ///
    /// `owner` carries the last module seen in the same call, because
    /// the Phoenix target is a *pair* — `PageController, :home` — and
    /// pairing them here rather than emitting two unrelated names is
    /// what lets a rule bind the right `index` in a project with a
    /// dozen of them. The bare action is emitted too: it binds when the
    /// name is unique, and the shared name index drops it rather than
    /// guessing when it is not.
    fn registrar_arg(
        &mut self,
        arg: Node,
        context: &str,
        route: &str,
        owner: &mut Option<String>,
        depth: u8,
    ) {
        if depth > 3 {
            return;
        }
        let line = (arg.start_position().row as u32) + 1;
        let byte = arg.start_byte() as u32;
        match arg.kind() {
            // `PageController`, `MyAppWeb.PageController`, `DashLive`.
            "alias" => {
                let name = super::registrar::last_segment(self.text(arg)).to_string();
                if name.is_empty() {
                    return;
                }
                *owner = Some(name.clone());
                self.push_registrar_ref(
                    name,
                    cgg_core::VALUE_REF_HINT,
                    line,
                    byte,
                    context,
                    route,
                );
            }
            // `:home` — the action on the module named just before it.
            "atom" => {
                let action = self.text(arg).trim_start_matches(':').to_string();
                if !is_function_name(&action) {
                    return;
                }
                if let Some(o) = owner.clone() {
                    self.push_registrar_ref(
                        format!("{o}::{action}"),
                        cgg_core::STRING_REF_HINT,
                        line,
                        byte,
                        context,
                        route,
                    );
                }
                self.push_registrar_ref(
                    action,
                    cgg_core::VALUE_REF_HINT,
                    line,
                    byte,
                    context,
                    route,
                );
            }
            // A bare local function name handed to the macro.
            "identifier" => {
                let name = self.text(arg).to_string();
                if !is_function_name(&name) {
                    return;
                }
                self.push_registrar_ref(
                    name,
                    cgg_core::VALUE_REF_HINT,
                    line,
                    byte,
                    context,
                    route,
                );
            }
            // `&MyMod.handler/2` — the captured function is the handler.
            "dot" => {
                let text = self.text(arg).to_string();
                let name = super::registrar::last_segment(&text).to_string();
                if !is_function_name(&name) {
                    return;
                }
                let module = text
                    .strip_suffix(&name)
                    .map(|m| super::registrar::last_segment(m.trim_end_matches('.')))
                    .filter(|m| !m.is_empty())
                    .map(|m| m.to_string());
                if let Some(m) = module {
                    self.push_registrar_ref(
                        format!("{m}::{name}"),
                        cgg_core::STRING_REF_HINT,
                        line,
                        byte,
                        context,
                        route,
                    );
                }
                self.push_registrar_ref(
                    name,
                    cgg_core::VALUE_REF_HINT,
                    line,
                    byte,
                    context,
                    route,
                );
            }
            // Containers the target hides in: `to: MyApp.Router`,
            // `&Mod.fun/2`, a list of plugs.
            "keywords" | "pair" | "unary_operator" | "binary_operator" | "list"
            | "tuple" => {
                let mut c = arg.walk();
                let children: Vec<Node> = arg.named_children(&mut c).collect();
                for child in children {
                    self.registrar_arg(child, context, route, owner, depth + 1);
                }
            }
            // Only the callee of a nested call — its own arguments
            // belong to it, and the walker visits them on their turn.
            "call" => {
                if let Some(target) = arg.child_by_field_name("target") {
                    self.registrar_arg(target, context, route, owner, depth + 1);
                }
            }
            _ => {}
        }
    }

    fn push_registrar_ref(
        &mut self,
        name: String,
        hint: &str,
        line: u32,
        byte: u32,
        context: &str,
        route: &str,
    ) {
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: hint.to_string(),
            site_line: line,
            site_byte: byte,
            context: context.to_string(),
            route: route.to_string(),
        });
    }

    /// Name the `do … end` body of a route macro so it can be a handler.
    ///
    /// `Plug.Router` writes its entire HTTP surface as `get "/x" do …
    /// end`: the body is an anonymous block, so there is no callable
    /// for an entry rule to point at and the framework enumerates
    /// nothing. Mirrors ruby.rs's Sinatra case and is gated the same
    /// way — registrar verb plus a leading string literal — so an
    /// ordinary `test "works" do … end` mints nothing.
    ///
    /// The path must be the call's *only* argument. `scope "/", MyAppWeb
    /// do … end` clears every other gate — `scope` is a registrar verb,
    /// its first argument is a path — but its block is a container full
    /// of route macros, not a handler, and naming it would invent a
    /// callable that owns every route declared inside it.
    fn extract_inline_handler(&mut self, node: Node, context: &str, route: &str) {
        let only_the_path = super::registrar::arguments_of(node).is_some_and(|args| {
            let mut c = args.walk();
            args.named_children(&mut c).count() == 1
        });
        if !only_the_path {
            return;
        }
        let mut cursor = node.walk();
        let blocks: Vec<Node> = node
            .named_children(&mut cursor)
            .filter(|c| c.kind() == "do_block")
            .collect();
        for block in blocks {
            let line = (block.start_position().row as u32) + 1;
            let simple = format!("handler_at_{line}");
            let qn = self.qn(&simple);
            self.facts.definitions.push(DefRecord {
                simple_name: simple.clone(),
                qualified_name: qn,
                variant: DefVariant::NamedClosure,
                start_line: line,
                end_line: (block.end_position().row as u32) + 1,
                start_byte: block.start_byte() as u32,
                end_byte: block.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(block)),
                visibility: String::new(),
                attributes: vec!["synthetic".to_string()],
                ..Default::default()
            });
            self.push_registrar_ref(
                simple,
                cgg_core::VALUE_REF_HINT,
                line,
                block.start_byte() as u32,
                context,
                route,
            );
        }
    }
}

/// Whether an atom or identifier could name an Elixir function.
///
/// `?` and `!` are kept: `:valid?` is the real name of the definition a
/// rule would have to bind to, and trimming them would make it miss.
fn is_function_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '?' | '!'))
        && s.chars().any(|c| c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_elixir::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        ElixirPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.ex"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn defmodule_and_functions() {
        let src = "defmodule MyModule do\n  def greet(name) do\n    IO.puts(\"Hello, #{name}\")\n  end\nend\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "Expected definitions, got none");
    }

    #[test]
    fn a_definition_head_is_not_a_call_to_itself() {
        // Regression: `def run(x) do … end` parses its head as a nested
        // `call`, and recording it produced a reference named `run` at
        // the definition site. Those resolved either to the function
        // itself (a self-loop) or to a same-named function in another
        // module (a bogus cross-file edge). 1404 of phoenix's edges —
        // just under half — were this artifact, and every one of them
        // also made its own function look reachable to `--dead-code`.
        let src = "defmodule M do\n  def run(x) do\n    :ok\n  end\nend\n";
        let f = extract(src);
        assert_eq!(f.definitions.len(), 1);
        assert!(
            !f.references.iter().any(|r| r.name == "run"),
            "definition head recorded as a call: {:?}",
            f.references.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn calls_in_the_body_survive_head_suppression() {
        // The suppression is keyed on the head's start offset, so
        // anything starting later — the body, and guards — is untouched.
        let src = "defmodule M do\n  def run(x) when is_integer(x) do\n    helper(x)\n  end\n\n  def helper(y) do\n    y\n  end\nend\n";
        let f = extract(src);
        let names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"helper"), "body call lost: {names:?}");
        assert!(names.contains(&"is_integer"), "guard call lost: {names:?}");
        assert!(!names.contains(&"run"), "head still recorded: {names:?}");
    }

    #[test]
    fn phoenix_route_macro_names_its_controller_and_action() {
        // `get "/", PageController, :home` is the whole Phoenix routing
        // vocabulary: the target is a module plus an action atom, in
        // argument position. Without these records neither half is
        // visible to a framework rule and every controller action reads
        // as unreachable.
        let src = "defmodule MyAppWeb.Router do\n  use Phoenix.Router\n\n  scope \"/\", MyAppWeb do\n    get \"/\", PageController, :home\n  end\nend\n";
        let f = extract(src);
        let reg: Vec<(&str, &str, &str)> = f
            .references
            .iter()
            .filter(|r| {
                r.receiver_hint == cgg_core::VALUE_REF_HINT
                    || r.receiver_hint == cgg_core::STRING_REF_HINT
            })
            .map(|r| (r.name.as_str(), r.context.as_str(), r.route.as_str()))
            .collect();
        assert!(
            reg.contains(&("PageController", "get", "/")),
            "module not captured: {reg:?}"
        );
        assert!(
            reg.contains(&("PageController::home", "get", "/")),
            "controller/action pair not captured: {reg:?}"
        );
        assert!(
            reg.contains(&("home", "get", "/")),
            "action not captured: {reg:?}"
        );
    }

    #[test]
    fn plug_router_do_block_becomes_a_named_handler() {
        // `Plug.Router` puts the handler in an anonymous `do … end`
        // block, so there is no callable for an entry node to point at
        // until one is synthesized.
        let src = "defmodule MyApp.Router do\n  use Plug.Router\n\n  get \"/health\" do\n    send_resp(conn, 200, \"ok\")\n  end\nend\n";
        let f = extract(src);
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name.starts_with("handler_at_"))
            .unwrap_or_else(|| {
                panic!(
                    "no synthesized handler: {:?}",
                    f.definitions
                        .iter()
                        .map(|d| &d.simple_name)
                        .collect::<Vec<_>>()
                )
            });
        assert_eq!(d.variant, DefVariant::NamedClosure);
        assert!(d.attributes.iter().any(|a| a == "synthetic"));
        assert!(
            f.references.iter().any(|r| r.name == d.simple_name
                && r.receiver_hint == cgg_core::VALUE_REF_HINT
                && r.route == "/health"),
            "handler not referenced from the route"
        );
    }

    #[test]
    fn a_verb_without_a_route_string_captures_nothing() {
        // `get` and `put` are registered verbs, and neither of these is
        // a route. The path in *first* position is what separates them:
        // accepting a string anywhere in the call made
        // `Map.put(params, :url, "https://…")` a route bound to a
        // function named `url` on the plausible tree.
        let src = "defmodule M do\n  def f(m, params) do\n    Map.get(m, :key)\n    Map.put(params, :url, \"https://example.com/\")\n  end\nend\n";
        let f = extract(src);
        assert!(
            !f.references.iter().any(|r| {
                r.receiver_hint == cgg_core::VALUE_REF_HINT
                    || r.receiver_hint == cgg_core::STRING_REF_HINT
            }),
            "captured a non-route: {:?}",
            f.references
                .iter()
                .map(|r| (&r.name, &r.receiver_hint))
                .collect::<Vec<_>>()
        );
        assert!(
            !f.definitions
                .iter()
                .any(|d| d.simple_name.starts_with("handler_at_")),
            "synthesized a handler for a plain block"
        );
    }

    #[test]
    fn a_scope_block_is_not_a_handler() {
        // `scope` is a registrar verb (Rails uses it) and its first
        // argument is a path, so every other gate passes — but its block
        // holds the route macros rather than a handler body. Naming it
        // would invent a callable owning every route inside it.
        let src = "defmodule R do\n  use Phoenix.Router\n\n  scope \"/\", MyAppWeb do\n    get \"/\", PageController, :home\n  end\nend\n";
        let f = extract(src);
        assert!(
            !f.definitions
                .iter()
                .any(|d| d.simple_name.starts_with("handler_at_")),
            "named a scope container: {:?}",
            f.definitions
                .iter()
                .map(|d| &d.simple_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn keyword_and_captured_targets_are_read() {
        // `to: Module` is how Plug and Rails-style macros name a target,
        // and `&Mod.fun/2` is Elixir's function capture. Written here
        // with `get`, which is a registered verb; `forward` becomes one
        // the moment a rule lists it.
        let src = "defmodule R do\n  use Plug.Router\n\n  get \"/api\", to: MyApp.ApiRouter\n  get \"/cap\", &MyMod.handler/2\nend\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .references
            .iter()
            .filter(|r| {
                r.receiver_hint == cgg_core::VALUE_REF_HINT
                    || r.receiver_hint == cgg_core::STRING_REF_HINT
            })
            .map(|r| r.name.as_str())
            .collect();
        assert!(
            names.contains(&"ApiRouter"),
            "keyword target lost: {names:?}"
        );
        assert!(names.contains(&"handler"), "captured fn lost: {names:?}");
        assert!(
            names.contains(&"MyMod::handler"),
            "captured fn owner lost: {names:?}"
        );
    }

    #[test]
    fn genuine_recursion_is_still_a_call() {
        // The head is suppressed by offset, not by name, so a real
        // recursive call in the body is kept.
        let src = "defmodule M do\n  def loop(n) do\n    loop(n - 1)\n  end\nend\n";
        let f = extract(src);
        assert_eq!(
            f.references.iter().filter(|r| r.name == "loop").count(),
            1,
            "expected exactly the body's recursive call"
        );
    }
}
