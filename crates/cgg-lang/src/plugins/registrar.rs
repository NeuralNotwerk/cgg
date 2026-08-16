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

/// Second-to-last segment of a dotted path: `views.SiteView.as_view`
/// -> `SiteView`.
fn qualifier_of(path: &str) -> Option<&str> {
    let cut = path.rfind(['.', ':', '>'])?;
    let head = path[..cut].trim_end_matches([':', '-', '.', '>']);
    let seg = last_segment(head);
    (!seg.is_empty()).then_some(seg)
}

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
        .chain(p.rfind(['.', '/', '>', '\\']).map(|i| i + 1))
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
/// inline handler: `verb("identity", ...closure)` or `verb(closure)`.
///
/// The gate on synthesizing a name for an anonymous handler. Naming
/// *every* anonymous callback would mint a node for every `.map(x =>
/// …)` and every promise chain in the tree — a large cost paid on every
/// run for a signal only a framework rule can use.
///
/// The leading string used to be **required**, which cost three whole
/// platforms: `Deno.serve((req) => …)`, Firebase's
/// `onRequest((req, res) => …)` and Express middleware `app.use(fn)`
/// carry no route, and every one of them enumerated nothing. The string
/// was never what made this safe — the *caller's* verb gate is, and
/// `describe`, `it`, `map`, `then` and `setTimeout` are not registrar
/// verbs in any rule. So a closure alone is now enough, and the route
/// comes back empty rather than absent.
pub(crate) fn is_registration_shape(call: Node, source: &[u8]) -> Option<String> {
    let args = arguments_of(call)?;
    let mut cursor = args.walk();
    let named: Vec<Node> = args.named_children(&mut cursor).collect();
    if named.len() > 4 {
        return None;
    }
    let route = string_within(*named.first()?, source).unwrap_or_default();
    // A closure sitting in an options object counts as handed off, the
    // same as one in argument position — `app.http("name", { handler:
    // async () => … })` is a registration by any reading.
    // When there is no route string the closure can be argument zero
    // (`Deno.serve(handler)`), so scan every slot rather than skipping
    // the first.
    let scan_from = if route.is_empty() { 0 } else { 1 };
    let has_closure = named.iter().skip(scan_from).any(|n| {
        is_kind(*n, CLOSURE_KINDS)
            || (n.kind() == "object" && {
                let mut c = n.walk();
                n.named_children(&mut c).any(|pair| {
                    let mut v = pair.walk();
                    pair.named_children(&mut v)
                        .any(|x| is_kind(x, CLOSURE_KINDS))
                })
            })
    });
    if !has_closure && trailing_block(call).is_none() {
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
/// ones, ones inside an options object, and the trailing block.
///
/// The options-object form is not an edge case. Azure Functions' v4
/// model writes every handler as
/// `app.http("name", { handler: async (req, ctx) => … })`, and a whole
/// runtime enumerating nothing is what looking only at argument
/// position costs. One level deep: a closure nested further than that
/// is not the thing being registered.
pub(crate) fn inline_closures<'t>(call: Node<'t>) -> Vec<Node<'t>> {
    let mut out = Vec::new();
    if let Some(args) = arguments_of(call) {
        let mut cursor = args.walk();
        for arg in args.named_children(&mut cursor) {
            if is_closure(arg) {
                out.push(arg);
                continue;
            }
            if arg.kind() == "object" {
                let mut pc = arg.walk();
                for pair in arg.named_children(&mut pc) {
                    let mut vc = pair.walk();
                    out.extend(pair.named_children(&mut vc).filter(|n| is_closure(*n)));
                }
            }
        }
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
            && let Some(base) = t.trim().strip_suffix("::class")
        {
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
pub(crate) fn capture(
    ctx: &crate::ExtractCtx<'_>,
    call: Node,
    source: &[u8],
    context: &str,
) -> Vec<RefRecord> {
    let mut out = Vec::new();
    // Only calls whose verb some framework rule could match. Without
    // this gate every `foo(x)` in the tree pays for an argument scan and
    // contributes an inert record — measured on TypeORM that was the
    // whole of a 74% slowdown.
    if !ctx.is_registrar_verb(last_segment(context)) {
        return out;
    }
    let _s = cgg_core::profile::span("extract::registrar-capture");
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
                kwargs: Vec::new(),
            });
            continue;
        }

        collect_value_refs(ctx, arg, source, context, &route, line, &mut out, 0);
    }
    out
}

/// Value references in the argument slots of an *ordinary* call.
///
/// [`capture`] is gated to verbs some framework rule could match,
/// because computing a route and scanning strings for every call in a
/// tree was measured as the whole of a 74% slowdown on TypeORM. But a
/// function passed as a value is not a framework concern:
/// `callback=_validate_key` in a click decorator,
/// `event.listen(cls, "before_update", cls._updated_at)`,
/// `staticmethod(_lazy_sha1)`, `synonym(descriptor=property(_get, _set))`.
/// No rule registers those verbs, yet each reference is the only thing
/// keeping its target alive. Measured across flask, httpie, black,
/// flaskbb and dispatch, their absence produced 28 of 45 false
/// positives in the dead-code top band.
///
/// So this pass runs ungated but does strictly less than [`capture`]:
/// no route, no string references, value references only. Strings are
/// both what made the registrar path expensive and what makes it
/// framework-specific; a bare identifier in argument position is
/// neither.
pub(crate) fn capture_value_refs(
    ctx: &crate::ExtractCtx<'_>,
    call: Node,
    source: &[u8],
    context: &str,
) -> Vec<RefRecord> {
    // Registrar verbs already went through `capture`, which emits a
    // superset of this. Running both would duplicate every record.
    if ctx.is_registrar_verb(last_segment(context)) {
        return Vec::new();
    }
    let Some(args) = arguments_of(call) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut cursor = args.walk();
    for arg in args.named_children(&mut cursor) {
        // A string in an ordinary call is data, not a handler name.
        // Only the framework rules can tell `'photos#index'` from a
        // log message, and they are not consulted here.
        if string_within(arg, source).is_some() {
            continue;
        }
        let line = (arg.start_position().row as u32) + 1;
        collect_value_refs(ctx, arg, source, context, "", line, &mut out, 0);
    }
    // `collect_value_refs` still mints string refs from inside
    // containers (`[C::class, 'method']`). Those belong to the gated
    // path; drop them so this pass cannot manufacture a routing claim.
    out.retain(|r| r.receiver_hint == VALUE_REF_HINT);
    out
}

