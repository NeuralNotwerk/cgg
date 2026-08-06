//! Root discovery.
//!
//! Reachability is only as good as its roots, so this is the part of the
//! engine that decides whether the whole analysis is useful or noise.
//! Every rule reads only fields cgg actually populates today, and every
//! root it produces carries the rule that fired and the evidence for it
//! — a whole-program claim that cannot be inspected cannot be argued
//! with, and therefore should not be believed.

use std::collections::{HashMap, HashSet};

use cgg_core::audit::{AuditUnresolvedCall, UnresolvedReason};
use cgg_core::deadcode::{RootKind, RootRecord};
use cgg_core::graph::Graph;
use cgg_core::ids::CallableId;
use cgg_core::{FileFacts, ImportRecord};

/// Names that are program entry points in every language cgg supports.
const UNIVERSAL_ENTRY: &[&str] = &["main"];

/// Language-specific entry points, keyed by plugin id.
const LANG_ENTRY: &[(&str, &[&str])] = &[
    ("go", &["init", "TestMain"]),
    ("elixir", &["start", "start_link", "init"]),
    ("erlang", &["start", "start_link", "init"]),
    ("c", &["_start", "WinMain", "DllMain"]),
    ("cpp", &["_start", "WinMain", "DllMain"]),
    ("zig", &["_start"]),
    ("asm", &["_start"]),
    ("rust", &["_start"]),
];

/// Rust traits whose methods are invoked by syntax, an operator, or a
/// derive rather than by any call a graph can see. `<T as Display>::fmt`
/// has no visible caller and never will.
const IMPLICIT_RUST_TRAITS: &[&str] = &[
    "Drop", "Display", "Debug", "Default", "From", "Into", "TryFrom", "TryInto", "Deref",
    "DerefMut", "Iterator", "IntoIterator", "DoubleEndedIterator", "ExactSizeIterator",
    "PartialEq", "Eq", "PartialOrd", "Ord", "Hash", "Clone", "Copy", "Serialize",
    "Deserialize", "Error", "FromStr", "Add", "Sub", "Mul", "Div", "Rem", "Neg", "Not",
    "AddAssign", "SubAssign", "Index", "IndexMut", "Fn", "FnMut", "FnOnce", "Future",
    "Send", "Sync", "Write", "Read", "AsRef", "AsMut", "Borrow", "BorrowMut", "ToString",
    // serde drives these from inside the deserializer.
    "Visitor", "Serializer", "Deserializer", "SeqAccess", "MapAccess",
];

/// Lifecycle hooks a framework or runtime invokes directly.
const LIFECYCLE: &[(&str, &[&str])] = &[
    ("erlang", &["handle_call", "handle_cast", "handle_info", "terminate", "code_change", "handle_continue"]),
    ("elixir", &["handle_call", "handle_cast", "handle_info", "terminate", "code_change", "handle_continue"]),
    ("java", &["main", "run", "call", "onCreate", "onStart", "onResume", "onPause", "onDestroy"]),
    ("kotlin", &["main", "run", "onCreate", "onStart", "onResume", "onPause", "onDestroy"]),
    ("csharp", &["Main", "Dispose", "ToString", "Equals", "GetHashCode"]),
    ("swift", &["viewDidLoad", "viewWillAppear", "applicationDidFinishLaunching"]),
    ("objc", &["viewDidLoad", "applicationDidFinishLaunching"]),
];

/// Dunder / conventional names invoked by the language runtime.
const DUNDER_LANGS: &[&str] = &["python", "ruby", "php", "lua", "javascript", "typescript"];
const CONVENTIONAL_METHODS: &[&str] = &[
    "initialize", "to_s", "toString", "equals", "hashCode", "Dispose", "describe",
];

/// Attribute markers that mean "called from outside this source tree".
const FFI_EXPORT_ATTRS: &[&str] = &[
    "no_mangle", "export_name", "pyfunction", "pymethods", "pyclass", "wasm_bindgen",
    "napi", "uniffi::export", "unsafe(no_mangle)",
];

