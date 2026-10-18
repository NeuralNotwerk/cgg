//! Lightweight type propagation for receiver-hint rewriting.
//!
//! Scans each file's definitions and references to infer variable types
//! from:
//! 1. **Parameter type annotations** — `fn foo(x: Service)` means `x`
//!    has type `Service` inside `foo`.
//! 2. **Constructor assignments** — `let x = Foo::new()`, `x = Foo()`,
//!    `var x = new Foo()` means `x` has type `Foo`.
//! 3. **Typed variable declarations** — `Foo x = ...` (Java/C#/C++).
//!
//! The output is a rewritten set of `RefRecord`s where `receiver_hint`
//! has been replaced with the inferred type name when possible. This
//! lets the intra-file linker match `x.method()` against
//! `Foo::method` / `Foo.method`.

use cgg_core::{
    DEREF_SUFFIX, DefRecord, FileFacts, last_scope_segment, template_pointee,
};
use std::collections::HashMap;

/// Per language: lowercased return type → `(function, return type)`.
type ByLowerRet<'a> = HashMap<&'a str, HashMap<String, Vec<(&'a str, &'a str)>>>;

/// `return_types`, pre-indexed by the lowercased return type.
///
/// Strategy 4 asks, for one receiver name, "which functions return a
/// type whose lowercased name equals this receiver, or equals it minus
/// a trailing `s`". Answering that by walking the whole map is
/// O(receivers x map), and the map is corpus-wide: on `erlang-otp` that
/// was **817,607,605 iterations**, each allocating a `to_lowercase()`
/// and a `format!`. The lowercasing depends only on the map, so it is
/// hoisted here and done once per run instead of once per receiver.
///
/// Keys are the language, then the lowercased return type. A receiver
/// `configs` finds exact matches under `configs` and plural matches under
/// `config`, which is the same two-way test the filter did inline. A
/// receiver is only ever typed from its own language's functions.
#[derive(Debug, Default)]
pub struct ReturnTypeIndex<'a> {
    by_lower_ret: ByLowerRet<'a>,
    /// `(language, function)` → the return type its definitions agree on.
    by_name: HashMap<(&'a str, &'a str), &'a str>,
    empty: bool,
}

impl<'a> ReturnTypeIndex<'a> {
    /// `return_types` maps `(language, function simple name)` to the
    /// return type every definition of that name in the language declares
    /// ([`build_return_type_map`]).
    pub fn build(return_types: &HashMap<(&'a str, &'a str), &'a str>) -> Self {
        let mut by_lower_ret: ByLowerRet<'a> = HashMap::new();
        for (&(lang, f), &r) in return_types {
            by_lower_ret
                .entry(lang)
                .or_default()
                .entry(r.to_lowercase())
                .or_default()
                .push((f, r));
        }
        // Sorted once, here, rather than per lookup. The old code sorted
        // each candidate list by function name after filtering; function
        // names are unique per language in the map, so sorting each bucket
        // once gives every lookup the same order it used to compute.
        for v in by_lower_ret.values_mut().flat_map(|m| m.values_mut()) {
            v.sort_unstable_by(|a, b| a.0.cmp(b.0));
        }
        Self {
            empty: return_types.is_empty(),
            by_lower_ret,
            by_name: return_types.clone(),
        }
    }

    /// The return type every definition of `name` in `lang` declares.
    fn get(&self, lang: &str, name: &str) -> Option<&'a str> {
        self.by_name.get(&(lang, name)).copied()
    }

    fn is_empty(&self) -> bool {
        self.empty
    }

    /// Candidates for `rh_lower`, in the exact order the old inline
    /// filter-then-sort produced: exact matches first (sorted by
    /// function name), then plural matches (likewise).
    ///
    /// The old comparator was `exact(a).cmp(&exact(b)).then(a.0.cmp(b.0))`
    /// where `exact` is false for an exact match, so exact sorted before
    /// plural and ties broke on the function name. Two pre-sorted buckets
    /// concatenated in that order reproduce it exactly, and a candidate
    /// cannot land in both: that would need `rl == rh_lower` and
    /// `rl + "s" == rh_lower` simultaneously.
    fn candidates(
        &self,
        lang: &str,
        rh_lower: &str,
    ) -> impl Iterator<Item = (&'a str, &'a str)> {
        let by_ret = self.by_lower_ret.get(lang);
        let exact = by_ret.and_then(|m| m.get(rh_lower));
        let plural = rh_lower
            .strip_suffix('s')
            .and_then(|stem| by_ret.and_then(|m| m.get(stem)));
        exact
            .into_iter()
            .flatten()
            .chain(plural.into_iter().flatten())
            .copied()
    }
}

/// Rewrite receiver hints in-place using inferred type information.
pub fn propagate_types(facts: &mut FileFacts) {
    propagate_types_with_returns(facts, &ReturnTypeIndex::default());
}

/// `(owner, field) -> nominal type`, first declaration wins.
///
/// Built once per run. Field declarations live in headers and the calls
/// live in `.cpp` files, so a per-file map cannot see them.
pub fn field_index(all_facts: &[FileFacts]) -> HashMap<String, HashMap<String, String>> {
    let mut out: HashMap<String, HashMap<String, String>> = HashMap::new();
    for facts in all_facts {
        for ft in &facts.field_types {
            out.entry(ft.owner.clone())
                .or_default()
                .entry(ft.field.clone())
                .or_insert_with(|| ft.type_name.clone());
        }
    }
    out
}

/// C++ type alias → the class it names, run-wide, first definition wins.
///
/// `using Rect = TRect<float>;` maps `Rect` to `TRect`: template arguments,
/// cv-qualifiers, `typename`, and namespace qualification are stripped to
/// the bare class name the owner index is keyed on. Chains
/// (`using A = B; using B = C<int>;`) resolve up to eight levels; a cycle
/// or a builtin target (`using Index = int;`) yields no entry.
pub fn type_alias_index(all_facts: &[FileFacts]) -> HashMap<String, String> {
    let mut raw: HashMap<String, String> = HashMap::new();
    for facts in all_facts {
        if facts.language != "cpp" {
            continue;
        }
        for a in &facts.type_aliases {
            if let Some(stem) = alias_stem(&a.replacement) {
                raw.entry(a.name.clone()).or_insert(stem);
            }
        }
    }
    let mut out = HashMap::new();
    for name in raw.keys() {
        let mut cur = &raw[name];
        let mut depth = 0;
        while let Some(next) = raw.get(cur.as_str()) {
            if depth >= 8 || next == name {
                break;
            }
            cur = next;
            depth += 1;
        }
        if cur != name {
            out.insert(name.clone(), cur.clone());
        }
    }
    out
}

fn alias_stem(target: &str) -> Option<String> {
    let t = target.split('<').next().unwrap_or(target);
    let t = t
        .split_whitespace()
        .filter(|w| {
            !matches!(
                *w,
                "const" | "volatile" | "typename" | "struct" | "class" | "enum"
            )
        })
        .next_back()?;
    let t = t.trim_end_matches(['*', '&']).rsplit("::").next()?.trim();
    let first = t.chars().next()?;
    if !(first.is_alphabetic() || first == '_')
        || t.chars().any(|c| !(c.is_alphanumeric() || c == '_'))
    {
        return None;
    }
    if is_primitive(t) || t.starts_with(char::is_lowercase) {
        return None;
    }
    Some(t.to_string())
}

/// Object-like macro name → replacement text, first definition wins.
pub fn macro_index(all_facts: &[FileFacts]) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for facts in all_facts {
        for m in &facts.macro_aliases {
            out.entry(m.name.clone())
                .or_insert_with(|| m.replacement.clone());
        }
    }
    out
}

/// Resolve each object-like macro to a nominal type via its replacement
/// and the class-field index. `#define THEKERNEL Kernel::instance` with
/// `static Kernel* instance` becomes `Kernel`; `#define THEROBOT
/// THEKERNEL->robot` walks that type through the `robot` field.
pub fn resolve_macro_types(
    macros: &HashMap<String, String>,
    fields: &HashMap<String, HashMap<String, String>>,
) -> HashMap<String, String> {
    let mut out: HashMap<String, String> = HashMap::new();
    for name in macros.keys() {
        if let Some(ty) = type_macro_expr(name, macros, fields, 0) {
            out.insert(name.clone(), ty);
        }
    }
    out
}

const MACRO_EXPAND_DEPTH: u32 = 8;

/// Type of a macro replacement or receiver fragment: another macro, a
/// `Owner::field` static, or a `base->field` / `base.field` chain.
fn type_macro_expr(
    expr: &str,
    macros: &HashMap<String, String>,
    fields: &HashMap<String, HashMap<String, String>>,
    depth: u32,
) -> Option<String> {
    if depth >= MACRO_EXPAND_DEPTH {
        return None;
    }
    let expr = expr.trim();
    if expr.is_empty() {
        return None;
    }
    if let Some(repl) = macros.get(expr) {
        return type_macro_expr(repl, macros, fields, depth + 1);
    }
    if expr.contains("->") {
        let mut parts = expr.split("->");
        let mut ty = type_macro_expr(parts.next()?.trim(), macros, fields, depth + 1)?;
        for field in parts {
            let field = field.trim();
            if !is_plain_ident(field) {
                return None;
            }
            ty = fields.get(&ty)?.get(field)?.clone();
        }
        return capitalize_type(ty);
    }
    if expr.contains('.') && !expr.contains("::") {
        let mut parts = expr.split('.');
        let mut ty = type_macro_expr(parts.next()?.trim(), macros, fields, depth + 1)?;
        for field in parts {
            let field = field.trim();
            if !is_plain_ident(field) {
                return None;
            }
            ty = fields.get(&ty)?.get(field)?.clone();
        }
        return capitalize_type(ty);
    }
    if let Some((owner, field)) = split_owner_field(expr) {
        let ty = fields.get(owner)?.get(field)?.clone();
        return capitalize_type(ty);
    }
    None
}

fn split_owner_field(expr: &str) -> Option<(&str, &str)> {
    let (owner, field) = expr.rsplit_once("::")?;
    let owner = owner.rsplit("::").next()?.trim();
    let field = field.trim();
    if !is_plain_ident(owner) || !is_plain_ident(field) {
        return None;
    }
    Some((owner, field))
}

fn capitalize_type(ty: String) -> Option<String> {
    if ty.starts_with(char::is_uppercase) {
        Some(ty)
    } else {
        None
    }
}

/// The named type a definition's signature declares it returns, if any
/// (a primitive or unit is none).
fn declared_return<'a>(lang: &str, def: &'a DefRecord) -> Option<&'a str> {
    extract_return_type(lang, &def.signature_hint)
        .filter(|ret| ret.starts_with(char::is_uppercase) && !is_primitive(ret))
}

/// Folds one more definition of a name into what its definitions agree
/// on: `Some(ty)` while each declared `ty`, `None` once one declares
/// another type or none at all.
fn agree<'a>(slot: &mut Option<Option<&'a str>>, ret: Option<&'a str>) {
    *slot = match *slot {
        None => Some(ret),
        Some(prev) if prev == ret => Some(prev),
        Some(_) => Some(None),
    };
}

/// Build a map of `(language, function simple name)` -> return type from
/// all definitions across all files. Parses return types from
/// signature_hint.
///
/// A name is typed only when every definition of it in the language
/// declares the same return type (WS19). The map used to be keyed by the
/// bare name, first definition wins: a Swift `make() -> Service` typed a
/// Python `service = make()` whose own `make` returns something else, and
/// of two Python `make`s the one that happened to be read first decided
/// for both.
pub fn build_return_type_map<'a>(
    all_facts: &'a [FileFacts],
) -> HashMap<(&'a str, &'a str), &'a str> {
    let mut seen: HashMap<(&'a str, &'a str), Option<Option<&'a str>>> = HashMap::new();
    for facts in all_facts {
        for def in &facts.definitions {
            let ret = declared_return(&facts.language, def);
            agree(
                seen.entry((facts.language.as_str(), def.simple_name.as_str()))
                    .or_default(),
                ret,
            );
        }
    }
    seen.into_iter()
        .filter_map(|(k, v)| v.flatten().map(|ret| (k, ret)))
        .collect()
}

/// Per-language tally names for the type propagator.
///
/// `profile::count` takes a `&'static str`, so attributing a count to a
/// language needs a fixed table rather than a formatted name — the
/// alternative is leaking a string per language, which `docs-check.py`
/// check 9 exists to prevent.
///
/// The list is the languages that actually appear in the corpus repos
/// under investigation plus the common ones; anything else shares
/// `other::`. A catch-all is fine for "is this one language's problem"
/// right up until the catch-all *is* the answer, which is what happened
/// on the first pass here: `erlang-otp` put 1.86 billion inner-scan
/// steps in `types::`, and only naming C and C++ showed they were the
/// ones spending it, not Erlang.
struct LangTallies {
    refs: &'static str,
    defs: &'static str,
    enclosing_scans: &'static str,
    s4_entered: &'static str,
    s4_inner_scan: &'static str,
}

macro_rules! lang_tallies_table {
    ($($lang:literal),* $(,)?) => {
        fn lang_tallies(lang: &str) -> LangTallies {
            match lang {
                $(
                    $lang => LangTallies {
                        refs: concat!($lang, "::refs"),
                        defs: concat!($lang, "::defs"),
                        enclosing_scans: concat!($lang, "::enclosing-def-steps"),
                        s4_entered: concat!($lang, "::s4-entered"),
                        s4_inner_scan: concat!($lang, "::s4-inner-scan-steps"),
                    },
                )*
                _ => LangTallies {
                    refs: "other::refs",
                    defs: "other::defs",
                    enclosing_scans: "other::enclosing-def-steps",
                    s4_entered: "other::s4-entered",
                    s4_inner_scan: "other::s4-inner-scan-steps",
                },
            }
        }
    };
}

lang_tallies_table!(
    "erlang",
    "c",
    "cpp",
    "rust",
    "python",
    "java",
    "javascript",
    "typescript",
    "go",
    "perl",
    "elixir",
    "bash",
    "ruby",
    "php",
    "csharp",
    "kotlin",
    "swift",
);

/// Rewrite receiver hints using both local type info and a global
/// return-type map built from all files' definitions.
pub fn propagate_types_with_returns(
    facts: &mut FileFacts,
    return_types: &ReturnTypeIndex<'_>,
) {
    let empty_fields = HashMap::new();
    let empty_macros = HashMap::new();
    propagate_types_with_fields(
        facts,
        return_types,
        &empty_fields,
        &empty_macros,
        &empty_macros,
    );
}