/// Value references in a single value position that is not an argument
/// slot — the right-hand side of an assignment, most often.
///
/// A dispatch table (`DAEMONIZED_TASKS = {'check_status': _check_status}`),
/// a class-level filter map (`_FILTERS = {Post: _post_clause}`) or a
/// plain alias (`__setitem__ = _fail`) names its targets somewhere
/// [`capture_value_refs`] never looks, because no call encloses them.
/// Measured on flaskbb, httpie, black and flask these were 6 of the
/// remaining dead-code false positives.
///
/// The caller decides which nodes are value positions; passing a call
/// node here would double-count the callee.
pub(crate) fn capture_value_position(
    ctx: &crate::ExtractCtx<'_>,
    node: Node,
    source: &[u8],
    context: &str,
) -> Vec<RefRecord> {
    let mut out = Vec::new();
    let line = (node.start_position().row as u32) + 1;
    collect_value_refs(ctx, node, source, context, "", line, &mut out, 0);
    out.retain(|r| r.receiver_hint == VALUE_REF_HINT);
    out
}

/// Pull every callable-shaped name out of one argument slot.
///
/// Recurses into nested calls and array/hash literals so `get(handler)`
/// (axum), `[Controller::class, 'method']` (Laravel 8+) and
/// `to: 'photos#index'` (Rails) all give up their payload.
// Eight parameters, one over clippy's threshold, because `ctx` joined the
// recursion's existing walk state. Same rationale as the crate-level allow
// in `cgg/src/lib.rs`: threading it explicitly is what removed the
// process-globals, and bundling the walk state into a struct to satisfy a
// count would put the state back behind an indirection.
#[allow(clippy::too_many_arguments)]
fn collect_value_refs(
    ctx: &crate::ExtractCtx<'_>,
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
            // Python's dict/set literals. A dispatch table is the
            // commonest way to name a handler without calling it:
            // `{'check_status': _check_status}`.
            | "dictionary"
            | "set"
            | "hash"
            | "keyword_argument"
            | "tuple"
            | "argument"
            // A JS/TS options object. `pair` was already here, but
            // nothing descended into the `object` holding the pairs, so
            // every options-bag registration was invisible:
            // `new lambda.Function(this, "Api", { handler: "app.handler" })`
            // is how CDK names every Lambda in a TypeScript stack.
            | "object"
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
                kwargs: Vec::new(),
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
                    kwargs: Vec::new(),
                });
            } else {
                collect_value_refs(ctx, child, source, context, route, line, out, depth);
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
        // Only inside a call that carries a route. A registrar verb on
        // its own proves nothing: Django's ORM is `Model.objects.get(...)`
        // and `.filter(...)`, and `get`/`filter` are route verbs too, so
        // an ungated descent walked every ORM call in the tree. On netbox
        // that produced ~95,000 references that bound to nothing. A
        // wrapped handler always sits beside the path it is registered
        // at, so requiring the route costs no real case and removes the
        // false ones.
        if route.is_empty() || depth >= MAX_WRAPPER_DEPTH {
            return;
        }
        let callee = callee_of(arg)
            .map(|n| n.utf8_text(source).unwrap_or("").trim().to_string())
            .unwrap_or_default();
        if callee.is_empty() || ctx.is_registrar_verb(last_segment(&callee)) {
            return;
        }
        let before = out.len();
        if let Some(inner) = arguments_of(arg) {
            let mut cursor = inner.walk();
            for child in inner.named_children(&mut cursor) {
                collect_value_refs(
                    ctx,
                    child,
                    source,
                    context,
                    route,
                    line,
                    out,
                    depth + 1,
                );
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
                kwargs: Vec::new(),
            });
            // `SiteView.as_view()` is a class adapter: the callable the
            // framework ends up invoking is a *method of the qualifier*,
            // not `as_view` itself, and cgg has no node for a type to
            // point at. Emit the qualifier too and let the rule engine
            // try it as an owner.
            //
            // Only when the registration carries a route. Without that
            // gate this fires on every wrapped call in the tree: on
            // `black` it emitted ~2,400 module-level references that
            // bound to nothing, inflating the unresolved-call count from
            // 575 to 3,009 and costing ~25% of the run. A qualifier is
            // only ever useful when there is a route to attach its
            // handler to.
            if !route.is_empty()
                && let Some(owner) = qualifier_of(&callee)
            {
                out.push(RefRecord {
                    name: owner.to_string(),
                    receiver_hint: VALUE_REF_HINT.to_string(),
                    site_line: line,
                    site_byte: arg.start_byte() as u32,
                    context: context.to_string(),
                    route: route.to_string(),
                    kwargs: Vec::new(),
                });
            }
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
        kwargs: Vec::new(),
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