/// Attribute markers for framework-invoked callables.
const FRAMEWORK_ATTRS: &[&str] = &[
    "app.route", "route", "get", "post", "put", "delete", "patch", "pytest.fixture",
    "fixture", "click.command", "click.group", "celery.task", "task", "tokio::main",
    "actix_web::main", "command", "event_handler",
];

/// Attribute markers for test cases and harness hooks.
const TEST_ATTRS: &[&str] = &[
    "test", "tokio::test", "async_std::test", "bench", "rstest", "proptest",
    "quickcheck", "test_case", "pytest.mark", "Test", "Fact", "Theory", "TestMethod",
];

/// The discovered root set, split by whether liveness proved through it
/// counts as production liveness or only as test liveness.
#[derive(Debug, Default)]
pub(crate) struct RootSet {
    pub(crate) records: Vec<RootRecord>,
    /// Node indices of production roots.
    pub(crate) production: Vec<u32>,
    /// Node indices of test roots.
    pub(crate) test: Vec<u32>,
    /// Which rules fired, per language, for the capability table.
    pub(crate) rules_by_language: HashMap<String, Vec<String>>,
}

impl RootSet {
    fn push(
        &mut self,
        graph: &Graph,
        id: CallableId,
        kind: RootKind,
        rule: &str,
        detail: String,
        seen: &mut HashSet<CallableId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        let Some(node) = graph.callables.get(&id) else { return };
        let Some(pos) = graph.callables.get_index_of(&id) else { return };
        if kind.is_test() {
            self.test.push(pos as u32);
        } else {
            self.production.push(pos as u32);
        }
        let rules = self.rules_by_language.entry(node.language.clone()).or_default();
        if !rules.iter().any(|r| r == rule) {
            rules.push(rule.to_string());
        }
        self.records.push(RootRecord {
            id,
            qualified_name: node.qualified_name.clone(),
            language: node.language.clone(),
            kind,
            rule: rule.to_string(),
            detail,
        });
    }
}

/// Strip the punctuation around an attribute so `#[tokio::test]`,
/// `@pytest.fixture(scope="module")` and `[Fact]` all compare as their
/// bare key. Mirrors vulture's `@foo.bar(x, y)` -> `@foo.bar` rule.
///
/// **Discards arguments by design.** A rule asking "is this a route?"
/// must not care which route. Anything that needs the route string
/// wants [`attribute_string_arg`] instead — the two are deliberately
/// separate accessors so neither can be mistaken for the other.
pub fn attribute_key(attr: &str) -> &str {
    let a = attr.trim();
    let a = a.strip_prefix("#[").unwrap_or(a);
    let a = a.strip_prefix('[').unwrap_or(a);
    let a = a.strip_prefix('@').unwrap_or(a);
    let a = a.strip_suffix(']').unwrap_or(a);
    let a = a.split('(').next().unwrap_or(a);
    let a = a.split('=').next().unwrap_or(a);
    a.trim()
}

/// First string-literal argument of an attribute, unquoted.
///
/// `@app.route("/users", methods=["POST"])` -> `Some("/users")`.
/// `#[get("/")]` -> `Some("/")`. `@celery.task` -> `None`.
///
/// This is the accessor [`attribute_key`] deliberately is not: an entry
/// node's identity is the route, and `attribute_key` throws the route
/// away. Handles `"`, `'` and Python's raw/f prefixes; stops at the
/// first literal because every framework in the inventory puts the path
/// first.
pub fn attribute_string_arg(attr: &str) -> Option<String> {
    let open = attr.find('(')?;
    let body = &attr[open + 1..];
    let mut chars = body.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '"' && c != '\'' {
            continue;
        }
        // Walk to the matching close quote, honouring backslash escapes.
        let mut j = i + c.len_utf8();
        let bytes = body.as_bytes();
        while j < bytes.len() {
            if bytes[j] == b'\\' {
                j += 2;
                continue;
            }
            if bytes[j] == c as u8 {
                return Some(body[i + 1..j].to_string());
            }
            j += 1;
        }
        return None;
    }
    None
}