/// [`propagate_types_with_returns`] plus class fields, so
/// `kernel->streams->printf()` rewrites to receiver `StreamOutput`.
///
/// `macros` maps object-like macro names already resolved to a nominal
/// type (`THEKERNEL` → `Kernel`), so `THEKERNEL->streams` and a bare
/// `THEROBOT` rewrite the same way a typed local would.
pub fn propagate_types_with_fields(
    facts: &mut FileFacts,
    return_types: &ReturnTypeIndex<'_>,
    fields: &HashMap<String, HashMap<String, String>>,
    macros: &HashMap<String, String>,
    aliases: &HashMap<String, String>,
) {
    // An Erlang receiver is the module atom of `m:f(…)` — never a variable,
    // so never a value to type. Renaming `file` to a `File` some other
    // language's function returns would lose the module.
    if facts.language == "erlang" {
        return;
    }
    if facts.language == "rust" {
        settle_unwrapped_bindings(facts);
    }
    // Pass 1: Extract type hints from definition signatures.
    //
    // The names and types are slices of `def.signature_hint`, and `facts`
    // is mutably borrowed at the end of this function, so the map cannot
    // borrow them directly. They go into an owned store that this function
    // drops instead.
    //
    // That store is why there is no `leak_str` here any more. It used to
    // `Box::leak` a copy of every parameter name and type, justified in a
    // comment by "we're in a short-lived analysis pass" — true while the
    // pipeline was a private function in a binary that analyzed once and
    // exited, and false from 0.6.0, when `cgg::analyze` became callable in
    // a loop by a library, a Python module and a C ABI. Measured at ~161
    // bytes per call on cgg's own tree, and it accumulated forever in a
    // long-lived host process. The allocation *count* is unchanged — the
    // leaked version called `to_string()` too — so this costs nothing; the
    // strings are simply freed now.
    let cpp = facts.language == "cpp";
    let rust = facts.language == "rust";
    // Go records a value made through a package at the end of its `:=`
    // statement, so Rust's scoped lookup applies to it (Strategy 3).
    let go = facts.language == "go";
    // Zig records its locals' types at the end of their declaration
    // (WS14b), so they are looked up where they are in scope, as Rust's.
    let zig = facts.language == "zig";
    let kotlin = facts.language == "kotlin";
    let js = matches!(facts.language.as_str(), "javascript" | "typescript");
    let param_store: Vec<(u32, String, String)> = facts
        .definitions
        .iter()
        .flat_map(|d| {
            // Go is name-first; it has its own reader.
            let mut v = if go {
                go_param_types(d)
            } else {
                collect_param_types(d, &facts.language)
            };
            if zig {
                for (_, _, ty) in &mut v {
                    *ty = zig_peel_type(ty).to_string();
                }
            }
            v
        })
        .collect();
    // Key: (enclosing_start_byte, variable_name) -> type_name. Last write
    // wins, exactly as the repeated `map.insert` did.
    let type_map: HashMap<(u32, &str), &str> = param_store
        .iter()
        .map(|(byte, name, ty)| ((*byte, name.as_str()), ty.as_str()))
        .collect();

    // Pass 2: Scan references for constructor patterns that reveal types.
    // We look for assignment-like patterns in the source by examining
    // refs that look like constructors (name matches a type pattern).
    let constructor_types = find_constructor_assignments(facts);

    // Pass 2b: self-field LocalTypes, keyed by the method's body
    // start_byte so we don't bleed Type A's `self.store` into Type B's
    // methods within the same file. Ordinary locals are looked up per
    // call site (`lookup_local_type`) so two functions sharing a
    // variable name do not last-write-win.
    let mut self_field_map: HashMap<(u32, &str), &str> = HashMap::new();
    // File-wide, last-write-wins map of ordinary locals: the lookup every
    // plugin except C++ was written against (see Strategy 3 below).
    let mut local_type_map: HashMap<&str, &str> = HashMap::new();
    for lt in &facts.local_types {
        // A bare call's result (`make()`) is read where it is in scope,
        // by `lookup_call_local`.
        if call_result_callee(&facts.language, &lt.type_name).is_some()
            && CALL_RESULT_LANGS.contains(&facts.language.as_str())
            || js
        {
            continue;
        }
        if lt.var_name.starts_with("self.") {
            self_field_map
                .insert((lt.scope_byte, lt.var_name.as_str()), lt.type_name.as_str());
        } else if go
            && (lt.type_name.is_empty()
                || lt.type_name.starts_with('<')
                || go_package_made(&lt.type_name))
        {
            // A Go value made through a package (`r := jobs.NewRunner()`,
            // `w := &jobs.Runner{}`) is typed only where its `:=` is in
            // scope. File-wide it would retype every `r` in the file —
            // the common short names repeat across functions.
        } else {
            local_type_map.insert(lt.var_name.as_str(), lt.type_name.as_str());
        }
    }

    let rust_bindings = rust.then(|| RustBindings::build(facts));
    let rust_lets = if rust || go || zig {
        RustLets::build(facts)
    } else {
        RustLets::default()
    };

    // Pass 2c: "is `fn_name` called bare, before this site?" — answered
    // by scanning at first, by a map once scanning stops being cheaper.
    //
    // The original asked this with `facts.references.iter().any(...)`
    // per candidate, so the cost was O(receivers x candidates x
    // references) — **1,864,725,063 steps** on `erlang-otp`, 93% of them
    // from its 171 vendored C++ files.
    //
    // A map answers it in O(1), but building one is O(references) *with
    // hashing*, where a scan is O(references) with an integer compare
    // and a string compare that usually fails on the first byte. So the
    // map only wins once there are enough lookups to amortise it, and
    // choosing either strategy up front loses somewhere:
    //
    //   * always scan   -> the erlang-otp blowup above
    //   * always build  -> every file in every language pays for a map
    //                      most of them never use. Measured: 15 repos
    //                      >5% slower, `asyncapi-spec` +82.6%.
    //   * build lazily  -> better, but a file making one or two lookups
    //                      still pays more than the scans it replaced.
    //                      Measured: 4 repos slower in both runs,
    //                      `gcp-functions-samples` +83.4%.
    //
    // So: scan for the first `SCAN_BUDGET` lookups, then build. A file
    // that asks once pays one scan; a file that asks a million times
    // pays eight scans and a map. Both answer the identical predicate,
    // so the graph is unchanged either way.
    const SCAN_BUDGET: u32 = 8;
    let lookups = std::cell::Cell::new(0u32);
    let called_map: std::cell::OnceCell<HashMap<&str, u32>> = std::cell::OnceCell::new();
    let called_before = |fn_name: &str, before: u32| -> bool {
        let n = lookups.get();
        lookups.set(n.saturating_add(1));
        if n < SCAN_BUDGET && called_map.get().is_none() {
            return facts.references.iter().any(|r| {
                r.name == fn_name
                    && r.receiver_hint.is_empty()
                    && !r.value_ref
                    && r.site_byte < before
            });
        }
        called_map
            .get_or_init(|| {
                let mut m: HashMap<&str, u32> = HashMap::new();
                // A value reference names the function without calling
                // it: nothing it returns is in scope.
                for r in &facts.references {
                    if r.receiver_hint.is_empty() && !r.value_ref {
                        m.entry(r.name.as_str())
                            .and_modify(|b| *b = (*b).min(r.site_byte))
                            .or_insert(r.site_byte);
                    }
                }
                m
            })
            .get(fn_name)
            .is_some_and(|&b| b < before)
    };

    // Closure parameters and typed patterns, by name (see
    // `FileFacts::scoped_types`).
    let mut scoped: HashMap<&str, Vec<(u32, u32, &str)>> = HashMap::new();
    for st in &facts.scoped_types {
        scoped.entry(st.name.as_str()).or_default().push((
            st.start_byte,
            st.end_byte,
            st.type_name.as_str(),
        ));
    }

    // Pass 3: Rewrite receiver_hints.
    let t = lang_tallies(&facts.language);
    cgg_core::profile::count(t.refs, facts.references.len() as u64);
    cgg_core::profile::count(t.defs, facts.definitions.len() as u64);
    let _s3 = cgg_core::profile::span("types::pass3");
    let mut rewrites: Vec<(usize, String)> = Vec::new();
    // The rewrites above that are guesses rather than a declaration in
    // scope (see `RefRecord::weak_receiver_type`): indices into `rewrites`'
    // reference positions.
    let mut weak: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // JS/TS and Go rewrites (WS23) whose typed lookup may fall back to the
    // receiver as written when it finds nothing (`untyped_receivers`): a
    // JS local's construction, a Go declared parameter or receiver.
    let mut fallback: std::collections::HashSet<usize> = std::collections::HashSet::new();
    // The calling file's own functions' return types, built on first use
    // by Strategy 4 (see there).
    type OwnReturns<'f> = (HashMap<&'f str, Option<Option<&'f str>>>, bool);
    let own_returns: std::cell::OnceCell<OwnReturns<'_>> = std::cell::OnceCell::new();
    // Kotlin: each property a class declares (`private lateinit var vm:
    // Vm`, `val x: T = …`, a constructor's `val`/`var` parameter), with
    // the span of the class, or the file's own top-level ones. A Kotlin
    // class is recorded as a constructor spanning its body, so
    // `lookup_local_type` read such a declaration as some function's local
    // and the file-wide reading as a guess. It is a declaration, scoped to
    // its class (WS19): see [`kotlin_property_at`].
    let kotlin_fields_cell: std::cell::OnceCell<Vec<KotlinProperty>> =
        std::cell::OnceCell::new();
    let own_returns_of = || {
        own_returns.get_or_init(|| {
            let mut m: HashMap<&str, Option<Option<&str>>> = HashMap::new();
            for d in &facts.definitions {
                agree(
                    m.entry(d.simple_name.as_str()).or_default(),
                    declared_return(&facts.language, d),
                );
            }
            let any = m.values().any(|v| v.flatten().is_some());
            (m, any)
        })
    };
    // What a bare call `name(…)` returns (WS23): the return type the
    // calling file's own definitions of `name` agree on, or, when the file
    // defines none and imports no such name, the one every definition of
    // it in the language declares. A declaration, agreed on, so not a
    // guess.
    // A name the file imports (a Java `import static a.B.status;`, a C#
    // `using static`) is that import's, which may be outside the tree.
    let imported_name = |name: &str| {
        facts.imports.iter().any(|imp| {
            imp.alias == name || imp.path.rsplit(['.', ':', '/']).next() == Some(name)
        })
    };
    let call_return = |name: &str| -> Option<&str> {
        match own_returns_of().0.get(name) {
            Some(v) => v.flatten(),
            None if imported_name(name) => None,
            None => return_types.get(&facts.language, name),
        }
    };
    let call_results = CALL_RESULT_LANGS.contains(&facts.language.as_str());
    // Names worth a scoped lookup: a local recorded as a call's result, a
    // JS/TS local recorded with a type.
    let call_local_names: std::collections::HashSet<&str> = if call_results {
        facts
            .local_types
            .iter()
            .filter(|lt| call_result_callee(&facts.language, &lt.type_name).is_some())
            .map(|lt| lt.var_name.as_str())
            .collect()
    } else {
        Default::default()
    };
    let js_local_names: std::collections::HashSet<&str> = if js {
        facts
            .local_types
            .iter()
            .map(|lt| lt.var_name.as_str())
            .collect()
    } else {
        Default::default()
    };
    let local_index: std::cell::OnceCell<LocalIndex<'_>> = std::cell::OnceCell::new();
    let local_index_of = || local_index.get_or_init(|| LocalIndex::build(facts));
    for (i, rref) in facts.references.iter().enumerate() {
        let rh = rref.receiver_hint.as_str();
        // A cast or construction receiver is already its type.
        if rref.typed_receiver {
            continue;
        }
        if rh.is_empty()
            || rh == "self"
            || rh == "Self"
            || rh == "cls"
            || rh == "this"
            || rh == cgg_core::VALUE_REF_HINT
            // A C++ member initializer (`: Base(1)`, receiver `super`)
            // names the caller's bases, never a variable; a `super::f()`
            // is a path, which a `using super = Base;` may type.
            || (facts.language == "cpp" && rh == "super" && !rref.path_receiver)
        {
            continue;
        }

        // `make().work()`, `[F make].work`: the receiver is a bare call's
        // result, typed by what that call is declared to return. A call
        // whose return type is unknown is left as written.
        if call_results && let Some(callee) = call_result_callee(&facts.language, rh) {
            if let Some(ty) = call_return(callee) {
                rewrites.push((i, ty.to_string()));
            }
            continue;
        }
        // `var t = make(); t.work()`: a local the plugin recorded as a bare
        // call's result (`make()`) has that call's declared return type.
        // Groovy's `def` local may be reassigned a value of another type,
        // so there the reading is a guess.
        if call_results
            && call_local_names.contains(rh)
            && let Some(callee) =
                lookup_call_local(facts, local_index_of(), rref.site_byte, rh)
            && let Some(ty) = call_return(callee)
        {
            if facts.language == "groovy" {
                weak.insert(i);
            }
            rewrites.push((i, ty.to_string()));
            continue;
        }

        // `kernel->streams` / `this->streams` / `obj.streams`. The base
        // is a local, a parameter, `this`, or an object-like macro; each
        // following identifier is a field. A chain that does not fully
        // resolve is left as written — a partial type would name the
        // wrong receiver.
        if facts.language == "cpp" && (rh.contains("->") || rh.contains('.')) {
            if let Some(ty) = member_chain_type(
                rh,
                rref.arrow,
                facts,
                rref.site_byte,
                &type_map,
                fields,
                macros,
            ) {
                rewrites.push((i, ty));
            }
            continue;
        }

        // A lock guard (`x.lock()`, `self.store.write().unwrap()`) derefs
        // to what the lock holds; the plugin and the parameter parser
        // record that type under `<base>#guard` wherever a declared type
        // went through `Mutex`/`RwLock`/`RefCell`.
        if rust && let Some(base) = rust_guard_base(rh) {
            let key = format!("{base}{GUARD_SUFFIX}");
            let enc = enclosing_def(facts, rref.site_byte);
            let ty = if base.starts_with("self.") {
                enc.and_then(|e| {
                    self_field_map.get(&(e.start_byte, key.as_str())).copied()
                })
            } else {
                enc.and_then(|e| type_map.get(&(e.start_byte, key.as_str())).copied())
                    .or_else(|| {
                        rust_lets.visible(
                            &key,
                            rref.site_byte,
                            enc.map(|d| (d.start_byte, d.end_byte)),
                        )
                    })
            };
            if let Some(ty) = ty {
                rewrites.push((i, ty.to_string()));
            }
            continue;
        }

        // Special-case `self.<field>` BEFORE the dot/colon filter
        // below — the field's type comes from the per-method scoped
        // self_field_map populated by the Rust and Python extractors.
        if rh.starts_with("self.") {
            if let Some(enc) = enclosing_def(facts, rref.site_byte)
                && let Some(&ty) = self_field_map.get(&(enc.start_byte, rh))
            {
                rewrites.push((i, ty.to_string()));
                continue;
            }
            // No match — leave as-is so the resolver can still try a
            // direct lookup downstream.
            continue;
        }

        // `#define THEKERNEL Kernel::instance` — the name is uppercase,
        // so it would otherwise be skipped as a type-looking path.
        // Both tables come from C/C++ sources; a Python or Java receiver
        // that happens to share the name is not that macro or alias.
        if matches!(facts.language.as_str(), "c" | "cpp") {
            if let Some(ty) = macros.get(rh) {
                rewrites.push((i, ty.clone()));
                continue;
            }
            if let Some(ty) = aliases.get(rh) {
                rewrites.push((i, ty.clone()));
                continue;
            }
        }

        // A path receiver (`config::load()`) names a module or crate,
        // never a variable, even when a local of the same name exists.
        if rh.starts_with(char::is_uppercase)
            || rh.contains("::")
            || rh.contains('.')
            || rref.path_receiver
        {
            continue;
        }

        // A closure's typed parameter (or a typed pattern) is in scope
        // only over its own span, and shadows every outer binding of the
        // name there.
        if let Some(ty) = scoped.get(rh).and_then(|spans| {
            spans
                .iter()
                .filter(|(s, e, _)| *s <= rref.site_byte && rref.site_byte < *e)
                .min_by_key(|(s, e, _)| (e - s, std::cmp::Reverse(*s)))
                .map(|(_, _, ty)| *ty)
        }) {
            rewrites.push((i, ty.to_string()));
            continue;
        }

        cgg_core::profile::count(t.enclosing_scans, facts.definitions.len() as u64);
        let enclosing = {
            let _s = cgg_core::profile::span("types::enclosing-def");
            enclosing_def(facts, rref.site_byte)
        };

        // `p->f()` on a C++ variable declared as a class template reaches
        // what its `operator->` returns, recorded under `p#deref`; `p.f()`
        // and a raw pointer's `p->f()` reach the declared type itself.
        let deref_key = (cpp && rref.arrow).then(|| format!("{rh}{DEREF_SUFFIX}"));

        // Strategy 1: parameter type annotations. A Rust parameter a
        // later `let` re-binds no longer has its declared type there.
        if let Some(enc) = enclosing
            && let Some(&ty) = deref_key
                .as_deref()
                .and_then(|k| type_map.get(&(enc.start_byte, k)))
                .or_else(|| type_map.get(&(enc.start_byte, rh)))
            && !rust_bindings
                .as_ref()
                .is_some_and(|b| b.shadowed_since(rh, enc.start_byte, rref.site_byte))
            // Go (WS23): a declaration in an inner block, or a function
            // literal's own parameter, hides the parameter there; the
            // plugin records it as a local, ended where its block ends.
            && !(go
                && rust_lets
                    .visible_at(
                        rh,
                        rref.site_byte,
                        Some((enc.start_byte, enc.end_byte)),
                    )
                    .is_some_and(|(at, ty)| at > enc.start_byte && !ty.is_empty()))
        {
            // A Go type that declares no such method may promote it from an
            // embedded type of another package, which the base chain
            // cannot always follow: the receiver is then read as written.
            if go {
                fallback.insert(i);
            }
            rewrites.push((i, ty.to_string()));
            continue;
        }
        // Kotlin (WS14b): a lambda or an object expression's method sees
        // the parameters of the functions around it.
        if kotlin
            && let Some(ty) = facts
                .definitions
                .iter()
                .filter(|d| d.start_byte <= rref.site_byte && rref.site_byte < d.end_byte)
                .filter(|d| enclosing.is_none_or(|e| d.start_byte != e.start_byte))
                // A function, not a class: a primary constructor's
                // parameters are not its properties' declared types.
                .filter(|d| d.variant != cgg_core::DefVariant::Constructor)
                .max_by_key(|d| (d.start_byte, std::cmp::Reverse(d.end_byte)))
                .and_then(|d| type_map.get(&(d.start_byte, rh)))
        {
            // `CacheRequest?` is a `CacheRequest` once the call is made.
            rewrites.push((i, ty.trim_end_matches('?').to_string()));
            continue;
        }

        // Strategy 3: explicit local variable type declarations. C++ looks
        // them up per call site, scoped to the enclosing callable, so two
        // `stream` locals in one file do not last-write-win. Every other
        // plugin keeps the file-wide map: their `scope_byte` conventions
        // differ (PHP records none), and scoping them changed resolution
        // in every language on the corpus, losing 1,698 high-confidence
        // edges outside C/C++.
        //
        // Rust's `scope_byte` is the `let` statement itself, so it can be
        // scoped too: the latest `let` before the site whose enclosing
        // callable also encloses the site. File-wide last-write-wins
        // retyped every `map` in a file as the external `http::HeaderMap`
        // once one test wrote `let map = http::HeaderMap::new()`, and the
        // local `HeaderMap`'s own calls stopped resolving. A site with no
        // visible `let` keeps the file-wide answer it always had.
        // Whether the reading below is a declaration in scope at the site:
        // C++, Zig and a Go package-made value are looked up where they
        // are; a Rust `let` visible at the site, or a C#/Kotlin one in the
        // calling function, likewise. The file-wide map is a declaration
        // somewhere in the file — scoped only when the calling function
        // (or the file's own top level: a field) declares the name with
        // that type too.
        let mut scoped_reading = cpp || zig;
        let local_ty = if cpp {
            deref_key
                .as_deref()
                .and_then(|k| lookup_local_type(facts, rref.site_byte, k))
                .or_else(|| lookup_local_type(facts, rref.site_byte, rh))
        } else if rust {
            // The `let` in scope, unless a later untyped binding hides it.
            // A site with no visible `let` keeps the file-wide answer it
            // always had: refusing it whenever some untyped binding of the
            // name is in scope was measured (WS13) to drop 161 high edges,
            // about 70% of them right, to fix two dead-code findings.
            let bindings = rust_bindings.as_ref();
            match rust_lets.visible_at(
                rh,
                rref.site_byte,
                enclosing.map(|d| (d.start_byte, d.end_byte)),
            ) {
                Some((at, ty)) => {
                    scoped_reading = true;
                    (!bindings.is_some_and(|b| b.shadowed_since(rh, at, rref.site_byte)))
                        .then_some(ty)
                }
                None => local_type_map.get(rh).copied(),
            }
        } else if zig {
            rust_lets.visible(
                rh,
                rref.site_byte,
                enclosing.map(|d| (d.start_byte, d.end_byte)),
            )
        } else if go {
            // Go's own reading stays file-wide, as before; only a value
            // made through a package is looked up where it is in scope.
            local_type_map.get(rh).copied().or_else(|| {
                rust_lets
                    .visible(
                        rh,
                        rref.site_byte,
                        enclosing.map(|d| (d.start_byte, d.end_byte)),
                    )
                    .filter(|ty| go_package_made(ty))
                    .inspect(|_| scoped_reading = true)
            })
        } else if matches!(facts.language.as_str(), "csharp" | "kotlin") {
            // A declaration in the calling function, before the site, is
            // the one in scope there; only without one does the file-wide
            // reading apply. File-wide last-write-wins retyped `JArray a`
            // as the `IEnumerable? a` another method declares further down.
            lookup_local_type(facts, rref.site_byte, rh)
                .filter(|_| {
                    facts.local_types.iter().any(|lt| {
                        lt.var_name == rh
                            && lt.scope_byte <= rref.site_byte
                            && enclosing.is_some_and(|d| {
                                d.start_byte <= lt.scope_byte
                                    && lt.scope_byte < d.end_byte
                            })
                    })
                })
                .or_else(|| {
                    // A Kotlin property of the class around the site (or
                    // the file's own) is declared, not guessed.
                    kotlin
                        .then(|| {
                            kotlin_property_at(
                                kotlin_fields_cell
                                    .get_or_init(|| kotlin_properties(facts)),
                                rh,
                                rref.site_byte,
                            )
                        })
                        .flatten()
                        .inspect(|_| scoped_reading = true)
                })
                .or_else(|| local_type_map.get(rh).copied())
        } else if js {
            // JS/TS (WS23): the declaration in scope at the site, by the
            // plugin's scope-spanned bindings — never another function's
            // local of the name.
            js_local_names
                .contains(rh)
                .then(|| js_local_type(facts, local_index_of(), rref.site_byte, rh))
                .flatten()
                .inspect(|_| {
                    scoped_reading = true;
                    fallback.insert(i);
                })
        } else {
            local_type_map.get(rh).copied()
        };
        if let Some(ty) = local_ty {
            if !scoped_reading
                && (rust || lookup_local_type(facts, rref.site_byte, rh) != Some(ty))
                || facts.module_typed_locals.iter().any(|v| v == rh)
            {
                weak.insert(i);
            }
            rewrites.push((i, ty.to_string()));
            continue;
        }
        // Zig locals are typed from what is written (WS14b): the name
        // heuristics below would type a `page` as some `Page` a function
        // returns, and the container lookup would take that guess as
        // the receiver's type.
        if zig {
            continue;
        }

        // Strategy 4: return-type inference. If the receiver variable
        // was assigned from a function call whose return type we know,
        // use that. We check if any ref in this file is a bare call
        // to a function with a known return type, appearing before
        // this ref, and the ref's name matches our receiver.
        // Simplified: just check if receiver_hint matches a known
        // function name's return type (covers `let x = getService(); x.run()`)
        // A name the calling file itself defines is that definition (its
        // return type, or untyped when it declares none or its
        // definitions disagree); only a name it does not define is read
        // from the language's run-wide map.
        let (own, own_has_returns) = own_returns_of();
        // Not for the type-first languages (WS23): their call results are
        // typed from the call itself above, and a receiver merely named
        // like a type some called function returns (`fileSystem` beside a
        // `getFileSystem()` returning the JDK's `FileSystem`) was measured
        // to mistype fields and parameters.
        if (!return_types.is_empty() || *own_has_returns) && !call_results {
            let _s4 = cgg_core::profile::span("types::strategy4");
            cgg_core::profile::count(t.s4_entered, 1);
            // Check if there's a ref earlier in this file that calls
            // a function whose return type matches. We use a heuristic:
            // if the variable name is a common derivative of the return
            // type (e.g., "service" from "Service", "config" from "Config")
            // OR if we find a bare call to a function returning that type.
            let rh_lower = rh.to_lowercase();
            // The order here is load-bearing, not tidiness. The match is
            // deliberately loose — it accepts an exact name match OR a
            // plural, so `Config` and `Configs` BOTH claim a receiver
            // called `configs`. Taking whichever came first out of hash
            // order meant a 20-line file produced two different graphs
            // across 25 identical single-threaded runs with default
            // flags. `ReturnTypeIndex::candidates` yields exact matches
            // before plural ones, each bucket sorted by function name,
            // which is the order the old filter-then-sort computed.
            let plural_of = |ret: &str| {
                let l = ret.to_lowercase();
                if l == rh_lower {
                    Some(false)
                } else if rh_lower.strip_suffix('s') == Some(l.as_str()) {
                    Some(true)
                } else {
                    None
                }
            };
            let mut cands: Vec<(bool, &str, &str)> = return_types
                .candidates(&facts.language, &rh_lower)
                .filter(|(f, _)| !own.contains_key(f))
                .filter_map(|(f, r)| plural_of(r).map(|p| (p, f, r)))
                .collect();
            if *own_has_returns {
                cands.extend(own.iter().filter_map(|(f, r)| {
                    let r = (*r).flatten()?;
                    plural_of(r).map(|p| (p, *f, r))
                }));
                cands.sort_unstable();
            }
            for (_, fn_name, ret_type) in cands {
                cgg_core::profile::count(t.s4_inner_scan, 1);
                // Verify this function is actually called in this scope.
                let called = called_before(fn_name, rref.site_byte);
                if called {
                    weak.insert(i);
                    rewrites.push((i, ret_type.to_string()));
                    break;
                }
            }
            if rewrites.last().map(|(idx, _)| *idx) == Some(i) {
                continue;
            }
        }

        // Strategy 2: constructor/lowercase heuristic
        if let Some(ty) = constructor_types.get(rh) {
            weak.insert(i);
            rewrites.push((i, ty.clone()));
        }
    }
    // Rust: a call's argument that is a variable (`From::from(id)`,
    // `T::from(x.clone())`) has the type that variable has where the call
    // is: a parameter's declared type, the visible `let`, a closure's
    // typed parameter, a `self` field. Read with the same tables the
    // receivers above are; a variable none of them types leaves the
    // argument's type unknown.
    let mut arg_rewrites: Vec<(usize, Option<String>)> = Vec::new();
    if rust {
        for (i, rref) in facts.references.iter().enumerate() {
            let Some(var) = rref
                .arg_type
                .as_deref()
                .and_then(|a| a.strip_prefix(cgg_core::ARG_VAR_PREFIX))
            else {
                continue;
            };
            let enclosing = enclosing_def(facts, rref.site_byte);
            let ty: Option<&str> = if var.starts_with("self.") {
                enclosing.and_then(|e| self_field_map.get(&(e.start_byte, var)).copied())
            } else if let Some(ty) = scoped.get(var).and_then(|spans| {
                spans
                    .iter()
                    .filter(|(s, e, _)| *s <= rref.site_byte && rref.site_byte < *e)
                    .min_by_key(|(s, e, _)| (e - s, std::cmp::Reverse(*s)))
                    .map(|(_, _, ty)| *ty)
            }) {
                Some(ty)
            } else {
                let bindings = rust_bindings.as_ref();
                let param = enclosing.and_then(|enc| {
                    rust_param_decl_type(&enc.signature_hint, var).filter(|_| {
                        !bindings.is_some_and(|b| {
                            b.shadowed_since(var, enc.start_byte, rref.site_byte)
                        })
                    })
                });
                param.or_else(|| {
                    rust_lets
                        .visible_at(
                            var,
                            rref.site_byte,
                            enclosing.map(|d| (d.start_byte, d.end_byte)),
                        )
                        .and_then(|(at, ty)| {
                            (!bindings.is_some_and(|b| {
                                b.shadowed_since(var, at, rref.site_byte)
                            }))
                            .then_some(ty)
                        })
                })
            };
            arg_rewrites.push((i, ty.map(compact_rust_type)));
        }
    }
    for (i, ty) in arg_rewrites {
        facts.references[i].arg_type = ty;
    }
    for (i, ty) in rewrites {
        // A local typed `Rect` is a `TRect` when `Rect` is an alias.
        let ty = if facts.language == "cpp" {
            aliases.get(&ty).cloned().unwrap_or(ty)
        } else {
            ty
        };
        if (matches!(facts.language.as_str(), "cpp" | "zig") || fallback.contains(&i))
            && facts.references[i].receiver_hint != ty
        {
            let r = &facts.references[i];
            facts.untyped_receivers.push((
                r.site_byte,
                r.name.clone(),
                r.receiver_hint.clone(),
            ));
        }
        // A Go receiver typed from a declaration (a parameter, the method's
        // receiver) as an unexported type (`*store`) names that type: the
        // resolver reads a lowercase receiver as a variable otherwise.
        if go
            && !weak.contains(&i)
            && ty.starts_with(char::is_lowercase)
            && !ty.contains('.')
        {
            facts.references[i].typed_receiver = true;
        }
        facts.references[i].weak_receiver_type = weak.contains(&i);
        facts.references[i].receiver_hint = ty;
    }
}

/// A Kotlin property: name, declared type, and the span of the class
/// declaring it (`None` for a top-level one).
type KotlinProperty = (String, String, Option<(u32, u32)>);

/// The properties a Kotlin file declares: class-body and top-level
/// declarations (`local_types` outside every function — a class is
/// recorded as a constructor spanning its body), and a primary
/// constructor's `val`/`var` parameters.
fn kotlin_properties(facts: &FileFacts) -> Vec<KotlinProperty> {
    let is_class = |d: &&DefRecord| d.variant == cgg_core::DefVariant::Constructor;
    let class_at = |byte: u32| {
        facts
            .definitions
            .iter()
            .filter(is_class)
            .filter(|d| d.start_byte <= byte && byte < d.end_byte)
            .min_by_key(|d| d.end_byte - d.start_byte)
            .map(|d| (d.start_byte, d.end_byte))
    };
    let mut out: Vec<KotlinProperty> = facts
        .local_types
        .iter()
        .filter(|lt| !lt.type_name.is_empty())
        .filter(|lt| {
            !facts.definitions.iter().any(|d| {
                !is_class(&d)
                    && d.start_byte <= lt.scope_byte
                    && lt.scope_byte < d.end_byte
            })
        })
        .map(|lt| {
            (
                lt.var_name.clone(),
                lt.type_name.clone(),
                class_at(lt.scope_byte),
            )
        })
        .collect();
    for d in facts.definitions.iter().filter(is_class) {
        let Some(params) = param_list(&d.signature_hint) else {
            continue;
        };
        for piece in param_pieces(params, false) {
            let piece = piece.trim();
            let Some((name, ty)) = parse_colon_param("kotlin", piece) else {
                continue;
            };
            // Only a `val`/`var` parameter is a property; a plain one is
            // the constructor's alone.
            let property = piece
                .split_whitespace()
                .take_while(|t| !t.starts_with(name))
                .any(|t| t == "val" || t == "var");
            if property {
                out.push((
                    name.to_string(),
                    ty.trim_end_matches('?').to_string(),
                    Some((d.start_byte, d.end_byte)),
                ));
            }
        }
    }
    out
}

