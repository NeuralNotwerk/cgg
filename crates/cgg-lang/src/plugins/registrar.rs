//! Registrar-call argument capture — the shape-B/C/E extraction step.
//!
//! A framework hands control to user code in ways that are not calls.
//! Three of the six shapes in the design put the handler in *argument
//! position* of a registration call:
//!
//! ```text
//! app.get("/users", listUsers)          // B — callable passed by name
//! app.get("/users", (req, res) => {})   // C — inline closure
//! Route::get('/users', 'C@index')       // E — a string names the target
//! ```
//!
//! In all three the interesting facts are the same: which call this was
//! (`app.get`), what identity it carries (`"/users"`), and what sits in
//! the remaining argument slots. This module extracts exactly that, for
//! any tree-sitter grammar, and emits it as [`RefRecord`]s carrying the
//! [`VALUE_REF_HINT`] / [`STRING_REF_HINT`] sentinels.
//!
//! **It knows nothing about frameworks.** It cannot: deciding that
//! `app.get` is a route and `logger.get` is not requires knowing which
//! packages the file imports, which is a whole-project question. So this
//! pass is deliberately over-eager and the framework rule engine in
//! `cgg-resolve::frameworks` does the gating. An ungated record here is
//! inert — value refs already never reach the unresolved buckets, and
//! string refs never become edges at all.

use cgg_core::{RefRecord, STRING_REF_HINT, VALUE_REF_HINT};
use tree_sitter::Node;

/// Node kinds holding an argument list, across the grammars cgg links.
const ARG_LIST_KINDS: &[&str] = &[
    "arguments",
    "argument_list",
    "argument_list_with_parens",
    "parenthesized_expression",
];

/// Node kinds that are a bare reference to something nameable.
const IDENT_KINDS: &[&str] = &[
    "identifier",
    "scoped_identifier",
    "qualified_identifier",
    "field_expression",
    "member_expression",
    "selector_expression",
    "attribute",
    "name",
    "qualified_name",
    "simple_identifier",
    "navigation_expression",
    "constant",
    "scope_resolution",
    "variable_name",
    "class_constant_access_expression",
    "reference_expression",
    "symbol",
    "simple_symbol",
    "hash",
    "pair",
    "keyword_argument",
    "array_creation_expression",
    "array",
    "list",
];

/// Node kinds that are a string literal.
const STRING_KINDS: &[&str] = &[
    "string",
    "string_literal",
    "interpreted_string_literal",
    "raw_string_literal",
    "encapsed_string",
    "string_content",
    "char_literal",
    "verbatim_string_literal",
    "line_str_text",
    "template_string",
    "quoted_attribute_value",
];

/// Node kinds that are a call.
const CALL_KINDS: &[&str] = &[
    "call_expression",
    "call",
    "method_invocation",
    "invocation_expression",
    "function_call_expression",
    "method_call",
    "scoped_call_expression",
];

/// Node kinds that are an anonymous function written in place.
const CLOSURE_KINDS: &[&str] = &[
    "arrow_function",
    "function_expression",
    "function",
    "lambda",
    "lambda_expression",
    "closure_expression",
    "anonymous_function",
    "anonymous_function_creation_expression",
    "func_literal",
    "block",
    "do_block",
    "lambda_literal",
];

/// How far to follow a chain of handler wrappers before giving up.
const MAX_WRAPPER_DEPTH: u8 = 3;

/// The callee of a call node, across the grammars cgg links.
fn callee_of<'t>(call: Node<'t>) -> Option<Node<'t>> {
    for f in ["function", "method", "name"] {
        if let Some(n) = call.child_by_field_name(f) {
            return Some(n);
        }
    }
    call.named_child(0)
}

fn is_kind(node: Node, set: &[&str]) -> bool {
    set.contains(&node.kind())
}