/// Last `.`/`::`-separated segment of an attribute key: the verb.
/// `app.route` -> `route`, `org.springframework.GetMapping` ->
/// `GetMapping`, `get` -> `get`.
pub fn attribute_verb(attr: &str) -> &str {
    let key = attribute_key(attr);
    key.rsplit(|c| c == '.' || c == ':').next().unwrap_or(key)
}

fn has_attr(attrs: &[String], wanted: &[&str]) -> Option<String> {
    for a in attrs {
        let key = attribute_key(a);
        if wanted.iter().any(|w| key == *w) {
            return Some(a.trim().to_string());
        }
    }
    None
}

/// Discover roots.
///
/// `user_patterns` are `(pattern_label, matcher)` pairs supplied by the
/// caller; matching is left to the caller so this module stays free of
/// regex/glob policy.
pub(crate) fn discover(
    graph: &Graph,
    facts: &[FileFacts],
    user_matches: &[(String, CallableId)],
    framework_matches: &[(String, CallableId)],
) -> RootSet {
    let mut set = RootSet::default();
    let mut seen: HashSet<CallableId> = HashSet::new();

    // --- Rule: synthesized framework entry nodes -------------------------
    //
    // An entry node is where control enters the tree, so it is a root by
    // construction — and unlike the fiction it replaces, this one is
    // literally true. The node genuinely has in-degree zero and the
    // handler it points at genuinely has a caller.
    //
    // This is the one place a synthetic node may be a root. Every other
    // synthetic node is a *sink* (`<external>`, `<stdlib>`); marking one
    // of those live would say nothing.
    for (id, node) in &graph.callables {
        if node.framework_entry.is_some() {
            set.push(
                graph,
                *id,
                RootKind::FrameworkCallback,
                "framework:entry-node",
                format!("synthesized entry node `{}`", node.qualified_name),
                &mut seen,
            );
        }
    }

    // --- Rule: framework-invoked callables with no node ------------------
    //
    // Bucket D per §8: `Encoder.forward` has a framework caller but mints
    // no entry node, because one `Module.forward` node fanning out to
    // every model is visually useless. Without this rule the handler and
    // every private helper it calls are reported — the cascade that
    // doubles the cost of each missed entry point.
    for (label, id) in framework_matches {
        set.push(
            graph,
            *id,
            RootKind::FrameworkCallback,
            "framework:rule",
            format!("invoked by framework rule `{label}`"),
            &mut seen,
        );
    }

    // --- Rule: top-level invocation (all 44 languages) -------------------
    //
    // `intra_file` narrows a call to exactly one definition and then
    // discards the edge when there is no enclosing callable to hang it
    // on (a call at module top level). The callee is genuinely used, but
    // nothing in the graph says so. Without this rule every Python, JS,
    // Ruby, Lua and shell entry point reads as dead.
    let mut toplevel: HashSet<(String, String)> = HashSet::new();
    for u in &graph.unresolved {
        if matches!(u.reason, UnresolvedReason::NoEnclosingCallable) {
            let lang = graph
                .files
                .get(&u.file)
                .map(|f| f.language.clone())
                .unwrap_or_default();
            toplevel.insert((lang, u.name.clone()));
        }
    }
    if !toplevel.is_empty() {
        let by_site: HashMap<(String, String), &AuditUnresolvedCall> = graph
            .unresolved
            .iter()
            .filter(|u| matches!(u.reason, UnresolvedReason::NoEnclosingCallable))
            .map(|u| {
                let lang = graph
                    .files
                    .get(&u.file)
                    .map(|f| f.language.clone())
                    .unwrap_or_default();
                ((lang, u.name.clone()), u)
            })
            .collect();
        for (id, node) in &graph.callables {
            if node.synthetic {
                continue;
            }
            let key = (node.language.clone(), node.simple_name.clone());
            if let Some(u) = by_site.get(&key) {
                let path = graph
                    .files
                    .get(&u.file)
                    .map(|f| f.path.display().to_string())
                    .unwrap_or_default();
                set.push(
                    graph,
                    *id,
                    RootKind::TopLevelInvocation,
                    "toplevel:invocation",
                    format!("called from module top level at {path}:{}", u.site_line),
                    &mut seen,
                );
            }
        }
    }

    // --- Remaining per-node rules ---------------------------------------
    for (id, node) in &graph.callables {
        if node.synthetic {
            continue;
        }
        let lang = node.language.as_str();

        // Program entry points.
        let lang_entries = LANG_ENTRY
            .iter()
            .find(|(l, _)| *l == lang)
            .map(|(_, n)| *n)
            .unwrap_or(&[]);
        if UNIVERSAL_ENTRY.contains(&node.simple_name.as_str())
            || lang_entries.contains(&node.simple_name.as_str())
        {
            set.push(
                graph,
                *id,
                RootKind::ProgramEntry,
                "builtin:main",
                format!("entry point `{}`", node.simple_name),
                &mut seen,
            );
            continue;
        }

        // FFI exports: callers are outside this tree by definition.
        // Uses the shared classifier so the export/import distinction is
        // decided in exactly one place.
        if let Some((family, crate::ffi::FfiDirection::Export)) =
            crate::ffi::classify_ffi(&node.attributes)
        {
            set.push(
                graph,
                *id,
                RootKind::FfiExport,
                "ffi:export",
                format!("exported over {family}"),
                &mut seen,
            );
            continue;
        }
        if let Some(a) = has_attr(&node.attributes, FFI_EXPORT_ATTRS) {
            set.push(graph, *id, RootKind::FfiExport, "ffi:export", a, &mut seen);
            continue;
        }

        // Test cases and harness hooks.
        if let Some(a) = has_attr(&node.attributes, TEST_ATTRS) {
            set.push(graph, *id, RootKind::TestEntry, "builtin:test-attr", a, &mut seen);
            continue;
        }
        if is_test_name(lang, &node.simple_name) {
            set.push(
                graph,
                *id,
                RootKind::TestEntry,
                "builtin:test-name",
                format!("name `{}` matches a test convention", node.simple_name),
                &mut seen,
            );
            continue;
        }

        // Framework callbacks.
        if let Some(a) = has_attr(&node.attributes, FRAMEWORK_ATTRS) {
            set.push(
                graph,
                *id,
                RootKind::FrameworkCallback,
                "builtin:framework-attr",
                a,
                &mut seen,
            );
            continue;
        }

        // Trait methods invoked by syntax, an operator, or a derive.
        if lang == "rust" {
            if let Some(t) = &node.trait_impl_target {
                // `trait_impl_target` keeps generic arguments, so
                // `From<OutputFormatArg>` must be compared as `From`.
                let bare = t.split('<').next().unwrap_or(t).trim();
                if IMPLICIT_RUST_TRAITS.contains(&bare) {
                    set.push(
                        graph,
                        *id,
                        RootKind::LifecycleCallback,
                        "rust:implicit-trait",
                        format!("implements `{bare}`, invoked implicitly"),
                        &mut seen,
                    );
                    continue;
                }
            }
        }

        // Runtime-invoked lifecycle and conventional names.
        let lifecycle = LIFECYCLE
            .iter()
            .find(|(l, _)| *l == lang)
            .map(|(_, n)| *n)
            .unwrap_or(&[]);
        if lifecycle.contains(&node.simple_name.as_str())
            || CONVENTIONAL_METHODS.contains(&node.simple_name.as_str())
            || (DUNDER_LANGS.contains(&lang) && is_dunder(&node.simple_name))
        {
            set.push(
                graph,
                *id,
                RootKind::LifecycleCallback,
                "builtin:lifecycle",
                format!("`{}` is invoked by the runtime", node.simple_name),
                &mut seen,
            );
            continue;
        }
    }

    // --- Rule: declared exports are public API ---------------------------
    //
    // An exported name is reachable from outside the analyzed tree by
    // definition, so nothing inside it needs to call it.
    let mut exported: HashSet<String> = HashSet::new();
    for f in facts {
        for e in &f.exports {
            exported.insert(e.name.clone());
        }
    }
    if !exported.is_empty() {
        for (id, node) in &graph.callables {
            if node.synthetic {
                continue;
            }
            if exported.contains(&node.simple_name) {
                set.push(
                    graph,
                    *id,
                    RootKind::ExportedApi,
                    "lang:export",
                    format!("exported as `{}`", node.simple_name),
                    &mut seen,
                );
            }
        }
    }

    let reexported = collect_rust_reexports(facts);
    if !reexported.is_empty() {
        for (id, node) in &graph.callables {
            if node.synthetic || node.language != "rust" {
                continue;
            }
            let last = node
                .qualified_name
                .rsplit("::")
                .next()
                .unwrap_or(&node.qualified_name);
            if reexported.contains(last) {
                set.push(
                    graph,
                    *id,
                    RootKind::ExportedApi,
                    "rust:reexport",
                    format!("re-exported via `pub use` as `{last}`"),
                    &mut seen,
                );
            }
        }
    }

    // --- User-declared roots --------------------------------------------
    for (label, id) in user_matches {
        set.push(
            graph,
            *id,
            RootKind::UserDeclared,
            "user:roots",
            format!("matched `{label}`"),
            &mut seen,
        );
    }

    set.records.sort_by(|a, b| a.id.as_u32().cmp(&b.id.as_u32()));
    set.production.sort_unstable();
    set.test.sort_unstable();
    for v in set.rules_by_language.values_mut() {
        v.sort();
    }
    set
}