/// The declared type of the Kotlin property `name` visible at `site`: the
/// innermost class around the site that declares one, else the file's
/// top-level one — only when every declaration there agrees.
fn kotlin_property_at<'f>(
    props: &'f [KotlinProperty],
    name: &str,
    site: u32,
) -> Option<&'f str> {
    let width = |o: &Option<(u32, u32)>| o.map_or(u32::MAX, |(a, b)| b - a);
    let visible: Vec<&KotlinProperty> = props
        .iter()
        .filter(|(n, _, o)| n == name && o.is_none_or(|(a, b)| a <= site && site < b))
        .collect();
    let innermost = visible.iter().map(|(_, _, o)| width(o)).min()?;
    let mut at = visible.iter().filter(|(_, _, o)| width(o) == innermost);
    let first = at.next()?;
    at.all(|(_, t, _)| *t == first.1)
        .then_some(first.1.as_str())
}

/// What an associated function's declared return type says about the
/// value a `Type::f(..)` call produces.
#[derive(Clone, Debug, PartialEq, Eq)]
enum AssocReturn {
    /// `Self`, or the owner type itself, possibly behind `Result`,
    /// `Option`, a reference or a smart pointer.
    OwnType,
    /// Another named type, from its last path segment on (`Data<Ctx>`).
    Other(String),
    /// Nothing nameable: `()`, a primitive, `impl Trait`, a tuple.
    Unknown,
}

/// Reads a Rust signature's return type for [`AssocReturn`], with
/// `owner` the bare name of the type the function is declared on.
fn rust_assoc_return(sig: &str, owner: &str) -> AssocReturn {
    let Some(params) = param_list(sig) else {
        return AssocReturn::Unknown;
    };
    // The offset just past the parameter list's closing `)`.
    let after = params.as_ptr() as usize - sig.as_ptr() as usize + params.len() + 1;
    let Some(rest) = sig.get(after..) else {
        return AssocReturn::Unknown;
    };
    let Some(arrow) = rest.find("->") else {
        return AssocReturn::Unknown;
    };
    let mut t = return_type_text(&rest[arrow + 2..]);
    for _ in 0..8 {
        t = t.trim();
        // `&'a mut T`, `&T`
        if let Some(r) = t.strip_prefix('&') {
            t = r.trim_start();
            if t.starts_with('\'') {
                t = t.split_once(' ').map_or("", |(_, r)| r);
            }
            t = t.trim_start().strip_prefix("mut ").unwrap_or(t);
            continue;
        }
        let head = t.split('<').next().unwrap_or(t).trim();
        let last = head.rsplit("::").next().unwrap_or(head);
        if cgg_core::rust_types::is_return_wrapper(last)
            && let Some(open) = t.find('<')
            && let Some(close) = t.rfind('>')
            && close > open
        {
            let args = &t[open + 1..close];
            t = split_top_level(args, |c| c == ',', true)
                .first()
                .copied()
                .unwrap_or("");
            continue;
        }
        break;
    }
    let head = t.split(['<', ' ', '(', '[']).next().unwrap_or(t).trim();
    let last = head.rsplit("::").next().unwrap_or(head);
    if last == "Self" || last == owner {
        AssocReturn::OwnType
    } else if last.starts_with(char::is_uppercase) && !is_primitive(last) {
        // The type from its last path segment on, type arguments kept:
        // `-> web::Data<Ctx>` is `Data<Ctx>`, which still reaches `Ctx`.
        let t = t.trim();
        let from = head.len() - last.len();
        AssocReturn::Other(t.get(from..).unwrap_or(last).trim().to_string())
    } else {
        AssocReturn::Unknown
    }
}

/// Settle the Rust locals typed from a `Type::f(..)` path call
/// ([`cgg_core::AssocCallLocal`]) against `f`'s declared return type.
///
/// The extractor types `let x = Type::f(..)` as `Type`, which is right
/// for a constructor (`new`, `default`, `from`, `open`) and wrong for an
/// associated function returning something else (`Type::load(..) ->
/// Config`): every method call on `x` then looked for `Type`'s methods.
/// Here, with every file's definitions known, the declarations of `f` on
/// a type of that name decide:
///
/// * all return `Self` / the type (possibly as `Result`/`Option`/a
///   pointer) — the reading stands;
/// * all return one other named type — the local is retyped to it;
/// * anything else (a primitive, `()`, `impl Trait`, declarations that
///   disagree) — the local is left untyped, as any local cgg cannot type.
///
/// No declaration in the tree (an external type's `File::open`) leaves
/// the constructor reading unchanged. Order-independent: the verdicts
/// are compared as a set.
pub fn settle_assoc_call_locals(all_facts: &mut [FileFacts]) {
    if !all_facts.iter().any(|f| !f.assoc_call_locals.is_empty()) {
        return;
    }
    let wanted: std::collections::HashSet<(String, String)> = all_facts
        .iter()
        .flat_map(|f| {
            f.assoc_call_locals.iter().filter_map(move |a| {
                let lt = f.local_types.iter().rev().find(|lt| {
                    lt.var_name == a.var_name && lt.scope_byte == a.scope_byte
                })?;
                let ty = crate::names::normalize_owner(
                    lt.type_name.rsplit("::").next().unwrap_or(&lt.type_name),
                );
                Some((ty.to_string(), a.assoc_fn.clone()))
            })
        })
        .collect();
    let mut verdicts: HashMap<(String, String), Vec<AssocReturn>> = HashMap::new();
    for f in all_facts.iter().filter(|f| f.language == "rust") {
        for d in &f.definitions {
            let Some(owner) = crate::names::owner_from_qn(&d.qualified_name) else {
                continue;
            };
            let key = (owner.to_string(), d.simple_name.clone());
            if !wanted.contains(&key) {
                continue;
            }
            let v = rust_assoc_return(&d.signature_hint, owner);
            let slot = verdicts.entry(key).or_default();
            if !slot.contains(&v) {
                slot.push(v);
            }
        }
    }
    for f in all_facts.iter_mut() {
        if f.assoc_call_locals.is_empty() {
            continue;
        }
        let pending = std::mem::take(&mut f.assoc_call_locals);
        for a in pending {
            let Some(idx) = f.local_types.iter().rposition(|lt| {
                lt.var_name == a.var_name && lt.scope_byte == a.scope_byte
            }) else {
                continue;
            };
            let ty = {
                let t = &f.local_types[idx].type_name;
                crate::names::normalize_owner(t.rsplit("::").next().unwrap_or(t))
                    .to_string()
            };
            let Some(vs) = verdicts.get(&(ty, a.assoc_fn.clone())) else {
                continue;
            };
            match vs.as_slice() {
                [AssocReturn::OwnType] => {}
                [AssocReturn::Other(other)] => {
                    f.local_types[idx].type_name = other.clone();
                }
                _ => {
                    f.local_types.remove(idx);
                }
            }
        }
    }
}

/// Type the parameters of untyped closures passed to a bare call
/// ([`FileFacts::closure_args`]) from the callee's declared callback type:
/// `each((x) => x.run())` with `void each(void Function(Foo) cb)` types `x`
/// as `Foo` inside the closure, as `(Foo x) => …` would have.
///
/// Conservative by construction: the callee is matched by name among the
/// same language's definitions, and every definition of that name must
/// declare the same concrete callback parameter types at that position
/// (an overload that does not, a type parameter such as `T`, or an arity
/// that does not match the closure leaves the closure untyped, with the
/// reading it had).
pub fn settle_closure_args(all_facts: &mut [FileFacts]) {
    if !all_facts.iter().any(|f| !f.closure_args.is_empty()) {
        return;
    }
    let wanted: std::collections::HashSet<(&str, &str)> = all_facts
        .iter()
        .flat_map(|f| {
            f.closure_args
                .iter()
                .map(|c| (f.language.as_str(), c.callee.as_str()))
        })
        .collect();
    // (language, name, index) -> the callback's parameter types, or `None`
    // once two definitions disagree. `spoke` counts the definitions that
    // have a parameter at that position; one that has none disagrees with
    // one that does, so a key is used only when every definition of the
    // name (`seen`) spoke for it.
    type Key = (String, String, u32);
    let mut index: HashMap<Key, Option<Vec<String>>> = HashMap::new();
    let mut spoke: HashMap<Key, u32> = HashMap::new();
    let mut seen: HashMap<(String, String), u32> = HashMap::new();
    for f in all_facts.iter() {
        for d in &f.definitions {
            if !wanted.contains(&(f.language.as_str(), d.simple_name.as_str())) {
                continue;
            }
            *seen
                .entry((f.language.clone(), d.simple_name.clone()))
                .or_default() += 1;
            let types = callback_params_of(&f.language, &d.signature_hint);
            for (i, t) in types.into_iter().enumerate() {
                let key = (f.language.clone(), d.simple_name.clone(), i as u32);
                *spoke.entry(key.clone()).or_default() += 1;
                match index.get_mut(&key) {
                    None => {
                        index.insert(key, t);
                    }
                    Some(prev) if *prev != t => *prev = None,
                    Some(_) => {}
                }
            }
        }
    }
    for f in all_facts.iter_mut() {
        if f.closure_args.is_empty() {
            continue;
        }
        let lang = f.language.clone();
        let mut add = Vec::new();
        for c in &f.closure_args {
            let key = (lang.clone(), c.callee.clone(), c.arg_index);
            let all = seen
                .get(&(lang.clone(), c.callee.clone()))
                .copied()
                .unwrap_or(0);
            if all == 0 || spoke.get(&key).copied().unwrap_or(0) != all {
                continue;
            }
            let Some(Some(types)) = index.get(&key) else {
                continue;
            };
            if types.len() != c.params.len() {
                continue;
            }
            for (name, ty) in c.params.iter().zip(types) {
                if ty.is_empty() || name == "_" {
                    continue;
                }
                add.push(cgg_core::ScopedType {
                    name: name.clone(),
                    type_name: ty.clone(),
                    start_byte: c.start_byte,
                    end_byte: c.end_byte,
                });
            }
        }
        f.scoped_types.extend(add);
    }
}

/// Per positional parameter of `sig`: `Some(types)` when it is declared
/// as a callback type, the nominal types of the callback's parameters
/// (an empty string for one that is not a concrete class); `None` for a
/// parameter of any other type.
fn callback_params_of(lang: &str, sig: &str) -> Vec<Option<Vec<String>>> {
    let Some(params) = param_list(sig) else {
        return Vec::new();
    };
    let generics = def_type_params(sig);
    let mut out = Vec::new();
    for piece in split_top_level(params, |c| c == ',', false) {
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        // Dart named parameters are not positional.
        if lang == "dart" && piece.starts_with('{') {
            break;
        }
        let piece = piece.trim_start_matches('[').trim_end_matches(']');
        let piece = strip_default_value(piece).trim();
        let ty = if lang == "scala" {
            match piece.split_once(':') {
                Some((_, t)) => t.trim(),
                None => {
                    out.push(None);
                    continue;
                }
            }
        } else {
            // Type-first: everything before the name.
            match piece.rfind(|c: char| c.is_whitespace()) {
                Some(at) => piece[..at].trim(),
                None => {
                    out.push(None);
                    continue;
                }
            }
        };
        out.push(callback_param_types(lang, ty).map(|ts| {
            ts.into_iter()
                .map(|t| concrete_class(lang, t, &generics).unwrap_or_default())
                .collect()
        }));
    }
    out
}

/// The parameter types of the callback type `ty`, as written: Scala
/// `Foo => R` / `(Foo, Bar) => R`, and the named constructors of
/// [`cgg_core::stdlib::CALLBACK_TYPE_CONSTRUCTORS`].
fn callback_param_types<'a>(lang: &str, ty: &'a str) -> Option<Vec<&'a str>> {
    let ty = cgg_core::declared_type(lang, ty)?.trim();
    if lang == "scala" {
        let arrow = ty.find("=>")?;
        let lhs = ty[..arrow].trim();
        if lhs.is_empty() {
            return None; // by-name `=> R`
        }
        let lhs = lhs
            .strip_prefix('(')
            .and_then(|l| l.strip_suffix(')'))
            .unwrap_or(lhs);
        return Some(
            split_top_level(lhs, |c| c == ',', false)
                .into_iter()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect(),
        );
    }
    for &(l, head, last_is_result) in cgg_core::stdlib::CALLBACK_TYPE_CONSTRUCTORS {
        if l != lang {
            continue;
        }
        let Some(at) = find_word(ty, head) else {
            continue;
        };
        let open = at + head.len();
        let bytes = ty.as_bytes();
        let close = match bytes.get(open) {
            Some(b'(') => matching_paren(bytes, open)?,
            Some(b'<') => {
                let mut depth = 0i32;
                let mut end = None;
                for (i, c) in ty[open..].char_indices() {
                    match c {
                        '<' => depth += 1,
                        '>' => {
                            depth -= 1;
                            if depth == 0 {
                                end = Some(open + i);
                                break;
                            }
                        }
                        _ => {}
                    }
                }
                end?
            }
            _ => continue,
        };
        let mut args: Vec<&str> =
            split_top_level(&ty[open + 1..close], |c| c == ',', false)
                .into_iter()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .collect();
        if last_is_result {
            args.pop();
        }
        // Dart names its callback parameters: `Function(Foo item)`.
        if lang == "dart" {
            for a in &mut args {
                if let Some(w) = param_words(a).first() {
                    *a = w;
                }
            }
        }
        return Some(args);
    }
    None
}

/// Index of `word` in `s` standing alone (not part of a longer name).
fn find_word(s: &str, word: &str) -> Option<usize> {
    let is_id = |c: char| c.is_alphanumeric() || c == '_';
    s.match_indices(word).map(|(i, _)| i).find(|&i| {
        !s[..i].chars().next_back().is_some_and(is_id)
            && !s[i + word.len()..].chars().next().is_some_and(is_id)
    })
}