/// Unquote a string literal node's text. Handles `"`, `'`, backticks,
/// and Rust/Python raw prefixes; returns the inner text unchanged if no
/// recognisable quoting is present.
pub(crate) fn unquote(raw: &str) -> String {
    let s = raw.trim();
    // Drop a leading literal prefix: r"", b"", f"", u"", rb"", @"".
    // Only when a quote actually follows it — otherwise the token is a
    // bare identifier and must come back unchanged.
    let s = match s.find(['"', '\'', '`']) {
        Some(q)
            if q > 0
                && s[..q]
                    .chars()
                    .all(|c| c.is_ascii_alphabetic() || c == '@' || c == '#') =>
        {
            &s[q..]
        }
        _ => s,
    };
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        if matches!(first, b'"' | b'\'' | b'`') && bytes[bytes.len() - 1] == first {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}

/// Last `.`/`::`/`->`/`/`-separated segment of a dotted path.
pub(crate) fn last_segment(path: &str) -> &str {
    let p = path.trim();
    let cut = p
        .rfind("::")
        .map(|i| i + 2)
        .into_iter()
        .chain(
            p.rfind(['.', '/', '>', '\\'])
                .map(|i| i + 1),
        )
        .max();
    match cut {
        Some(i) if i < p.len() => &p[i..],
        _ => p,
    }
}

/// Find the argument-list node of a call.
pub(crate) fn arguments_of<'t>(call: Node<'t>) -> Option<Node<'t>> {
    for f in ["arguments", "argument_list", "parameters"] {
        if let Some(n) = call.child_by_field_name(f) {
            return Some(n);
        }
    }
    let mut cursor = call.walk();
    call.named_children(&mut cursor)
        .find(|c| is_kind(*c, ARG_LIST_KINDS))
}

/// Whether a call has the shape of a registration that hands off an
/// inline handler: `verb("identity", ...closure)`.
///
/// The gate on synthesizing a name for an anonymous handler. Naming
/// *every* anonymous callback would mint a node for every `.map(x =>
/// …)` and every promise chain in the tree — a large cost paid on every
/// run for a signal only a framework rule can use. Requiring a leading
/// string literal and a short argument list restricts it to the shape
/// that actually carries an identity worth naming.
pub(crate) fn is_registration_shape(call: Node, source: &[u8]) -> Option<String> {
    let args = arguments_of(call)?;
    let mut cursor = args.walk();
    let named: Vec<Node> = args.named_children(&mut cursor).collect();
    if named.len() > 4 {
        return None;
    }
    let route = string_within(*named.first()?, source)?;
    if !named.iter().skip(1).any(|n| is_kind(*n, CLOSURE_KINDS))
        && trailing_block(call).is_none()
    {
        return None;
    }
    Some(route)
}

/// A trailing `do … end` / `{ … }` block, which Ruby's grammar hangs off
/// the call's `block` field rather than putting in its argument list.
///
/// Sinatra writes every route this way — `get "/x" do … end` — so
/// without this the handler is not in argument position and the whole
/// framework enumerates nothing.
pub(crate) fn trailing_block<'t>(call: Node<'t>) -> Option<Node<'t>> {
    let b = call.child_by_field_name("block")?;
    is_kind(b, CLOSURE_KINDS).then_some(b)
}

/// Every inline closure a registration call hands off: argument-position
/// ones and the trailing block.
pub(crate) fn inline_closures<'t>(call: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    if let Some(args) = arguments_of(call) {
        let mut cursor = args.walk();
        out.extend(args.named_children(&mut cursor).filter(|n| is_closure(*n)));
    }
    out.extend(trailing_block(call));
    out
}

/// First string-literal argument of `call`, if any.
fn string_arg_of(call: Node, source: &[u8]) -> Option<String> {
    let args = arguments_of(call)?;
    let mut cursor = args.walk();
    for arg in args.named_children(&mut cursor) {
        if let Some(s) = string_within(arg, source) {
            return Some(s);
        }
    }
    None
}