fn is_dunder(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__") && name.len() > 4
}

/// Name-based test detection. A stopgap until real per-file test
/// classification lands; it is deliberately conservative, and notably
/// rejects vulture's `*test*` glob, which swallows `latest`, `contest`
/// and `protest`.
fn is_test_name(lang: &str, name: &str) -> bool {
    match lang {
        "go" => {
            (name.starts_with("Test")
                || name.starts_with("Benchmark")
                || name.starts_with("Fuzz")
                || name.starts_with("Example"))
                && name.len() > 4
        }
        "python" => name.starts_with("test_") || name == "setUp" || name == "tearDown",
        "ruby" => name.starts_with("test_"),
        "rust" => false, // covered by #[test]; a bare `test_` fn is not a test
        _ => name.starts_with("test_") || name.starts_with("Test"),
    }
}

/// Names re-exported by a Rust `pub use`.
///
/// Reads `ImportRecord { kind: "pub-use" }` directly rather than
/// refactoring `cross_file::resolve`, whose re-export map is a local in
/// a 50 KB function.
fn collect_rust_reexports(facts: &[FileFacts]) -> HashSet<String> {
    let mut out = HashSet::new();
    for f in facts {
        if f.language != "rust" {
            continue;
        }
        for imp in &f.imports {
            if imp.kind != "pub-use" {
                continue;
            }
            let ImportRecord { path, alias, .. } = imp;
            let name = if alias.is_empty() {
                path.rsplit("::").next().unwrap_or(path).to_string()
            } else {
                alias.clone()
            };
            if !name.is_empty() && name != "*" {
                out.insert(name);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deadcode::testutil::{graph_with, node};

    #[test]
    fn attribute_key_normalizes_every_syntax() {
        assert_eq!(attribute_key("#[test]"), "test");
        assert_eq!(attribute_key("#[tokio::test]"), "tokio::test");
        assert_eq!(attribute_key("@pytest.fixture"), "pytest.fixture");
        assert_eq!(attribute_key("@app.route('/x')"), "app.route");
        assert_eq!(attribute_key("[Fact]"), "Fact");
        assert_eq!(attribute_key("#[export_name = \"x\"]"), "export_name");
    }

    #[test]
    fn main_is_a_production_root() {
        let g = graph_with(vec![node(0, "crate::main", "main", "rust")]);
        let set = discover(&g, &[], &[], &[]);
        assert_eq!(set.production.len(), 1);
        assert!(set.test.is_empty());
        assert_eq!(set.records[0].kind, RootKind::ProgramEntry);
    }

    #[test]
    fn ffi_exports_are_roots_because_callers_are_out_of_tree() {
        let mut n = node(0, "crate::ffi_entry", "ffi_entry", "rust");
        n.attributes = vec!["#[no_mangle]".into()];
        let set = discover(&graph_with(vec![n]), &[], &[], &[]);
        assert_eq!(set.records[0].kind, RootKind::FfiExport);
    }

    #[test]
    fn tests_are_a_separate_root_class_not_production() {
        let mut n = node(0, "crate::tests::it_works", "it_works", "rust");
        n.attributes = vec!["#[test]".into()];
        let set = discover(&graph_with(vec![n]), &[], &[], &[]);
        assert!(set.production.is_empty(), "a test must not be a production root");
        assert_eq!(set.test.len(), 1);
        assert_eq!(set.records[0].kind, RootKind::TestEntry);
    }

    #[test]
    fn implicit_rust_traits_are_roots() {
        let mut n = node(0, "<F as Display>::fmt", "fmt", "rust");
        n.trait_impl_target = Some("Display".into());
        let set = discover(&graph_with(vec![n]), &[], &[], &[]);
        assert_eq!(set.records[0].kind, RootKind::LifecycleCallback);

        // A user trait is NOT implicitly invoked.
        let mut m = node(0, "<F as Storage>::put", "put", "rust");
        m.trait_impl_target = Some("Storage".into());
        assert!(discover(&graph_with(vec![m]), &[], &[], &[]).records.is_empty());
    }

    #[test]
    fn go_test_naming_requires_more_than_the_prefix() {
        assert!(is_test_name("go", "TestFoo"));
        assert!(!is_test_name("go", "Test"));
        assert!(!is_test_name("python", "latest"), "vulture's *test* glob bug");
        assert!(!is_test_name("python", "contest"));
        assert!(is_test_name("python", "test_thing"));
    }

    #[test]
    fn dunder_methods_are_runtime_invoked() {
        let n = node(0, "m.C.__init__", "__init__", "python");
        let set = discover(&graph_with(vec![n]), &[], &[], &[]);
        assert_eq!(set.records[0].kind, RootKind::LifecycleCallback);
        assert!(!is_dunder("__x"));
        assert!(!is_dunder("____"));
        assert!(is_dunder("__init__"));
    }

    #[test]
    fn synthetic_nodes_are_never_roots() {
        let mut n = node(0, "ext::main", "main", "rust");
        n.synthetic = true;
        assert!(discover(&graph_with(vec![n]), &[], &[], &[]).records.is_empty());
    }

    #[test]
    fn roots_are_deduplicated_and_ordered_by_id() {
        let g = graph_with(vec![
            node(0, "crate::main", "main", "rust"),
            node(1, "crate::helper", "helper", "rust"),
        ]);
        // Declare main again as a user root; it must not appear twice.
        let set = discover(&g, &[], &[("^main$".into(), CallableId::new(0))], &[]);
        assert_eq!(set.records.len(), 1);
        assert_eq!(set.records[0].rule, "builtin:main");
    }

    #[test]
    fn generic_trait_arguments_do_not_defeat_the_implicit_trait_rule() {
        // `trait_impl_target` keeps generics, so an exact-match check
        // silently missed every parameterised std trait impl.
        let mut n = node(0, "<T as From<OutputFormatArg>>::from", "from", "rust");
        n.trait_impl_target = Some("From<OutputFormatArg>".into());
        let set = discover(&graph_with(vec![n]), &[], &[], &[]);
        assert_eq!(set.records.len(), 1, "From<...> should match From");
        assert_eq!(set.records[0].kind, RootKind::LifecycleCallback);
    }

    #[test]
    fn serde_visitor_methods_are_runtime_invoked() {
        let mut n = node(0, "<R as Visitor<'de>>::visit_map", "visit_map", "rust");
        n.trait_impl_target = Some("Visitor<'de>".into());
        assert_eq!(discover(&graph_with(vec![n]), &[], &[], &[]).records.len(), 1);
    }

}