/// The declared type parameters of a definition (`def f[T, U](…)`,
/// `void F<T>(…)`, `R f<R>(…)`): names that are never a concrete class.
fn def_type_params(sig: &str) -> Vec<&str> {
    let Some(params) = param_list(sig) else {
        return Vec::new();
    };
    let head = &sig[..params.as_ptr() as usize - sig.as_ptr() as usize];
    let Some(open) = head.rfind(['[', '<']) else {
        return Vec::new();
    };
    head[open + 1..]
        .trim_end_matches(['(', ' '])
        .trim_end_matches([']', '>'])
        .split(',')
        .filter_map(|t| {
            t.trim()
                .trim_start_matches(['+', '-'])
                .split([' ', ':', '<'])
                .next()
                .map(str::trim)
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// `ty` as a concrete class name: nullability peeled, type arguments cut,
/// capitalised, not one of the definition's type parameters, not a
/// one-letter name.
fn concrete_class(lang: &str, ty: &str, generics: &[&str]) -> Option<String> {
    let ty = cgg_core::declared_type(lang, ty)?;
    let ty = ty.split(['<', '[', '(', ' ']).next()?.trim();
    let last = ty.rsplit('.').next().unwrap_or(ty);
    if last.chars().count() < 2
        || !last.starts_with(char::is_uppercase)
        || generics.contains(&ty)
        || cgg_core::stdlib::is_top_type(lang, last)
        || !ty
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        return None;
    }
    Some(ty.to_string())
}

/// Settle each Kotlin bare call made inside a `x.run { … }` /
/// `x.apply { … }` / `with(x) { … }` lambda (`RefRecord::implicit_receiver`,
/// receiver already typed): when the type `x` was typed as declares a
/// member of that name, the call is that member's, through the receiver;
/// otherwise it is a call to an enclosing scope, and goes back to being
/// the bare call it was written as (WS14b).
pub fn settle_implicit_receivers(all_facts: &mut [FileFacts]) {
    if !all_facts
        .iter()
        .any(|f| f.references.iter().any(|r| r.implicit_receiver))
    {
        return;
    }
    let mut members: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for f in all_facts.iter().filter(|f| f.language == "kotlin") {
        for d in &f.definitions {
            if let Some(owner) = crate::names::owner_from_qn(&d.qualified_name) {
                members.insert((owner.to_string(), d.simple_name.clone()));
            }
        }
    }
    for f in all_facts.iter_mut() {
        for r in f.references.iter_mut().filter(|r| r.implicit_receiver) {
            let ty = r.receiver_hint.rsplit('.').next().unwrap_or("");
            let ty = ty.split('<').next().unwrap_or(ty).trim_end_matches('?');
            if !members.contains(&(ty.to_string(), r.name.clone())) {
                r.receiver_hint.clear();
                r.typed_receiver = false;
                r.implicit_receiver = false;
            }
        }
    }
}

/// Parameter `(enclosing_start_byte, name, type)` triples from one
/// definition's `signature_hint`.
///
/// Returns owned strings rather than writing borrowed ones into a map: the
/// caller needs them to outlive its borrow of `facts`, and owning them
/// there is what lets the caller free them. See the call site.
fn collect_param_types(def: &DefRecord, lang: &str) -> Vec<(u32, String, String)> {
    let cpp = lang == "cpp";
    let rust = lang == "rust";
    let mut out = Vec::new();
    // Parse parameter types from signature_hint.
    // Patterns we recognize:
    //   Rust:   `fn foo(x: Service, y: &Helper)`
    //   Python: `def foo(self, x: Service, y: Helper):`
    //   Java:   `public void foo(Service x, Helper y)`
    //   TS:     `foo(x: Service, y: Helper)`
    //   Go:     `func foo(x Service, y *Helper)`
    //   Kotlin: `fun foo(x: Service, y: Helper)`
    //   C#:     `void Foo(Service x, Helper y)`
    let sig = &def.signature_hint;
    if sig.is_empty() {
        return out;
    }

    let Some(params_str) = param_list(sig) else {
        return out;
    };

    // Only a comma outside every bracket separates two parameters. A
    // split on every comma read `Map<String, Handler> h` as a parameter
    // `Map<String` and a parameter `Handler> h`, typing `h` as one of its
    // own type arguments (FINDINGS A2).
    for param in param_pieces(params_str, rust) {
        let param = param.trim();
        if param.is_empty() {
            continue;
        }

        // Try "name: Type" pattern (Rust, Python, TS, Kotlin)
        let colon = if rust {
            parse_colon_param_rust(param)
        } else {
            parse_colon_param(lang, param).map(|(n, t)| (n, t, false))
        };
        if let Some((name, ty, locked)) = colon {
            if locked {
                out.push((
                    def.start_byte,
                    format!("{name}{GUARD_SUFFIX}"),
                    ty.to_string(),
                ));
            }
            out.push((def.start_byte, name.to_string(), ty.to_string()));
            continue;
        }

        // Try "Type name" pattern (Java, C#, C++, Go). C++ gets the parser
        // that understands `const StreamOutput *stream`, modifiers and
        // defaults; every other plugin keeps the parser it was written
        // against, because the C++ one rejects `ContainerBuilder $container`
        // and cost PHP its typed receivers corpus-wide.
        if cpp {
            if let Some((name, ty, pointee)) = parse_type_first_param_cpp(param) {
                if let Some(p) = pointee {
                    out.push((
                        def.start_byte,
                        format!("{name}{DEREF_SUFFIX}"),
                        p.to_string(),
                    ));
                }
                if let Some(ty) = ty {
                    out.push((def.start_byte, name.to_string(), ty.to_string()));
                }
            }
        } else if let Some((name, ty)) = parse_type_first_param(lang, param) {
            out.push((def.start_byte, name.to_string(), ty.to_string()));
        }
    }
    out
}

/// Go's receiver and parameters with a named type, as `(start, name,
/// type)`: the receiver of `func (s *Server) Handle(…)` (`s` → `Server`),
/// a same-package type written bare (`q *Server`, `r client`), and a
/// package-qualified one (`t *util.T`, `a, b jobs.Runner`). A qualifier
/// is kept: the resolver follows it through the file's imports to the one
/// package it names (`modules`), where the bare `T` would match every
/// type of that name in the tree. Go writes the name first, which the
/// generic type-first parser reads back to front (`q *Server` as a type
/// `q`), so this is Go's only parameter reader. A predeclared type
/// (`int`, `error`, [`cgg_core::stdlib::GO_PREDECLARED_TYPES`]), one of
/// the function's own type parameters, and any composite type (slice,
/// map, channel, function, interface literal) type nothing.
fn go_param_types(def: &DefRecord) -> Vec<(u32, String, String)> {
    let mut out = Vec::new();
    let sig = def.signature_hint.trim();
    let sig = sig.strip_prefix("func").map_or(sig, str::trim_start);
    // A method's receiver list (`func (s *Server) Handle(…)`) comes
    // first; the parameters are the list after the name.
    // `func Map[T any, U Shape](…)` and `func (b *Box[T]) Get(…)`: `T`
    // and `U` are type parameters.
    let bracket_words = |s: &str| -> Vec<String> {
        match (s.find('['), s.find('(')) {
            (Some(open), paren) if paren.is_none_or(|p| open < p) => {
                let end = paren.unwrap_or(s.len());
                let close = s[open..].find(']').map_or(end, |c| open + c);
                param_pieces(&s[open + 1..close], false)
                    .into_iter()
                    .filter_map(|p| p.split_whitespace().next())
                    .map(str::to_string)
                    .collect()
            }
            _ => Vec::new(),
        }
    };
    let mut type_params: Vec<String> = Vec::new();
    let sig = match sig.starts_with('(') {
        true => match matching_paren(sig.as_bytes(), 0) {
            Some(close) => {
                type_params.extend(bracket_words(&sig[1..close]));
                go_typed_names(&sig[1..close], &[], def.start_byte, &mut out);
                &sig[close + 1..]
            }
            None => return out,
        },
        false => sig,
    };
    type_params.extend(bracket_words(sig));
    let type_params: Vec<&str> = type_params.iter().map(String::as_str).collect();
    let Some(params) = param_list(sig) else {
        return out;
    };
    go_typed_names(params, &type_params, def.start_byte, &mut out);
    out
}

/// The `(start, name, type)` of each parameter in one Go parameter list
/// (see [`go_param_types`]). Names declared without a type take the next
/// one's (`a, b T`).
fn go_typed_names(
    params: &str,
    type_params: &[&str],
    start: u32,
    out: &mut Vec<(u32, String, String)>,
) {
    let mut pending: Vec<&str> = Vec::new();
    for piece in param_pieces(params, false) {
        let words = param_words(piece.trim());
        match words.as_slice() {
            [name] if is_plain_ident(name) => pending.push(name),
            [name, ty] if is_plain_ident(name) => {
                let ty = ty.trim_start_matches('*');
                // A generic instantiation (`Box[T]`) is its type.
                let ty = ty.split('[').next().unwrap_or(ty);
                let named = match ty.split_once('.') {
                    Some((pkg, t)) => {
                        is_plain_ident(pkg)
                            && pkg.starts_with(|c: char| c.is_lowercase())
                            && is_plain_ident(t)
                            && t.starts_with(char::is_uppercase)
                    }
                    None => {
                        is_plain_ident(ty)
                            // `map[K]V`, `func(…)`, `chan T`, `struct{…}`.
                            && !matches!(ty, "map" | "func" | "chan" | "struct" | "interface")
                            && !cgg_core::stdlib::GO_PREDECLARED_TYPES.contains(&ty)
                            && !type_params.contains(&ty)
                    }
                };
                if named {
                    for n in pending.drain(..).chain(std::iter::once(*name)) {
                        // `_` names no variable.
                        if n != "_" {
                            out.push((start, n.to_string(), ty.to_string()));
                        }
                    }
                }
                pending.clear();
            }
            _ => pending.clear(),
        }
    }
}

/// The text inside a signature's parameter list: the first parenthesised
/// group at the top level that is not an annotation's argument list
/// (`@SuppressWarnings("x")`, `@get:JvmName("y")`; a C# `[Attr(…)]` or PHP
/// `#[…]` is skipped as a bracketed group) nor a keyword's (Rust
/// `pub(crate)`, C/C++ `__attribute__((…))`, `decltype(…)`, see
/// [`is_modifier_args`]). It used to be everything from
/// the first `(` to the last `)`, which started inside a leading
/// annotation; only splitting on every comma, which this replaces, ever
/// recovered the parameters behind it. A list that never closes (a
/// truncated signature) runs to the last `)`, as it did.
fn param_list(sig: &str) -> Option<&str> {
    param_list_span(sig).map(|(open, end)| &sig[open + 1..end])
}

/// The byte positions of [`param_list`]'s opening parenthesis and of its
/// end (the closing parenthesis).
fn param_list_span(sig: &str) -> Option<(usize, usize)> {
    let bytes = sig.as_bytes();
    let mut depth = 0u32; // `[` / `{` nesting
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            // Only `"`: a `'` is a Rust lifetime as often as a quote.
            b'"' => {
                i = skip_quoted(bytes, i);
                continue;
            }
            b'[' | b'{' => depth += 1,
            b']' | b'}' => depth = depth.saturating_sub(1),
            b'(' if depth == 0 => {
                let close = matching_paren(bytes, i);
                if !is_modifier_args(&sig[..i]) {
                    let end = close.or_else(|| sig.rfind(')').filter(|&c| c > i))?;
                    return Some((i, end));
                }
                match close {
                    Some(c) => i = c,
                    None => return None,
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Languages that overload by parameter count and whose plugins record a
/// call's argument count ([`cgg_core::RefRecord::arity`]).
/// C# records the count for constructions and constructor delegation
/// only; Groovy (WS23) for method calls too, except one passing named
/// arguments or a spread.
pub(crate) const OVERLOAD_ARITY_LANGS: &[&str] =
    &["java", "scala", "cpp", "csharp", "groovy", "swift"];

/// Whether a callable declared with signature `sig` can take `arity`
/// positional arguments: `Some(false)` only when its parameter list is
/// read whole and provably cannot (too few or too many, after defaults,
/// varargs — `T...`, Scala `T*`, a C++ pack — and implicit lists).
/// `None` when the list cannot be read.
pub(crate) fn accepts_arity(sig: &str, arity: u32) -> Option<bool> {
    let list = param_list(sig)?;
    let trimmed = list.trim();
    // An implicit / `using` list is supplied by the compiler.
    if trimmed.starts_with("implicit ") || trimmed.starts_with("using ") {
        return Some(arity == 0);
    }
    let params: Vec<&str> = split_top_level(list, |c| c == ',', false)
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if params.len() == 1 && params[0] == "void" {
        return Some(arity == 0);
    }
    let mut required = 0u32;
    let mut variadic = false;
    for p in &params {
        // `T...`, Scala `x: T*`, C# `params T[] x`.
        if p.contains("...")
            || (p.contains(':') && p.trim_end().ends_with('*'))
            || p.starts_with("params ")
        {
            variadic = true;
            continue;
        }
        if !has_top_level_eq(p) {
            required += 1;
        }
    }
    let max = params.len() as u32;
    Some(arity >= required && (variadic || arity <= max))
}

/// A `=` at bracket depth 0 that is not part of `==`, `<=`, `>=`, `!=`,
/// `=>`: a parameter's default value.
fn has_top_level_eq(p: &str) -> bool {
    let b = p.as_bytes();
    let mut depth = 0i32;
    for i in 0..b.len() {
        match b[i] {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' => depth -= 1,
            b'=' if depth <= 0 => {
                let prev = if i > 0 { b[i - 1] } else { b' ' };
                let next = b.get(i + 1).copied().unwrap_or(b' ');
                if !matches!(prev, b'=' | b'<' | b'>' | b'!')
                    && !matches!(next, b'=' | b'>')
                {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Keywords whose parenthesised argument can stand in front of a
/// parameter list: Rust's visibility restriction, C/C++ attributes and
/// type operators.
const MODIFIER_KEYWORDS: &[&str] = &[
    "pub",
    "__attribute__",
    "__declspec",
    "alignas",
    "_Alignas",
    "decltype",
    "__typeof__",
];

/// Whether the text in front of a `(` makes the group a modifier's
/// arguments rather than a parameter list: an annotation name (`@Name`,
/// `@a.b.Name`, `@get:Name`) or one of [`MODIFIER_KEYWORDS`].
fn is_modifier_args(before: &str) -> bool {
    let head = before.trim_end();
    let name_start = head
        .char_indices()
        .rev()
        .find(|&(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '$')))
        .map_or(0, |(i, c)| i + c.len_utf8());
    if name_start < head.len() && MODIFIER_KEYWORDS.contains(&&head[name_start..]) {
        return true;
    }
    name_start > 0 && name_start < head.len() && head[..name_start].ends_with('@')
}

/// Index of the `)` closing the `(` at `open`, skipping string literals
/// and nested brackets of every kind but `<>` (an operator in a default
/// value as often as a bracket).
fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0u32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_quoted(bytes, i);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return (bytes[i] == b')').then_some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Index just past the `"` string literal starting at `at`, honouring
/// backslash escapes; the end of input if it never closes.
fn skip_quoted(bytes: &[u8], at: usize) -> usize {
    let quote = bytes[at];
    let mut i = at + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// `s` cut at every character `is_sep` accepts that is not nested inside
/// `<…>`, `(…)`, `[…]`, `{…}` or a string literal. Empty pieces are kept;
/// callers skip them.
///
/// A `>` that is the second half of an arrow (`->`, `=>`) closes nothing,
/// and `<<` / `<=` are operators (in a default value), not openers. `>>`
/// needs no special case: it closes two levels, as it does in
/// `Box<Map<K, V>>`. A closer with nothing open is ignored, so an
/// unbalanced default value (`x = a > b`) cannot drive the depth negative
/// and glue the following parameters together.
///
/// A `"` always opens a string. A `'` does only outside Rust, where it
/// marks a lifetime (`&'a T`), and only in value position (after `=`, an
/// opening bracket, a comma, a colon, or at the start): `x'` is a primed
/// identifier in Haskell-family languages, not a quote.
fn split_top_level(s: &str, is_sep: impl Fn(char) -> bool, rust: bool) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0u32;
    let mut start = 0usize;
    let mut prev: Option<char> = None;
    // Last character that was not whitespace, for the quote rule.
    let mut prev_solid: Option<char> = None;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => {
                skip_string(&mut chars, '"');
            }
            '\'' if !rust
                && prev_solid
                    .is_none_or(|p| matches!(p, '=' | '(' | '[' | '{' | ',' | ':')) =>
            {
                skip_string(&mut chars, '\'');
            }
            '<' if chars.peek().is_some_and(|&(_, n)| n == '<' || n == '=') => {
                chars.next();
            }
            '<' | '(' | '[' | '{' => depth += 1,
            '>' if matches!(prev, Some('-' | '=')) => {}
            '>' | ')' | ']' | '}' => depth = depth.saturating_sub(1),
            c if depth == 0 && is_sep(c) => {
                out.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
        prev = Some(c);
        if !c.is_whitespace() {
            prev_solid = Some(c);
        }
    }
    out.push(&s[start..]);
    out
}

/// Advance past a string literal whose opening `quote` was just read,
/// honouring backslash escapes. An unterminated literal runs to the end.
fn skip_string(chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>, quote: char) {
    while let Some((_, c)) = chars.next() {
        if c == '\\' {
            chars.next();
        } else if c == quote {
            return;
        }
    }
}

/// Mirrors the Rust plugin's guard marker: `x#guard` is the type `x`'s
/// lock guard dereferences to.
const GUARD_SUFFIX: &str = "#guard";

use cgg_core::rust_types::SMART_POINTERS_AND_LOCKS as RUST_WRAPPERS;

/// Rust `name: Type`, seeing through wrapper chains: `x: Arc<Mutex<dyn
/// Reporter>>` is a `Reporter`, not an `Arc`. The generic `parse_colon_param`
/// cuts at the first `<`, which typed every such parameter as the std
/// wrapper — a type with no methods of interest, so its call sites became
/// unresolvable. Paths are kept (`a::Store`): the resolver needs them to
/// tell a workspace type from an external one. Up to three wrappers deep.
fn parse_colon_param_rust(param: &str) -> Option<(&str, &str, bool)> {
    let (name, _) = parse_colon_param("rust", param)?;
    let (_, rest) = param.split_once(':')?;
    let mut ty = rest.trim();
    let mut locked = false;
    for _ in 0..=3 {
        ty = ty.trim_start_matches('&').trim_start();
        if ty.starts_with('\'') {
            ty = ty.split_once(' ').map_or(ty, |(_, t)| t).trim_start();
        }
        for kw in ["mut ", "dyn ", "impl "] {
            ty = ty.strip_prefix(kw).unwrap_or(ty).trim_start();
        }
        let Some(open) = ty.find('<') else { break };
        let head = &ty[..open];
        if !RUST_WRAPPERS.contains(&head.rsplit("::").next().unwrap_or(head)) {
            break;
        }
        let Some(close) = ty.rfind('>') else { break };
        if close <= open {
            break;
        }
        locked |= cgg_core::rust_types::LOCK_WRAPPERS
            .contains(&head.rsplit("::").next().unwrap_or(head));
        ty = &ty[open + 1..close];
    }
    // `State<AppState>`: kept whole, see `deref_wrapper_split`.
    if cgg_core::rust_types::deref_wrapper_split(ty.trim()).is_some() {
        return Some((name, ty.trim(), locked));
    }
    // `dyn Store + Send + 'static` -> `Store`; `Store<T>` -> `Store`.
    let ty = ty
        .split(['<', '[', ',', ')', '+'])
        .next()
        .unwrap_or(ty)
        .trim();
    if ty.is_empty()
        || is_primitive(ty)
        || !ty
            .rsplit("::")
            .next()
            .is_some_and(|l| l.starts_with(char::is_uppercase))
    {
        return None;
    }
    Some((name, ty, locked))
}

/// Rust: settle the pattern bindings the plugin recorded as
/// [`cgg_core::ARG_VAR_PREFIX`]`Some:x` / `Ok:x` (`if let Some(m) = x`):
/// `m` has the type `x`'s declared `Option<T>` / `Result<T, _>` holds —
/// `x` a parameter of the enclosing function, or a `let` before the
/// pattern in it. A binding whose source is not typed that way is dropped,
/// leaving `m` as untyped as it was.
fn settle_unwrapped_bindings(facts: &mut FileFacts) {
    if !facts
        .local_types
        .iter()
        .any(|lt| lt.type_name.starts_with(cgg_core::ARG_VAR_PREFIX))
    {
        return;
    }
    let mut settled: Vec<Option<String>> = Vec::with_capacity(facts.local_types.len());
    for (i, lt) in facts.local_types.iter().enumerate() {
        let Some((wrapper, src)) = lt
            .type_name
            .strip_prefix(cgg_core::ARG_VAR_PREFIX)
            .and_then(|r| r.split_once(':'))
        else {
            settled.push(None);
            continue;
        };
        let enclosing = enclosing_def(facts, lt.scope_byte);
        // The declared type of the source: a parameter, else the latest
        // plain `let` of it before the pattern in the same function.
        let declared: Option<String> = enclosing
            .and_then(|d| rust_param_decl_type(&d.signature_hint, src))
            .map(str::to_string)
            .or_else(|| {
                facts.local_types[..i]
                    .iter()
                    .rev()
                    .find(|o| {
                        o.var_name == src
                            && o.scope_byte < lt.scope_byte
                            && !o.type_name.starts_with(cgg_core::ARG_VAR_PREFIX)
                            && enclosing.is_some_and(|d| {
                                d.start_byte <= o.scope_byte && o.scope_byte < d.end_byte
                            })
                    })
                    .map(|o| format!("{wrapper}<{}>", o.type_name))
            });
        let inner = declared.and_then(|t| unwrap_option_like(&t, wrapper));
        let ty = inner.and_then(|t| {
            let param = format!("x: {t}");
            parse_colon_param_rust(&param).map(|(_, ty, _)| ty.to_string())
        });
        settled.push(Some(ty.unwrap_or_default()));
    }
    let mut keep = Vec::with_capacity(facts.local_types.len());
    for (mut lt, s) in std::mem::take(&mut facts.local_types)
        .into_iter()
        .zip(settled)
    {
        match s {
            None => keep.push(lt),
            Some(ty) if !ty.is_empty() => {
                lt.type_name = ty;
                keep.push(lt);
            }
            Some(_) => {}
        }
    }
    facts.local_types = keep;
}

/// `Option<&T>` (for `Some`) / `Result<T, E>` (for `Ok`), references
/// first stripped: the `T`. `None` for any other shape.
fn unwrap_option_like(ty: &str, wrapper: &str) -> Option<String> {
    let ty = ty.trim().trim_start_matches('&').trim_start();
    let ty = ty.strip_prefix("mut ").unwrap_or(ty).trim_start();
    let want = match wrapper {
        "Some" => "Option",
        "Ok" => "Result",
        _ => return None,
    };
    let open = ty.find('<')?;
    let head = ty[..open].trim();
    if head.rsplit("::").next() != Some(want) || !ty.ends_with('>') {
        return None;
    }
    let args = &ty[open + 1..ty.len() - 1];
    let first = split_top_level(args, |c| c == ',', true)
        .into_iter()
        .next()?
        .trim()
        .to_string();
    (!first.is_empty()).then_some(first)
}

/// A Rust parameter's type exactly as declared (`String`, `&str`,
/// `&'a [u8]`), unlike [`parse_colon_param_rust`], which keeps only types
/// worth a method lookup.
fn rust_param_decl_type<'a>(sig: &'a str, name: &str) -> Option<&'a str> {
    let list = param_list(sig)?;
    param_pieces(list, true).into_iter().find_map(|p| {
        let (n, ty) = p.split_once(':')?;
        let n = n.trim().trim_start_matches("mut ").trim();
        (n == name).then(|| ty.trim()).filter(|t| !t.is_empty())
    })
}

/// A Rust type compared as [`cgg_core::RefRecord::arg_type`]: no
/// whitespace, no lifetimes, `&mut` read as `&`.
fn compact_rust_type(ty: &str) -> String {
    let mut out = String::with_capacity(ty.len());
    let mut chars = ty.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            while chars
                .peek()
                .is_some_and(|n| n.is_alphanumeric() || *n == '_')
            {
                chars.next();
            }
            continue;
        }
        if !c.is_whitespace() {
            out.push(c);
        }
    }
    out.replace("&mut", "&")
}

/// `x.lock()`, `self.inner.write().unwrap()`, `cell.borrow_mut()` -> the
/// receiver the guard was taken from (`x`, `self.inner`, `cell`). `None`
/// for anything else. The plugin peels `.await`/`?` off a receiver before
/// it gets here.
fn rust_guard_base(rh: &str) -> Option<&str> {
    let mut rh = rh;
    if let Some(stripped) = rh.strip_suffix(".unwrap()") {
        rh = stripped;
    } else if let Some(idx) = rh.rfind(".expect(")
        && rh.ends_with(')')
    {
        rh = &rh[..idx];
    }
    const GUARDS: &[&str] = &[
        ".lock()",
        ".read()",
        ".write()",
        ".borrow()",
        ".borrow_mut()",
        ".try_lock()",
        ".try_read()",
        ".try_write()",
        ".blocking_lock()",
    ];
    let base = GUARDS.iter().find_map(|g| rh.strip_suffix(g))?;
    let plain = base
        .strip_prefix("self.")
        .unwrap_or(base)
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_');
    (plain && !base.is_empty()).then_some(base)
}

/// `name: Type` (Python, TypeScript, Kotlin, Swift, Scala, Rust …) as
/// `(name, type)`. The type is the one a method call on the parameter
/// reaches ([`cgg_core::declared_type`]): `x: Optional[Foo]`, `x: Foo |
/// None`, `x: "Foo"`, `x: Foo?` are all a `Foo`. A default value is not
/// part of the type (`x: Foo = None` used to type `x` as `Foo = None`), and
/// a TypeScript optional parameter's `?` is not part of its name. A union
/// of several real types (`x: Foo | Bar`) leaves the parameter untyped:
/// choosing one member would be a guess, and the reading it gets instead
/// is the one an unannotated parameter gets.
fn parse_colon_param<'a>(lang: &str, param: &'a str) -> Option<(&'a str, &'a str)> {
    // "x: Service" or "x: &Service" or "x: *Service". The separator is a
    // lone `:` — never half of a `::` path separator, which a C++ or C#
    // type-first parameter (`ns::Foo x`, `global::A.B y`) contains and
    // which made `ns` the parameter's name.
    let bytes = param.as_bytes();
    let at = (0..bytes.len()).find(|&i| {
        bytes[i] == b':'
            && (i == 0 || bytes[i - 1] != b':')
            && bytes.get(i + 1) != Some(&b':')
    })?;
    let (name, rest) = (&param[..at], &param[at + 1..]);
    // `x :: xs : List α` (a Lean cons pattern) binds no single name.
    if name.contains("::") {
        return None;
    }
    let name = name
        .trim()
        .trim_start_matches("mut ")
        .trim()
        .rsplit(' ')
        .next()
        .unwrap_or(name.trim())
        .trim_end_matches('?');
    let ty = cgg_core::declared_type(lang, strip_default_value(rest))?;
    let ty = ty
        .trim()
        .trim_start_matches('&')
        .trim_start_matches("mut ")
        .trim_start_matches('*')
        .trim();
    // Take just the type identifier (before any <, [, etc.)
    let ty = ty.split(['<', '[', ',', ')']).next().unwrap_or(ty).trim();
    if name.is_empty() || ty.is_empty() {
        return None;
    }
    // Skip primitive types
    if is_primitive(ty) {
        return None;
    }
    Some((name, ty))
}

/// `s` up to a top-level `=` that starts a default value — not one of
/// `=>`, `==`, `<=`, `>=`, `!=`, and never one inside brackets or a
/// string. Without a default, all of `s`.
fn strip_default_value(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                i = skip_quoted(bytes, i);
                continue;
            }
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b'>' if i > 0 && matches!(bytes[i - 1], b'-' | b'=') => {}
            b')' | b']' | b'}' | b'>' => depth = (depth - 1).max(0),
            b'=' if depth == 0
                && bytes.get(i + 1).is_none_or(|&n| n != b'>' && n != b'=')
                && (i == 0 || !matches!(bytes[i - 1], b'<' | b'>' | b'!' | b'=')) =>
            {
                return &s[..i];
            }
            _ => {}
        }
        i += 1;
    }
    s
}

/// The parameters of a parameter list: its top-level comma-separated
/// pieces, with a piece that is wholly one `{…}` or `[…]` group — Dart's
/// named (`{Type a, Type b}`) and optional positional (`[Type a]`)
/// parameters — opened up into the parameters it holds.
fn param_pieces(params: &str, rust: bool) -> Vec<&str> {
    let mut out = Vec::new();
    for piece in split_top_level(params, |c| c == ',', rust) {
        let t = piece.trim();
        let wrapped = matches!(
            (t.as_bytes().first(), t.as_bytes().last()),
            (Some(b'{'), Some(b'}')) | (Some(b'['), Some(b']'))
        ) && matching_group(t.as_bytes()) == Some(t.len() - 1);
        if wrapped {
            out.extend(split_top_level(&t[1..t.len() - 1], |c| c == ',', rust));
        } else {
            out.push(piece);
        }
    }
    out
}

/// Index of the bracket closing the `{`/`[` that opens `bytes`.
fn matching_group(bytes: &[u8]) -> Option<usize> {
    let mut depth = 0u32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_quoted(bytes, i);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Whitespace-separated words of a parameter, a bracketed type argument
/// list staying one word: `Map<String, Handler> h` is `Map<String,
/// Handler>` and `h`.
fn param_words(param: &str) -> Vec<&str> {
    split_top_level(param, char::is_whitespace, false)
        .into_iter()
        .filter(|w| !w.is_empty())
        .collect()
}

/// Words in front of a type-first parameter's type that are not the
/// type: modifiers (Java/Dart `final`, Dart `required`/`covariant`, C#
/// `ref`/`in`/`params`/`this`, PHP promoted-property visibility).
const TYPE_FIRST_MODIFIERS: &[&str] = &[
    "final",
    "const",
    "var",
    "val",
    "required",
    "covariant",
    "late",
    "readonly",
    "public",
    "private",
    "protected",
    "ref",
    "in",
    "params",
    "this",
    "scoped",
];

/// The pre-0.9 parser for every language except C++, reading words
/// bracket-aware (see [`param_words`]). The declared type is everything
/// between the modifiers and the name, read through
/// [`cgg_core::declared_type`]: `Foo? x`, `?Foo $x`, `Foo|null $x` and
/// `Nullable<Foo> x` are all a `Foo`. Annotations and attributes in front
/// of the type (`@Nullable Foo x`, `[FromBody] Foo x`) are skipped, and a
/// default value is not part of the parameter (`Foo x = null` named the
/// parameter `null`).
fn parse_type_first_param<'a>(lang: &str, param: &'a str) -> Option<(&'a str, &'a str)> {
    // "Service x" or "final Service x" or "Service<T> x"
    let param = strip_default_value(param).trim();
    let parts = param_words(param);
    let mut start = 0;
    while start < parts.len()
        && (TYPE_FIRST_MODIFIERS.contains(&parts[start])
            || parts[start].starts_with('@')
            || parts[start].starts_with('['))
    {
        start += 1;
    }
    if parts.len() < start + 2 {
        return None;
    }
    let name = parts[parts.len() - 1];
    // The type's words, as one slice of `param`: `Foo | null $x` is one
    // union, not a `Foo` followed by noise.
    let first = parts[start];
    let last = parts[parts.len() - 2];
    let ty_start = first.as_ptr() as usize - param.as_ptr() as usize;
    let ty_end = last.as_ptr() as usize - param.as_ptr() as usize + last.len();
    let ty = cgg_core::declared_type(lang, &param[ty_start..ty_end])?;
    // Words after the type that are not the name (Solidity `Foo memory
    // x`) keep the reading they always had: the first word.
    let ty = param_words(ty).first().copied().unwrap_or(ty);
    // `Map<String, Handler>` is a `Map`: the declared type is the generic
    // itself, never one of its arguments, and it is named without them —
    // as `parse_colon_param` names `m: Map<K, V>`, and as the Java and
    // Kotlin plugins name a generic local.
    let ty = ty.split('<').next().unwrap_or("").trim();
    if ty.is_empty() || name.is_empty() {
        return None;
    }
    if !ty.starts_with(char::is_uppercase) {
        return None;
    }
    if is_primitive(ty) {
        return None;
    }
    Some((name, ty))
}

/// `(name, declared type, pointee)` of a C++ parameter. The declared type
/// is the class itself (`QMap<Key, H> m` → `QMap`); the pointee is what
/// `m->f()` reaches — a class template's first argument (`sk_sp<Kernel>`,
/// `std::unique_ptr<Kernel>` → `Kernel`), or nothing for a raw pointer,
/// through which `->` reaches the declared type.
fn parse_type_first_param_cpp(param: &str) -> Option<(&str, Option<&str>, Option<&str>)> {
    // "Service x", "final Service x", "Service<T> x",
    // "const StreamOutput *stream", "Module* module".
    let param = strip_default_suffix(param);
    let parts = param_words(param);
    let mut start = 0;
    while start < parts.len() && is_param_modifier(parts[start]) {
        start += 1;
    }
    if parts.len() - start < 2 {
        return None;
    }
    let name = param_name_token(parts.last()?)?;
    let ty_tok = parts[start];
    let ty = nominal_type_token(ty_tok).filter(|t| !is_primitive(t));
    let raw_pointer =
        ty_tok.ends_with('*') || parts[start + 1..].iter().any(|w| w.contains('*'));
    let pointee = if raw_pointer {
        None
    } else {
        template_pointee(ty_tok.trim_end_matches(['*', '&']))
            .and_then(nominal_type_token)
            .filter(|t| !is_primitive(t))
    };
    if ty.is_none() && pointee.is_none() {
        return None;
    }
    Some((name, ty, pointee))
}

fn strip_default_suffix(param: &str) -> &str {
    let mut depth = 0i32;
    for (i, ch) in param.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            '=' if depth == 0 => return param[..i].trim(),
            _ => {}
        }
    }
    param
}