/// Text of `node` if it is a string literal, or of the single string
/// literal wrapped inside it.
///
/// The wrapper hop matters: PHP wraps every argument in an `argument`
/// node, Ruby wraps content in `string_content`. Without it the route
/// string is invisible and every entry node loses its identity.
fn string_within(node: Node, source: &[u8]) -> Option<String> {
    if is_kind(node, STRING_KINDS) {
        let raw = node.utf8_text(source).ok()?;
        let v = unquote(raw);
        return if v.is_empty() { None } else { Some(v) };
    }
    if matches!(
        node.kind(),
        "argument" | "array_element_initializer" | "spread_element"
    ) {
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(s) = string_within(child, source) {
                return Some(s);
            }
        }
    }
    None
}

/// `[UserController::class, 'index']` -> `("UserController", "index")`.
///
/// Laravel 8 replaced the `'Controller@method'` string with this array,
/// and both forms are still in the wild. Pairing them here rather than
/// emitting two unrelated references is what lets the rule engine bind
/// the *right* `index` when a project has a dozen of them.
fn class_method_pair(node: Node, source: &[u8]) -> Option<(String, String)> {
    let mut class_name: Option<String> = None;
    let mut method: Option<String> = None;
    fn scan(
        n: Node,
        source: &[u8],
        class_name: &mut Option<String>,
        method: &mut Option<String>,
        depth: u8,
    ) {
        if depth > 4 {
            return;
        }
        if (n.kind() == "class_constant_access_expression"
            || n.kind() == "scoped_property_access_expression")
            && let Ok(t) = n.utf8_text(source)
                && let Some(base) = t.trim().strip_suffix("::class") {
                    *class_name = Some(last_segment(base.trim()).to_string());
                    return;
                }
        if let Some(s) = string_within(n, source) {
            if method.is_none() {
                *method = Some(s);
            }
            return;
        }
        let mut cursor = n.walk();
        for c in n.named_children(&mut cursor) {
            scan(c, source, class_name, method, depth + 1);
        }
    }
    scan(node, source, &mut class_name, &mut method, 0);
    match (class_name, method) {
        (Some(c), Some(m)) => Some((c, m)),
        _ => None,
    }
}

/// Route identity for a call: its own first string argument, or — when
/// it has none — the first string argument of an enclosing call.
///
/// The fallback is what makes axum work. `Router::route("/users",
/// get(list_users))` puts the handler inside `get(...)`, which carries
/// no path of its own; without inheriting the outer call's string the
/// entry node would be an anonymous `get`, and a list of anonymous
/// routes is not an attack-surface map.
fn route_for(call: Node, source: &[u8]) -> String {
    if let Some(s) = string_arg_of(call, source) {
        return s;
    }
    let mut cur = call;
    // Two levels is enough for every builder chain in the inventory and
    // keeps an unrelated outer string from being claimed as a route.
    for _ in 0..2 {
        let Some(parent) = enclosing_call(cur) else {
            break;
        };
        if let Some(s) = string_arg_of(parent, source) {
            return s;
        }
        cur = parent;
    }
    String::new()
}

/// The route identity a call carries, for plugins that do their own
/// argument walking and only need this part.
pub(crate) fn route_of(call: Node, source: &[u8]) -> String {
    route_for(call, source)
}

/// Nearest enclosing call expression, stopping at any function body so
/// we never traverse out of the expression we are describing.
fn enclosing_call<'t>(node: Node<'t>) -> Option<Node<'t>> {
    let mut cur = node.parent()?;
    loop {
        if cur.kind().contains("call") || cur.kind() == "method_invocation" {
            return Some(cur);
        }
        if is_kind(cur, CLOSURE_KINDS)
            || cur.kind().contains("function_definition")
            || cur.kind().contains("function_declaration")
            || cur.kind() == "function_item"
            || cur.kind() == "method_declaration"
        {
            return None;
        }
        cur = cur.parent()?;
    }
}

/// Capture the argument slots of a registration-shaped call.
///
/// `context` is the callee as written (`app.get`, `Route::get`,
/// `HandleFunc`); it lands on every emitted record so the framework rule
/// engine can ask which call this was. Returns an empty vector for calls
/// with no reference-shaped or string-shaped arguments, which is the
/// overwhelming majority — this pass adds records only where a handoff
/// could plausibly be happening.
pub(crate) fn capture(call: Node, source: &[u8], context: &str) -> Vec<RefRecord> {
    let mut out = Vec::new();
    // Only calls whose verb some framework rule could match. Without
    // this gate every `foo(x)` in the tree pays for an argument scan and
    // contributes an inert record — measured on TypeORM that was the
    // whole of a 74% slowdown.
    if !crate::is_registrar_verb(last_segment(context)) {
        return out;
    }
    let Some(args) = arguments_of(call) else {
        return out;
    };
    let route = route_for(call, source);
    let mut seen_first_string = false;
    let arg_count = {
        let mut c = args.walk();
        args.named_children(&mut c).count()
    };

    let mut cursor = args.walk();
    for arg in args.named_children(&mut cursor) {
        let line = (arg.start_position().row as u32) + 1;
        let byte = arg.start_byte() as u32;

        if let Some(s) = string_within(arg, source) {
            // The first string is the route itself, already captured;
            // later ones may name the handler (`'photos#index'`).
            //
            // Unless it is the *only* argument: `new Worker('./w.js')`
            // is shape F, where the identity and the target are the same
            // string. Dropping it would leave the worker module with no
            // caller at all — the case where an entire file reads as
            // dead.
            if !seen_first_string && s == route && arg_count > 1 {
                seen_first_string = true;
                continue;
            }
            out.push(RefRecord {
                name: s,
                receiver_hint: STRING_REF_HINT.to_string(),
                site_line: line,
                site_byte: byte,
                context: context.to_string(),
                route: route.clone(),
            });
            continue;
        }

        collect_value_refs(arg, source, context, &route, line, &mut out, 0);
    }
    out
}