fn is_param_modifier(tok: &str) -> bool {
    matches!(
        tok,
        "final"
            | "const"
            | "volatile"
            | "var"
            | "val"
            | "unsigned"
            | "signed"
            | "static"
            | "extern"
            | "mutable"
            | "register"
            | "typename"
            | "inline"
            | "virtual"
            | "explicit"
    )
}

/// `Widget*` → `Widget`, `QMap<Key, Handler>` → `QMap`,
/// `std::unique_ptr<Kernel>` → `unique_ptr` (dropped: not a nominal type).
/// A class template is its own type, never one of its arguments; what
/// `->` reaches through it is recorded separately (see
/// [`parse_type_first_param_cpp`]).
fn nominal_type_token(tok: &str) -> Option<&str> {
    let mut tok = tok.trim_matches(|c: char| c == '*' || c == '&').trim();
    // A pointee arrives with its qualifiers (`shared_ptr<const Robot>`),
    // which the word-level modifier skip never saw.
    while let Some(rest) = tok
        .strip_prefix("const ")
        .or_else(|| tok.strip_prefix("volatile "))
    {
        tok = rest.trim_start();
    }
    // `std::map<std::string, H>`'s last scope segment is `map<…>`, not the
    // `string, H>` a plain `rsplit("::")` lands on.
    let tok = last_scope_segment(tok);
    let tok = tok.split('<').next().unwrap_or(tok).trim();
    let tok = tok.trim_matches(|c: char| c == '*' || c == '&');
    if tok.is_empty() || !tok.starts_with(char::is_uppercase) {
        return None;
    }
    Some(tok)
}

fn param_name_token(tok: &str) -> Option<&str> {
    let tok = tok.trim_matches(|c: char| c == '*' || c == '&');
    if tok.is_empty() {
        return None;
    }
    if !tok.chars().all(|c| c == '_' || c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(tok)
}

/// Type of `base->field->...` / `base.field...`.
///
/// `this` is the enclosing method's class. Anything else is a parameter,
/// a local, or an object-like macro already resolved to a type. Each
/// later piece must be a field of the type so far.
fn member_chain_type(
    rh: &str,
    arrow: bool,
    facts: &FileFacts,
    site_byte: u32,
    type_map: &HashMap<(u32, &str), &str>,
    fields: &HashMap<String, HashMap<String, String>>,
    macros: &HashMap<String, String>,
) -> Option<String> {
    // Each piece with the operator that follows it: `a->b.c` is `a` `->`,
    // `b` `.`, then `c` followed by the call's own operator. A piece
    // followed by `->` is dereferenced: declared as a class template, it
    // reaches its `#deref` type (see `DEREF_SUFFIX`).
    let mut pieces: Vec<(&str, bool)> = Vec::new();
    let mut rest = rh;
    loop {
        let arrow_at = rest.find("->");
        let dot_at = rest.find('.');
        let (at, len, deref) = match (arrow_at, dot_at) {
            (Some(a), Some(d)) if a < d => (a, 2, true),
            (Some(a), None) => (a, 2, true),
            (_, Some(d)) => (d, 1, false),
            (None, None) => break,
        };
        pieces.push((rest[..at].trim(), deref));
        rest = &rest[at + len..];
    }
    pieces.push((rest.trim(), arrow));
    if pieces.len() < 2 {
        return None;
    }
    let (base, base_deref) = pieces[0];
    let enclosing = enclosing_def(facts, site_byte);
    let deref_name = |n: &str| format!("{n}{DEREF_SUFFIX}");
    let mut ty = if base == "this" {
        enclosing.and_then(|d| {
            crate::names::owner_from_qn(&d.qualified_name).map(str::to_string)
        })?
    } else if let Some(enc) = enclosing
        && let Some(&param_ty) = base_deref
            .then(|| deref_name(base))
            .and_then(|k| type_map.get(&(enc.start_byte, k.as_str())).copied())
            .as_ref()
            .or_else(|| type_map.get(&(enc.start_byte, base)))
    {
        param_ty.to_string()
    } else if let Some(local_ty) = base_deref
        .then(|| deref_name(base))
        .and_then(|k| lookup_local_type(facts, site_byte, &k))
        .or_else(|| lookup_local_type(facts, site_byte, base))
    {
        local_ty.to_string()
    } else if let Some(macro_ty) = macros.get(base) {
        macro_ty.clone()
    } else {
        return None;
    };
    for &(field, deref) in &pieces[1..] {
        if !is_plain_ident(field) {
            return None;
        }
        let of_ty = fields.get(&ty)?;
        ty = deref
            .then(|| of_ty.get(&deref_name(field)))
            .flatten()
            .or_else(|| of_ty.get(field))?
            .clone();
    }
    if ty.starts_with(char::is_uppercase) {
        Some(ty)
    } else {
        None
    }
}

fn is_plain_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn find_constructor_assignments(facts: &FileFacts) -> HashMap<String, String> {
    // Look for refs that are constructor calls and try to find the
    // variable they're assigned to. We use a heuristic: if a RefRecord
    // has no receiver_hint and its name starts with uppercase (looks
    // like a type), it's likely a constructor call. We then look for
    // other refs in the same function that use that type name as a
    // receiver.
    //
    // Actually, we can do better: scan the definitions for constructor
    // variants and map their simple_name to qualified_name prefix.
    // Then for any ref whose name matches a type, we know that variable
    // assignments of that type exist.
    //
    // Simplest approach: collect all type names from definitions (class
    // names = any def whose qualified_name has the type as a segment).
    let mut type_names: std::collections::HashSet<&str> =
        std::collections::HashSet::new();
    for d in &facts.definitions {
        // Each segment of the qualified name that starts with uppercase
        // is likely a type name.
        for seg in d.qualified_name.split([':', '.']) {
            if seg.starts_with(char::is_uppercase) && !seg.is_empty() {
                type_names.insert(seg);
            }
        }
    }

    // Now scan refs: if a ref has name matching a type_name and no
    // receiver (bare call like `Foo()` or `new Foo()`), it's a
    // constructor. We can't easily find the variable name from the
    // AST at this point (we only have RefRecords), so we rely on
    // the parameter-type approach for most cases.
    //
    // For the common pattern where the variable name matches the type
    // (lowercased), we can infer: `service.run()` -> type `Service`.
    let mut map = HashMap::new();
    // Sorted. `type_names` is a HashSet and the inserts below are
    // last-write-wins, so two type names whose lowercase forms collide
    // (`HttpClient`/`HTTPClient`, `Repo`/`REPO`) fought for the same key
    // and the winner depended on hash order.
    let mut type_names_sorted: Vec<&&str> = type_names.iter().collect();
    type_names_sorted.sort_unstable();
    for ty in type_names_sorted {
        // `ty[..1]` panics whenever the first character is multi-byte,
        // and identifiers may legally be non-ASCII in Python, Java, C#
        // and Rust. Split on the first *character* instead of the first
        // byte.
        let mut chars = ty.chars();
        let lower = match chars.next() {
            Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
            None => continue,
        };
        map.insert(lower, ty.to_string());
        // Also try full lowercase
        map.insert(ty.to_lowercase(), ty.to_string());
    }
    map
}

/// A Go local type the plugin recorded for a value made through a
/// package: the call (`jobs.NewRunner()`) or the qualified type
/// (`jobs.Runner`). The older readings never carry a qualifier.
fn go_package_made(ty: &str) -> bool {
    ty.ends_with(')') || ty.contains('.')
}

/// Rust `let` bindings by name, each with the span of the innermost
/// callable it was declared in, sorted by position. Built once per file.
#[derive(Default)]
struct RustLets<'a> {
    by_name: HashMap<&'a str, Vec<RustLet<'a>>>,
}

/// `(let position, enclosing callable span, type)`.
type RustLet<'a> = (u32, Option<(u32, u32)>, &'a str);

impl<'a> RustLets<'a> {
    fn build(facts: &'a FileFacts) -> Self {
        let mut by_name: HashMap<&'a str, Vec<RustLet<'a>>> = HashMap::new();
        for lt in &facts.local_types {
            if lt.var_name.starts_with("self.") {
                continue;
            }
            let span =
                enclosing_def(facts, lt.scope_byte).map(|d| (d.start_byte, d.end_byte));
            by_name.entry(lt.var_name.as_str()).or_default().push((
                lt.scope_byte,
                span,
                lt.type_name.as_str(),
            ));
        }
        for v in by_name.values_mut() {
            // Stable: equal positions keep extraction order, and the last
            // of them wins below exactly as the file-wide map's did.
            v.sort_by_key(|(b, _, _)| *b);
        }
        RustLets { by_name }
    }

    /// The type of the latest `let name` before `site` that is in scope
    /// there: declared inside a callable that also contains `site` (the
    /// site's own, or one it is nested in — a closure sees its parent's
    /// locals). `None` when no such `let` exists.
    fn visible(
        &self,
        name: &str,
        site: u32,
        site_fn: Option<(u32, u32)>,
    ) -> Option<&'a str> {
        self.visible_at(name, site, site_fn).map(|(_, ty)| ty)
    }

    /// [`Self::visible`], with the position of the `let` it found.
    fn visible_at(
        &self,
        name: &str,
        site: u32,
        site_fn: Option<(u32, u32)>,
    ) -> Option<(u32, &'a str)> {
        let lets = self.by_name.get(name)?;
        site_fn?;
        lets.iter()
            .rev()
            .filter(|(b, _, _)| *b <= site)
            .find(|(_, span, _)| span.is_some_and(|(s, e)| s <= site && site < e))
            .map(|(b, _, ty)| (*b, *ty))
    }
}

/// Rust bindings by name, from the scope-spanned [`FileFacts::locals`]:
/// `(declaration byte, scope start, scope end)`. What decides whether a
/// type recorded for a name still applies at a site.
struct RustBindings<'a> {
    by_name: HashMap<&'a str, Vec<(u32, u32, u32)>>,
}

impl<'a> RustBindings<'a> {
    fn build(facts: &'a FileFacts) -> Self {
        let mut by_name: HashMap<&'a str, Vec<(u32, u32, u32)>> = HashMap::new();
        for b in &facts.locals {
            by_name.entry(b.name.as_str()).or_default().push((
                b.decl_byte,
                b.start_byte,
                b.end_byte,
            ));
        }
        Self { by_name }
    }

    fn of(&self, name: &str) -> &[(u32, u32, u32)] {
        let base = name.strip_suffix(GUARD_SUFFIX).unwrap_or(name);
        self.by_name.get(base).map_or(&[], |v| v.as_slice())
    }

    /// Whether the type recorded for `name` at `typed_at` (a `let`, or a
    /// function's start for its parameters) is hidden at `site` by a
    /// later binding: `let mut r = Router::build(); let mut r =
    /// r.finish(); r.recognize()` — the second `r` has a type cgg did not
    /// record, and the first one's no longer applies. The binding in
    /// scope at the site that was declared last is the one the site
    /// sees; the typed one is shadowed when another binding of `name`
    /// was declared between `typed_at` and it. Nothing recorded for the
    /// name (locals are skipped in minified files) shadows nothing.
    fn shadowed_since(&self, name: &str, typed_at: u32, site: u32) -> bool {
        let bs = self.of(name);
        let Some(seen) = bs
            .iter()
            .filter(|(d, s, e)| *s <= site && site < *e && *d >= typed_at)
            .map(|(d, _, _)| *d)
            .max()
        else {
            return false;
        };
        bs.iter().any(|(d, _, _)| *d >= typed_at && *d < seen)
    }
}

/// A file's local declarations and scope-spanned bindings by name, for
/// the WS23 scoped lookups (built once per file, on first use).
struct LocalIndex<'a> {
    types: HashMap<&'a str, Vec<&'a cgg_core::LocalType>>,
    binds: HashMap<&'a str, Vec<&'a cgg_core::LocalBinding>>,
}

impl<'a> LocalIndex<'a> {
    fn build(facts: &'a FileFacts) -> Self {
        let mut types: HashMap<&'a str, Vec<&'a cgg_core::LocalType>> = HashMap::new();
        for lt in &facts.local_types {
            types.entry(lt.var_name.as_str()).or_default().push(lt);
        }
        let mut binds: HashMap<&'a str, Vec<&'a cgg_core::LocalBinding>> = HashMap::new();
        for b in &facts.locals {
            binds.entry(b.name.as_str()).or_default().push(b);
        }
        Self { types, binds }
    }

    fn types(&self, name: &str) -> &[&'a cgg_core::LocalType] {
        self.types.get(name).map_or(&[], Vec::as_slice)
    }

    fn binds(&self, name: &str) -> &[&'a cgg_core::LocalBinding] {
        self.binds.get(name).map_or(&[], Vec::as_slice)
    }
}

/// The type of the JS/TS local `name` at `site` (WS23): the latest
/// declaration of it before the site that the plugin's scope-spanned
/// [`FileFacts::locals`] say is the binding in scope there, read from its
/// annotation (`const t: Svc`) or its construction (`const t = new
/// Svc()`). A constructed `let`/`var` assigned anywhere in its scope types
/// nothing; neither does a type that is not one plain class name (an
/// array, a union of several types, a function or object type, a
/// qualified name), nor a file without recorded bindings (minified).
fn js_local_type<'a>(
    facts: &'a FileFacts,
    ix: &LocalIndex<'a>,
    site: u32,
    name: &str,
) -> Option<&'a str> {
    let mut decls: Vec<&'a cgg_core::LocalType> = ix
        .types(name)
        .iter()
        .copied()
        .filter(|lt| lt.scope_byte <= site && lt.type_name != cgg_core::REASSIGNED_LOCAL)
        .collect();
    if decls.is_empty() {
        return None;
    }
    decls.sort_by_key(|lt| std::cmp::Reverse(lt.scope_byte));
    let (decl, span) = decls.into_iter().find_map(|lt| {
        visible_binding_span(ix, name, lt.scope_byte, site).map(|span| (lt, span))
    })?;
    let reassigned = ix.types(name).iter().any(|lt| {
        lt.type_name == cgg_core::REASSIGNED_LOCAL
            && span.0 <= lt.scope_byte
            && lt.scope_byte < span.1
    });
    if reassigned {
        return None;
    }
    let ty = cgg_core::declared_type(&facts.language, &decl.type_name)?;
    let ty = ty.split('<').next().unwrap_or(ty).trim();
    (is_plain_ident(ty) && ty.starts_with(char::is_uppercase) && !is_primitive(ty))
        .then_some(ty)
}

/// The scope span of the binding of `name` in scope at `site`, when it is
/// the declaration starting at `decl_at` (see [`local_decl_visible`]).
/// `None` when another binding is the one in scope, or when the file
/// records no binding of `name`.
fn visible_binding_span(
    ix: &LocalIndex<'_>,
    name: &str,
    decl_at: u32,
    site: u32,
) -> Option<(u32, u32)> {
    let mut visible: Option<&cgg_core::LocalBinding> = None;
    let mut first_after: Option<u32> = None;
    for &b in ix.binds(name) {
        if b.start_byte <= site
            && site < b.end_byte
            && b.decl_byte <= site
            && visible.is_none_or(|v| b.decl_byte > v.decl_byte)
        {
            visible = Some(b);
        }
        if b.decl_byte >= decl_at {
            first_after = Some(first_after.map_or(b.decl_byte, |v| v.min(b.decl_byte)));
        }
    }
    let v = visible?;
    (Some(v.decl_byte) == first_after).then_some((v.start_byte, v.end_byte))
}

/// Languages whose bare call results type a receiver (WS23): the call
/// as a receiver (`make().work()`) and a local the plugin recorded as a
/// call's result (`var t = make()`, type `make()`).
const CALL_RESULT_LANGS: &[&str] =
    &["java", "csharp", "dart", "groovy", "c", "cpp", "objc"];

/// The function a receiver written as a bare call names: `make()`,
/// `Make(a, b)` → `make`/`Make`; ObjC `[F make]` → `make`. `None` for
/// anything else (a member call `a.make()`, a construction).
fn call_result_callee<'a>(lang: &str, rh: &'a str) -> Option<&'a str> {
    if lang == "objc" {
        let inner = rh.strip_prefix('[')?.strip_suffix(']')?;
        let mut words = inner.split_whitespace();
        let (_recv, sel) = (words.next()?, words.next()?);
        return (words.next().is_none() && is_plain_ident(sel)).then_some(sel);
    }
    if !rh.ends_with(')') {
        return None;
    }
    let open = rh.find('(')?;
    let name = &rh[..open];
    (is_plain_ident(name) && matching_paren(rh.as_bytes(), open) == Some(rh.len() - 1))
        .then_some(name)
}

/// The callee of the call-result local ([`CALL_RESULT_LANGS`]) `name`
/// that is in scope at `site_byte`: the latest such declaration before the
/// site in the calling function, else at the file's top level. A later
/// declaration of the name with a written type hides it.
fn lookup_call_local<'a>(
    facts: &'a FileFacts,
    ix: &LocalIndex<'a>,
    site_byte: u32,
    name: &str,
) -> Option<&'a str> {
    let enclosing = enclosing_def(facts, site_byte);
    let in_enclosing =
        |b: u32| enclosing.is_some_and(|d| d.start_byte <= b && b < d.end_byte);
    // The calling function's declarations before the site, latest first,
    // then the file's top-level ones: the first whose binding is the one
    // in scope (an inner block's that has ended is not).
    let mut decls: Vec<&cgg_core::LocalType> =
        ix.types(name)
            .iter()
            .copied()
            .filter(|lt| lt.scope_byte <= site_byte)
            .filter(|lt| {
                in_enclosing(lt.scope_byte)
                    || !facts.definitions.iter().any(|d| {
                        d.start_byte <= lt.scope_byte && lt.scope_byte < d.end_byte
                    })
            })
            .collect();
    decls.sort_by_key(|lt| {
        (
            !in_enclosing(lt.scope_byte),
            std::cmp::Reverse(lt.scope_byte),
        )
    });
    let lt = decls
        .into_iter()
        .find(|lt| local_decl_visible(ix, name, lt.scope_byte, site_byte))?;
    call_result_callee(&facts.language, &lt.type_name)
}

/// Whether the declaration of `name` that starts at `decl_at` is the
/// binding of `name` in scope at `site`, by the plugin's scope-spanned
/// [`FileFacts::locals`]: the binding visible there (the latest declared
/// whose scope holds the site) is the first one declared at or after
/// `decl_at`. An inner block's own `x`, a loop variable or a closure
/// parameter of the name hides the declaration. A file that records no
/// binding of `name` (a language without a locals table, a minified
/// file) cannot tell, and the declaration stands.
fn local_decl_visible(ix: &LocalIndex<'_>, name: &str, decl_at: u32, site: u32) -> bool {
    let mut any = false;
    let mut visible: Option<u32> = None;
    let mut first_after: Option<u32> = None;
    for &b in ix.binds(name) {
        any = true;
        if b.start_byte <= site && site < b.end_byte && b.decl_byte <= site {
            visible = Some(visible.map_or(b.decl_byte, |v| v.max(b.decl_byte)));
        }
        if b.decl_byte >= decl_at {
            first_after = Some(first_after.map_or(b.decl_byte, |v| v.min(b.decl_byte)));
        }
    }
    !any || visible.is_some() && visible == first_after
}

fn lookup_local_type<'a>(
    facts: &'a FileFacts,
    site_byte: u32,
    name: &str,
) -> Option<&'a str> {
    lookup_local_type_any(facts, site_byte, name).filter(|ty| {
        !(CALL_RESULT_LANGS.contains(&facts.language.as_str())
            && call_result_callee(&facts.language, ty).is_some())
    })
}

/// [`lookup_local_type`], call-result locals included.
fn lookup_local_type_any<'a>(
    facts: &'a FileFacts,
    site_byte: u32,
    name: &str,
) -> Option<&'a str> {
    lookup_local_type_at(facts, site_byte, name).map(|(_, ty)| ty)
}