/// Pull every callable-shaped name out of one argument slot.
///
/// Recurses into nested calls and array/hash literals so `get(handler)`
/// (axum), `[Controller::class, 'method']` (Laravel 8+) and
/// `to: 'photos#index'` (Rails) all give up their payload.
fn collect_value_refs(
    arg: Node,
    source: &[u8],
    context: &str,
    route: &str,
    line: u32,
    out: &mut Vec<RefRecord>,
    depth: u8,
) {
    let kind = arg.kind();

    // Composite argument: descend into the literal containers that hold
    // a target — an array, a keyword pair, PHP's `argument` wrapper.
    //
    // Deliberately NOT into a nested call. The walker visits every call
    // itself, so `get(handler)` inside `.route("/x", …)` is captured on
    // its own turn, with the route inherited upward. Descending here as
    // well would do the same work once per enclosing level — quadratic
    // in the depth of a builder chain, which is exactly where these
    // expressions live.
    if matches!(
        kind,
        "array_creation_expression"
            | "array"
            | "list"
            | "pair"
            | "hash"
            | "keyword_argument"
            | "tuple"
            | "argument"
    ) {
        // `[C::class, 'method']` is one target, not two loose names.
        if let Some((owner, method)) = class_method_pair(arg, source) {
            out.push(RefRecord {
                name: format!("{owner}::{method}"),
                receiver_hint: STRING_REF_HINT.to_string(),
                site_line: line,
                site_byte: arg.start_byte() as u32,
                context: context.to_string(),
                route: route.to_string(),
            });
            return;
        }
        let mut cursor = arg.walk();
        for child in arg.named_children(&mut cursor) {
            if let Some(s) = string_within(child, source) {
                out.push(RefRecord {
                    name: s,
                    receiver_hint: STRING_REF_HINT.to_string(),
                    site_line: line,
                    site_byte: child.start_byte() as u32,
                    context: context.to_string(),
                    route: route.to_string(),
                });
            } else {
                collect_value_refs(child, source, context, route, line, out, depth);
            }
        }
        return;
    }

    // A handler wrapped in a helper: `r.Get("/x",
    // chain.ToHandlerFunc(ctrl.Handle()))`. The wrapper is a call, and
    // the walker's own visit to it contributes nothing — `capture` bails
    // on a verb no framework registers — so descending here is not the
    // double work the comment above warns about. Registrar verbs are
    // still left alone; those are captured on their own turn.
    //
    // Only the innermost callee is the handler. Recursing first and
    // emitting this call's own name only when the recursion found
    // nothing is what distinguishes `ToHandlerFunc` (a wrapper, to be
    // skipped) from `ctrl.Handle` (the target).
    if is_kind(arg, CALL_KINDS) {
        if depth >= MAX_WRAPPER_DEPTH {
            return;
        }
        let callee = callee_of(arg)
            .map(|n| n.utf8_text(source).unwrap_or("").trim().to_string())
            .unwrap_or_default();
        if callee.is_empty() || crate::is_registrar_verb(last_segment(&callee)) {
            return;
        }
        let before = out.len();
        if let Some(inner) = arguments_of(arg) {
            let mut cursor = inner.walk();
            for child in inner.named_children(&mut cursor) {
                collect_value_refs(child, source, context, route, line, out, depth + 1);
            }
        }
        if out.len() == before {
            out.push(RefRecord {
                name: last_segment(&callee).to_string(),
                receiver_hint: VALUE_REF_HINT.to_string(),
                site_line: line,
                site_byte: arg.start_byte() as u32,
                context: context.to_string(),
                route: route.to_string(),
            });
        }
        return;
    }

    if !is_kind(arg, IDENT_KINDS) {
        return;
    }
    let Ok(text) = arg.utf8_text(source) else {
        return;
    };
    let text = text.trim();
    if text.is_empty() || text.len() > 200 {
        return;
    }
    // A reference has to look like one. Anything with an operator or a
    // space in it is an expression, not a name.
    if text.contains(|c: char| {
        c.is_whitespace() || matches!(c, '(' | ')' | '{' | '}' | '+' | '=' | '|')
    }) {
        return;
    }
    let simple = last_segment(text).trim_start_matches(&[':', '&', '$'][..]);
    if simple.is_empty() || !simple.starts_with(|c: char| c.is_alphabetic() || c == '_') {
        return;
    }
    out.push(RefRecord {
        name: simple.to_string(),
        receiver_hint: VALUE_REF_HINT.to_string(),
        site_line: line,
        site_byte: arg.start_byte() as u32,
        context: context.to_string(),
        route: route.to_string(),
    });
}

/// Whether a node is an inline anonymous function.
pub(crate) fn is_closure(node: Node) -> bool {
    is_kind(node, CLOSURE_KINDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquote_strips_every_quoting_style() {
        assert_eq!(unquote("\"/users\""), "/users");
        assert_eq!(unquote("'/users'"), "/users");
        assert_eq!(unquote("r\"/users\""), "/users");
        assert_eq!(unquote("`/users`"), "/users");
        assert_eq!(unquote("f\"/users\""), "/users");
        // Not a quoted literal — returned unchanged.
        assert_eq!(unquote("handler"), "handler");
    }

    #[test]
    fn last_segment_handles_every_path_joiner() {
        assert_eq!(last_segment("views.index"), "index");
        assert_eq!(last_segment("a::b::c"), "c");
        assert_eq!(last_segment("App\\Http\\C"), "C");
        assert_eq!(last_segment("bare"), "bare");
        // Trailing separator must not produce an empty segment.
        assert_eq!(last_segment("a."), "a.");
    }
}