/// [`lookup_local_type_any`], with where the declaration starts.
fn lookup_local_type_at<'a>(
    facts: &'a FileFacts,
    site_byte: u32,
    name: &str,
) -> Option<(u32, &'a str)> {
    let enclosing = enclosing_def(facts, site_byte);
    let mut best_fn: Option<(u32, &'a str)> = None;
    let mut best_file: Option<(u32, &'a str)> = None;
    for lt in &facts.local_types {
        if lt.var_name != name || lt.scope_byte > site_byte {
            continue;
        }
        let inside_enclosing = enclosing
            .is_some_and(|d| d.start_byte <= lt.scope_byte && lt.scope_byte < d.end_byte);
        if inside_enclosing {
            if best_fn.is_none_or(|(b, _)| lt.scope_byte >= b) {
                best_fn = Some((lt.scope_byte, lt.type_name.as_str()));
            }
            continue;
        }
        let inside_other = facts
            .definitions
            .iter()
            .any(|d| d.start_byte <= lt.scope_byte && lt.scope_byte < d.end_byte);
        if !inside_other && best_file.is_none_or(|(b, _)| lt.scope_byte >= b) {
            best_file = Some((lt.scope_byte, lt.type_name.as_str()));
        }
    }
    best_fn.or(best_file)
}

fn enclosing_def(facts: &FileFacts, byte: u32) -> Option<&DefRecord> {
    let mut best: Option<(&DefRecord, u32)> = None;
    for d in &facts.definitions {
        if d.start_byte <= byte && byte < d.end_byte {
            let span = d.end_byte - d.start_byte;
            match best {
                None => best = Some((d, span)),
                Some((_, b)) if span < b => best = Some((d, span)),
                _ => {}
            }
        }
    }
    best.map(|(d, _)| d)
}

/// The declared return type of `sig`, read as the type a method call on
/// the returned value reaches ([`cgg_core::declared_type`]): `-> Optional[Foo]`,
/// `-> Foo?`, `): Foo | undefined` and `-> "Foo"` all return a `Foo`. A
/// union of several real types returns nothing typed (it used to return
/// its first member).
fn extract_return_type<'a>(lang: &str, sig: &'a str) -> Option<&'a str> {
    // Java/C#/Dart/Groovy/C/C++: `public Service getService()`, the type
    // in front of the name (WS23); ObjC: `- (Service *)service`. A C++
    // trailing `auto f() -> T` is read by the `->` branch below.
    if TYPE_FIRST_RETURN_LANGS.contains(&lang)
        && let Some(ret) = type_first_return(lang, sig)
    {
        return Some(ret);
    }
    // Rust: `fn foo() -> Config`
    if let Some(pos) = sig.find("->") {
        let ret = return_type_text(&sig[pos + 2..]);
        let ret = cgg_core::declared_type(lang, ret)?;
        let ret = ret
            .trim_start_matches('&')
            .trim_start_matches("mut ")
            .trim();
        let ret = ret
            .split(['<', '[', '{', ',', ' '])
            .next()
            .unwrap_or(ret)
            .trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) {
            return Some(ret);
        }
    }
    // TS/Kotlin/Swift/Scala: `fun foo(): Config` or `foo(): Config`. (A
    // Java/C# type-first return was read by the first branch above.)
    if let Some(pos) = sig.find("): ") {
        let ret = return_type_text(&sig[pos + 3..]);
        let ret = cgg_core::declared_type(lang, ret)?;
        let ret = ret.split(['<', '{', ' ', '?']).next().unwrap_or(ret).trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) {
            return Some(ret);
        }
    }
    // Go: `func Foo() Config {` — return type after ) and before {
    if let Some(paren_close) = sig.rfind(')') {
        let after = sig[paren_close + 1..].trim();
        let after = after.trim_start_matches('*');
        let ret = after.split(['{', ',', ' ']).next().unwrap_or("").trim();
        if !ret.is_empty() && ret.starts_with(char::is_uppercase) && !is_primitive(ret) {
            return Some(ret);
        }
    }
    None
}

/// Languages that write a function's return type in front of its name
/// (or, ObjC, in the parentheses after `-`/`+`).
const TYPE_FIRST_RETURN_LANGS: &[&str] =
    &["java", "csharp", "dart", "groovy", "c", "cpp", "objc"];

/// Words in front of a type-first function's name that are not its
/// return type.
const RETURN_TYPE_MODIFIERS: &[&str] = &[
    "public",
    "private",
    "protected",
    "internal",
    "static",
    "final",
    "abstract",
    "synchronized",
    "native",
    "default",
    "strictfp",
    "virtual",
    "override",
    "sealed",
    "async",
    "extern",
    "unsafe",
    "new",
    "partial",
    "readonly",
    "inline",
    "constexpr",
    "consteval",
    "explicit",
    "friend",
    "external",
    "factory",
    "operator",
    "def",
    "void",
    "auto",
    "var",
    "dynamic",
    "struct",
    "class",
    "enum",
    "union",
    "typename",
    "const",
    "volatile",
    "register",
    "transient",
];

/// The declared return type of a type-first signature (see
/// [`TYPE_FIRST_RETURN_LANGS`]), as the named type a method call on the
/// result reaches: `public static Tool make()` → `Tool`, `const Other&
/// other()` → `Other`, `List<Tool> many()` → `List`, `- (Tool *)make` →
/// `Tool`. `void`, `auto`, `def`, `dynamic` and a primitive return nothing
/// typed, nor does a top type (`Object`, [`cgg_core::stdlib::TOP_TYPES`])
/// or an awaitable (`Task<Tool>`, `Future<Tool>`): what a call on it
/// reaches depends on whether the call site awaits it
/// ([`cgg_core::stdlib::AWAITABLE_TYPES`]).
fn type_first_return<'a>(lang: &str, sig: &'a str) -> Option<&'a str> {
    let sig = sig.trim();
    if lang == "objc" {
        let rest = sig.strip_prefix(['-', '+'])?.trim_start();
        let rest = rest.strip_prefix('(')?;
        let close = rest.find(')')?;
        return nominal_return(lang, rest[..close].trim());
    }
    let (open, _) = param_list_span(sig)?;
    let head = sig[..open].trim_end();
    // The callable's name: `make`, `Factory::make`, `~T`, `operator()`.
    let name_start = head
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || matches!(c, '_' | ':' | '~' | '$'))
        .last()
        .map(|(i, _)| i)?;
    let before = head[..name_start].trim_end();
    // `Tool *make`, `const Other &other`: the pointer belongs to the type.
    let before = before.trim_end_matches(['*', '&', ' ']);
    let words = param_words(before);
    let ty = words
        .iter()
        .rev()
        .find(|w| !w.starts_with('@') && !w.starts_with('['))
        .copied()?;
    // A Java/C# generic method's type parameters (`<T> T pick(…)`) are
    // not its return type, nor is anything inside a C++ `template<…>`.
    if ty.starts_with('<') || ty.ends_with('>') && ty.starts_with("template") {
        return None;
    }
    nominal_return(lang, ty)
}

/// [`type_first_return`]'s reading of one written type.
fn nominal_return<'a>(lang: &str, ty: &'a str) -> Option<&'a str> {
    let ty = ty.trim().trim_end_matches(['*', '&', '?', ' ', '^']);
    let ty = cgg_core::declared_type(lang, ty)?;
    let ty = ty.trim_start_matches("const ").trim();
    if RETURN_TYPE_MODIFIERS.contains(&ty) {
        return None;
    }
    let head = ty.split('<').next().unwrap_or(ty).trim();
    let head = last_scope_segment(head);
    let head = head.rsplit('.').next().unwrap_or(head);
    let head = head.trim_end_matches(['*', '&', '?']);
    // A one-letter name is a type parameter far more often than a class
    // (`<T> T pick()`, `template <class T> T get()`).
    if head.chars().count() < 2
        || !head.chars().all(|c| c.is_alphanumeric() || c == '_')
        || cgg_core::stdlib::is_awaitable_type(lang, head)
        // `Object make()` says nothing about the class of what it returns.
        || cgg_core::stdlib::is_top_type(lang, head)
    {
        return None;
    }
    Some(head)
}

/// The return-type text at the start of `s`: up to a top-level body
/// (`{`), expression body (`=`, `=>`), Python body colon, or Rust `where`
/// clause, trimmed.
fn return_type_text(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i = skip_quoted(bytes, i);
                continue;
            }
            b'(' | b'[' | b'<' => depth += 1,
            b'>' if i > 0 && matches!(bytes[i - 1], b'-' | b'=') => {}
            b')' | b']' | b'>' => depth = (depth - 1).max(0),
            b'{' | b'=' if depth == 0 => return s[..i].trim(),
            b':' if depth == 0
                && bytes.get(i + 1) != Some(&b':')
                && (i == 0 || bytes[i - 1] != b':') =>
            {
                return s[..i].trim();
            }
            _ => {}
        }
        i += 1;
    }
    if let Some(w) = s.find(" where ") {
        return s[..w].trim();
    }
    s.trim()
}

/// A Zig parameter type with its optional, pointer and `const` markers
/// removed (`?*const Surface` → `Surface`), so the receiver is the type
/// the resolver can follow through the file's imports.
fn zig_peel_type(mut ty: &str) -> &str {
    loop {
        let t = ty.trim_start();
        let next = t
            .strip_prefix('?')
            .or_else(|| t.strip_prefix('*'))
            .or_else(|| t.strip_prefix("const "))
            .or_else(|| t.strip_prefix("volatile "));
        match next {
            Some(n) => ty = n,
            None => return t.trim(),
        }
    }
}

fn is_primitive(ty: &str) -> bool {
    matches!(
        ty,
        "int"
            | "i32"
            | "i64"
            | "u32"
            | "u64"
            | "f32"
            | "f64"
            | "bool"
            | "str"
            | "String"
            | "string"
            | "void"
            | "char"
            | "byte"
            | "short"
            | "long"
            | "float"
            | "double"
            | "usize"
            | "isize"
            | "u8"
            | "i8"
            | "u16"
            | "i16"
            | "number"
            | "boolean"
            | "any"
            | "object"
            | "Int"
            | "Long"
            | "Float"
            | "Double"
            | "Boolean"
            | "Unit"
            | "Nothing"
            | "Void"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use cgg_core::{DefVariant, LocalType, RefRecord};
    use std::path::PathBuf;

    fn mk_facts(defs: Vec<DefRecord>, refs: Vec<RefRecord>) -> FileFacts {
        let mut f = FileFacts::new(FileId::new(0), PathBuf::from("/tmp/test.rs"), "rust");
        f.definitions = defs;
        f.references = refs;
        f
    }

    fn mk_def(qn: &str, sig: &str, start: u32, end: u32) -> DefRecord {
        DefRecord {
            simple_name: qn.rsplit("::").next().unwrap_or(qn).to_string(),
            qualified_name: qn.to_string(),
            variant: DefVariant::FreeFunction,
            start_line: 1,
            end_line: 10,
            start_byte: start,
            end_byte: end,
            signature_hint: sig.to_string(),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        }
    }

    /// WS13: `let x = Type::f(..)` is typed `Type` only when `f` returns
    /// it. Another named return type retypes the local; a primitive
    /// leaves it untyped; a type with no declaration of `f` in the tree
    /// (an external constructor) keeps the reading.
    #[test]
    fn assoc_call_locals_follow_the_declared_return_type() {
        let lib = mk_facts(
            vec![
                mk_def("app::Loader::new", "pub fn new(p: String) -> Self", 0, 10),
                mk_def(
                    "app::Loader::load",
                    "pub fn load(p: &str) -> Config",
                    10,
                    20,
                ),
                mk_def(
                    "app::Loader::open",
                    "fn open(p: &str) -> io::Result<Loader>",
                    20,
                    30,
                ),
                mk_def("app::Loader::count", "fn count() -> usize", 30, 40),
                mk_def("app::Loader::ctx", "fn ctx() -> web::Data<Ctx>", 50, 60),
                mk_def(
                    "app::Loader::wrap",
                    "fn wrap(cb: impl Fn(u8) -> Config) -> Option<Arc<Self>>",
                    40,
                    50,
                ),
            ],
            vec![],
        );
        let mut user = FileFacts::new(FileId::new(1), PathBuf::from("/tmp/u.rs"), "rust");
        for (var, ty, f) in [
            ("a", "Loader", "new"),
            ("b", "Loader", "load"),
            ("c", "app::Loader", "open"),
            ("d", "Loader", "count"),
            ("e", "Loader", "wrap"),
            ("g", "File", "open"),
            ("k", "Loader", "ctx"),
        ] {
            user.local_types.push(cgg_core::LocalType {
                var_name: var.into(),
                type_name: ty.into(),
                scope_byte: 100,
            });
            user.assoc_call_locals.push(cgg_core::AssocCallLocal {
                var_name: var.into(),
                scope_byte: 100,
                assoc_fn: f.into(),
            });
        }
        // Typed some other way (a parameter): never revisited.
        user.local_types.push(cgg_core::LocalType {
            var_name: "h".into(),
            type_name: "Loader".into(),
            scope_byte: 100,
        });
        let mut all = vec![lib, user];
        settle_assoc_call_locals(&mut all);
        let ty = |v: &str| {
            all[1]
                .local_types
                .iter()
                .find(|lt| lt.var_name == v)
                .map(|lt| lt.type_name.as_str())
        };
        assert_eq!(ty("a"), Some("Loader"));
        assert_eq!(ty("b"), Some("Config"));
        assert_eq!(ty("c"), Some("app::Loader"));
        assert_eq!(ty("d"), None);
        assert_eq!(ty("e"), Some("Loader"));
        assert_eq!(ty("g"), Some("File"));
        assert_eq!(ty("h"), Some("Loader"));
        assert_eq!(ty("k"), Some("Data<Ctx>"));
        assert!(all[1].assoc_call_locals.is_empty());
    }

    /// WS13: `let mut r = Builder::new(); let mut r = r.finish();` — the
    /// second `r` has no recorded type, and the first one's no longer
    /// applies after it. A same-named binding in a block the site is not
    /// in leaves the typed one visible; inside that block it is hidden.
    #[test]
    fn an_untyped_rust_rebinding_ends_the_typed_one() {
        let mut f = mk_facts(vec![mk_def("m::run", "fn run()", 0, 500)], vec![]);
        let bind = |decl: u32, start: u32, end: u32| cgg_core::LocalBinding {
            name: "r".into(),
            start_byte: start,
            end_byte: end,
            decl_byte: decl,
            param: false,
        };
        // typed `let r` at 10 (decl 18, in scope 40..500), a block-local
        // `r` (decl 60, in scope 70..90), the shadowing `let r` (decl
        // 108, in scope 130..500).
        f.locals = vec![bind(18, 40, 500), bind(60, 70, 90), bind(108, 130, 500)];
        let b = RustBindings::build(&f);
        assert!(!b.shadowed_since("r", 10, 50));
        assert!(!b.shadowed_since("r", 10, 95));
        assert!(b.shadowed_since("r", 10, 80));
        assert!(b.shadowed_since("r", 10, 200));
        assert!(b.shadowed_since("r#guard", 10, 200));
        assert!(!b.shadowed_since("q", 10, 200));
    }

    /// WS13, end to end: a parameter re-bound by an untyped `let` loses
    /// its declared type after the `let`; a name with no `let` visible
    /// keeps the file-wide reading it always had.
    #[test]
    fn rust_receivers_follow_the_binding_in_scope() {
        let call = |name: &str, recv: &str, byte: u32| RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            site_byte: byte,
            ..Default::default()
        };
        let mut f = mk_facts(
            vec![
                mk_def("m::a", "fn a(args: Stream)", 0, 100),
                mk_def("m::b", "fn b()", 100, 200),
                mk_def("m::c", "fn c(specs: &[S])", 200, 300),
            ],
            vec![
                call("run", "args", 40),
                call("run", "args", 90),
                call("go", "spec", 150),
                call("go", "spec", 260),
            ],
        );
        let bind = |name: &str, decl: u32, start: u32, end: u32| cgg_core::LocalBinding {
            name: name.into(),
            start_byte: start,
            end_byte: end,
            decl_byte: decl,
            param: false,
        };
        f.locals = vec![
            bind("args", 5, 5, 100),
            bind("args", 60, 70, 100),
            bind("spec", 110, 130, 200),
            bind("spec", 240, 250, 300),
        ];
        f.local_types.push(cgg_core::LocalType {
            var_name: "spec".into(),
            type_name: "ColorSpec".into(),
            scope_byte: 105,
        });
        propagate_types(&mut f);
        let hints: Vec<&str> = f
            .references
            .iter()
            .map(|r| r.receiver_hint.as_str())
            .collect();
        assert_eq!(hints, vec!["Stream", "args", "ColorSpec", "ColorSpec"]);
    }

    /// Two declarations of `f` on same-named types that disagree leave
    /// the local untyped rather than pick one.
    #[test]
    fn disagreeing_assoc_declarations_leave_the_local_untyped() {
        let lib = mk_facts(
            vec![
                mk_def("a::Store::get", "fn get() -> Self", 0, 10),
                mk_def("b::Store::get", "fn get() -> Entry", 10, 20),
            ],
            vec![],
        );
        let mut user = FileFacts::new(FileId::new(1), PathBuf::from("/tmp/u.rs"), "rust");
        user.local_types.push(cgg_core::LocalType {
            var_name: "s".into(),
            type_name: "Store".into(),
            scope_byte: 5,
        });
        user.assoc_call_locals.push(cgg_core::AssocCallLocal {
            var_name: "s".into(),
            scope_byte: 5,
            assoc_fn: "get".into(),
        });
        let mut all = vec![lib, user];
        settle_assoc_call_locals(&mut all);
        assert!(all[1].local_types.is_empty());
    }

    /// FINDINGS A5, Go: a parameter of a package-qualified type keeps its
    /// qualifier (`t *util.T` → `util.T`), names declared together share
    /// the type, unqualified and composite types stay untyped, and a
    /// method's receiver list is not its parameters.
    #[test]
    fn a_scope_separator_is_not_a_name_type_colon() {
        // `ns::Foo x` (C++) / `global::A.B y` (C#) are type-first; only a
        // lone `:` separates a name from its type.
        assert_eq!(parse_colon_param("cpp", "ns::Foo x"), None);
        assert_eq!(parse_colon_param("cpp", "const ns::Foo& y"), None);
        assert_eq!(
            parse_colon_param("rust", "x: a::Store"),
            Some(("x", "a::Store"))
        );
        assert_eq!(
            parse_colon_param("kotlin", "svc: Service"),
            Some(("svc", "Service"))
        );
        assert_eq!(parse_colon_param("lean", "x :: xs : List α"), None);
    }

    #[test]
    fn go_parameters_of_a_package_type_keep_the_package() {
        let d = mk_def(
            "main.use",
            "func (s *Server) use(t *util.T, a, b jobs.Runner, n int, xs []util.T, q Local)",
            0,
            100,
        );
        let got: Vec<(String, String)> = go_param_types(&d)
            .into_iter()
            .map(|(_, n, t)| (n, t))
            .collect();
        assert_eq!(
            got,
            vec![
                ("s".to_string(), "Server".to_string()),
                ("t".to_string(), "util.T".to_string()),
                ("a".to_string(), "jobs.Runner".to_string()),
                ("b".to_string(), "jobs.Runner".to_string()),
                ("q".to_string(), "Local".to_string()),
            ]
        );
    }

    /// WS23 gap 26: type-first languages declare the return type in front
    /// of the name. Modifiers, annotations, pointers and generic arguments
    /// are not it; `void`/`def`/`auto`, a top type, an awaitable and a
    /// one-letter type parameter type nothing.
    #[test]
    fn type_first_return_types_are_read_in_front_of_the_name() {
        fn ret<'a>(lang: &str, sig: &'a str) -> Option<&'a str> {
            extract_return_type(lang, sig)
        }
        assert_eq!(
            ret("java", "@Override public static Tool make()"),
            Some("Tool")
        );
        assert_eq!(
            ret("java", "public <T> List<T> many(int n) throws IOException"),
            Some("List")
        );
        // A Java `Optional` is the receiver of `.get()`, not a `Tool`.
        assert_eq!(
            ret("java", "public Optional<Tool> find()"),
            Some("Optional")
        );
        assert_eq!(ret("java", "Object any()"), None);
        assert_eq!(ret("java", "<T> T pick(T a)"), None);
        assert_eq!(ret("java", "void run()"), None);
        assert_eq!(
            ret("csharp", "[HttpGet] public async Task<Tool> MakeAsync()"),
            None
        );
        assert_eq!(ret("csharp", "public Tool? Find(string id)"), Some("Tool"));
        assert_eq!(ret("dart", "Tool make() => Tool()"), Some("Tool"));
        assert_eq!(ret("dart", "Future<Tool> load() async"), None);
        assert_eq!(ret("groovy", "def dyn()"), None);
        assert_eq!(ret("groovy", "static Tool make()"), Some("Tool"));
        assert_eq!(ret("cpp", "static const Other& other()"), Some("Other"));
        assert_eq!(ret("cpp", "Tool* Factory::make(int n)"), Some("Tool"));
        assert_eq!(ret("cpp", "ns::Gear *gear()"), Some("Gear"));
        assert_eq!(ret("cpp", "auto make() -> Tool"), Some("Tool"));
        assert_eq!(ret("cpp", "Factory::Factory(int n)"), None);
        assert_eq!(ret("objc", "+ (Tool *)make"), Some("Tool"));
        assert_eq!(ret("objc", "- (void)work"), None);
        // Not a type-first language: the name-first readers apply.
        assert_eq!(ret("kotlin", "fun make(): Tool"), Some("Tool"));
    }

    /// WS23 gap 26: a receiver written as a bare call names its callee;
    /// a member call, a construction-shaped text with trailing text, or a
    /// chained call does not.
    #[test]
    fn call_result_receivers_name_their_callee() {
        assert_eq!(call_result_callee("java", "make()"), Some("make"));
        assert_eq!(call_result_callee("csharp", "Make(a, (b))"), Some("Make"));
        assert_eq!(call_result_callee("java", "a.make()"), None);
        assert_eq!(call_result_callee("java", "make().other()"), None);
        assert_eq!(call_result_callee("java", "(make())"), None);
        assert_eq!(call_result_callee("objc", "[F make]"), Some("make"));
        assert_eq!(call_result_callee("objc", "[F make:x]"), None);
    }

    /// WS23 gap 10: a Go receiver and same-package parameters are typed
    /// name-first. Predeclared types, type parameters, composite types
    /// and `_` type nothing.
    #[test]
    fn go_receivers_and_bare_parameter_types_are_read_name_first() {
        let read = |sig: &str| -> Vec<(String, String)> {
            go_param_types(&mk_def("main.f", sig, 0, 100))
                .into_iter()
                .map(|(_, n, t)| (n, t))
                .collect()
        };
        let pair = |n: &str, t: &str| (n.to_string(), t.to_string());
        assert_eq!(read("func (c client) Ping()"), vec![pair("c", "client")]);
        assert_eq!(
            read("func (b *Box[T]) Get(x T, e error, s string) T"),
            vec![pair("b", "Box")]
        );
        assert_eq!(
            read("func Map[T any, U Shape](xs []T, f func(T) U, u U, k Key) U"),
            vec![pair("k", "Key")]
        );
        assert_eq!(
            read(
                "func f(_ *Server, m map[string]Server, ch chan Server, i interface{ zip() })"
            ),
            Vec::<(String, String)>::new()
        );
        assert_eq!(
            read("func (*Server) Anon(p *Peer)"),
            vec![pair("p", "Peer")]
        );
    }

    /// FINDINGS A5, Go: `r := jobs.NewRunner()` types `r` only where that
    /// `:=` is in scope. The same short name in another function is not
    /// that call's result, so it is left as written.
    #[test]
    fn go_package_made_locals_are_scoped_to_their_function() {
        let defs = vec![
            mk_def("main.a", "func a()", 0, 100),
            mk_def("main.b", "func b()", 100, 200),
            mk_def("main.c", "func c()", 200, 300),
        ];
        let call = |byte: u32| RefRecord {
            name: "M".into(),
            receiver_hint: "r".into(),
            site_line: 1,
            site_byte: byte,
            ..Default::default()
        };
        let mut facts = mk_facts(defs, vec![call(50), call(150), call(250)]);
        facts.language = "go".into();
        facts.local_types = vec![
            LocalType {
                var_name: "r".into(),
                type_name: "jobs.NewRunner()".into(),
                scope_byte: 10,
            },
            LocalType {
                var_name: "r".into(),
                type_name: "other.Open()".into(),
                scope_byte: 110,
            },
        ];
        propagate_types(&mut facts);
        let got: Vec<&str> = facts
            .references
            .iter()
            .map(|r| r.receiver_hint.as_str())
            .collect();
        assert_eq!(got, vec!["jobs.NewRunner()", "other.Open()", "r"]);
    }

    /// R1: a Go value made through a package is typed from the end of its
    /// `:=` (the right side's `pp` is still the package) to the end of its
    /// block (a sibling block's `pp.New()` is the package again). A
    /// variable's existing file-wide reading is untouched.
    #[test]
    fn go_package_made_locals_respect_declaration_and_block() {
        let defs = vec![mk_def("main.f", "func f()", 0, 300)];
        let call = |byte: u32| RefRecord {
            name: "M".into(),
            receiver_hint: "pp".into(),
            site_line: 1,
            site_byte: byte,
            ..Default::default()
        };
        let mut facts = mk_facts(defs, vec![call(40), call(80), call(150)]);
        facts.language = "go".into();
        facts.local_types = vec![
            // `pp, err := pp.New()` spanning 30..60, in a block ending at 100.
            LocalType {
                var_name: "pp".into(),
                type_name: "pp.New()".into(),
                scope_byte: 60,
            },
            LocalType {
                var_name: "pp".into(),
                type_name: String::new(),
                scope_byte: 100,
            },
        ];
        propagate_types(&mut facts);
        let got: Vec<&str> = facts
            .references
            .iter()
            .map(|r| r.receiver_hint.as_str())
            .collect();
        assert_eq!(got, vec!["pp", "pp.New()", "pp"]);
    }

    /// WS19 gap 4: the return-type map is keyed by language and name,
    /// and a name is typed only when every definition of it in the
    /// language declares the same type. It used to be keyed by the bare
    /// name, first definition wins: a Swift `make() -> Service` typed a
    /// Python `service = make()`, and of two Python `make`s whichever was
    /// read first decided for both.
    #[test]
    fn the_return_type_map_is_per_language_and_needs_agreement() {
        let file = |id: u32, lang: &str, defs: Vec<DefRecord>| {
            let mut f =
                FileFacts::new(FileId::new(id), PathBuf::from(format!("f{id}")), lang);
            f.definitions = defs;
            f
        };
        let all = vec![
            file(
                0,
                "swift",
                vec![mk_def("make", "func make() -> Service", 0, 10)],
            ),
            file(
                1,
                "python",
                vec![mk_def("make", "def make() -> Worker", 0, 10)],
            ),
            file(
                2,
                "python",
                vec![mk_def("make", "def make() -> Service", 0, 10)],
            ),
            file(
                3,
                "python",
                vec![mk_def("load", "def load() -> Config", 0, 10)],
            ),
            file(
                4,
                "python",
                vec![mk_def("load", "def load() -> Config", 0, 10)],
            ),
            // An untyped definition leaves the name's type unknown.
            file(5, "rust", vec![mk_def("open", "fn open() -> Conn", 0, 10)]),
            file(6, "rust", vec![mk_def("open", "fn open()", 0, 10)]),
        ];
        let map = build_return_type_map(&all);
        assert_eq!(map.get(&("swift", "make")), Some(&"Service"));
        assert_eq!(map.get(&("python", "make")), None, "{map:?}");
        assert_eq!(map.get(&("python", "load")), Some(&"Config"));
        assert_eq!(map.get(&("rust", "open")), None, "{map:?}");
    }

    /// WS19 gap 4: a receiver is typed only from its own language's
    /// functions, and a function the calling file defines is read from
    /// that definition, not the run-wide map. Near miss: a name only
    /// another file of the language defines still types it.
    #[test]
    fn strategy_four_reads_the_callers_language_and_file_first() {
        let mut ret: HashMap<(&str, &str), &str> = HashMap::new();
        ret.insert(("swift", "make"), "Service");
        ret.insert(("python", "build"), "Service");
        let index = ReturnTypeIndex::build(&ret);
        let call = |name: &str, recv: &str, at: u32| RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            site_byte: at,
            ..Default::default()
        };
        let run = |own: Vec<DefRecord>, called: &str| {
            let mut defs = vec![mk_def("main", "def main()", 0, 100)];
            defs.extend(own);
            let mut facts =
                mk_facts(defs, vec![call(called, "", 10), call("run", "service", 20)]);
            facts.language = "python".into();
            propagate_types_with_returns(&mut facts, &index);
            (
                facts.references[1].receiver_hint.clone(),
                facts.references[1].weak_receiver_type,
            )
        };
        // Swift's `make` says nothing about a Python call of `make`.
        assert_eq!(run(vec![], "make").0, "service");
        // Python's `build` does (weakly, as ever).
        assert_eq!(run(vec![], "build"), ("Service".to_string(), true));
        // The file's own untyped `build` is the one it calls.
        let own = mk_def("build", "def build()", 200, 300);
        assert_eq!(run(vec![own], "build").0, "service");
        // The file's own typed `make` types it.
        let own = mk_def("make", "def make() -> Service", 200, 300);
        assert_eq!(run(vec![own], "make"), ("Service".to_string(), true));
    }

    /// FINDINGS R14: an Erlang receiver is the module of `m:f(…)`; the
    /// return-type heuristic that turns a `file` receiver into a `File`
    /// some function returns never applies to it. The near miss: the same
    /// shape in another language is still rewritten.
    #[test]
    fn erlang_module_receivers_are_never_typed() {
        let mut ret: HashMap<(&str, &str), &str> = HashMap::new();
        ret.insert(("erlang", "file"), "File");
        ret.insert(("python", "file"), "File");
        let index = ReturnTypeIndex::build(&ret);
        for (lang, want) in [("erlang", "file"), ("python", "File")] {
            let defs = vec![mk_def("m:run", "run(Fd) -> ok", 0, 100)];
            let refs = vec![
                RefRecord {
                    name: "file".into(),
                    site_byte: 10,
                    ..Default::default()
                },
                RefRecord {
                    name: "file:position".into(),
                    receiver_hint: "file".into(),
                    site_byte: 20,
                    ..Default::default()
                },
            ];
            let mut facts = mk_facts(defs, refs);
            facts.language = lang.into();
            propagate_types_with_returns(&mut facts, &index);
            assert_eq!(facts.references[1].receiver_hint, want, "{lang}");
        }
    }

    /// WS26: the return-type heuristic types a receiver by a function
    /// called earlier in the file. A value reference to that function
    /// (`mapT(mapEntry)`) calls nothing, so it types nothing; the call
    /// itself still does.
    #[test]
    fn a_value_reference_is_not_a_call_for_the_return_type_heuristic() {
        let mut ret: HashMap<(&str, &str), &str> = HashMap::new();
        ret.insert(("python", "make_entry"), "Entry");
        let index = ReturnTypeIndex::build(&ret);
        for (value_ref, want) in [(true, "entry"), (false, "Entry")] {
            let defs = vec![mk_def("run", "def run()", 0, 100)];
            let refs = vec![
                RefRecord {
                    name: "make_entry".into(),
                    site_byte: 10,
                    value_ref,
                    ..Default::default()
                },
                RefRecord {
                    name: "save".into(),
                    receiver_hint: "entry".into(),
                    site_byte: 20,
                    ..Default::default()
                },
            ];
            let mut facts = mk_facts(defs, refs);
            facts.language = "python".into();
            propagate_types_with_returns(&mut facts, &index);
            assert_eq!(facts.references[1].receiver_hint, want, "{value_ref}");
        }
    }

    #[test]
    fn param_type_rewrites_receiver() {
        let defs = vec![
            mk_def("Foo::run", "fn run(&self)", 0, 100),
            mk_def("main", "fn main(svc: Service)", 100, 200),
        ];
        let refs = vec![RefRecord {
            name: "run".into(),
            receiver_hint: "svc".into(),
            site_line: 5,
            site_byte: 150,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Service");
    }

    #[test]
    fn cpp_pointer_and_reference_params_use_the_nominal_type() {
        let defs = vec![mk_def(
            "shout",
            "void shout(const StreamOutput *stream, Module* module, ConfigValue &result)",
            0,
            120,
        )];
        let refs = vec![
            RefRecord {
                name: "printf".into(),
                receiver_hint: "stream".into(),
                site_line: 2,
                site_byte: 40,
                ..Default::default()
            },
            RefRecord {
                name: "add_module".into(),
                receiver_hint: "module".into(),
                site_line: 3,
                site_byte: 60,
                ..Default::default()
            },
            RefRecord {
                name: "as_bool".into(),
                receiver_hint: "result".into(),
                site_line: 4,
                site_byte: 80,
                ..Default::default()
            },
        ];
        let mut facts = mk_facts(defs, refs);
        facts.language = "cpp".into();
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "StreamOutput");
        assert_eq!(facts.references[1].receiver_hint, "Module");
        assert_eq!(facts.references[2].receiver_hint, "ConfigValue");
    }

    #[test]
    fn cpp_field_chain_rewrites_to_the_field_type() {
        let defs = vec![mk_def("Kernel::add_module", "void add_module(int)", 0, 100)];
        let refs = vec![
            RefRecord {
                name: "printf".into(),
                receiver_hint: "kernel->streams".into(),
                site_line: 2,
                site_byte: 40,
                ..Default::default()
            },
            RefRecord {
                name: "printf".into(),
                receiver_hint: "this->streams".into(),
                site_line: 3,
                site_byte: 20,
                ..Default::default()
            },
        ];
        let mut facts = mk_facts(defs, refs);
        facts.language = "cpp".into();
        facts.local_types.push(cgg_core::LocalType {
            var_name: "kernel".into(),
            type_name: "Kernel".into(),
            scope_byte: 0,
        });
        let mut fields: HashMap<String, HashMap<String, String>> = HashMap::new();
        fields
            .entry("Kernel".into())
            .or_default()
            .insert("streams".into(), "StreamOutput".into());
        propagate_types_with_fields(
            &mut facts,
            &ReturnTypeIndex::default(),
            &fields,
            &HashMap::new(),
            &HashMap::new(),
        );
        assert_eq!(facts.references[0].receiver_hint, "StreamOutput");
        assert_eq!(facts.references[1].receiver_hint, "StreamOutput");
    }

    #[test]
    fn locals_of_the_same_name_are_scoped_to_the_enclosing_function() {
        let defs = vec![
            mk_def("a", "void a()", 0, 50),
            mk_def("b", "void b()", 50, 100),
        ];
        let refs = vec![
            RefRecord {
                name: "printf".into(),
                receiver_hint: "stream".into(),
                site_line: 2,
                site_byte: 20,
                ..Default::default()
            },
            RefRecord {
                name: "add_module".into(),
                receiver_hint: "stream".into(),
                site_line: 6,
                site_byte: 70,
                ..Default::default()
            },
        ];
        let mut facts = mk_facts(defs, refs);
        facts.language = "cpp".into();
        facts.local_types.push(cgg_core::LocalType {
            var_name: "stream".into(),
            type_name: "StreamOutput".into(),
            scope_byte: 10,
        });
        facts.local_types.push(cgg_core::LocalType {
            var_name: "stream".into(),
            type_name: "Kernel".into(),
            scope_byte: 60,
        });
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "StreamOutput");
        assert_eq!(facts.references[1].receiver_hint, "Kernel");
    }

    /// The corpus-wide regression the scoping caused when it ran for every
    /// language: PHP alone lost 8,142 typed-receiver edges. Outside C++ the
    /// file-wide, last-write-wins lookup stays exactly as it was.
    #[test]
    fn locals_outside_cpp_keep_the_file_wide_lookup() {
        let defs = vec![
            mk_def("a", "void a()", 0, 50),
            mk_def("b", "void b()", 50, 100),
        ];
        let refs = vec![
            RefRecord {
                name: "printf".into(),
                receiver_hint: "stream".into(),
                site_line: 2,
                site_byte: 20,
                ..Default::default()
            },
            RefRecord {
                name: "add_module".into(),
                receiver_hint: "stream".into(),
                site_line: 6,
                site_byte: 70,
                ..Default::default()
            },
        ];
        let mut facts = mk_facts(defs, refs);
        // Rust is scoped too since its `scope_byte` is the `let` itself
        // (`rust_locals_resolve_to_the_let_in_scope`); this guards the rest.
        facts.language = "java".into();
        facts.local_types.push(cgg_core::LocalType {
            var_name: "stream".into(),
            type_name: "StreamOutput".into(),
            scope_byte: 10,
        });
        facts.local_types.push(cgg_core::LocalType {
            var_name: "stream".into(),
            type_name: "Kernel".into(),
            scope_byte: 60,
        });
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Kernel");
        assert_eq!(facts.references[1].receiver_hint, "Kernel");
    }

    #[test]
    fn cpp_object_like_macros_type_as_their_expansion() {
        let defs = vec![mk_def("init", "void init()", 0, 200)];
        let refs = vec![
            RefRecord {
                name: "add_module".into(),
                receiver_hint: "THEKERNEL".into(),
                site_line: 2,
                site_byte: 40,
                ..Default::default()
            },
            RefRecord {
                name: "printf".into(),
                receiver_hint: "THEKERNEL->streams".into(),
                site_line: 3,
                site_byte: 60,
                ..Default::default()
            },
            RefRecord {
                name: "on_idle".into(),
                receiver_hint: "THEROBOT".into(),
                site_line: 4,
                site_byte: 80,
                ..Default::default()
            },
        ];
        let mut facts = mk_facts(defs, refs);
        facts.language = "cpp".into();
        let mut fields: HashMap<String, HashMap<String, String>> = HashMap::new();
        fields
            .entry("Kernel".into())
            .or_default()
            .insert("instance".into(), "Kernel".into());
        fields
            .entry("Kernel".into())
            .or_default()
            .insert("streams".into(), "StreamOutput".into());
        fields
            .entry("Kernel".into())
            .or_default()
            .insert("robot".into(), "Robot".into());
        let mut replacements = HashMap::new();
        replacements.insert("THEKERNEL".into(), "Kernel::instance".into());
        replacements.insert("THEROBOT".into(), "THEKERNEL->robot".into());
        let macros = resolve_macro_types(&replacements, &fields);
        assert_eq!(macros.get("THEKERNEL").map(String::as_str), Some("Kernel"));
        assert_eq!(macros.get("THEROBOT").map(String::as_str), Some("Robot"));
        propagate_types_with_fields(
            &mut facts,
            &ReturnTypeIndex::default(),
            &fields,
            &macros,
            &HashMap::new(),
        );
        assert_eq!(facts.references[0].receiver_hint, "Kernel");
        assert_eq!(facts.references[1].receiver_hint, "StreamOutput");
        assert_eq!(facts.references[2].receiver_hint, "Robot");
    }

    #[test]
    fn java_style_type_first_param() {
        let defs = vec![
            mk_def("Helper.add", "public int add(int a)", 0, 50),
            mk_def("Main.run", "public void run(Helper h)", 50, 150),
        ];
        let refs = vec![RefRecord {
            name: "add".into(),
            receiver_hint: "h".into(),
            site_line: 3,
            site_byte: 100,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Helper");
    }

    /// `(name, type)` pairs `collect_param_types` reads off `sig` in `lang`.
    fn lang_params(lang: &str, sig: &str) -> Vec<(String, String)> {
        let d = mk_def("f", sig, 0, 10);
        collect_param_types(&d, lang)
            .into_iter()
            .map(|(_, n, t)| (n, t))
            .collect()
    }

    /// FINDINGS C3: an optional, nullable or forward-referenced annotation
    /// types the parameter as the class it names; a union of two real
    /// types leaves it untyped (never its first member).
    #[test]
    fn optional_and_nullable_parameters_are_their_type() {
        assert_eq!(
            lang_params(
                "python",
                "def f(a: Optional[Foo], b: Foo | None, c: \"Foo\", d: Union[Foo, None], \
                 e: typing.Optional[Foo] = None, g: Foo = None, h: Foo | Bar, i: Union[Foo, Bar])"
            ),
            pairs(&[
                ("a", "Foo"),
                ("b", "Foo"),
                ("c", "Foo"),
                ("d", "Foo"),
                ("e", "Foo"),
                ("g", "Foo"),
            ])
        );
        assert_eq!(
            lang_params(
                "typescript",
                "function f(a: Foo | undefined, b?: Foo, c: null | Foo, d: Foo | Bar, e: Foo = new Foo())"
            ),
            pairs(&[("a", "Foo"), ("b", "Foo"), ("c", "Foo"), ("e", "Foo")])
        );
        assert_eq!(
            lang_params("kotlin", "fun f(a: Foo?, b: List<Foo>?)"),
            pairs(&[("a", "Foo"), ("b", "List")])
        );
        assert_eq!(
            lang_params("swift", "func f(a: Foo?, b: Foo!, with c: Optional<Foo>)"),
            pairs(&[("a", "Foo"), ("b", "Foo"), ("c", "Foo")])
        );
        assert_eq!(
            lang_params("scala", "def f(a: Foo | Null, b: Foo | Bar)"),
            pairs(&[("a", "Foo")])
        );
        // Near miss: a container is the container, not its element.
        assert_eq!(
            lang_params("python", "def f(a: list[Foo], b: Optional[list[Foo]])"),
            pairs(&[("a", "list"), ("b", "list")])
        );
    }

    #[test]
    fn nullable_type_first_parameters_are_their_type() {
        assert_eq!(
            lang_params(
                "csharp",
                "void F(Foo? a, Nullable<Pt> b, ref Foo c, this Foo d, Foo e = null)"
            ),
            pairs(&[
                ("a", "Foo"),
                ("b", "Pt"),
                ("c", "Foo"),
                ("d", "Foo"),
                ("e", "Foo")
            ])
        );
        assert_eq!(
            lang_params(
                "php",
                "function f(?Foo $a, Foo|null $b, null|Foo $c, Foo|Bar $d, private ?Foo $e)"
            ),
            pairs(&[("$a", "Foo"), ("$b", "Foo"), ("$c", "Foo"), ("$e", "Foo")])
        );
        assert_eq!(
            lang_params("dart", "void f(Foo? a, {required Foo b, Foo? c})"),
            pairs(&[("a", "Foo"), ("b", "Foo"), ("c", "Foo")])
        );
        assert_eq!(
            lang_params(
                "java",
                "void f(@Nullable Foo a, final @NonNull Bar b, int c)"
            ),
            pairs(&[("a", "Foo"), ("b", "Bar")])
        );
    }

    #[test]
    fn optional_return_types_are_their_type() {
        assert_eq!(
            extract_return_type("python", "def f() -> Optional[Foo]"),
            Some("Foo")
        );
        assert_eq!(
            extract_return_type("python", "def f() -> \"Foo\""),
            Some("Foo")
        );
        assert_eq!(
            extract_return_type("python", "def f() -> Foo | None"),
            Some("Foo")
        );
        assert_eq!(
            extract_return_type("swift", "func f() -> Foo?"),
            Some("Foo")
        );
        assert_eq!(
            extract_return_type("kotlin", "fun f(): Foo? = null"),
            Some("Foo")
        );
        assert_eq!(
            extract_return_type("typescript", "function f(): Foo | undefined"),
            Some("Foo")
        );
        assert_eq!(
            extract_return_type("php", "function f(): ?Foo"),
            Some("Foo")
        );
        // Near misses: a union of two real types is not its first member;
        // a container is the container; Rust keeps its wrapper reading.
        assert_eq!(
            extract_return_type("typescript", "function f(): Foo | Bar"),
            None
        );
        assert_eq!(
            extract_return_type("python", "def f() -> List[Foo]"),
            Some("List")
        );
        assert_eq!(extract_return_type("python", "def f() -> None"), None);
        assert_eq!(
            extract_return_type("rust", "fn f() -> Result<Config, Error>"),
            Some("Result")
        );
    }

    #[test]
    fn a_typed_closure_parameter_types_its_receiver_only_in_the_closure() {
        let defs = vec![mk_def("Main.run", "void run(Bar x)", 0, 200)];
        let refs = vec![
            RefRecord {
                name: "go".into(),
                receiver_hint: "x".into(),
                site_byte: 50,
                ..Default::default()
            },
            RefRecord {
                name: "go".into(),
                receiver_hint: "x".into(),
                site_byte: 150,
                ..Default::default()
            },
        ];
        let mut facts = mk_facts(defs, refs);
        facts.language = "dart".into();
        facts.scoped_types.push(cgg_core::ScopedType {
            name: "x".into(),
            type_name: "Foo".into(),
            start_byte: 40,
            end_byte: 80,
        });
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Foo");
        // Outside the closure, the method's own parameter.
        assert_eq!(facts.references[1].receiver_hint, "Bar");
    }

    fn closure_file(
        lang: &str,
        defs: Vec<DefRecord>,
        callee: &str,
        n: usize,
    ) -> FileFacts {
        let mut f = mk_facts(defs, Vec::new());
        f.language = lang.into();
        f.closure_args.push(cgg_core::ClosureArg {
            callee: callee.into(),
            arg_index: 0,
            params: (0..n).map(|i| format!("p{i}")).collect(),
            start_byte: 1000,
            end_byte: 1100,
        });
        f
    }

    fn scoped_of(f: &FileFacts) -> Vec<(String, String)> {
        f.scoped_types
            .iter()
            .map(|s| (s.name.clone(), s.type_name.clone()))
            .collect()
    }

    #[test]
    fn an_untyped_closure_takes_the_callees_declared_callback_types() {
        let mut all = vec![
            closure_file(
                "scala",
                vec![mk_def(
                    "withTempDir",
                    "def withTempDir(f: File => Unit): Unit",
                    0,
                    50,
                )],
                "withTempDir",
                1,
            ),
            closure_file(
                "dart",
                vec![mk_def(
                    "pair",
                    "void pair(void Function(Foo a, Bar b) cb)",
                    0,
                    50,
                )],
                "pair",
                2,
            ),
            closure_file(
                "csharp",
                vec![mk_def("Map", "R Map<R>(Func<Foo, R> f)", 0, 50)],
                "Map",
                1,
            ),
            // Near misses: a type parameter, a top type, an arity that does
            // not match, and two overloads that disagree.
            closure_file(
                "scala",
                vec![mk_def("each", "def each[T](f: T => Unit): Unit", 0, 50)],
                "each",
                1,
            ),
            closure_file(
                "scala",
                vec![mk_def("anyOf", "def anyOf(f: Any => Unit): Unit", 0, 50)],
                "anyOf",
                1,
            ),
            closure_file(
                "dart",
                vec![mk_def("one", "void one(void Function(Foo) cb)", 0, 50)],
                "one",
                2,
            ),
            closure_file(
                "csharp",
                vec![
                    mk_def("Run", "void Run(Action<Foo> f)", 0, 50),
                    mk_def("Run", "void Run(Action<Bar> f)", 60, 90),
                ],
                "Run",
                1,
            ),
        ];
        settle_closure_args(&mut all);
        assert_eq!(scoped_of(&all[0]), pairs(&[("p0", "File")]));
        assert_eq!(scoped_of(&all[1]), pairs(&[("p0", "Foo"), ("p1", "Bar")]));
        assert_eq!(scoped_of(&all[2]), pairs(&[("p0", "Foo")]));
        for f in &all[3..] {
            assert_eq!(
                scoped_of(f),
                Vec::<(String, String)>::new(),
                "{:?}",
                f.closure_args
            );
        }
    }

    /// `(name, type)` pairs `collect_param_types` reads off `sig`.
    fn params(sig: &str, cpp: bool, rust: bool) -> Vec<(String, String)> {
        let d = mk_def("f", sig, 0, 10);
        let lang = match (cpp, rust) {
            (true, _) => "cpp",
            (_, true) => "rust",
            _ => "java",
        };
        collect_param_types(&d, lang)
            .into_iter()
            .map(|(_, n, t)| (n, t))
            .collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(n, t)| ((*n).to_string(), (*t).to_string()))
            .collect()
    }

    #[test]
    fn a_generic_parameter_is_its_generic_type_not_an_argument() {
        // FINDINGS A2: the list was split on every comma, so the piece
        // after the comma inside `<…>` read as a parameter of the
        // argument's type — `h` typed `Handler`, `r` typed `Integer`.
        // Java / C# (type-first).
        assert_eq!(
            params(
                "void lookup(Map<String, Handler> h, Helper x)",
                false,
                false
            ),
            pairs(&[("h", "Map"), ("x", "Helper")])
        );
        assert_eq!(
            params("void g(Repo<String, Integer> r)", false, false),
            pairs(&[("r", "Repo")])
        );
        assert_eq!(
            params(
                "void g(final Box<Map<String, Integer>> b, Crate c)",
                false,
                false
            ),
            pairs(&[("b", "Box"), ("c", "Crate")])
        );
        assert_eq!(
            params(
                "void Lookup(Dictionary<string, Handler> handlers)",
                false,
                false
            ),
            pairs(&[("handlers", "Dictionary")])
        );
        // `name: Type` languages already cut at `<`; the phantom second
        // piece is gone, and a function type's own parameters (`b: B`)
        // are not this function's.
        assert_eq!(
            params(
                "function f(handlers: Map<string, Handler>, cb: (a: A, b: B) => void, x: Foo)",
                false,
                false
            ),
            pairs(&[("handlers", "Map"), ("cb", "(a: A"), ("x", "Foo")])
        );
        // A comma or bracket inside a string default splits nothing.
        assert_eq!(
            params(
                "def f(self, m: Dict[str, Foo], sep=\",<\", y: Bar)",
                false,
                false
            ),
            pairs(&[("m", "Dict"), ("y", "Bar")])
        );
    }

    #[test]
    fn a_cpp_template_parameter_is_never_typed_as_its_argument() {
        // `std::map<std::string, Handler>` used to type `handlers` as
        // `Handler>` (the piece after the comma). A class template is its
        // own type; what `->` reaches through it is a separate `#deref`
        // entry, read only for `p->f()`.
        assert_eq!(
            params("void f(QMap<Key, Handler> handlers, Widget w)", true, false),
            pairs(&[
                ("handlers#deref", "Key"),
                ("handlers", "QMap"),
                ("w", "Widget")
            ])
        );
        assert_eq!(
            params("void f(vector<Widget> ws)", true, false),
            pairs(&[("ws#deref", "Widget")])
        );
        assert_eq!(
            params(
                "void f(unique_ptr<Handler> h, const sk_sp<Kernel>& k, QSharedPointer<Kernel> q)",
                true,
                false
            ),
            pairs(&[
                ("h#deref", "Handler"),
                ("k#deref", "Kernel"),
                ("q#deref", "Kernel"),
                ("q", "QSharedPointer"),
            ])
        );
        // A raw pointer to a template: `->` reaches the template itself.
        assert_eq!(
            params("void f(QMap<Key, Handler>* m)", true, false),
            pairs(&[("m", "QMap")])
        );
        // The scope path is read outside the argument list.
        assert_eq!(nominal_type_token("Foo<Bar>::Inner&"), Some("Inner"));
        assert_eq!(nominal_type_token("std::map<std::string, Handler>"), None);
        assert_eq!(nominal_type_token("std::shared_ptr<const Robot>"), None);
        assert_eq!(nominal_type_token("const Robot"), Some("Robot"));
    }

    #[test]
    fn a_cpp_arrow_call_reaches_the_pointee_and_a_dot_call_the_template() {
        let defs = vec![mk_def(
            "run",
            "void run(sk_sp<DisplayList> dl, QSharedPointer<Kernel> k)",
            0,
            200,
        )];
        let call = |name: &str, recv: &str, byte: u32, arrow: bool| RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            site_line: 2,
            site_byte: byte,
            arrow,
            ..Default::default()
        };
        let refs = vec![
            call("Dispatch", "dl", 40, true),
            call("reset", "dl", 60, false),
            call("start", "k", 80, true),
            call("data", "k", 100, false),
        ];
        let mut facts = mk_facts(defs, refs);
        facts.language = "cpp".into();
        propagate_types(&mut facts);
        let hints: Vec<&str> = facts
            .references
            .iter()
            .map(|r| r.receiver_hint.as_str())
            .collect();
        // `dl.reset()` is the smart pointer's own method: left untyped,
        // never read as `DisplayList::reset`.
        assert_eq!(hints, vec!["DisplayList", "dl", "Kernel", "QSharedPointer"]);
    }

    #[test]
    fn rust_parameters_split_only_between_parameters() {
        // A wrapper chain whose innermost type has two arguments used to
        // be cut inside it, leaving `Arc` (not even unwrapped).
        assert_eq!(
            params(
                "fn f(s: Arc<RwLock<HashMap<String, Session>>>, cb: impl Fn(A, B) -> C, h: &Helper)",
                false,
                true
            ),
            pairs(&[
                ("s#guard", "HashMap"),
                ("s", "HashMap"),
                ("cb", "Fn(A"),
                ("h", "Helper"),
            ])
        );
        // Lifetimes are not quotes.
        assert_eq!(
            params("fn f<'a>(x: &'a Foo<'a, T>, y: Bar)", false, true),
            pairs(&[("x", "Foo"), ("y", "Bar")])
        );
    }

    #[test]
    fn plain_parameter_lists_are_unchanged() {
        // Near miss: no brackets, nothing to respect.
        assert_eq!(
            params("public void run(Service x, final Helper y)", false, false),
            pairs(&[("x", "Service"), ("y", "Helper")])
        );
        assert_eq!(
            params("fun run(x: Service, y: Helper)", false, false),
            pairs(&[("x", "Service"), ("y", "Helper")])
        );
        assert_eq!(
            params(
                "void shout(const StreamOutput *stream, Module* module)",
                true,
                false
            ),
            pairs(&[("stream", "StreamOutput"), ("module", "Module")])
        );
    }

    #[test]
    fn the_parameter_list_is_found_behind_annotations() {
        // An annotation's argument list used to open the "parameter list".
        assert_eq!(
            param_list(
                "@SuppressWarnings(\"x\") private <T> Object f(Object a, AttributeEntry<T> e)"
            ),
            Some("Object a, AttributeEntry<T> e")
        );
        assert_eq!(
            params(
                "@SuppressWarnings(\"deprecation\") private <T> Object f(Object a, AttributeEntry<T> entry)",
                false,
                false
            ),
            pairs(&[("a", "Object"), ("entry", "AttributeEntry")])
        );
        assert_eq!(
            param_list("@get:JvmName(\"y\") @Deprecated(\"(z\") fun g(x: Foo)"),
            Some("x: Foo")
        );
        assert_eq!(
            param_list("[HttpGet(\"{id}\")] public IActionResult Get(int id)"),
            Some("int id")
        );
        assert_eq!(
            param_list("fn f<'a>(x: &'a Foo) -> impl Fn(A)"),
            Some("x: &'a Foo")
        );
        assert_eq!(
            param_list("pub(crate) async fn f(h: &Helper, c: &Data<Ctx>)"),
            Some("h: &Helper, c: &Data<Ctx>")
        );
        assert_eq!(param_list("pub(in crate::a) fn g(x: X)"), Some("x: X"));
        assert_eq!(
            param_list("__attribute__((noinline)) static void h(Foo* f)"),
            Some("Foo* f")
        );
        assert_eq!(
            param_list("def f(self, a=(1, 2), b: Foo = None)"),
            Some("self, a=(1, 2), b: Foo = None")
        );
        // Near miss: no parameter list at all.
        assert_eq!(param_list("class Foo"), None);
    }

    #[test]
    fn dart_named_and_optional_parameter_groups_hold_parameters() {
        assert_eq!(
            params(
                "Foo make(Bar b, {FileSystem fs, Logger? log, Terminal Function(Platform)? t})",
                false,
                false
            ),
            pairs(&[
                ("b", "Bar"),
                ("fs", "FileSystem"),
                ("log", "Logger"),
                ("t", "Terminal")
            ])
        );
        assert_eq!(
            params("void f([Baz z])", false, false),
            pairs(&[("z", "Baz")])
        );
        // Near miss: a destructuring pattern with a type is one parameter.
        assert_eq!(
            params("fn f([a, b]: [u8; 2], c: Cfg)", false, true),
            pairs(&[("c", "Cfg")])
        );
    }

    #[test]
    fn accepts_arity_reads_defaults_varargs_and_packs() {
        let f = |s: &str, n| accepts_arity(s, n);
        assert_eq!(
            f("public ISE(String formatText, Object... arguments)", 1),
            Some(true)
        );
        // C# `params` takes any count; a default makes a parameter optional.
        assert_eq!(f("public JArray(params object[] content)", 3), Some(true));
        assert_eq!(f("public P(int a, string b = \"x\")", 1), Some(true));
        assert_eq!(f("public P(int a, string b = \"x\")", 3), Some(false));
        assert_eq!(
            f(
                "public ISE(Throwable t, String formatText, Object... arguments)",
                1
            ),
            Some(false)
        );
        assert_eq!(
            f("public CalculatedField(CalculatedField other)", 0),
            Some(false)
        );
        assert_eq!(f("public CalculatedField()", 0), Some(true));
        assert_eq!(
            f(
                "Result status(int status, File content, boolean inline, FileMimeTypes m)",
                2
            ),
            Some(false)
        );
        assert_eq!(
            f("def toSQLValue(v: Int, radix: Int = 10): String", 1),
            Some(true)
        );
        assert_eq!(f("def apply(xs: Int*): Seq[Int]", 4), Some(true));
        assert_eq!(f("def run(implicit ec: Ctx): Unit", 0), Some(true));
        assert_eq!(
            f(
                "int to_buffer(char* first, char* last, T v, fmt f, int p)",
                3
            ),
            Some(false)
        );
        assert_eq!(
            f(
                "template <typename... AV> std::string cat(AV const&... args)",
                7
            ),
            Some(true)
        );
        assert_eq!(f("int f(void)", 0), Some(true));
        assert_eq!(
            f("void g(Map<String, Integer> m, @Ann(x = 1) int y)", 2),
            Some(true)
        );
        assert_eq!(f("def size: Int", 0), None);
    }

    #[test]
    fn split_top_level_respects_brackets_strings_and_arrows() {
        let s = |x: &str, rust: bool| -> Vec<String> {
            split_top_level(x, |c| c == ',', rust)
                .into_iter()
                .map(|p| p.trim().to_string())
                .collect()
        };
        assert_eq!(s("a: A<B, C>, d: D", false), vec!["a: A<B, C>", "d: D"]);
        assert_eq!(s("a: A<B<C, D>>, e", false), vec!["a: A<B<C, D>>", "e"]);
        assert_eq!(s("f: (A, B) -> C, g", true), vec!["f: (A, B) -> C", "g"]);
        assert_eq!(s("f: (a, b) => c, g", false), vec!["f: (a, b) => c", "g"]);
        assert_eq!(s("x = \"a,b\", y", false), vec!["x = \"a,b\"", "y"]);
        assert_eq!(s("x = ',', y", false), vec!["x = ','", "y"]);
        assert_eq!(s("x = a << 2, y", false), vec!["x = a << 2", "y"]);
        assert_eq!(s("x = a > b, y", false), vec!["x = a > b", "y"]);
        assert_eq!(s("x: &'a A, y: &'b B", true), vec!["x: &'a A", "y: &'b B"]);
        assert_eq!(s("x' y, z", false), vec!["x' y", "z"]);
    }

    #[test]
    fn lowercase_variable_matches_type() {
        let defs = vec![
            mk_def("Service.run", "fn run(&self)", 0, 50),
            mk_def("main", "fn main()", 50, 150),
        ];
        let refs = vec![RefRecord {
            name: "run".into(),
            receiver_hint: "service".into(),
            site_line: 3,
            site_byte: 100,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Service");
    }

    #[test]
    fn uppercase_receiver_not_rewritten() {
        let defs = vec![mk_def("Foo.bar", "fn bar()", 0, 50)];
        let refs = vec![RefRecord {
            name: "bar".into(),
            receiver_hint: "Foo".into(),
            site_line: 1,
            site_byte: 10,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        // Already uppercase — should not be touched
        assert_eq!(facts.references[0].receiver_hint, "Foo");
    }

    #[test]
    fn self_not_rewritten() {
        let defs = vec![mk_def("Foo.bar", "fn bar(&self)", 0, 50)];
        let refs = vec![RefRecord {
            name: "baz".into(),
            receiver_hint: "self".into(),
            site_line: 1,
            site_byte: 10,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "self");
    }

    // F10, annotation side: `parse_colon_param` cut at the first `<`, so
    // `Arc<Mutex<dyn Reporter>>` typed the parameter as `Arc`.
    #[test]
    fn rust_params_see_through_wrapper_chains() {
        let sig = "async fn drive(reporter: Arc<Mutex<dyn Reporter + Send>>, \
                   p: &mut dyn Printer, s: Arc<rules::Scheduler>, n: Box<u32>)";
        let defs = vec![mk_def("app::drive", sig, 0, 400)];
        let r = |name: &str, hint: &str, byte| RefRecord {
            name: name.into(),
            receiver_hint: hint.into(),
            site_line: 5,
            site_byte: byte,
            ..Default::default()
        };
        let refs = vec![
            r("update", "reporter", 300),
            r("print", "p", 310),
            r("spawn", "s", 320),
            r("pow", "n", 330),
        ];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        let hints: Vec<&str> = facts
            .references
            .iter()
            .map(|r| r.receiver_hint.as_str())
            .collect();
        assert_eq!(hints, ["Reporter", "Printer", "rules::Scheduler", "n"]);
    }

    // A lock guard derefs to what the lock holds: `reporter.lock()` on
    // `reporter: Arc<Mutex<dyn Reporter>>` is a `Reporter`. A guard taken
    // from something not declared through a lock is left alone.
    #[test]
    fn rust_lock_guards_type_as_their_contents() {
        let sig = "async fn drive(reporter: Arc<Mutex<dyn Reporter>>, file: File)";
        let defs = vec![mk_def("app::drive", sig, 0, 400)];
        let r = |hint: &str, byte| RefRecord {
            name: "update".into(),
            receiver_hint: hint.into(),
            site_line: 5,
            site_byte: byte,
            ..Default::default()
        };
        let refs = vec![
            r("reporter.lock()", 300),
            r("reporter.lock().unwrap()", 310),
            r("file.read()", 320),
        ];
        let mut facts = mk_facts(defs, refs);
        propagate_types(&mut facts);
        let hints: Vec<&str> = facts
            .references
            .iter()
            .map(|r| r.receiver_hint.as_str())
            .collect();
        assert_eq!(hints, ["Reporter", "Reporter", "file.read()"]);
    }

    // Rust locals are scoped: the latest visible `let` wins, not the last
    // one written anywhere in the file.
    #[test]
    fn rust_locals_resolve_to_the_let_in_scope() {
        let defs = vec![
            mk_def("m::a", "fn a()", 0, 100),
            mk_def("m::b", "fn b()", 100, 200),
        ];
        let r = |byte| RefRecord {
            name: "get".into(),
            receiver_hint: "map".into(),
            site_line: 5,
            site_byte: byte,
            ..Default::default()
        };
        let mut facts = mk_facts(defs, vec![r(50), r(150)]);
        for (ty, at) in [("HeaderMap", 10), ("http::HeaderMap", 110)] {
            facts.local_types.push(LocalType {
                var_name: "map".into(),
                type_name: ty.into(),
                scope_byte: at,
            });
        }
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "HeaderMap");
        assert_eq!(facts.references[1].receiver_hint, "http::HeaderMap");
    }

    #[test]
    fn self_field_rewrites_to_the_declared_type() {
        let defs = vec![mk_def("App.connect", "def connect(self):", 0, 100)];
        let refs = vec![RefRecord {
            name: "open".into(),
            receiver_hint: "self.controller".into(),
            site_line: 5,
            site_byte: 50,
            ..Default::default()
        }];
        let mut facts = mk_facts(defs, refs);
        facts.local_types.push(LocalType {
            var_name: "self.controller".into(),
            type_name: "Controller".into(),
            scope_byte: 0,
        });
        propagate_types(&mut facts);
        assert_eq!(facts.references[0].receiver_hint, "Controller");
    }

    /// WS14b: inside `b.apply { … }` a call the type of `b` declares stays
    /// on `b`; one it does not (an enclosing class's member) becomes the
    /// bare call it was written as. A lambda in an object expression sees
    /// the enclosing function's parameter types.
    #[test]
    fn implicit_receivers_settle_by_the_receivers_members() {
        let mut box_file = mk_facts(
            vec![DefRecord {
                simple_name: "open".into(),
                qualified_name: "ex.Box.open".into(),
                ..Default::default()
            }],
            vec![],
        );
        box_file.language = "kotlin".into();
        let implicit = |name: &str, at: u32| RefRecord {
            name: name.into(),
            receiver_hint: "b".into(),
            site_byte: at,
            implicit_receiver: true,
            ..Default::default()
        };
        let mut user = mk_facts(
            vec![
                mk_def("ex.go", "fun go(b: Box)", 0, 100),
                mk_def("ex.go.inner", "override fun inner()", 10, 90),
            ],
            vec![implicit("open", 20), implicit("member", 30)],
        );
        user.language = "kotlin".into();
        propagate_types_with_returns(&mut user, &ReturnTypeIndex::default());
        let mut all = vec![box_file, user];
        settle_implicit_receivers(&mut all);
        let got: Vec<(&str, &str, bool)> = all[1]
            .references
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    r.receiver_hint.as_str(),
                    r.implicit_receiver,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![("open", "Box", true), ("member", "", false)],
            "{got:?}"
        );
    }

    /// WS16 (j): `if let Some(m) = meter` with `meter: Option<&ChangeMeter>`
    /// types `m` as `ChangeMeter` — and so hides another function's `let m:
    /// Row` that the file-wide fallback would otherwise apply. A source with
    /// no `Option` type leaves the binding untyped (dropped).
    #[test]
    fn unwrapped_pattern_bindings_take_the_options_inner_type() {
        let defs = vec![
            mk_def(
                "k::upsert",
                "fn upsert(meter: Option<&ChangeMeter>, n: u32)",
                0,
                100,
            ),
            mk_def("k::rows", "fn rows()", 100, 200),
            mk_def("k::plain", "fn plain(meter: u32)", 200, 300),
        ];
        let call = |byte: u32| RefRecord {
            name: "record_only".into(),
            receiver_hint: "m".into(),
            site_line: 1,
            site_byte: byte,
            ..Default::default()
        };
        let mut facts = mk_facts(defs, vec![call(50), call(250)]);
        facts.language = "rust".into();
        let marker = format!("{}Some:meter", cgg_core::ARG_VAR_PREFIX);
        facts.local_types = vec![
            LocalType {
                var_name: "m".into(),
                type_name: marker.clone(),
                scope_byte: 20,
            },
            LocalType {
                var_name: "m".into(),
                type_name: "Row".into(),
                scope_byte: 120,
            },
            LocalType {
                var_name: "m".into(),
                type_name: marker,
                scope_byte: 220,
            },
        ];
        propagate_types(&mut facts);
        let got: Vec<&str> = facts
            .references
            .iter()
            .map(|r| r.receiver_hint.as_str())
            .collect();
        assert_eq!(got[0], "ChangeMeter");
        assert!(
            facts
                .local_types
                .iter()
                .all(|lt| !lt.type_name.starts_with(cgg_core::ARG_VAR_PREFIX))
        );
        assert_eq!(facts.local_types.len(), 2, "{:?}", facts.local_types);
    }

    /// WS16 (h): a call's variable argument is typed where the call is,
    /// from the declared parameter or the visible `let`; an untyped one
    /// leaves the argument's type unknown.
    #[test]
    fn variable_arguments_are_typed_at_the_call() {
        let defs = vec![mk_def(
            "k::f",
            "fn f(id: String, name: &'a str, d: Doc)",
            0,
            100,
        )];
        let arg = |v: &str, byte: u32| RefRecord {
            name: "from".into(),
            receiver_hint: "DocId".into(),
            path_receiver: true,
            site_line: 1,
            site_byte: byte,
            arg_type: Some(format!("{}{v}", cgg_core::ARG_VAR_PREFIX)),
            ..Default::default()
        };
        let mut facts = mk_facts(
            defs,
            vec![
                arg("id", 10),
                arg("name", 20),
                arg("owned", 40),
                arg("other", 50),
            ],
        );
        facts.language = "rust".into();
        facts.local_types = vec![LocalType {
            var_name: "owned".into(),
            type_name: "String".into(),
            scope_byte: 30,
        }];
        propagate_types(&mut facts);
        let got: Vec<Option<&str>> = facts
            .references
            .iter()
            .map(|r| r.arg_type.as_deref())
            .collect();
        assert_eq!(
            got,
            vec![Some("String"), Some("&str"), Some("String"), None]
        );
    }
}
