// Pipeline helpers thread run state explicitly rather than through a
// context struct, which keeps each stage's inputs visible at the call
// site. The arity is the point, not an accident.
#![allow(clippy::too_many_arguments)]
//! Cross-file scope-aware resolver.
//!
//! This is a lightweight companion to the stack-graphs resolver. It
//! walks each file's declared imports and, for every call-site whose
//! simple name matches an imported symbol (or an imported module's
//! member), emits a cross-file `CallEdge` with `confidence=Medium`
//! and `resolver="cross-file:imports"`.
//!
//! It is deliberately conservative: it emits an edge only when there's
//! an unambiguous imported target. Ambiguous cases are left alone
//! (either the stack-graphs resolver or the intra-file linker will
//! have already made a decision about them).
//!
//! Rules, per file:
//!
//! * Python — `from m import foo` + call `foo(...)` → edge to
//!   `m.foo`. `import m as mod` + call `mod.bar(...)` → edge to
//!   `m.bar`. Matching on the file whose qualified-name chain
//!   begins with `m`.
//! * JS / TS — `import { foo } from "./m.js"` and ESM aliases likewise.
//!
//! For languages where the extractor produces well-formed imports
//! (Task 4 does this for Python and Rust) the resolver is effective.
//! For languages where Task 4 only stubbed extraction, this pass is a
//! no-op.

use std::collections::HashMap;

use rayon::prelude::*;

use cgg_core::{
    FileFacts,
    audit::{AuditUnresolvedCall, UnresolvedReason},
    graph::{CallEdge, CallableKind, Confidence, Graph, Via},
    ids::{CallableId, FileId, ResolverId},
};

use crate::names::owner_from_qn;

/// Std/core types whose bare name MUST NOT be captured by a local trait
/// impl in `by_owner_method` (c13, `ab2/SPECS.md`). `impl From<Foo> for
/// String` is indexed under the qualified-name owner `String` by
/// [`owner_from_qn`] exactly like an inherent method would be, so without
/// this list a local trait impl for a std type answers every bare
/// `String::from(..)` call in the tree. Verified false edge:
/// `String::from("root")` at
/// `/home/dev/Documents/Github/llmitm-v5/vmm/src/builder.rs:1192`
/// resolving to `clippy_tracing::<String as From<StripVisitor>>::from`.
const STD_CORE_TYPES: &[&str] = &[
    "String", "str", "Vec", "Option", "Result", "Box", "Arc", "Rc", "Cow", "HashMap",
    "HashSet", "BTreeMap", "BTreeSet", "VecDeque", "PathBuf", "Path", "OsString", "i8",
    "i16", "i32", "i64", "i128", "u8", "u16", "u32", "u64", "u128", "isize", "usize",
    "f32", "f64", "bool", "char", "()",
];

/// For `…::<T as Trait>::m`, the trait's bare name (`Trait`); `None` for
/// any other shape.
fn trait_of_impl_qn(qn: &str) -> Option<String> {
    let (prefix, _) = crate::names::split_last_segment(qn)?;
    let (_, owner) = crate::names::split_last_segment(prefix)?;
    let owner = owner.trim();
    if !(owner.starts_with('<') && owner.contains(" as ")) {
        return None;
    }
    let after = owner.split(" as ").nth(1)?;
    let bare = after.trim_end_matches('>').trim();
    let bare = bare.split('<').next().unwrap_or(bare);
    Some(bare.rsplit("::").next().unwrap_or(bare).to_string())
}

/// Base recorded by the Rust plugin for a type whose `Deref::Target` is
/// its own type parameter. Mirrors `cgg-lang`'s `rust_type_links`.
const DEREF_TO_ARG: &str = "#deref-arg";

/// Per-language index behind [`ScopedSimple`]: every definition by simple
/// name, as `(position of its file in the language's file list, id)`, in
/// file-list then declaration order.
struct ScopedIndex<'a> {
    files: &'a [(FileId, String)],
    by_name: HashMap<&'a str, Vec<(u32, CallableId)>>,
}

impl<'a> ScopedIndex<'a> {
    fn build(
        files: &'a [(FileId, String)],
        def_cids: &HashMap<FileId, Vec<(&'a str, CallableId)>>,
    ) -> Self {
        let mut by_name: HashMap<&'a str, Vec<(u32, CallableId)>> = HashMap::new();
        for (i, (fid, _)) in files.iter().enumerate() {
            for (simple, cid) in def_cids.get(fid).map(Vec::as_slice).unwrap_or(&[]) {
                by_name.entry(simple).or_default().push((i as u32, *cid));
            }
        }
        Self { files, by_name }
    }
}

/// The callables defined in files whose path contains one of a file's
/// import path fragments (`Text.Pandoc` → `*/text/pandoc*`), looked up by
/// simple name (step 1c).
///
/// Evaluated per name on demand. Building the whole map up front read
/// every file of the language against every fragment, for every file —
/// O(files² × fragments): 70 s of a 90 s `--jobs 1` run on a
/// 56k-callable PHP repo, 20 s of CPU on a 9k-file Go tree — while a file
/// queries only the names it calls. The answer for a name is the same
/// list, in the same order, the eager map held.
struct ScopedSimple<'a> {
    index: Option<&'a ScopedIndex<'a>>,
    fragments: Vec<String>,
    /// Whether the file at a position in the language's file list
    /// matches a fragment, memoized.
    matched: std::cell::RefCell<HashMap<u32, bool>>,
}

impl<'a> ScopedSimple<'a> {
    fn empty() -> Self {
        Self {
            index: None,
            fragments: Vec::new(),
            matched: Default::default(),
        }
    }

    fn new(index: &'a ScopedIndex<'a>, fragments: Vec<String>) -> Self {
        Self {
            index: Some(index),
            fragments,
            matched: Default::default(),
        }
    }

    /// The candidates named `name`, at most `cap` of them: `None` when
    /// there are none or more than `cap` (the caller treats both as no
    /// scoped answer, so counting stops at `cap + 1`).
    fn get_capped(&self, name: &str, cap: usize) -> Option<Vec<CallableId>> {
        let index = self.index?;
        if self.fragments.is_empty() {
            return None;
        }
        let defs = index.by_name.get(name)?;
        let mut matched = self.matched.borrow_mut();
        let mut out = Vec::new();
        for (pos, cid) in defs {
            let hit = *matched.entry(*pos).or_insert_with(|| {
                let path = index.files[*pos as usize].1.as_str();
                self.fragments.iter().any(|f| path.contains(f.as_str()))
            });
            if hit {
                if out.len() == cap {
                    return None;
                }
                out.push(*cid);
            }
        }
        (!out.is_empty()).then_some(out)
    }
}

/// The `T` of a wasm-bindgen extern method `fn m(this: &T, …)`.
fn wasm_bindgen_method_owner(c: &cgg_core::graph::CallableNode) -> Option<String> {
    if !c
        .attributes
        .iter()
        .any(|a| a.contains("wasm_bindgen") && a.contains("method"))
    {
        return None;
    }
    let params = c.signature_hint.split_once('(')?.1;
    let first = params.split([',', ')']).next()?.trim();
    let ty = first
        .strip_prefix("this")?
        .trim_start()
        .strip_prefix(':')?
        .trim();
    let ty = ty.trim_start_matches('&').trim_start_matches("mut ").trim();
    let ty = ty.split('<').next()?.trim();
    let ty = ty.rsplit("::").next()?;
    ty.starts_with(char::is_uppercase).then(|| ty.to_string())
}

/// `--guess-unique-names`: the one callable named `r.name`, when binding
/// to it is a defensible guess. Every refusal mirrors a rule the default
/// resolver already keeps:
///
/// * a standard-library name (`iter`, `join`, `map_err`) is almost never
///   the workspace's one same-named method;
/// * `self`/`Self` and a typed receiver (uppercase — a type whose own
///   lookup already failed) never degrade to name matching;
/// * a path whose head is an external crate is a call into that crate —
///   judged on crate-root membership, so a hint that carries no `::` is
///   no loophole;
/// * a test-module helper is no production caller's target, and the
///   candidate must be reachable from the caller at all.
#[allow(clippy::too_many_arguments)]
fn unique_name_guess(
    graph: &Graph,
    r: &cgg_core::RefRecord,
    caller_file: FileId,
    caller_qn: Option<&str>,
    by_simple: &HashMap<(String, String), Vec<CallableId>>,
    rust_path_heads: &RustPathVocab,
    workspace_owner_names: &std::collections::HashSet<String>,
    direct_imports: &HashMap<String, Vec<String>>,
    module_aliases: &HashMap<String, String>,
) -> Option<CallableId> {
    if r.receiver_hint == cgg_core::VALUE_REF_HINT
        || r.receiver_hint == cgg_core::STRING_REF_HINT
    {
        return None;
    }
    let [cid] = by_simple
        .get(&("rust".to_string(), r.name.clone()))?
        .as_slice()
    else {
        return None;
    };
    if cgg_core::stdlib::stdlib_names("rust")
        .is_some_and(|std| std.contains(r.name.as_str()))
    {
        return None;
    }
    let rh = r.receiver_hint.as_str();
    if matches!(rh, "self" | "Self") || rh.starts_with(char::is_uppercase) {
        return None;
    }
    let head = rh
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or("");
    let path_like = r.path_receiver || rh.starts_with(&format!("{head}::"));
    if !head.is_empty()
        && path_like
        && is_external_rust_head(
            head,
            rust_path_heads,
            workspace_owner_names,
            direct_imports,
            module_aliases,
            caller_qn,
        )
    {
        return None;
    }
    let in_tests = |qn: &str| {
        qn.split("::")
            .any(|seg| seg == "tests" || seg.ends_with("_tests"))
    };
    let target = graph.callables.get(cid)?;
    if in_tests(&target.qualified_name) && !caller_qn.is_some_and(in_tests) {
        return None;
    }
    rust_candidate_reachable(graph, *cid, caller_file, caller_qn, direct_imports)
        .then_some(*cid)
}

/// F6: the one in-graph declaration `T::name` when every candidate is an
/// implementation of workspace trait `T` (or that declaration itself).
///
/// The trait is read from `trait_impl_target` or, failing that, the
/// `<X as T>` wrapper in the qualified name. Candidates in test modules
/// (`tests`, `store_tests`) are left out of the vote — a stub impl there
/// says nothing about production dispatch — and synthetic closures never
/// vote. Any inherent method (no wrapper, not the declaration) makes the
/// set heterogeneous: an inherent `in_flight` and a trait's `in_flight`
/// share the name and the owner index, and are different calls. `None`
/// unless exactly one declaration exists, so a std trait (no declaration
/// node) keeps the cap.
fn homogeneous_trait_declaration(
    graph: &Graph,
    cids: &[CallableId],
    name: &str,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
) -> Option<CallableId> {
    let is_test_module = |qn: &str| {
        crate::names::split_last_segment(qn).is_some_and(|(prefix, _)| {
            prefix
                .split("::")
                .any(|seg| seg == "tests" || seg.ends_with("_tests"))
        })
    };
    let mut trait_name: Option<String> = None;
    let mut impls = 0usize;
    for c in cids {
        let node = graph.callables.get(c)?;
        let qn = node.qualified_name.as_str();
        if qn.contains("closure_at_") || qn.contains("async_at_") || is_test_module(qn) {
            continue;
        }
        let t = node
            .trait_impl_target
            .clone()
            .or_else(|| trait_of_impl_qn(qn));
        let t = match t {
            Some(t) => {
                impls += 1;
                t
            }
            // Not an impl: only the trait's own declaration may stand here.
            None => owner_from_qn(qn)?.to_string(),
        };
        match &trait_name {
            None => trait_name = Some(t),
            Some(prev) if *prev == t => {}
            Some(_) => return None,
        }
    }
    let t = trait_name?;
    if impls == 0 {
        return None;
    }
    let decls: Vec<CallableId> = by_owner_method
        .get(&("rust".to_string(), t, name.to_string()))?
        .iter()
        .copied()
        .filter(|c| {
            graph.callables.get(c).is_some_and(|n| {
                !n.qualified_name.contains(" as ") && n.trait_impl_target.is_none()
            })
        })
        .collect();
    match decls.as_slice() {
        [d] => Some(*d),
        _ => None,
    }
}

/// True when `qn` is `…::Trait::m` with no impl wrapper and `Trait` is
/// one of `traits` — a declaration some sibling candidate implements.
fn is_trait_declaration_of(qn: &str, traits: &[String]) -> bool {
    if qn.contains(" as ") {
        return false;
    }
    let Some((prefix, _)) = crate::names::split_last_segment(qn) else {
        return false;
    };
    let owner = prefix.rsplit("::").next().unwrap_or(prefix);
    traits.iter().any(|t| t == owner)
}

/// True when `qn` is a Rust trait-impl callable (`<Owner as Trait>::method`)
/// whose owner is a std/core type. Such a callable MUST NOT be indexed
/// under the bare owner name in `by_owner_method`; it stays reachable
/// through the exact qualified-name lookup (`by_qn`) via an explicit
/// `<String as From<X>>::from(..)` receiver.
fn is_std_type_local_trait_impl(lang: &str, qn: &str) -> bool {
    if lang != "rust" {
        return false;
    }
    let Some((prefix, _simple)) = crate::names::split_last_segment(qn) else {
        return false;
    };
    let Some((_, owner_raw)) = crate::names::split_last_segment(prefix) else {
        return false;
    };
    let owner_raw = owner_raw.trim();
    if !(owner_raw.starts_with('<') && owner_raw.contains(" as ")) {
        return false;
    }
    STD_CORE_TYPES.contains(&crate::names::normalize_owner(owner_raw))
}

/// Output of the cross-file resolver.
#[derive(Debug, Default)]
pub struct CrossFileOutput {
    pub edges: Vec<CallEdge>,
    /// Sites this pass saw but could not turn into an edge, and that no
    /// earlier pass recorded either. Currently only module-scope value
    /// references — see the `VALUE_REF_HINT` arm in [`resolve`].
    pub unresolved: Vec<AuditUnresolvedCall>,
}

/// What a language calls the method a constructor call lands on.
///
/// `Widget(3)` names a class; the callable it enters is that class's
/// initializer. cgg has no node for a type, so without this mapping
/// "who constructs X?" is unanswerable — in the field report, 107
/// constructors had zero inbound edges out of 1206.
fn constructor_names(lang: &str) -> &'static [&'static str] {
    match lang {
        "python" => &["__init__"],
        "javascript" | "typescript" => &["constructor"],
        "php" => &["__construct"],
        "ruby" => &["initialize"],
        _ => &[],
    }
}

/// The method an instance-call `x(...)` enters when `x` is an object.
fn call_operator_names(lang: &str) -> &'static [&'static str] {
    match lang {
        "python" => &["__call__"],
        "php" => &["__invoke"],
        "ruby" => &["call"],
        _ => &[],
    }
}

/// Walk a type's declared bases looking for one that owns `method`.
///
/// Python resolves an inherited call through the MRO; cgg matched only
/// the instantiated class, so `w.apply()` on a subclass that inherits
/// `apply` produced no edge while `w.extra()` declared on the subclass
/// did. Bounded and visited-guarded: a base list read from syntax can be
/// cyclic, and depth is not evidence.
/// Store the bare type name: the owner index is keyed that way, and a
/// base is written as `generic.ObjectListView` or `Handler<T>` as often
/// as plainly.
fn push_bare_base(slot: &mut Vec<String>, b: &str) {
    let bare = b.split(['<', '[']).next().unwrap_or(b).trim();
    let bare = bare.rsplit(['.', ':', '\\']).next().unwrap_or(bare);
    if !bare.is_empty() && !slot.iter().any(|x| x == bare) {
        slot.push(bare.to_string());
    }
}

/// The classes that name each class as a base, per language: the reverse
/// of `bases_by_owner`, keyed the same way (bare names), each list sorted.
/// Only the languages whose bare calls `member_of_calling_class` binds.
fn derived_index(
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
) -> HashMap<(String, String), Vec<String>> {
    let mut out: HashMap<(String, String), Vec<String>> = HashMap::new();
    for ((lang, owner), bases) in bases_by_owner {
        if !matches!(lang.as_str(), "java" | "csharp") {
            continue;
        }
        for b in bases {
            out.entry((lang.clone(), b.clone()))
                .or_default()
                .push(owner.clone());
        }
    }
    for v in out.values_mut() {
        v.sort();
        v.dedup();
    }
    out
}

/// The overrides of `method` below `owners`: the same-named methods the
/// classes deriving from them (by the recorded bases, transitively,
/// depth 8) declare, in breadth-first order. A bare call reaching the
/// declaration in `owners` dispatches at run time to any of them.
fn overrides_below(
    lang: &str,
    owners: &[&str],
    method: &str,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    derived_by_base: &HashMap<(String, String), Vec<String>>,
) -> Vec<CallableId> {
    let mut out: Vec<CallableId> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = owners.iter().copied().collect();
    let mut frontier: Vec<&str> = owners.to_vec();
    for _ in 0..8 {
        let mut next: Vec<&str> = Vec::new();
        for t in frontier {
            let Some(derived) = derived_by_base.get(&(lang.to_string(), t.to_string()))
            else {
                continue;
            };
            for d in derived {
                if !seen.insert(d.as_str()) {
                    continue;
                }
                if let Some(v) = by_owner_method.get(&(
                    lang.to_string(),
                    d.clone(),
                    method.to_string(),
                )) {
                    for c in v {
                        if !out.contains(c) {
                            out.push(*c);
                        }
                    }
                }
                next.push(d.as_str());
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}

/// Copies of one class (the same qualified owner vendored into several
/// samples or modules), or same-named classes reached through a base
/// recorded by its bare name: the group the caller can see. A group is
/// one owner path in one directory — an overload set, or a C# partial
/// class split across files beside each other, stays together. The
/// group sharing the longest directory prefix with the caller wins; a
/// tie is ambiguous (`None`), never settled by indexing order.
fn nearest_class_copies(
    graph: &Graph,
    cids: &[CallableId],
    caller_file: FileId,
) -> Option<Vec<CallableId>> {
    let dir_of = |f: FileId| {
        graph
            .files
            .get(&f)
            .and_then(|r| r.path.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
    };
    let mut groups: Vec<((String, std::path::PathBuf), Vec<CallableId>)> = Vec::new();
    for c in cids {
        let Some(n) = graph.callables.get(c) else {
            continue;
        };
        let owner = crate::names::split_last_segment(&n.qualified_name)
            .map(|(o, _)| o.to_string())
            .unwrap_or_default();
        let key = (owner, dir_of(n.file));
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => {
                if !v.contains(c) {
                    v.push(*c);
                }
            }
            None => groups.push((key, vec![*c])),
        }
    }
    if groups.len() <= 1 {
        return groups.pop().map(|(_, v)| v);
    }
    let caller_dir = dir_of(caller_file);
    let shared = |d: &std::path::Path| {
        d.components()
            .zip(caller_dir.components())
            .take_while(|(a, b)| a == b)
            .count()
    };
    let best = groups.iter().map(|((_, d), _)| shared(d)).max()?;
    let mut winners = groups.into_iter().filter(|((_, d), _)| shared(d) == best);
    let first = winners.next()?;
    if winners.next().is_some() {
        return None;
    }
    Some(first.1)
}

/// Java/C#: a bare name is a member of the calling class or one it
/// inherits before any static import (JLS 15.12.1) — the whole overload
/// set, wherever in the class's files it is declared.
///
/// * The calling class is its qualified owner path, not its bare name:
///   a nested `X._Fields` reaches its own `_Fields.findByThriftId`, never
///   every generated class's `_Fields` (FINDINGS R17).
/// * Copies of the class (one qualified name in several samples) are
///   narrowed to the caller's copy (`nearest_class_copies`, R18).
/// * A member that a class derived from the calling class overrides is a
///   virtual call with several bodies: the member and those overrides, as
///   the bare-name fan-out bound them before (R20) — never the
///   declaration alone, which left each override looking uncalled. Over
///   `--fanout-cap` the call is refused and recorded as capped.
///
/// `None`: nothing in the class or its bases; `Some(None)`: refused.
#[allow(clippy::too_many_arguments)]
fn member_of_calling_class(
    graph: &Graph,
    lang: &str,
    name: &str,
    caller_qn: &str,
    caller_file: FileId,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
    derived_by_base: &HashMap<(String, String), Vec<String>>,
    fanout_cap: usize,
    capped: &mut u32,
) -> Option<Option<Vec<CallableId>>> {
    let (own_path, _) = crate::names::split_last_segment(caller_qn)?;
    let own = owner_from_qn(caller_qn)?;
    let owner_path_of = |c: &CallableId| {
        graph
            .callables
            .get(c)
            .and_then(|n| crate::names::split_last_segment(&n.qualified_name))
            .map(|(o, _)| o.to_string())
    };
    let direct: Vec<CallableId> = by_owner_method
        .get(&(lang.to_string(), own.to_string(), name.to_string()))
        .map(|v| {
            v.iter()
                .copied()
                .filter(|c| owner_path_of(c).as_deref() == Some(own_path))
                .collect()
        })
        .unwrap_or_default();
    let found = if direct.is_empty() {
        resolve_via_bases(lang, own, name, by_owner_method, bases_by_owner)?
    } else {
        direct
    };
    let Some(set) = nearest_class_copies(graph, &found, caller_file) else {
        return Some(None);
    };
    // A member a class derived from the CALLING class overrides is a
    // virtual call (`this` is the caller's class or below it — never a
    // sibling): the set is the member and those overrides, bounded by the
    // fan-out cap like any other several-bodied call; over the cap it is
    // refused and recorded.
    let overrides = overrides_below(lang, &[own], name, by_owner_method, derived_by_base);
    if overrides.is_empty() {
        return Some(Some(set));
    }
    let mut set = set;
    for c in overrides {
        if !set.contains(&c) {
            set.push(c);
        }
    }
    if set.len() > fanout_cap {
        *capped = set.len() as u32;
        return Some(None);
    }
    Some(Some(set))
}

fn resolve_via_bases(
    lang: &str,
    owner: &str,
    method: &str,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
) -> Option<Vec<CallableId>> {
    let _sp = cgg_core::profile::span("xfile::via-bases");
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut frontier: Vec<String> = vec![owner.to_string()];
    for _ in 0..8 {
        let mut next: Vec<String> = Vec::new();
        for t in std::mem::take(&mut frontier) {
            if !seen.insert(t.clone()) {
                continue;
            }
            if t != owner
                && let Some(cids) = by_owner_method.get(&(
                    lang.to_string(),
                    t.clone(),
                    method.to_string(),
                ))
                && !cids.is_empty()
            {
                return Some(cids.clone());
            }
            if let Some(bases) = bases_by_owner.get(&(lang.to_string(), t)) {
                next.extend(bases.iter().cloned());
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    None
}

/// Parameter names a signature accepts, and whether it takes `**kwargs`.
///
/// Parsed from `signature_hint`, which the extractors already record —
/// no new extraction, and no attempt to be a type checker. `None` means
/// "cannot tell", and every caller treats that as "accepts anything".
fn accepted_params(sig: &str) -> Option<(std::collections::HashSet<String>, bool)> {
    let open = sig.find('(')?;
    let rest = &sig[open + 1..];
    let mut depth = 0i32;
    let mut end = rest.len();
    for (i, c) in rest.char_indices() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' if depth == 0 => {
                end = i;
                break;
            }
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
    }
    let mut names = std::collections::HashSet::new();
    let mut star_star = false;
    let mut depth = 0i32;
    let mut cur = String::new();
    fn flush(
        cur: &mut String,
        names: &mut std::collections::HashSet<String>,
        star_star: &mut bool,
    ) {
        let t = cur.trim();
        if t.starts_with("**") {
            *star_star = true;
        } else {
            let name = t
                .trim_start_matches('*')
                .split([':', '='])
                .next()
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                names.insert(name.to_string());
            }
        }
        cur.clear();
    }
    for c in rest[..end].chars() {
        match c {
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth <= 0 => flush(&mut cur, &mut names, &mut star_star),
            _ => cur.push(c),
        }
    }
    flush(&mut cur, &mut names, &mut star_star);
    Some((names, star_star))
}

/// Whether `sig` could accept a call passing these keyword names.
///
/// Deliberately one-sided: it returns `false` only when a keyword is
/// provably not a parameter and the signature has no `**kwargs`. An
/// unparseable signature, or one cgg has no hint for, accepts anything —
/// narrowing fan-out must not become a way to lose real edges.
fn signature_accepts(sig: &str, kwargs: &[String]) -> bool {
    if kwargs.is_empty() || sig.is_empty() {
        return true;
    }
    let Some((params, star_star)) = accepted_params(sig) else {
        return true;
    };
    if star_star || params.is_empty() {
        return true;
    }
    kwargs.iter().all(|k| params.contains(k))
}

/// The default duck-typing fan-out cap.
///
/// When a method call's receiver type is unknown, cgg emits an edge to
/// every same-named method it can see. Past a handful that stops being
/// informative and starts being noise, so the set is dropped — but the
/// drop is recorded (`fanout-cap-exceeded`), never silent.
pub const DEFAULT_FANOUT_CAP: usize = 5;

/// Best-effort "owning module path" for a Rust file, used only to
/// resolve `self::`/`super::` in that file's own `use`/`pub use`
/// imports back to a workspace-qualified path.
///
/// The Rust plugin (`rust.rs`) does not put the module path on
/// `FileFacts` directly — only the crate root, as a synthetic
/// `"crate-root"` import. This recovers it from the file's own
/// definitions: every definition's `qualified_name` is
/// `crate_root::mod_a::mod_b::(Type::)leaf`, so the longest common
/// prefix across all of them, with each one's own leaf dropped, is the
/// module (and, for a file whose definitions all live in one `impl`
/// block, the type) that `self`/`super` are relative to. A file with no
/// definitions of its own (a re-export-only `lib.rs`) falls back to the
/// crate root, same as `self::x` meaning `crate::x` there.
///
/// **Known limitation, stated so it is not mistaken for a bug later.**
/// The single-`impl` case yields `module::Type` where the module alone
/// was wanted, so a `self::`-relative `use` in such a file rewrites to
/// a path that matches nothing. It fails *closed* — the import simply
/// does not resolve and no edge is invented — which is why it is not
/// fixed here: a fix (drop a trailing UpperCamelCase segment — reliable
/// in Rust because `non_camel_case_types` keeps types that shape and
/// `non_snake_case` keeps modules lowercase) changes the
/// resolved edge set, and that belongs in a change measured on its
/// own rather than folded into this one.
fn rust_own_module(facts: &FileFacts, crate_root: &str) -> String {
    let mut prefix: Option<Vec<&str>> = None;
    for d in &facts.definitions {
        let mut segs: Vec<&str> = d.qualified_name.split("::").collect();
        segs.pop(); // drop this definition's own leaf name
        prefix = Some(match prefix {
            None => segs,
            Some(p) => {
                let common = p
                    .iter()
                    .zip(segs.iter())
                    .take_while(|(a, b)| a == b)
                    .count();
                p[..common].to_vec()
            }
        });
    }
    match prefix {
        Some(p) if !p.is_empty() => p.join("::"),
        _ => crate_root.to_string(),
    }
}

/// Rewrite a Rust `use`/`pub use` path's leading `crate`, `self`, or
/// `super` segment into a workspace-qualified path — the form
/// definitions' `qualified_name` (and therefore `by_qn`/`reexports`)
/// are keyed on. The Rust plugin emits these sentinels verbatim
/// (`ImportRecord` is documented as "not yet interpreted"); resolution
/// is this crate's job. Any other leading segment (an external crate,
/// or a sibling module named literally) is left untouched.
fn rewrite_rust_use_path(path: &str, crate_root: &str, own_module: &str) -> String {
    if let Some(rest) = path.strip_prefix("crate::") {
        return format!("{crate_root}::{rest}");
    }
    if path == "crate" {
        return crate_root.to_string();
    }
    if let Some(rest) = path.strip_prefix("self::") {
        return if own_module.is_empty() {
            rest.to_string()
        } else {
            format!("{own_module}::{rest}")
        };
    }
    if path == "self" {
        return own_module.to_string();
    }
    if path == "super" || path.starts_with("super::") {
        // Each `super::` climbs one module level from `own_module`.
        let mut segments: Vec<&str> =
            own_module.split("::").filter(|s| !s.is_empty()).collect();
        let mut rest = path;
        loop {
            if let Some(stripped) = rest.strip_prefix("super::") {
                segments.pop();
                rest = stripped;
            } else if rest == "super" {
                segments.pop();
                rest = "";
                break;
            } else {
                break;
            }
        }
        let base = segments.join("::");
        return match (base.is_empty(), rest.is_empty()) {
            (true, true) => String::new(),
            (true, false) => rest.to_string(),
            (false, true) => base,
            (false, false) => format!("{base}::{rest}"),
        };
    }
    path.to_string()
}

/// Resolve call-site references across files using import tables.
pub fn resolve(
    graph: &Graph,
    facts: &[FileFacts],
    fanout_cap: usize,
    dynamic_dispatch: bool,
    guess_unique_names: bool,
) -> CrossFileOutput {
    // Edges already emitted by `intra_file`, keyed for O(1) lookup.
    //
    // The de-duplication test below used to scan every edge in the graph
    // per resolved reference. That is O(references x edges), which stayed
    // invisible while PHP resolved almost nothing and became ~4s of a
    // Laravel run the moment it started resolving properly.
    // Keyed by (src, dst, call-site byte offset).
    let existing_edges: std::collections::HashSet<(CallableId, CallableId, u32)> = graph
        .edges
        .iter()
        .map(|e| (e.src, e.dst, e.site_byte))
        .collect();
    // Keyed by (src, call-site byte offset, referenced NAME) — not just
    // dropping the dst, but requiring the name to match too.
    //
    // `existing_edges` above only suppresses a re-resolution that lands
    // on the *identical* dst. A duck-typed fan-out or an import-table
    // hit can resolve the same site to a DIFFERENT dst than the one
    // intra-file already bound it to — e.g. a local `mk` intra-file
    // binds the site, and cross-file's import table separately resolves
    // the same site to an imported same-named `mk` in another file.
    // Both checks then pass (the tuples differ) and the site ends up
    // with two outbound edges from one call. A site intra-file already
    // bound is settled: cross-file must not re-open it under a
    // different candidate for the SAME name. Measured: 299 of 1,976
    // medium edges landed on a site that already had an intra-file High
    // edge.
    //
    // The name is load-bearing, not decoration: tree-sitter gives a
    // chained call's inner and outer `call_expression` the SAME start
    // byte as the receiver they share — `classifier().classify(..)`
    // emits a ref for `classifier` and a ref for `classify` at one
    // `site_byte`. Keying on `(src, site_byte)` alone conflated them:
    // once intra-file bound the inner `classifier()`, the outer
    // `.classify(..)` at the identical byte was silently dropped before
    // cross-file ever got to resolve it. Measured on
    // discord_safety_bot: `classifier().classify(...)` and
    // `classifier().probe_grammar_door()` both lost their outer edge.
    let intra_bound_sites: std::collections::HashSet<(CallableId, u32, String)> = graph
        .edges
        .iter()
        .filter_map(|e| {
            graph
                .callables
                .get(&e.dst)
                .map(|c| (e.src, e.site_byte, c.simple_name.clone()))
        })
        .collect();
    let mut out = CrossFileOutput::default();
    let _sp_idx = cgg_core::profile::span("xfile::index-build");
    let resolver_id = ResolverId::new("cross-file:imports");

    // Index callables by (language, qualified_name) and (language, simple_name).
    // Also build a (language, owner_type, method) index (Issue 2) so a
    // method call on a receiver of known type resolves with an O(1)
    // lookup instead of scanning every qualified name.
    let mut by_qn: HashMap<(String, String), CallableId> = HashMap::new();
    let mut by_simple: HashMap<(String, String), Vec<CallableId>> = HashMap::new();
    let mut by_owner_method: HashMap<(String, String, String), Vec<CallableId>> =
        HashMap::new();
    // Keyed by the FULL (unreduced) owner path instead of its bare last
    // segment — `kvm_bindings::CpuId`, not `CpuId`. Used only when a
    // receiver's head names a crate outside the workspace: a bare-name
    // match there is a coincidence (`kvm_ioctls::Kvm::new()` must not
    // bind to an unrelated local `Kvm::new` just because both owners
    // reduce to "Kvm"), so an external-headed receiver is matched
    // against this full-path index instead of `by_owner_method`.
    let mut by_full_owner_method: HashMap<(String, String, String), Vec<CallableId>> =
        HashMap::new();
    // Trait impls for an instantiated or std type, keyed by the type's
    // BASE name: `<Option<&User> as Helper>::id` under `Option`, and the
    // std-type impls `by_owner_method` refuses (see `STD_CORE_TYPES`). A
    // *path* call (`String::from(..)`) must never reach them, but a
    // *value* typed as the base (`self.user.id()`, `user: Option<&User>`)
    // calling a method the type has no inherent version of can only mean
    // such an extension trait — step 4 consults this index for exactly
    // that case, after every exact owner lookup has missed.
    let mut std_owner_trait_impls: HashMap<(String, String, String), Vec<CallableId>> =
        HashMap::new();
    // Types the workspace itself declares, by bare name, from their
    // inherent methods (`Mutex::new` in a crate that defines its own
    // `Mutex`). A local trait impl for such a type is the type's own
    // method and must stay indexed under the bare owner; the std-type
    // guard below is only for impls written for the REAL std type.
    let workspace_inherent_owners: std::collections::HashSet<(String, String)> = graph
        .callables
        .values()
        .filter(|c| {
            crate::names::split_last_segment(&c.qualified_name)
                .is_some_and(|(prefix, _)| !prefix.contains(" as "))
        })
        .filter_map(|c| {
            owner_from_qn(&c.qualified_name).map(|o| (c.language.clone(), o.to_string()))
        })
        .collect();
    // Kotlin extension functions (`fun Foo.ext()`), by the file that
    // declares them, and as a set. See step 0b of `try_resolve_ref`.
    let mut kotlin_ext_by_file: HashMap<(FileId, String), Vec<CallableId>> =
        HashMap::new();
    let mut kotlin_ext_ids: std::collections::HashSet<CallableId> =
        std::collections::HashSet::new();
    // `(language, qualified name)` keys more than one callable carries.
    // `by_qn` keeps only the last of them, so a lookup of such a key by a
    // *bare* name (see step 2) would be a choice by indexing order; those
    // lookups consult this set and refuse instead.
    let mut qn_collisions: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for c in graph.callables.values() {
        let key = (c.language.clone(), c.qualified_name.clone());
        if let Some(prev) = by_qn.insert(key.clone(), c.id)
            && prev != c.id
        {
            qn_collisions.insert(key);
        }
        // Callables a macro expansion generated are reachable by path and
        // by owner, never by name alone: a template stamped out once per
        // newtype would otherwise multiply every same-named candidate set
        // (14 generated `from`s pushing a 5-candidate set over the cap).
        let macro_generated = c.attributes.iter().any(|a| a.starts_with("macro:"));
        if !macro_generated {
            by_simple
                .entry((c.language.clone(), c.simple_name.clone()))
                .or_default()
                .push(c.id);
        }
        if c.language == "kotlin"
            && is_kotlin_extension(&c.signature_hint, &c.simple_name)
        {
            kotlin_ext_by_file
                .entry((c.file, c.simple_name.clone()))
                .or_default()
                .push(c.id);
            kotlin_ext_ids.insert(c.id);
        }
        if let Some(owner) = owner_from_qn(&c.qualified_name) {
            let key = (c.language.clone(), owner.to_string(), c.simple_name.clone());
            let base = owner.split('<').next().unwrap_or(owner).trim();
            if base != owner
                || is_std_type_local_trait_impl(&c.language, &c.qualified_name)
            {
                std_owner_trait_impls
                    .entry((c.language.clone(), base.to_string(), c.simple_name.clone()))
                    .or_default()
                    .push(c.id);
            }
            if !is_std_type_local_trait_impl(&c.language, &c.qualified_name)
                || workspace_inherent_owners
                    .contains(&(c.language.clone(), owner.to_string()))
            {
                by_owner_method.entry(key).or_default().push(c.id);
            }
        }
        // A wasm-bindgen `#[wasm_bindgen(method)] fn m(this: &T, …)` in an
        // `extern` block is `T::m` to every caller (`storage.m()`), though
        // its qualified name is the module's.
        if c.language == "rust"
            && let Some(owner) = wasm_bindgen_method_owner(c)
        {
            by_owner_method
                .entry((c.language.clone(), owner, c.simple_name.clone()))
                .or_default()
                .push(c.id);
        }
        if let Some(owner) = crate::names::full_owner_from_qn(&c.qualified_name) {
            by_full_owner_method
                .entry((c.language.clone(), owner.to_string(), c.simple_name.clone()))
                .or_default()
                .push(c.id);
        }
    }

    // Crate-root first segments seen in Rust qualified names — the set
    // of crates cgg actually indexed for this workspace. A path-headed
    // receiver (`serde_json::de::from_str`, `toml::from_str`) whose head
    // is absent from this set (after import-alias substitution — see
    // `workspace_head` below) names a crate cgg never saw, so it cannot
    // be a local call.
    //
    // AMENDMENT 2 tried indexing every segment (not just the crate
    // root) to catch a receiver written relative to a nested module
    // (`arg_parser::Arguments`, real qn
    // `mycrate::utils::arg_parser::Arguments::single_value`). That was
    // WRONG and is reverted here: `qn.split("::")` does not understand
    // `<A as Trait<B>>` wrapper syntax, so splitting a trait-impl qn
    // whose wrapper embeds a qualified path — e.g.
    // `vmm::builder::<StartMicrovmError as
    // std::convert::From<linux_loader::cmdline::Error>>::from` — inserted
    // `std`, `linux_loader`, `cmdline` and `Error` into this set as if
    // they were real local crates, which is exactly how
    // `std::io::Error::from(..)` started binding to
    // `StartMicrovmError`'s `From` impl. Only the FIRST segment of a qn
    // is safe to take this way, because a plugin always emits the
    // file's own real crate root first, before any embedded generic
    // text. The nested-module case is instead handled by resolving the
    // receiver's head through the file's own `use` imports first
    // (`workspace_head`).
    //
    // Module paths are the other half of the vocabulary: every proper
    // prefix of a Rust qualified name, cut before the first segment
    // carrying `<` so a trait-impl wrapper's embedded paths (the
    // AMENDMENT 2 trap above) never enter it. A one-segment head that
    // is not a crate root is still in-workspace when the caller's own
    // module (or an ancestor) declares a child module by that name —
    // `mod admin_jobs;` + `admin_jobs::CascadeRunner { .. }.spawn()` with
    // no `use` — which is how rustc resolves a relative path.
    let mut rust_path_heads = RustPathVocab::default();
    for (lang_key, qn) in by_qn.keys() {
        if lang_key != "rust" {
            continue;
        }
        if let Some(head) = qn.split("::").next() {
            rust_path_heads.crate_roots.insert(head.to_string());
        }
        // Every plain segment is a path prefix except the callable's own
        // name — which is only among them when no wrapper cut the list.
        let segs = rust_plain_segments(qn);
        let last = if segs.len() < qn.split("::").count() {
            segs.len()
        } else {
            segs.len().saturating_sub(1)
        };
        for i in 1..=last {
            rust_path_heads.module_paths.insert(segs[..i].join("::"));
        }
    }
    // A crate made only of re-exports (`salvo`'s `lib.rs`) contributes no
    // qualified name, but every one of its files records its crate root.
    for f in facts.iter().filter(|f| f.language == "rust") {
        for i in f.imports.iter().filter(|i| i.kind == "crate-root") {
            if i.path != "crate" {
                rust_path_heads.crate_roots.insert(i.path.clone());
            }
        }
    }

    // Type names the workspace declares, so a receiver that leads with a
    // TYPE (`TrustKind::Network`) is not mistaken for an external crate.
    let workspace_owner_names: std::collections::HashSet<String> = by_owner_method
        .keys()
        .filter(|(l, _, _)| l == "rust")
        .map(|(_, owner, _)| owner.clone())
        .collect();

    // Declarations rather than implementations: a `typing.Protocol`
    // member, an `@abstractmethod`, or a bodiless interface/abstract
    // member a plugin tags `abstract` (TypeScript, Swift, Scala, Dart). They carry no body worth entering,
    // so counting them as call targets inflates "how many things does
    // this reach" — three implementations where two exist. Dropped from
    // duck-typed fan-out only when a concrete candidate survives, so a
    // call whose *only* visible target is the declaration still resolves
    // rather than vanishing.
    let stub_ids: std::collections::HashSet<CallableId> = {
        let mut protocol_owners: std::collections::HashSet<(&str, &str)> =
            std::collections::HashSet::new();
        for f in facts {
            for d in &f.definitions {
                if let Some(owner) = owner_from_qn(&d.qualified_name)
                    && d.base_types.iter().any(|b| {
                        let bare = b.split(['<', '[']).next().unwrap_or(b).trim();
                        let bare = bare.rsplit(['.', ':']).next().unwrap_or(bare);
                        matches!(bare, "Protocol" | "ABC" | "ABCMeta")
                    })
                {
                    protocol_owners.insert((f.language.as_str(), owner));
                }
            }
        }
        graph
            .callables
            .values()
            .filter(|c| {
                c.attributes
                    .iter()
                    .any(|a| a.contains("abstractmethod") || a == "abstract")
                    || owner_from_qn(&c.qualified_name).is_some_and(|o| {
                        protocol_owners.contains(&(c.language.as_str(), o))
                    })
            })
            .map(|c| c.id)
            .collect()
    };

    // Signature text per callable, for narrowing duck-typed fan-out by
    // what a candidate can actually accept.
    // Borrowed, not cloned: the graph outlives this pass, and cloning
    // one string per callable cost ~4% on a 20k-callable tree for
    // nothing.
    let signatures: HashMap<CallableId, &str> = graph
        .callables
        .values()
        .filter(|c| c.signature_hint.contains('('))
        .map(|c| (c.id, c.signature_hint.as_str()))
        .collect();

    // Every type cgg has at least one method for. Distinguishes "this
    // class declares no initializer" from "cgg has never seen this name".
    let known_owners: std::collections::HashSet<(String, String)> = by_owner_method
        .keys()
        .map(|(l, o, _)| (l.clone(), o.clone()))
        .collect();

    // Owner type -> its declared bases, for walking the inheritance
    // chain when a method is inherited rather than declared. Seeded from
    // class records where the plugin writes them (Python), because a
    // bodiless `class Middle(Base): pass` has no method to carry its
    // bases and the chain would otherwise end at it; then from methods,
    // for plugins that record bases only there.
    let mut bases_by_owner: HashMap<(String, String), Vec<String>> = HashMap::new();
    // Java/C# class records carry each class's bases exactly; a method's
    // own `base_types` may name an anonymous class's base instead
    // (`new Base() { … }`), which is not a base of the enclosing class.
    let mut exact_classes: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    for f in facts {
        for c in &f.classes {
            let Some(owner) = c.class_qn.rsplit('.').next() else {
                continue;
            };
            if crate::dispatch::CLASS_RECORDS_ARE_EXACT.contains(&f.language.as_str()) {
                exact_classes.insert((f.language.clone(), owner.to_string()));
            }
            let slot = bases_by_owner
                .entry((f.language.clone(), owner.to_string()))
                .or_default();
            for b in &c.base_types {
                push_bare_base(slot, b);
            }
        }
    }
    // The bases a method's own record names when they are not its
    // class's: a Java anonymous class body. `super.m()` there names them.
    let mut own_bases: HashMap<CallableId, Vec<String>> = HashMap::new();
    let by_span: HashMap<(FileId, u32), CallableId> = if exact_classes.is_empty() {
        HashMap::new()
    } else {
        graph
            .callables
            .values()
            .map(|c| ((c.file, c.start_byte), c.id))
            .collect()
    };
    for f in facts {
        for d in &f.definitions {
            if d.base_types.is_empty() {
                continue;
            }
            let Some(owner) = owner_from_qn(&d.qualified_name) else {
                continue;
            };
            if exact_classes.contains(&(f.language.clone(), owner.to_string())) {
                let mut mine: Vec<String> = Vec::new();
                for b in &d.base_types {
                    push_bare_base(&mut mine, b);
                }
                let class = bases_by_owner.get(&(f.language.clone(), owner.to_string()));
                if class.is_none_or(|c| *c != mine)
                    && let Some(&id) = by_span.get(&(f.file, d.start_byte))
                {
                    own_bases.insert(id, mine);
                }
                continue;
            }
            // Keyed by the bare type: `impl Trait for Tuple<'_>` declares
            // `Tuple<'_>`'s bases, and a receiver names `Tuple`.
            let owner = owner.split('<').next().unwrap_or(owner).trim();
            let slot = bases_by_owner
                .entry((f.language.clone(), owner.to_string()))
                .or_default();
            for b in &d.base_types {
                push_bare_base(slot, b);
            }
        }
    }

    let derived_by_base = derived_index(&bases_by_owner);
    let mut zig_packages: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for f in graph.files.values().filter(|f| f.language == "zig") {
        if let Some(stem) = f.path.file_stem() {
            zig_packages.insert(stem.to_string_lossy().into_owned());
        }
        if let Some(dir) = f.path.parent() {
            for c in dir.components() {
                if let std::path::Component::Normal(s) = c {
                    zig_packages.insert(s.to_string_lossy().into_owned());
                }
            }
        }
    }
    let mut kotlin_extensions = KotlinExtensions {
        by_file: kotlin_ext_by_file,
        ids: kotlin_ext_ids,
        ..KotlinExtensions::default()
    };
    if facts.iter().any(|f| f.language == "kotlin") {
        for f in facts.iter().filter(|f| f.language == "kotlin") {
            kotlin_extensions.file_scope.insert(
                f.file,
                (
                    f.packages.first().cloned().unwrap_or_default(),
                    f.imports
                        .iter()
                        .map(|i| i.path.trim().to_string())
                        .collect(),
                ),
            );
        }
        for c in graph.callables.values() {
            if c.language != "kotlin" || c.kind == CallableKind::Constructor {
                continue;
            }
            let pkg = kotlin_extensions
                .file_scope
                .get(&c.file)
                .map(|(p, _)| p.as_str())
                .unwrap_or("");
            let top_level = if pkg.is_empty() {
                c.qualified_name == c.simple_name
            } else {
                c.qualified_name.len() == pkg.len() + 1 + c.simple_name.len()
                    && c.qualified_name.starts_with(pkg)
                    && c.qualified_name.ends_with(c.simple_name.as_str())
            };
            if !top_level {
                continue;
            }
            if kotlin_extensions.ids.contains(&c.id) {
                kotlin_extensions
                    .top_level_pkg
                    .insert(c.id, pkg.to_string());
            } else {
                kotlin_extensions.top_level_fns.insert(c.id);
            }
        }
    }

    // Languages whose plugin records every class declaration, bodiless
    // ones included (`FileFacts::classes`). Only there does an
    // inheritance chain that finds nothing prove the method is outside the
    // tree: elsewhere a class with no methods carries no bases, and the
    // chain stops at it although the method lives one level further up.
    let class_record_langs: std::collections::BTreeSet<String> = facts
        .iter()
        .filter(|f| !f.classes.is_empty())
        .map(|f| f.language.clone())
        .collect();

    // Build a re-export map (Rust only, for now). Every `pub use` in a
    // file that lives under an identifiable crate makes that symbol
    // appear under the re-exporting crate's namespace. Example:
    //   crate cgg_core/src/lib.rs contains `pub use audit::AuditEvent;`
    //   => `cgg_core::AuditEvent` resolves to whatever
    //      `cgg_core::audit::AuditEvent` resolves to.
    let mut reexports: HashMap<(String, String), String> = HashMap::new();
    for f in facts {
        if f.language != "rust" {
            continue;
        }
        // Prefer the explicit crate-root marker emitted by the Rust
        // plugin; fall back to the first definition's crate prefix
        // or the literal "crate" sentinel.
        let crate_root = f
            .imports
            .iter()
            .find(|i| i.kind == "crate-root")
            .map(|i| i.path.clone())
            .or_else(|| {
                f.definitions
                    .first()
                    .and_then(|d| d.qualified_name.split("::").next())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "crate".to_string());
        let own_module = rust_own_module(f, &crate_root);
        for imp in &f.imports {
            if imp.kind != "pub-use" {
                continue;
            }
            let target = rewrite_rust_use_path(imp.path.trim(), &crate_root, &own_module);
            let exported_name = if imp.alias.is_empty() {
                target.rsplit("::").next().unwrap_or(&target).to_string()
            } else {
                imp.alias.clone()
            };
            let alias_qn = format!("{crate_root}::{exported_name}");
            reexports.insert(("rust".to_string(), alias_qn), target);
        }
    }

    // `(file, start, end) -> callable`, built once. See
    // `enclosing_callable_id`.
    let mut callables_by_span: HashMap<(FileId, u32, u32), CallableId> = HashMap::new();
    for c in graph.callables.values() {
        callables_by_span
            .entry((c.file, c.start_byte, c.end_byte))
            .or_insert(c.id);
    }

    let facts_by_id: HashMap<FileId, &FileFacts> =
        facts.iter().map(|f| (f.file, f)).collect();

    // Paths grouped by language, built once. The scoped-by-simple-name
    // fallback only ever considers files of the *same* language.
    //
    // An inverted `(language, path fragment) -> files` index was tried
    // here and reverted: the match is `path.contains(fragment)`, which
    // permits a fragment to begin *mid-segment*, and no segment-aligned
    // index reproduces that. The corpus caught it as -9,331 nodes and
    // edges across 34 repos. Any future index has to be proven against
    // the full corpus before it replaces this scan.
    // Each file's definitions as `(simple name, id)`, resolved once. The
    // scoped scan below visits a matching file once per *referencing*
    // file, and resolving there allocated two strings per definition per
    // visit — O(files²) allocations on a large tree.
    //
    // Resolved within the definition's own file: the last callable THAT
    // file declares under the qualified name (clauses and overloads of one
    // function share one, as before). It used to go through `by_qn`, which
    // keeps the last callable per qualified name across the whole tree, so
    // a definition whose qualified name another file shares (every
    // same-named JavaScript function, a Zig `assert` copied into a test
    // case) resolved to that other file's callable — the scoped lookup
    // then bound the call into a file it had not matched.
    let mut last_in_file: HashMap<(FileId, &str), CallableId> = HashMap::new();
    // Every callable one file declares under one qualified name: an
    // overload set (`add(sb, n, long)`, `add(sb, n, String)`). An exact
    // lookup that lands on one of them reaches them all.
    let mut overloads: HashMap<(FileId, &str), Vec<CallableId>> = HashMap::new();
    for c in graph.callables.values() {
        last_in_file.insert((c.file, c.qualified_name.as_str()), c.id);
        overloads
            .entry((c.file, c.qualified_name.as_str()))
            .or_default()
            .push(c.id);
    }
    let def_cids: HashMap<FileId, Vec<(&str, CallableId)>> = facts
        .iter()
        .map(|f| {
            let ids = f
                .definitions
                .iter()
                .filter_map(|d| {
                    last_in_file
                        .get(&(f.file, d.qualified_name.as_str()))
                        .map(|cid| (d.simple_name.as_str(), *cid))
                })
                .collect();
            (f.file, ids)
        })
        .collect();
    let mut files_by_lang: HashMap<&str, Vec<(FileId, String)>> = HashMap::new();
    for f in facts {
        files_by_lang
            .entry(f.language.as_str())
            .or_default()
            .push((f.file, f.path.to_string_lossy().to_ascii_lowercase()));
    }
    let scoped_index: HashMap<&str, ScopedIndex> = files_by_lang
        .iter()
        .map(|(lang, files)| (*lang, ScopedIndex::build(files, &def_cids)))
        .collect();

    // Include resolution used to scan every file in the tree for every
    // `#include`, of every file — O(files x includes x files). `HashMap`
    // iteration is unordered, so even the exact-match short-circuit read
    // half the map on average, and a miss read all of it. On
    // terraform-provider-aws (12,825 files) that was 82% of the whole
    // run: 341s of 415s CPU inside the import-table build alone.
    //
    // Two indexes built once instead. Exact path is the common case and
    // is now O(1); the suffix case keeps its old meaning — lowest FileId
    // among the matches — by bucketing on the last path segment and
    // verifying the full suffix, which is a handful of candidates rather
    // than the corpus.
    let mut include_by_exact: HashMap<&std::path::Path, &FileFacts> = HashMap::new();
    let mut include_by_last: HashMap<&std::ffi::OsStr, Vec<&FileFacts>> = HashMap::new();
    for f in facts {
        include_by_exact.entry(f.path.as_path()).or_insert(f);
        if let Some(name) = f.path.file_name() {
            include_by_last.entry(name).or_default().push(f);
        }
    }
    // No sort here on purpose. The comment above promises "lowest FileId
    // among the matches"; `facts` arrives in walk order and the loop
    // above pushes in that order, so each bucket already holds it — the
    // old `sort_by_key(|f| f.file.as_u32())` was a no-op that only
    // looked load-bearing. It stops being a no-op the moment ids become
    // content hashes, at which point it actively scrambles the buckets,
    // and this walk is order-sensitive: it memoizes by remaining depth
    // and caps candidates per name, so which header expands first
    // decides what resolves and at what confidence. Re-sorting by path
    // would restore the order but pay a `PathBuf` comparison per probe
    // — measured at +30% on erlang-otp. Keeping insertion order is both
    // correct and free.

    // Per-file and independent: the body reads the shared indexes
    // (`by_qn`, `by_simple`, `by_owner_method`, `reexports`) and writes
    // only into its own output. Collecting in parallel and concatenating
    // in input order keeps the edge sequence identical to the serial
    // form, which the determinism test in crates/cgg/tests pins.
    // What the tree declares as its own modules / packages / namespaces,
    // so a call through an import of a dependency is never name-matched
    // onto a workspace callable (FINDINGS A1; see `heads`).
    let vocab = crate::heads::WorkspaceVocab::build(graph, facts);
    // Import → file, for calls written through a workspace module
    // (FINDINGS A4/A5; see `modules`).
    let modix = crate::modules::ModuleIndex::build(graph, facts, &vocab);
    // Library scope (Dart imports, exports, parts; WS14; see `libscope`).
    let libix = crate::libscope::LibIndex::build(graph, facts);
    let phpix = crate::libscope::PhpIndex::build(graph, facts);
    // A module a build script declares is the tree's, whatever its root
    // file is named (`@import("pkg")` → `src/main.zig`).
    zig_packages.extend(libix.zig_module_names().map(str::to_string));

    drop(_sp_idx);
    let _sp_loop = cgg_core::profile::span("xfile::parallel-loop");
    let per_file: Vec<CrossFileOutput> = facts
        .par_iter()
        .map(|facts| {
            let _sp_file = cgg_core::profile::span("xfile::per-file");
            let mut out = CrossFileOutput::default();
            let lang = facts.language.clone();
            // Same crate-root / own-module resolution the re-export map
            // uses above, computed once per file rather than per `use`
            // item — see `rewrite_rust_use_path`.
            let rust_crate_root = if lang == "rust" {
                facts
                    .imports
                    .iter()
                    .find(|i| i.kind == "crate-root")
                    .map(|i| i.path.clone())
            } else {
                None
            };
            let rust_module_for_use = rust_crate_root
                .as_deref()
                .map(|cr| rust_own_module(facts, cr));

            // Variable -> its inferred type, for resolving a call *on an
            // instance* (`agent("prompt")` → `Agent.__call__`). A name
            // bound to two different types in one file is dropped rather
            // than guessed at: the whole point of this lookup is that the
            // receiver is known.
            let mut var_types: HashMap<String, String> = HashMap::new();
            {
                let mut conflicted: std::collections::HashSet<&str> =
                    std::collections::HashSet::new();
                for lt in &facts.local_types {
                    // An untyped entry only ends a Go variable's scope; a
                    // `<local>` one only marks a shadowed package name.
                    if lt.type_name.is_empty()
                        || lt.type_name.starts_with('<')
                        || conflicted.contains(lt.var_name.as_str())
                    {
                        continue;
                    }
                    match var_types.get(&lt.var_name) {
                        Some(prev) if prev != &lt.type_name => {
                            conflicted.insert(lt.var_name.as_str());
                            var_types.remove(&lt.var_name);
                        }
                        Some(_) => {}
                        None => {
                            var_types.insert(lt.var_name.clone(), lt.type_name.clone());
                        }
                    }
                }
            }

            // Normalize imports into lookup tables:
            //   imported_simple_name -> candidate qualified_names.
            // Python: `from helpers import greet` -> map "greet" ->
            //   "helpers.greet".
            //   `import helpers as h` -> map "h" -> "helpers" (module prefix).
            // Rust: `use a::b::c;` -> map "c" -> "a::b::c".
            let mut direct_imports: HashMap<String, Vec<String>> = HashMap::new();
            // Scoped to this file: a header is expanded once per
            // translation unit, which is exactly C's own semantics
            // under include guards.
            let mut include_visited: HashMap<FileId, u8> = HashMap::new();
            let _sp_imports = cgg_core::profile::span("xfile::import-table");
            let mut module_aliases: HashMap<String, String> = HashMap::new();
            // Namespace prefixes that bring symbols into scope unqualified —
            // e.g. Haskell `import Data.Map`, OCaml `open Foo`, Elixir
            // `import Foo`, F# `open System`, PowerShell `using namespace`.
            // Resolution tries `<prefix>.<ref-name>` (and `<prefix>::<name>`
            // for ::-joined languages) for each prefix.
            let mut unqualified_prefixes: Vec<String> = Vec::new();
            // Scala: the packages / objects the file imports from, for the
            // scoped lookup's file match only (an import binds no name
            // for its prefix).
            let mut scala_import_dirs: Vec<String> = Vec::new();

            // A declaration this file made itself, merged into a body elsewhere,
            // is still visible here — whether or not the file includes anything.
            for (simple, qualified) in &facts.unified_decls {
                let slot = direct_imports.entry(simple.clone()).or_default();
                if !slot.contains(qualified) {
                    slot.push(qualified.clone());
                }
            }
            for imp in &facts.imports {
                match imp.kind.as_str() {
                    "from-import" => {
                        // Python: imp.path is module; imp.alias is the
                        // items list ("greet, compute" or "greet as g").
                        // JS/TS: imp.path is relative path; items are
                        // exported names from that file.
                        let module = imp.path.trim();
                        // `from . import x` / `from ..pkg import y` name a
                        // module relative to this file's package. Made
                        // absolute here, the exact lookups below can find
                        // `pkg.x` instead of missing on `..x` and falling
                        // to a match by name.
                        let py_abs = if lang == "python" && module.starts_with('.') {
                            modix.py_abs(facts.file, module)
                        } else {
                            None
                        };
                        let module = py_abs.as_deref().unwrap_or(module);
                        for item in imp.alias.split(',') {
                            // Defensive: a plugin that forwards a
                            // parenthesised list verbatim must not
                            // register `(a` and `b)` as names.
                            let item = item.trim().trim_matches(|c| c == '(' || c == ')');
                            let (src, alias) = match item.split_once(" as ") {
                                Some((s, a)) => (s.trim(), a.trim()),
                                None => (item.trim(), item.trim()),
                            };
                            if src.is_empty() {
                                continue;
                            }
                            // Java `import static a.b.C.*;`: every static
                            // member of `C` is in scope unqualified.
                            if lang == "java" && src == "*" {
                                unqualified_prefixes.push(module.to_string());
                                continue;
                            }
                            // Java `import static a.b.C.m;` arrives with the
                            // member already on the path.
                            let qn = if lang == "java"
                                && module.rsplit('.').next() == Some(src)
                            {
                                module.to_string()
                            } else {
                                format!("{module}.{src}")
                            };
                            direct_imports
                                .entry(alias.to_string())
                                .or_default()
                                .push(qn);
                            // Definitions in some languages carry no
                            // module prefix, so the bare name is tried
                            // too. Not for JS/TS: a bare-name lookup there
                            // returns whichever same-named function was
                            // indexed last; their imports are followed to
                            // the file instead (`modules`).
                            if (module.starts_with('.') || module.starts_with('/'))
                                && !matches!(lang.as_str(), "javascript" | "typescript")
                            {
                                direct_imports
                                    .entry(alias.to_string())
                                    .or_default()
                                    .push(src.to_string());
                            }
                        }
                    }
                    "import"
                        if matches!(
                            lang.as_str(),
                            "python"
                                | "go"
                                | "javascript"
                                | "typescript"
                                | "swift"
                                | "zig"
                                | "r"
                                | "perl"
                        ) =>
                    {
                        // Python: `import a.b.c`               (no alias)
                        //         `import a.b.c as d`          (aliased)
                        // Go:     `import "fmt"`               (no alias)
                        //         `import "net/http"`          (no alias)
                        //         `import al "other/lib"`      (aliased)
                        //
                        // The call we want to resolve looks like
                        // `<root>.name()` — `<root>` is the alias if
                        // supplied, else the binding name implied by the
                        // path. That binding name is:
                        //   * Go (path contains '/'): last segment.
                        //   * Python (dotted path):   first segment.
                        //   * bare identifier:        the path itself.
                        // The target "module root" we map to:
                        //   * Go with slashes: the last segment
                        //                      (package name by
                        //                      convention = last dir).
                        //   * Python or bare: the full path.
                        let path = imp.path.trim();
                        let has_slash = path.contains('/');
                        let (binding, target) = if let Some(stripped_alias) =
                            Some(imp.alias.trim()).filter(|a| !a.is_empty() && *a != "_")
                        {
                            // Aliased — user wrote the binding name.
                            let target = if has_slash {
                                path.rsplit('/').next().unwrap_or(path).to_string()
                            } else {
                                path.to_string()
                            };
                            (stripped_alias.to_string(), target)
                        } else if has_slash {
                            let last =
                                path.rsplit('/').next().unwrap_or(path).to_string();
                            (last.clone(), last)
                        } else if path.contains('.') {
                            // Python dotted — bind first segment, target is full.
                            let first =
                                path.split('.').next().unwrap_or(path).to_string();
                            (first, path.to_string())
                        } else {
                            (path.to_string(), path.to_string())
                        };
                        if !binding.is_empty() {
                            module_aliases.insert(binding, target);
                        }
                    }
                    "use" | "pub-use" if lang == "rust" => {
                        // Rust: `a::b::c` or `a::b::c as d`. The Rust
                        // plugin emits `imp.path` verbatim, including a
                        // literal leading `crate`/`self`/`super` — those
                        // never match `by_qn`, which is keyed on the
                        // workspace-qualified path every definition
                        // actually carries. Rewrite it here, once
                        // resolution (not extraction) is the right place
                        // to interpret it.
                        let raw = imp.path.trim();
                        let full = match (&rust_crate_root, &rust_module_for_use) {
                            (Some(cr), Some(om)) => rewrite_rust_use_path(raw, cr, om),
                            _ => raw.to_string(),
                        };
                        let full = full.as_str();
                        let alias = if imp.alias.is_empty() {
                            full.rsplit("::").next().unwrap_or(full).to_string()
                        } else {
                            imp.alias.clone()
                        };
                        direct_imports
                            .entry(alias)
                            .or_default()
                            .push(full.to_string());
                    }
                    "using" if lang == "csharp" => {
                        // C#: `using X.Y.Z;`                 -> module alias Z -> X.Y.Z
                        //     `using Alias = X.Y.Z;`         -> alias Alias -> X.Y.Z
                        let full = imp.path.trim();
                        if !imp.alias.is_empty() {
                            module_aliases.insert(imp.alias.clone(), full.to_string());
                        } else if let Some(last) = full.rsplit('.').next() {
                            module_aliases.insert(last.to_string(), full.to_string());
                        }
                    }
                    "using-static" => {
                        // C#: `using static X.Y;` — every member of Y is
                        // callable unqualified. We record each definition
                        // by its leaf name once we've walked the graph;
                        // at resolve time we try `X.Y.<name>` directly.
                        let full = imp.path.trim().to_string();
                        unqualified_prefixes.push(full.clone());
                        direct_imports
                            .entry("__using_static__".into())
                            .or_default()
                            .push(full);
                    }
                    "include" if cgg_core::same_family(&lang, "c") => {
                        // C family: `#include "helpers.h"` / `#import "Foo.h"`
                        // — all definitions from the included file become
                        // available in this TU. We resolve the path relative
                        // to the current file and transitively chase includes
                        // up to 8 levels deep.
                        let included_path = imp.path.trim();
                        if !included_path.is_empty() {
                            collect_include_defs(
                                included_path,
                                facts,
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                8,
                                &mut include_visited,
                            );
                        }
                    }
                    // `#include <helper_cuda.h>` names a file in the tree
                    // whenever the build passes `-I` — CUDA samples,
                    // `<spdlog/spdlog.h>` in spdlog's own tests — and the
                    // suffix fallback finds it. A standard or platform
                    // header never does, and following one into a
                    // vendored libc copy would bind `strlen` to whichever
                    // copy sorts first, so those stay unfollowed.
                    "system-include"
                        if matches!(lang.as_str(), "c" | "cpp" | "objc")
                            && !is_platform_header(imp.path.trim()) =>
                    {
                        collect_include_defs(
                            imp.path.trim(),
                            facts,
                            &include_by_exact,
                            &include_by_last,
                            &mut direct_imports,
                            8,
                            &mut include_visited,
                        );
                    }
                    "source" => {
                        // Bash: `source ./lib.sh` — same semantics as
                        // C #include: all definitions from the sourced
                        // file become available.
                        let sourced_path = imp.path.trim();
                        if !sourced_path.is_empty() {
                            collect_include_defs(
                                sourced_path,
                                facts,
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                4,
                                &mut include_visited,
                            );
                        }
                    }
                    "require" => {
                        // Ruby: `require './helper'`
                        // Lua:  `local m = require('foo.bar')`  (path = "foo.bar")
                        // Clojure: `(:require [foo.bar :as fb])`
                        // Erlang: `-include("x.hrl").`           (kind="include" — handled above)
                        // All three want every definition from the named file
                        // to become reachable. We try a few path resolutions.
                        let req_path = imp.path.trim();
                        if req_path.is_empty() {
                            continue;
                        }

                        // 1) Direct file-include resolution.
                        for try_path in [
                            req_path.to_string(),
                            format!("{req_path}.rb"),
                            format!("{req_path}.lua"),
                            format!("{req_path}.clj"),
                            req_path.replace('.', "/") + ".lua",
                            req_path.replace('.', "/") + ".clj",
                        ] {
                            collect_include_defs(
                                &try_path,
                                facts,
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                4,
                                &mut include_visited,
                            );
                        }

                        // 2) Module-alias / unqualified-prefix.
                        if !imp.alias.is_empty() {
                            module_aliases
                                .insert(imp.alias.clone(), req_path.to_string());
                        } else {
                            let last = req_path
                                .rsplit(['.', '/', ':'])
                                .next()
                                .unwrap_or(req_path);
                            if !last.is_empty() {
                                module_aliases
                                    .insert(last.to_string(), req_path.to_string());
                            }
                        }
                        unqualified_prefixes.push(req_path.to_string());
                    }
                    "load" => {
                        // Starlark: `load("//path:file.bzl", "symbol", aliased="other")`.
                        // path is the .bzl file ref. We treat it as include-like
                        // (every def in the loaded file becomes available) since
                        // tracking the actual symbols list would require a
                        // separate Starlark-aware import representation.
                        let raw =
                            imp.path.trim().trim_start_matches("//").trim_matches('"');
                        let cleaned = raw.replace(':', "/");
                        for try_path in [cleaned.clone(), format!("{cleaned}.bzl")] {
                            collect_include_defs(
                                &try_path,
                                facts,
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                4,
                                &mut include_visited,
                            );
                        }
                    }
                    "open" => {
                        // OCaml / F#: `open Module` — brings every symbol in
                        // `Module` into scope unqualified.
                        let path = imp.path.trim();
                        if !path.is_empty() {
                            unqualified_prefixes.push(path.to_string());
                            // Last segment also aliases the module.
                            if let Some(last) = path.rsplit('.').next() {
                                module_aliases.insert(last.to_string(), path.to_string());
                            }
                        }
                    }
                    "alias" => {
                        // Elixir: `alias Foo.Bar` => Bar refers to Foo.Bar.
                        // `alias Foo.Bar, as: B` => B refers to Foo.Bar.
                        let path = imp.path.trim();
                        let alias = if imp.alias.is_empty() {
                            path.rsplit('.').next().unwrap_or(path).to_string()
                        } else {
                            imp.alias.clone()
                        };
                        if !alias.is_empty() {
                            module_aliases.insert(alias, path.to_string());
                        }
                    }
                    "use" => {
                        // Elixir `use Foo` / Fortran `use module` — typically
                        // brings module contents into scope unqualified.
                        // Rust `use` is handled above; this arm only fires for
                        // other languages because match arms above already
                        // claim the kind for Rust.
                        let path = imp.path.trim();
                        // Elixir `use M, opts`: the module precedes them.
                        let path = if lang == "elixir" {
                            path.split(',').next().unwrap_or(path).trim()
                        } else {
                            path
                        };
                        if !path.is_empty() {
                            unqualified_prefixes.push(path.to_string());
                        }
                    }
                    "import qualified" => {
                        // Haskell: `import qualified Data.Map [as M]`. Without
                        // an alias, the module is referenced by its full name
                        // (`Data.Map.lookup`); with an alias, by `M.lookup`.
                        let path = imp.path.trim();
                        let alias = if imp.alias.is_empty() {
                            path
                        } else {
                            imp.alias.as_str()
                        };
                        if !alias.is_empty() {
                            module_aliases.insert(alias.to_string(), path.to_string());
                        }
                    }
                    "import" => {
                        // Generic "import" — language-specific dispatch. The
                        // Python/Go/JS variant is handled by the earlier arm
                        // pattern via specific kinds; here we cover the langs
                        // whose plugins emit a bare "import" kind.
                        let path = imp.path.trim();
                        if path.is_empty() {
                            continue;
                        }
                        match lang.as_str() {
                            // Scala / Java-style: `import pkg.{A,B}` or `import pkg.A`.
                            "scala" | "java" | "kotlin" | "groovy" => {
                                if let Some(idx) = path.rfind('.') {
                                    let prefix = &path[..idx];
                                    let suffix = &path[idx + 1..];
                                    let suffix =
                                        suffix.trim_matches(|c| c == '{' || c == '}');
                                    if suffix == "_" || suffix == "*" {
                                        unqualified_prefixes.push(prefix.to_string());
                                    } else {
                                        for name in suffix.split(',') {
                                            let name = name.trim();
                                            if name.is_empty() {
                                                continue;
                                            }
                                            let (src, alias) = match name.split_once("=>")
                                            {
                                                Some((s, a)) => (s.trim(), a.trim()),
                                                None => (name, name),
                                            };
                                            direct_imports
                                                .entry(alias.to_string())
                                                .or_default()
                                                .push(format!("{prefix}.{src}"));
                                        }
                                    }
                                    // Scala binds only what the import
                                    // names (`import a.sql.QueryTest`
                                    // binds `QueryTest`, not `sql`).
                                    // Only an import that can name a
                                    // package object's member (a wildcard,
                                    // a lowercase member) points the
                                    // scoped lookup at its package; a class
                                    // import does not.
                                    if lang == "scala" {
                                        let members = suffix == "_"
                                            || suffix.split(',').any(|n| {
                                                n.trim().starts_with(char::is_lowercase)
                                            });
                                        if members {
                                            scala_import_dirs.push(prefix.to_string());
                                        }
                                    } else if let Some(last) = prefix.rsplit('.').next() {
                                        module_aliases
                                            .insert(last.to_string(), prefix.to_string());
                                    }
                                }
                            }
                            // Dart / Solidity / Nix: file-relative paths.
                            "dart" | "solidity" | "nix" => {
                                let cleaned = path.trim_matches(|c| {
                                    c == '\'' || c == '"' || c == '<' || c == '>'
                                });
                                for try_path in [
                                    cleaned.to_string(),
                                    format!("{cleaned}.sol"),
                                    format!("{cleaned}.dart"),
                                    format!("{cleaned}.nix"),
                                ] {
                                    collect_include_defs(
                                        &try_path,
                                        facts,
                                        &include_by_exact,
                                        &include_by_last,
                                        &mut direct_imports,
                                        4,
                                        &mut include_visited,
                                    );
                                }
                                if !imp.alias.is_empty() {
                                    let derived = cleaned
                                        .rsplit('/')
                                        .next()
                                        .unwrap_or(cleaned)
                                        .trim_end_matches(".dart")
                                        .trim_end_matches(".sol")
                                        .trim_end_matches(".nix")
                                        .to_string();
                                    module_aliases.insert(imp.alias.clone(), derived);
                                }
                            }
                            // Haskell / Erlang / Elixir / generic: dotted module name,
                            // unqualified import.
                            "haskell" | "erlang" | "elixir" | "fsharp" | "ocaml"
                            | "julia" | "lean" => {
                                // Elixir `import M, only: [f: 1]`: the
                                // module is what precedes the options.
                                let path = if lang == "elixir" {
                                    path.split(',').next().unwrap_or(path).trim()
                                } else {
                                    path
                                };
                                unqualified_prefixes.push(path.to_string());
                                if let Some(last) = path.rsplit('.').next() {
                                    module_aliases
                                        .insert(last.to_string(), path.to_string());
                                }
                            }
                            _ => {
                                // Fall back to module-alias on last segment.
                                let last =
                                    path.rsplit(['.', '/', ':']).next().unwrap_or(path);
                                if !last.is_empty() {
                                    module_aliases
                                        .insert(last.to_string(), path.to_string());
                                }
                            }
                        }
                    }
                    "using" if lang == "powershell" => {
                        // PowerShell `using namespace System.IO` — namespace open.
                        let path = imp.path.trim();
                        if !path.is_empty() {
                            unqualified_prefixes.push(path.to_string());
                        }
                    }
                    k if k.starts_with("using-") && lang == "powershell" => {
                        let path = imp.path.trim();
                        if !path.is_empty() {
                            unqualified_prefixes.push(path.to_string());
                        }
                    }
                    "import-module" | "dot-source" => {
                        // PowerShell: include-like.
                        let path = imp.path.trim();
                        for try_path in [
                            path.to_string(),
                            format!("{path}.psm1"),
                            format!("{path}.ps1"),
                        ] {
                            collect_include_defs(
                                &try_path,
                                facts,
                                &include_by_exact,
                                &include_by_last,
                                &mut direct_imports,
                                4,
                                &mut include_visited,
                            );
                        }
                        unqualified_prefixes.push(path.to_string());
                    }
                    "include" | "add_subdirectory" | "find_package" => {
                        // CMake-style file inclusion (the C/C++ "include" arm
                        // above already claims the kind for those languages).
                        if lang == "cmake"
                            || lang == "verilog"
                            || lang == "vhdl"
                            || lang == "erlang"
                            || lang == "fortran"
                        {
                            let path = imp.path.trim();
                            for try_path in [
                                path.to_string(),
                                format!("{path}.cmake"),
                                format!("{path}.v"),
                                format!("{path}.hrl"),
                                format!("{path}.f90"),
                                format!("{path}.f95"),
                                "CMakeLists.txt".to_string(),
                            ] {
                                collect_include_defs(
                                    &try_path,
                                    facts,
                                    &include_by_exact,
                                    &include_by_last,
                                    &mut direct_imports,
                                    4,
                                    &mut include_visited,
                                );
                            }
                        }
                    }
                    "using-namespace" => {
                        let path = imp.path.trim();
                        if !path.is_empty() {
                            unqualified_prefixes.push(path.to_string());
                        }
                    }
                    _ => {}
                }
            }

            // Compute "scoped candidates": for each unqualified_prefix,
            // collect the callables defined in files whose path matches
            // the prefix (e.g. `Text.Pandoc` matches `*/Text/Pandoc.hs`).
            // This is what lets Haskell / OCaml — whose plugins don't put
            // module prefixes on qualified_name — still get cross-file
            // resolution without resorting to global by-simple noise.
            let mut scoped_simple = ScopedSimple::empty();
            // The modules this file imports, as lowercased path forms
            // (`data/map`, `lists`, `type.zig`): what tells a same-named
            // copy in the imported module from one elsewhere.
            let mut module_files: Vec<String> = Vec::new();
            for m in unqualified_prefixes.iter().chain(module_aliases.values()) {
                let m = m.trim().to_ascii_lowercase();
                if !m.is_empty() {
                    module_files.push(m.replace('.', "/"));
                    module_files.push(m);
                }
            }
            module_files.sort();
            module_files.dedup();
            if !unqualified_prefixes.is_empty()
                || !direct_imports.is_empty()
                || !module_aliases.is_empty()
            {
                let mut path_fragments: Vec<String> = unqualified_prefixes
                    .iter()
                    .map(|p| p.replace('.', "/").to_ascii_lowercase())
                    .collect();
                // Also include single-segment module aliases (Lua `require('foo')`,
                // Scala `import play.Foo` last segment, etc.)
                for target in module_aliases.values().chain(scala_import_dirs.iter()) {
                    path_fragments.push(target.replace('.', "/").to_ascii_lowercase());
                }
                path_fragments.retain(|f| !f.is_empty());
                path_fragments.sort();
                path_fragments.dedup();
                if let Some(index) = scoped_index.get(lang.as_str()) {
                    scoped_simple = ScopedSimple::new(index, path_fragments);
                }
            }

            let heads = crate::heads::FileHeads::build(facts, &vocab);
            let locals = &heads.locals;
            let scala_wild = if lang == "scala" {
                with_ancestors(
                    &lang,
                    unqualified_prefixes
                        .iter()
                        .filter_map(|p| p.rsplit('.').next())
                        .map(str::to_string)
                        .collect(),
                    &bases_by_owner,
                )
            } else {
                std::collections::BTreeSet::new()
            };
            let file_mods = crate::modules::FileModules::build(&modix, facts, &vocab);

            drop(_sp_imports);
            let _sp_refs = cgg_core::profile::span("xfile::ref-loop");
            for r in &facts.references {
                // A string literal that names a callable is never a call.
                // §8 is explicit: string routing may lower confidence, it
                // must not manufacture an edge.
                if r.receiver_hint == cgg_core::STRING_REF_HINT {
                    continue;
                }

                // Compute enclosing callable up front so we can pass its
                // qualified name into the resolver — needed for the
                // intra-crate qualified-path retry (e.g., `crawl::foo()`
                // inside `nkb_research::ResearchRunner::run` should find
                // `nkb_research::crawl::foo`).
                let enclosing =
                    enclosing_callable_id(&callables_by_span, facts, r.site_byte);

                // The intra-file pass already settled this NAME at this
                // call site — do not let cross-file re-open it under a
                // different candidate for the same name (see
                // `intra_bound_sites` above). This also covers a
                // `VALUE_REF_HINT` site that intra-file already bound;
                // a site it did NOT bind falls through to that arm
                // exactly as before. The name is required in the key so
                // a chained call's inner and outer references — which
                // tree-sitter gives the same `site_byte` — are not
                // conflated: binding `classifier` must not suppress
                // `classify` at the identical byte.
                if let Some(src) = enclosing
                    && intra_bound_sites.contains(&(src, r.site_byte, r.name.clone()))
                {
                    continue;
                }

                // Value references (`register(handler)`) resolve by name,
                // not through the import tables, and produce a
                // `Via::Reference` edge gated behind `--reference-edges`.
                //
                // Two gaps closed here at once: `intra_file` could only bind
                // a value ref to a callable in the *same file*, so
                // `app.get('/x', handler)` with the handler in another
                // module resolved to nothing; and letting the record fall
                // through to the generic path below tagged it `Via::Direct`,
                // which claims a call site that does not exist and escapes
                // the flag that is supposed to gate it.
                if r.receiver_hint == cgg_core::VALUE_REF_HINT {
                    // A value reference at module scope has no callable to
                    // hang the edge on: `callback=_validate_key` inside a
                    // click decorator, `event.listen(cls, "...", cls._x)`,
                    // a dispatch-table literal. Dropping it silently made
                    // the *target* look never-referenced, and — because
                    // nothing was on record to doubt it — promoted it to
                    // `High`. Measured on flask, httpie, black, flaskbb and
                    // dispatch, that single omission produced 28 of 45
                    // false positives in the top band.
                    //
                    // The reference is still real, so record it as an
                    // unresolved site. The dead-code pass correlates
                    // unresolved sites by name and will refuse to promote
                    // any callable one of them might target. The hint is
                    // cleared because `VALUE_REF_HINT` is plumbing, not a
                    // receiver type, and the correlation reads that field
                    // as a type.
                    // Reasons below are the `ValueRef*` pair, deliberately
                    // NOT `NoEnclosingCallable`/`AmbiguousInFile` — those
                    // are read by other consumers (the `ambiguous-in-file`
                    // metrics bucket, the dead-code correlation) as an
                    // ordinary ambiguous *call*, and on cgg's own corpus
                    // value refs were 940 of 972 entries in that bucket
                    // (VERIFIED.md §1a / §3.8).
                    let Some(src) = enclosing else {
                        out.unresolved.push(AuditUnresolvedCall::new(
                            None,
                            facts.file,
                            r.site_line,
                            r.site_byte,
                            r.name.clone(),
                            String::new(),
                            UnresolvedReason::ValueRefNoEnclosing,
                        ));
                        continue;
                    };
                    let Some(cands) = by_simple.get(&(lang.clone(), r.name.clone()))
                    else {
                        continue;
                    };
                    // Ambiguity is dropped rather than guessed: a reference
                    // edge to the wrong `handler` is worse than none. But
                    // dropping it *silently* is what turned flask's two
                    // `_make_timedelta` definitions into two `High`
                    // findings — the reference names one of them, and
                    // refusing to say which is not the same as there being
                    // no reference. Record the site so the name
                    // correlation can still see it.
                    let [cid] = cands.as_slice() else {
                        out.unresolved.push(AuditUnresolvedCall::new(
                            Some(src),
                            facts.file,
                            r.site_line,
                            r.site_byte,
                            r.name.clone(),
                            String::new(),
                            UnresolvedReason::ValueRefAmbiguous {
                                candidates: cands.len() as u32,
                            },
                        ));
                        continue;
                    };
                    if *cid == src {
                        continue;
                    }
                    // The target of a value-reference edge must be
                    // something that can actually be *called* by whatever
                    // eventually invokes the reference — a free function
                    // or a method. A closure bound to the same simple name
                    // (`on_click = lambda: …`) is never the real target:
                    // emitting an edge into it manufactures a call that
                    // does not exist. Silently skipped, matching the
                    // self-reference and dedup skips just below.
                    let target_kind = graph.callables.get(cid).map(|c| c.kind);
                    if matches!(target_kind, Some(CallableKind::Closure) | None) {
                        continue;
                    }
                    let dup = existing_edges.contains(&(src, *cid, r.site_byte));
                    if !dup {
                        out.edges.push(CallEdge {
                            src,
                            dst: *cid,
                            site_line: r.site_line,
                            site_byte: r.site_byte,
                            confidence: Confidence::Medium,
                            via: Via::Reference,
                            resolver: resolver_id.clone(),
                            weight: 1,
                        });
                    }
                    continue;
                }
                let caller_qn = enclosing
                    .and_then(|id| graph.callables.get(&id))
                    .map(|c| c.qualified_name.as_str());

                // A bare call through a parameter or local (`cb()`,
                // `debug("x")` after `const debug = makeDebug()`) calls
                // the value the name holds. No other file's function is
                // the callee, whatever shares its name (WS14).
                if locals.shadows_bare_call(r) {
                    // A local whose type is known is an instance: calling
                    // it enters its type's call operator (step 7's
                    // `agent("x")` → `Agent.__call__`), the one reading a
                    // value binding has.
                    let call_op: Option<Vec<CallableId>> =
                        var_types.get(&r.name).and_then(|ty| {
                            call_operator_names(&lang).iter().find_map(|op| {
                                by_owner_method
                                    .get(&(lang.clone(), ty.clone(), (*op).to_string()))
                                    .filter(|v| !v.is_empty())
                                    .cloned()
                                    .or_else(|| {
                                        resolve_via_bases(
                                            &lang,
                                            ty,
                                            op,
                                            &by_owner_method,
                                            &bases_by_owner,
                                        )
                                    })
                            })
                        });
                    if let (Some(cids), Some(src)) = (call_op, enclosing) {
                        for cid in cids {
                            if cid != src
                                && !existing_edges.contains(&(src, cid, r.site_byte))
                            {
                                out.edges.push(CallEdge {
                                    src,
                                    dst: cid,
                                    site_line: r.site_line,
                                    site_byte: r.site_byte,
                                    confidence: Confidence::Medium,
                                    via: Via::Direct,
                                    resolver: resolver_id.clone(),
                                    weight: 1,
                                });
                            }
                        }
                        continue;
                    }
                    if by_simple
                        .get(&(lang.clone(), r.name.clone()))
                        .is_some_and(|c| !c.is_empty())
                    {
                        out.unresolved.push(AuditUnresolvedCall::new(
                            enclosing,
                            facts.file,
                            r.site_line,
                            r.site_byte,
                            r.name.clone(),
                            r.receiver_hint.clone(),
                            UnresolvedReason::ShadowedByLocal,
                        ));
                    }
                    continue;
                }

                let _sp_ref = cgg_core::profile::span("xfile::resolve-ref");
                let mut capped = 0u32;
                let mut no_ctor = false;
                let mut external: Option<String> = None;
                let mut refused_module: Option<String> = None;
                let mut module_certain = false;
                let super_recv = is_super_receiver(&r.receiver_hint);
                let resolved = try_resolve_ref(
                    graph,
                    facts.file,
                    &lang,
                    r,
                    &heads,
                    &vocab,
                    &mut external,
                    &direct_imports,
                    &module_aliases,
                    &unqualified_prefixes,
                    &scoped_simple,
                    &module_files,
                    &by_qn,
                    &by_simple,
                    &by_owner_method,
                    &by_full_owner_method,
                    &std_owner_trait_impls,
                    &reexports,
                    &bases_by_owner,
                    &derived_by_base,
                    &zig_packages,
                    &known_owners,
                    &stub_ids,
                    &signatures,
                    &var_types,
                    caller_qn,
                    enclosing,
                    fanout_cap,
                    &rust_path_heads,
                    &workspace_owner_names,
                    &class_record_langs,
                    &own_bases,
                    &kotlin_extensions,
                    &modix,
                    &file_mods,
                    &libix,
                    &phpix,
                    &scala_wild,
                    &qn_collisions,
                    &overloads,
                    &mut refused_module,
                    &mut module_certain,
                    &mut capped,
                    &mut no_ctor,
                )
                .and_then(|cids| {
                    if super_recv {
                        without_own_class(
                            &lang,
                            &r.name,
                            caller_qn,
                            &by_owner_method,
                            cids,
                        )
                    } else {
                        Some(cids)
                    }
                });
                // A typed C++ receiver whose lookup found nothing falls back
                // to the receiver as written — exactly what 0.8.5 resolved
                // it with — and anything that finds is a guess, so medium.
                let mut untyped_fallback = false;
                let resolved = match resolved {
                    // Zig (WS14b): a receiver typed as something the
                    // container lookup could not follow (a generic's
                    // `const T = Fn(…)` alias) is read as written; a type
                    // that was followed and refused, or led out of the
                    // tree, stands.
                    None if lang == "cpp"
                        || (lang == "zig"
                            && refused_module.is_none()
                            && external.is_none()) =>
                    {
                        facts
                            .untyped_receivers
                            .iter()
                            .find(|(b, n, _)| *b == r.site_byte && *n == r.name)
                            .and_then(|(_, _, orig)| {
                                let mut r2 = r.clone();
                                r2.receiver_hint = orig.clone();
                                r2.typed_receiver = false;
                                let (mut c2, mut n2) = (0u32, false);
                                let mut e2: Option<String> = None;
                                let (mut m2, mut mc2) = (None, false);
                                try_resolve_ref(
                                    graph,
                                    facts.file,
                                    &lang,
                                    &r2,
                                    &heads,
                                    &vocab,
                                    &mut e2,
                                    &direct_imports,
                                    &module_aliases,
                                    &unqualified_prefixes,
                                    &scoped_simple,
                                    &module_files,
                                    &by_qn,
                                    &by_simple,
                                    &by_owner_method,
                                    &by_full_owner_method,
                                    &std_owner_trait_impls,
                                    &reexports,
                                    &bases_by_owner,
                                    &derived_by_base,
                                    &zig_packages,
                                    &known_owners,
                                    &stub_ids,
                                    &signatures,
                                    &var_types,
                                    caller_qn,
                                    enclosing,
                                    fanout_cap,
                                    &rust_path_heads,
                                    &workspace_owner_names,
                                    &class_record_langs,
                                    &own_bases,
                                    &kotlin_extensions,
                                    &modix,
                                    &file_mods,
                                    &libix,
                                    &phpix,
                                    &scala_wild,
                                    &qn_collisions,
                                    &overloads,
                                    &mut m2,
                                    &mut mc2,
                                    &mut c2,
                                    &mut n2,
                                )
                            })
                            .inspect(|_| {
                                untyped_fallback = true;
                                external = None;
                            })
                    }
                    other => other,
                };
                // A non-recursive binding is not the callee of a call in
                // its own body (OCaml `let f x = f …`, WS14b).
                let resolved = match (resolved, r.not_def) {
                    (Some(cids), Some(b)) => {
                        let kept: Vec<CallableId> = cids
                            .into_iter()
                            .filter(|c| {
                                graph.callables.get(c).is_none_or(|n| {
                                    !(n.file == facts.file && n.start_byte == b)
                                })
                            })
                            .collect();
                        (!kept.is_empty()).then_some(kept)
                    }
                    (other, _) => other,
                };
                // `--guess-unique-names`: a Rust site nothing placed, whose
                // name exactly one callable in the tree carries. Opt-in and
                // `low`, because a unique name is evidence, not proof.
                let mut guessed = false;
                let resolved = match resolved {
                    None if guess_unique_names
                        && lang == "rust"
                        && capped == 0
                        && !super_recv
                        && !no_ctor =>
                    {
                        unique_name_guess(
                            graph,
                            r,
                            facts.file,
                            caller_qn,
                            &by_simple,
                            &rust_path_heads,
                            &workspace_owner_names,
                            &direct_imports,
                            &module_aliases,
                        )
                        .map(|cid| vec![cid])
                        .inspect(|_| guessed = true)
                    }
                    other => other,
                };
                // An unambiguous binding is not a guess. A bare name
                // bound by `from x import y` in this very file, or a
                // module alias resolving to exactly one callable, is as
                // certain as same-file resolution — and while it scored
                // `medium`, a same-file method with a colliding name
                // could outrank the correct target at `high`. Anything
                // with more than one candidate stays `medium`: that is
                // fan-out, and fan-out is a hypothesis.
                let confidence = match &resolved {
                    _ if guessed => Confidence::Low,
                    _ if untyped_fallback => Confidence::Medium,
                    // Looked up by its exact name in the one module the
                    // call is written through.
                    Some(cids) if cids.len() == 1 && module_certain => Confidence::High,
                    Some(cids)
                        if cids.len() == 1
                            && (r.receiver_hint.is_empty()
                                && direct_imports.contains_key(&r.name)
                                || !r.receiver_hint.is_empty()
                                    && module_aliases
                                        .contains_key(r.receiver_hint.as_str())) =>
                    {
                        Confidence::High
                    }
                    // The same evidence for a Kotlin extension the file
                    // imports by name: `"…".toHttpUrl()` after `import
                    // okhttp3.HttpUrl.Companion.toHttpUrl`.
                    Some(cids)
                        if cids.len() == 1
                            && lang == "kotlin"
                            && !r.receiver_hint.is_empty()
                            && kotlin_extensions.ids.contains(&cids[0])
                            && graph.callables.get(&cids[0]).is_some_and(|c| {
                                direct_imports.get(&r.name).is_some_and(|qns| {
                                    qns.iter().any(|q| {
                                        kotlin_import_path(q) == c.qualified_name
                                    })
                                })
                            }) =>
                    {
                        Confidence::High
                    }
                    _ => Confidence::Medium,
                };
                if let Some(cids) = resolved {
                    for cid in cids {
                        // Skip self-edges that coincide with intra-file's
                        // ones (they'd be duplicates with the same resolver).
                        if let Some(src) = enclosing {
                            if src == cid {
                                continue;
                            }
                            // Avoid duplicating intra-file-emitted edges.
                            if existing_edges.contains(&(src, cid, r.site_byte)) {
                                continue;
                            }
                            out.edges.push(CallEdge {
                                src,
                                dst: cid,
                                site_line: r.site_line,
                                site_byte: r.site_byte,
                                confidence,
                                via: Via::Direct,
                                resolver: if guessed {
                                    ResolverId::new("cross-file:name-unique")
                                } else {
                                    resolver_id.clone()
                                },
                                weight: 1,
                            });
                        }
                    }
                } else if let Some(module) = external {
                    // A call into a dependency that shares a name with
                    // workspace callables. Recorded with the import it was
                    // attributed to — the driver files it in the external
                    // bucket, replacing the intra-file record of the site,
                    // so it is neither silent nor counted twice. A name no
                    // workspace callable carries changed nothing and keeps
                    // the intra-file record as it was.
                    if by_simple
                        .get(&(lang.clone(), r.name.clone()))
                        .is_some_and(|c| !c.is_empty())
                    {
                        let mut u = AuditUnresolvedCall::new(
                            enclosing,
                            facts.file,
                            r.site_line,
                            r.site_byte,
                            r.name.clone(),
                            r.receiver_hint.clone(),
                            UnresolvedReason::ExternalModule { module },
                        );
                        u.name_screen_applied = Some("external".to_string());
                        out.unresolved.push(u);
                    }
                } else if let Some(module) = refused_module {
                    // Written through a workspace module that provides
                    // nothing the call names: refused rather than bound to
                    // a same-named callable elsewhere, and recorded. A name
                    // no workspace callable carries changed nothing.
                    if by_simple
                        .get(&(lang.clone(), r.name.clone()))
                        .is_some_and(|c| !c.is_empty())
                    {
                        out.unresolved.push(AuditUnresolvedCall::new(
                            enclosing,
                            facts.file,
                            r.site_line,
                            r.site_byte,
                            r.name.clone(),
                            r.receiver_hint.clone(),
                            UnresolvedReason::NotInImportedModule { module },
                        ));
                    }
                } else if capped > 0 {
                    // A drop is never silent. Without this the call site
                    // is indistinguishable from one that calls nothing —
                    // in the field report a method with 24 grep-visible
                    // call sites showed 2 inbound edges and no signal
                    // that 22 were dropped.
                    out.unresolved.push(AuditUnresolvedCall::new(
                        enclosing,
                        facts.file,
                        r.site_line,
                        r.site_byte,
                        r.name.clone(),
                        r.receiver_hint.clone(),
                        UnresolvedReason::FanoutCapExceeded { candidates: capped },
                    ));
                } else if super_recv {
                    // `super()` with every candidate excluded means the
                    // base is not in the analyzed tree. Saying so beats
                    // both a wrong edge and silence.
                    out.unresolved.push(AuditUnresolvedCall::new(
                        enclosing,
                        facts.file,
                        r.site_line,
                        r.site_byte,
                        r.name.clone(),
                        r.receiver_hint.clone(),
                        UnresolvedReason::SuperBaseOutOfGraph,
                    ));
                } else if no_ctor {
                    out.unresolved.push(AuditUnresolvedCall::new(
                        enclosing,
                        facts.file,
                        r.site_line,
                        r.site_byte,
                        r.name.clone(),
                        r.receiver_hint.clone(),
                        UnresolvedReason::ClassWithoutExplicitInit,
                    ));
                } else if let Some(elsewhere) =
                    by_simple.get(&(lang.clone(), r.name.clone()))
                    && !elsewhere.is_empty()
                {
                    // The name *is* in the graph, just not reachable
                    // from here. `no-candidate-in-file` reads as "this
                    // name does not exist", which was being reported for
                    // names cgg had parsed and indexed — in one case
                    // with nine candidates.
                    out.unresolved.push(AuditUnresolvedCall::new(
                        enclosing,
                        facts.file,
                        r.site_line,
                        r.site_byte,
                        r.name.clone(),
                        r.receiver_hint.clone(),
                        UnresolvedReason::CandidatesInOtherFiles {
                            candidates: elsewhere.len() as u32,
                        },
                    ));
                }
            }

            let _ = &facts_by_id;
            out
        })
        .collect();
    for mut o in per_file {
        out.edges.append(&mut o.edges);
        out.unresolved.append(&mut o.unresolved);
    }

    // C++ virtual and member-pointer fan-out is `Via::Dynamic`, and every
    // dynamic edge is opt-in: `--dynamic-dispatch` (which `--dead-code`
    // turns on) never changes the default graph.
    if dynamic_dispatch {
        let extra = cpp_virtual_dispatch(graph, facts, &out.edges, &by_owner_method);
        out.edges.extend(extra);
    }
    out
}

/// C++ virtual calls and indexed member-pointer tables.
///
/// A typed call that already landed on `Shape::draw` also reaches every
/// override on a type that inherits from `Shape`. An indexed
/// `(m->*table[i])()` reaches every `&Type::method` stored in `table`,
/// then those methods' overrides. The override set is the real vtable,
/// so it is not sent through the duck-typing cap. The extra edges are
/// `Via::Dynamic` / low confidence — a site runs at most one of them —
/// and they stay in the default graph because the vtable is declared,
/// not guessed (`--dynamic-dispatch` is a separate, opt-in pass over
/// `trait_impl_target` and does not emit these).
fn cpp_virtual_dispatch(
    graph: &Graph,
    facts: &[FileFacts],
    new_edges: &[CallEdge],
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
) -> Vec<CallEdge> {
    let resolver = ResolverId::new("cross-file:cpp-virtual");
    let mut virtual_methods: std::collections::HashSet<(String, String, String)> =
        std::collections::HashSet::new();
    let mut bases_by_owner: HashMap<(String, String), Vec<String>> = HashMap::new();
    let mut tables: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for f in facts {
        if f.language != "cpp" {
            continue;
        }
        for d in &f.definitions {
            let Some(owner) = owner_from_qn(&d.qualified_name) else {
                continue;
            };
            if d.attributes.iter().any(|a| a == "virtual") {
                virtual_methods.insert((
                    f.language.clone(),
                    owner.to_string(),
                    d.simple_name.clone(),
                ));
            }
            if !d.base_types.is_empty() {
                let slot = bases_by_owner
                    .entry((f.language.clone(), owner.to_string()))
                    .or_default();
                for b in &d.base_types {
                    let bare = b.split(['<', '[']).next().unwrap_or(b).trim();
                    let bare = bare.rsplit(['.', ':', '\\']).next().unwrap_or(bare);
                    if !bare.is_empty() && !slot.iter().any(|x| x == bare) {
                        slot.push(bare.to_string());
                    }
                }
            }
        }
        for t in &f.member_ptr_takes {
            let slot = tables.entry(t.table.clone()).or_default();
            let pair = (t.owner.clone(), t.method.clone());
            if !slot.contains(&pair) {
                slot.push(pair);
            }
        }
    }
    if virtual_methods.is_empty() && tables.is_empty() {
        return Vec::new();
    }
    let mut derived_by_base: HashMap<(String, String), Vec<String>> = HashMap::new();
    for ((lang, owner), bases) in &bases_by_owner {
        for b in bases {
            let slot = derived_by_base
                .entry((lang.clone(), b.clone()))
                .or_default();
            if !slot.iter().any(|x| x == owner) {
                slot.push(owner.clone());
            }
        }
    }
    for kids in derived_by_base.values_mut() {
        kids.sort();
    }

    let mut existing: std::collections::HashSet<(CallableId, CallableId, u32)> = graph
        .edges
        .iter()
        .map(|e| (e.src, e.dst, e.site_byte))
        .collect();
    for e in new_edges {
        existing.insert((e.src, e.dst, e.site_byte));
    }

    let mut extra = Vec::new();
    for f in facts {
        if f.language != "cpp" {
            continue;
        }
        for r in &f.references {
            if r.name != "->*" {
                continue;
            }
            let Some(src) = innermost_at(graph, f.file, r.site_byte) else {
                continue;
            };
            let Some(takes) = tables.get(&r.receiver_hint) else {
                continue;
            };
            for (owner, method) in takes {
                let targets = cpp_override_set(
                    owner,
                    method,
                    &virtual_methods,
                    &bases_by_owner,
                    &derived_by_base,
                    by_owner_method,
                );
                for dst in targets {
                    if dst == src {
                        continue;
                    }
                    if !existing.insert((src, dst, r.site_byte)) {
                        continue;
                    }
                    extra.push(CallEdge {
                        src,
                        dst,
                        site_line: r.site_line,
                        site_byte: r.site_byte,
                        confidence: Confidence::Low,
                        via: Via::Dynamic,
                        resolver: resolver.clone(),
                        weight: 1,
                    });
                }
            }
        }
    }

    let seed: Vec<(CallableId, CallableId, u32, u32)> = graph
        .edges
        .iter()
        .chain(new_edges.iter())
        .chain(extra.iter())
        .map(|e| (e.src, e.dst, e.site_line, e.site_byte))
        .collect();
    for (src, dst, site_line, site_byte) in seed {
        let Some(node) = graph.callables.get(&dst) else {
            continue;
        };
        if node.language != "cpp" {
            continue;
        }
        if node.simple_name.starts_with('~') {
            continue;
        }
        let Some(owner) = owner_from_qn(&node.qualified_name) else {
            continue;
        };
        if owner == node.simple_name {
            continue;
        }
        if !cpp_slot_is_virtual(
            owner,
            &node.simple_name,
            &virtual_methods,
            &bases_by_owner,
        ) {
            continue;
        }
        for ov in
            cpp_descendants(owner, &node.simple_name, &derived_by_base, by_owner_method)
        {
            if ov == dst || ov == src {
                continue;
            }
            if !existing.insert((src, ov, site_byte)) {
                continue;
            }
            extra.push(CallEdge {
                src,
                dst: ov,
                site_line,
                site_byte,
                confidence: Confidence::Low,
                via: Via::Dynamic,
                resolver: resolver.clone(),
                weight: 1,
            });
        }
    }
    extra.sort_by_key(|e| (e.src.as_u64(), e.dst.as_u64(), e.site_byte));
    extra
}

fn innermost_at(graph: &Graph, file: FileId, site_byte: u32) -> Option<CallableId> {
    graph
        .callables
        .values()
        .filter(|c| c.file == file && c.start_byte <= site_byte && site_byte < c.end_byte)
        .min_by_key(|c| c.end_byte.saturating_sub(c.start_byte))
        .map(|c| c.id)
}

fn cpp_slot_is_virtual(
    owner: &str,
    method: &str,
    virtual_methods: &std::collections::HashSet<(String, String, String)>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
) -> bool {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut frontier = vec![owner.to_string()];
    for _ in 0..8 {
        let mut next = Vec::new();
        for t in std::mem::take(&mut frontier) {
            if !seen.insert(t.clone()) {
                continue;
            }
            if virtual_methods.contains(&("cpp".into(), t.clone(), method.to_string())) {
                return true;
            }
            if let Some(bases) = bases_by_owner.get(&("cpp".into(), t)) {
                next.extend(bases.iter().cloned());
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    false
}

fn cpp_descendants(
    owner: &str,
    method: &str,
    derived_by_base: &HashMap<(String, String), Vec<String>>,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
) -> Vec<CallableId> {
    let mut seen_types: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    seen_types.insert(owner.to_string());
    let mut frontier: Vec<String> = derived_by_base
        .get(&("cpp".into(), owner.to_string()))
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    let mut seen_ids: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    for _ in 0..8 {
        let mut next = Vec::new();
        for t in std::mem::take(&mut frontier) {
            if !seen_types.insert(t.clone()) {
                continue;
            }
            if let Some(cids) =
                by_owner_method.get(&("cpp".into(), t.clone(), method.to_string()))
            {
                for id in cids {
                    if seen_ids.insert(id.as_u64()) {
                        out.push(*id);
                    }
                }
            }
            if let Some(kids) = derived_by_base.get(&("cpp".into(), t)) {
                next.extend(kids.iter().cloned());
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}

fn cpp_override_set(
    owner: &str,
    method: &str,
    virtual_methods: &std::collections::HashSet<(String, String, String)>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
    derived_by_base: &HashMap<(String, String), Vec<String>>,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
) -> Vec<CallableId> {
    let mut out = Vec::new();
    let mut seen: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    if let Some(cids) =
        by_owner_method.get(&("cpp".into(), owner.to_string(), method.to_string()))
    {
        for id in cids {
            if seen.insert(id.as_u64()) {
                out.push(*id);
            }
        }
    }
    if cpp_slot_is_virtual(owner, method, virtual_methods, bases_by_owner) {
        for id in cpp_descendants(owner, method, derived_by_base, by_owner_method) {
            if seen.insert(id.as_u64()) {
                out.push(id);
            }
        }
    }
    out
}

/// A C/C++ standard, POSIX or OS header: `<stdio.h>`, `<vector>`,
/// `<sys/types.h>`, `<windows.h>`. These are provided by the toolchain,
/// never by the tree being analysed, even when the tree vendors a copy.
fn is_platform_header(path: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "sys/",
        "bits/",
        "linux/",
        "asm/",
        "asm-generic/",
        "netinet/",
        "arpa/",
        "net/",
        "mach/",
        "mach-o/",
        "gnu/",
        "machine/",
        "libkern/",
        "os/",
        "xlocale/",
    ];
    const NAMES: &[&str] = &[
        // C11/C17/C23
        "assert.h",
        "complex.h",
        "ctype.h",
        "errno.h",
        "fenv.h",
        "float.h",
        "inttypes.h",
        "iso646.h",
        "limits.h",
        "locale.h",
        "math.h",
        "setjmp.h",
        "signal.h",
        "stdalign.h",
        "stdarg.h",
        "stdatomic.h",
        "stdbit.h",
        "stdbool.h",
        "stdckdint.h",
        "stddef.h",
        "stdint.h",
        "stdio.h",
        "stdlib.h",
        "stdnoreturn.h",
        "string.h",
        "tgmath.h",
        "threads.h",
        "time.h",
        "uchar.h",
        "wchar.h",
        "wctype.h",
        // POSIX and common libc extensions
        "aio.h",
        "alloca.h",
        "cpio.h",
        "dirent.h",
        "dlfcn.h",
        "endian.h",
        "err.h",
        "execinfo.h",
        "fcntl.h",
        "features.h",
        "fmtmsg.h",
        "fnmatch.h",
        "ftw.h",
        "getopt.h",
        "glob.h",
        "grp.h",
        "iconv.h",
        "ifaddrs.h",
        "langinfo.h",
        "libgen.h",
        "libintl.h",
        "link.h",
        "malloc.h",
        "memory.h",
        "monetary.h",
        "mqueue.h",
        "ndbm.h",
        "netdb.h",
        "nl_types.h",
        "paths.h",
        "poll.h",
        "pthread.h",
        "pwd.h",
        "regex.h",
        "sched.h",
        "search.h",
        "semaphore.h",
        "spawn.h",
        "strings.h",
        "stropts.h",
        "syslog.h",
        "tar.h",
        "termios.h",
        "ucontext.h",
        "ulimit.h",
        "unistd.h",
        "utime.h",
        "utmp.h",
        "utmpx.h",
        "wordexp.h",
        "elf.h",
        "byteswap.h",
        "error.h",
        "sysexits.h",
        // Windows
        "windows.h",
        "winsock.h",
        "winsock2.h",
        "ws2tcpip.h",
        "windef.h",
        "winbase.h",
        "winuser.h",
        "winnt.h",
        "winerror.h",
        "tchar.h",
        "io.h",
        "process.h",
        "direct.h",
        "conio.h",
        "shlobj.h",
        "shellapi.h",
        "objbase.h",
        "intrin.h",
        "crtdbg.h",
        // Compiler intrinsics
        "immintrin.h",
        "emmintrin.h",
        "xmmintrin.h",
        "pmmintrin.h",
        "tmmintrin.h",
        "smmintrin.h",
        "nmmintrin.h",
        "wmmintrin.h",
        "x86intrin.h",
        "arm_neon.h",
        "cpuid.h",
    ];
    let p = path.trim().trim_matches(['<', '>']);
    // `<vector>`, `<cstdio>`, `<QtWidgets>`: an extensionless name is a
    // C++ standard or framework umbrella header, not a file in the tree.
    if !p.rsplit('/').next().unwrap_or(p).contains('.') {
        return true;
    }
    PREFIXES.iter().any(|pre| p.starts_with(pre)) || NAMES.contains(&p)
}

/// Whether a path has a `.` or `..` segment (`./x.h`, `../x.h`, `a/../b`).
fn has_dot_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|seg| seg == "." || seg == "..")
}

/// Resolve `.` and `..` components without touching the filesystem. A
/// `..` that would climb above the path's first component is kept.
fn normalize_lexically(p: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Collect definitions from an included header file and add them as
/// direct imports. Transitively follows `#include` directives in the
/// header up to `depth` levels.
fn collect_include_defs(
    include_path: &str,
    includer_facts: &FileFacts,
    include_by_exact: &HashMap<&std::path::Path, &FileFacts>,
    include_by_last: &HashMap<&std::ffi::OsStr, Vec<&FileFacts>>,
    direct_imports: &mut HashMap<String, Vec<String>>,
    depth: u8,
    // The best remaining depth each header has already been expanded
    // at, for *this* translation unit.
    //
    // Without any memo the walk is exponential: a C include graph is a
    // diamond, so `a.h` reached by four paths was expanded four times
    // and each of its own includes four times again. On Erlang/OTP's
    // `erts/emulator/beam` — 25 includes in `erl_process.h`, depth 8 —
    // that is 25^8 in the limit, recomputed per file. cgg did not finish
    // the directory in an hour; memoized it takes under a second.
    //
    // Why a depth map and not a plain visited set: `depth` counts down,
    // so a header first reached by a *long* path has little budget left
    // and stops early. A plain set would then refuse to re-expand it
    // when a *short* path arrives with budget to spare, silently losing
    // the deeper definitions — and which ones would depend on include
    // order. Re-expanding only on a strictly larger budget keeps the
    // result identical to the exhaustive walk while bounding the work at
    // O(headers x depth).
    //
    // The dedup is also correct on its own terms: the same definition
    // pushed N times inflated `direct_imports`, and step 1d rejects a
    // name with more than three candidates — so a diamond could push a
    // genuinely unique symbol over the cap and stop it resolving at all.
    visited: &mut HashMap<FileId, u8>,
) {
    if depth == 0 {
        return;
    }
    // Resolve the include path relative to the includer's directory.
    let includer_dir = includer_facts
        .path
        .parent()
        .unwrap_or(std::path::Path::new(""));
    // Lexically normalised: `#include "../common/util.h"` joined onto
    // `src/net` is `src/net/../common/util.h`, which matches no indexed
    // path exactly, and the suffix fallback cannot match `../` either.
    // Only a path with a `.`/`..` segment needs it: this runs once per
    // include/require edge of the transitive closure, and allocating a
    // normalised copy of every path cost 1.1s on Metabase's Clojure
    // `require` graph.
    let dotted = has_dot_segment(include_path);
    let joined = includer_dir.join(include_path);
    let resolved = if dotted {
        normalize_lexically(&joined)
    } else {
        joined
    };
    // Find the matching FileFacts by path suffix (handles both
    // absolute and relative paths in the index).
    //
    // The pick must be deterministic. A `HashMap`'s iteration order is
    // randomly seeded per process, so taking the first `.find()` match
    // made the `#include` closure — and therefore the emitted edge set
    // — vary between runs whenever more than one file matched the
    // suffix. That is routine in C/C++, where many directories hold
    // their own `common.h`. Prefer the exactly-resolved path, then the
    // lowest FileId: a total order over the candidates.
    // An exact path match is unique, so it can short-circuit; only the
    // ambiguous suffix case needs the full scan to find the lowest
    // FileId. Scanning unconditionally costs ~6% on include-heavy C/C++
    // trees, and the exact match is the common case.
    // Exact first, exactly as before. Then the suffix fallback, over the
    // few files sharing the include's last segment rather than all of
    // them — pre-sorted by FileId, so `find` yields the same lowest-id
    // winner the old scan did.
    let target: Option<&FileFacts> = include_by_exact
        .get(resolved.as_path())
        .copied()
        .or_else(|| {
            let last = std::path::Path::new(include_path).file_name()?;
            // `Path::ends_with` never matches a `..` component, so the
            // suffix is the include path with its leading `./`/`../`s off.
            let candidates = include_by_last.get(last)?;
            if !dotted {
                return candidates
                    .iter()
                    .find(|f| f.path.ends_with(include_path))
                    .copied();
            }
            let suffix: std::path::PathBuf = std::path::Path::new(include_path)
                .components()
                .skip_while(|c| {
                    matches!(
                        c,
                        std::path::Component::CurDir | std::path::Component::ParentDir
                    )
                })
                .collect();
            candidates
                .iter()
                .find(|f| f.path.ends_with(&suffix))
                .copied()
        });
    let Some(target) = target else { return };
    // Expand a header again only when this path has more budget left
    // than the one that reached it first.
    match visited.get(&target.file) {
        Some(&best) if best >= depth => return,
        _ => {
            visited.insert(target.file, depth);
        }
    }
    // Import all definitions from the target, plus the C++ declarations
    // it made whose prototypes were merged into a body elsewhere — the
    // header still declares them, so an includer still sees them.
    for d in &target.definitions {
        direct_imports
            .entry(d.simple_name.clone())
            .or_default()
            .push(d.qualified_name.clone());
    }
    for (simple, qualified) in &target.unified_decls {
        direct_imports
            .entry(simple.clone())
            .or_default()
            .push(qualified.clone());
    }
    // Transitively follow includes in the target.
    for imp in &target.imports {
        if imp.kind == "include"
            || (imp.kind == "system-include" && !is_platform_header(imp.path.trim()))
        {
            collect_include_defs(
                imp.path.trim(),
                target,
                include_by_exact,
                include_by_last,
                direct_imports,
                depth - 1,
                visited,
            );
        }
    }
}

/// Kotlin extension functions, indexed for [`kotlin_extension_in_scope`]
/// and [`KotlinExtensions::receiver_call_target`].
#[derive(Default)]
struct KotlinExtensions {
    /// (declaring file, simple name) → extensions, in graph order.
    by_file: HashMap<(FileId, String), Vec<CallableId>>,
    ids: std::collections::HashSet<CallableId>,
    /// Top-level extensions → the package that declares them.
    top_level_pkg: HashMap<CallableId, String>,
    /// Top-level Kotlin functions that are not extensions. The plugin
    /// files them as methods once the file has a package, so they are
    /// told apart by their qualified name (`<package>.<name>`).
    top_level_fns: std::collections::HashSet<CallableId>,
    /// Kotlin file → (its package, its import paths).
    file_scope: HashMap<FileId, (String, std::collections::HashSet<String>)>,
}

impl KotlinExtensions {
    /// Whether `cid` can be what a Kotlin receiver call (`x.f()`) from
    /// `caller_file` reaches. Only a member or an extension is ever called
    /// through a receiver, so a top-level function that is not an extension
    /// never is. An extension is, only where Kotlin resolves it: declared
    /// in the calling file, imported by name, or top-level in the caller's
    /// own package or one it imports with `*` — a test helper `fun
    /// List<LikeModel>.isEqualTo(…)` in a package the caller never imports
    /// is not a candidate for `assertThat(x).isEqualTo(y)`.
    fn receiver_call_target(
        &self,
        graph: &Graph,
        cid: CallableId,
        caller_file: FileId,
        direct_imports: &HashMap<String, Vec<String>>,
    ) -> bool {
        if !self.ids.contains(&cid) {
            return !self.top_level_fns.contains(&cid);
        }
        let Some(c) = graph.callables.get(&cid) else {
            return true;
        };
        if c.file == caller_file
            || direct_imports.get(&c.simple_name).is_some_and(|qns| {
                qns.iter()
                    .any(|q| kotlin_import_path(q) == c.qualified_name)
            })
        {
            return true;
        }
        let Some(pkg) = self.top_level_pkg.get(&cid) else {
            return false;
        };
        self.file_scope
            .get(&caller_file)
            .is_some_and(|(own, imports)| own == pkg || imports.contains(pkg))
    }
}

/// Whether a Kotlin declaration's signature declares an extension:
/// `fun Foo.ext(`, `private fun <T> List<T>.ext(`, `suspend fun
/// CoroutineScope.ext(` — a receiver type and a `.` in front of the name.
fn is_kotlin_extension(sig: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // Annotations come first in the signature; the declaration starts
    // at the `fun` keyword.
    let Some(fun_at) = sig.match_indices("fun").map(|(i, _)| i).find(|&i| {
        let before = sig[..i].chars().next_back();
        let after = sig[i + 3..].chars().next();
        before.is_none_or(|c| c.is_whitespace())
            && after.is_some_and(|c| c.is_whitespace() || c == '<')
    }) else {
        return false;
    };
    let decl = &sig[fun_at + 3..];
    let Some(at) = decl.find(&format!("{name}(")) else {
        return false;
    };
    decl[..at].trim_end().ends_with('.')
}

/// Step 0b of `try_resolve_ref`: the Kotlin extension functions named
/// `r.name` that are in scope at the call — declared in the calling file,
/// or bound by an explicit import of that name — unless the receiver is a
/// type the workspace declares a member of that name on.
fn kotlin_extension_in_scope(
    r: &cgg_core::RefRecord,
    caller_file: FileId,
    ext: &KotlinExtensions,
    direct_imports: &HashMap<String, Vec<String>>,
    by_qn: &HashMap<(String, String), CallableId>,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
) -> Option<Vec<CallableId>> {
    if ext.ids.is_empty() {
        return None;
    }
    let rh = r.receiver_hint.trim();
    let owner = rh.rsplit('.').next().unwrap_or(rh);
    if owner.starts_with(char::is_uppercase)
        && by_owner_method
            .get(&("kotlin".to_string(), owner.to_string(), r.name.clone()))
            .is_some_and(|c| !c.is_empty())
    {
        return None;
    }
    let mut out: Vec<CallableId> = ext
        .by_file
        .get(&(caller_file, r.name.clone()))
        .cloned()
        .unwrap_or_default();
    for qn in direct_imports.get(&r.name).into_iter().flatten() {
        if let Some(cid) = by_qn.get(&("kotlin".to_string(), kotlin_import_path(qn)))
            && ext.ids.contains(cid)
            && !out.contains(cid)
        {
            out.push(*cid);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// An imported Kotlin path as the plugin names the callable it denotes.
/// A member of a class's default companion object is imported through it
/// (`okhttp3.HttpUrl.Companion.toHttpUrl`) but extracted without it
/// (`okhttp3.HttpUrl.toHttpUrl`): the plugin gives a companion object no
/// scope of its own.
fn kotlin_import_path(path: &str) -> String {
    path.replace(".Companion.", ".")
}

/// Whether a receiver is Python's `super()` / Ruby's bare `super`.
///
/// It reaches the resolver verbatim, and `super()` starts with a
/// lowercase letter — so the duck-typing step read it as a variable name
/// and fanned out over every callable with that method name, the calling
/// class's own override included.
fn is_super_receiver(rh: &str) -> bool {
    let rh = rh.trim();
    rh == "super" || rh == "super()" || rh.starts_with("super(")
}

/// Drop the calling class's own methods from a `super()` candidate set.
///
/// `super().m()` means *explicitly not* this class's `m`. Resolving to it
/// produced a false edge and, where the subclass method also called the
/// one containing the `super()` call, a phantom cycle that reads as
/// infinite recursion. When the base is outside the analyzed tree this
/// leaves nothing, which is the correct answer — §8: never manufacture
/// an edge.
fn without_own_class(
    lang: &str,
    method: &str,
    caller_qn: Option<&str>,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    cids: Vec<CallableId>,
) -> Option<Vec<CallableId>> {
    let own = caller_qn.and_then(crate::names::owner_from_qn)?;
    let mine =
        by_owner_method.get(&(lang.to_string(), own.to_string(), method.to_string()));
    let Some(mine) = mine else { return Some(cids) };
    let kept: Vec<CallableId> = cids.into_iter().filter(|c| !mine.contains(c)).collect();
    if kept.is_empty() { None } else { Some(kept) }
}

/// Whether a Rust candidate turned up by the step-1d global by-simple-name
/// fallback or the step-5 duck-typed fan-out is actually reachable from
/// the caller, under Rust's own visibility rule. Neither fan-out consults
/// `visibility` today, so both were free to point at a private helper in
/// an unrelated file — measured at 797 medium edges on cgg-self, of
/// which 550 land in another file's `mod tests` (VERIFIED.md §1d).
///
/// A private (`visibility` empty) item is visible only within its own
/// file or from a *descendant* module of the one that declares it —
/// i.e. the candidate's module path must be an ancestor of (or equal
/// to) the caller's. `mod tests` is stricter still: even a `pub` helper
/// living under a `::tests::` segment is written for that module's own
/// use, so it is dropped unless the caller's own qualified name shares
/// that exact `::tests::` prefix.
///
/// A concrete trait-impl method (`<Type as Trait>::method`) is exempt
/// from both rules — its callability follows the trait it implements,
/// not the enclosing module, and cgg has no separate model for that.
/// So is a `macro_rules!` item: `rust.rs`'s `record_macro` (~758-785)
/// files it as `Vis::Private` with no real module-privacy meaning — a
/// macro is either exported (`#[macro_export]`, crate-global) or reached
/// through the `use` that brought it into scope, never through the
/// module-nesting rule an ordinary item follows — and marks it with the
/// `"macro"` attribute for exactly this kind of downstream distinction.
fn rust_candidate_reachable(
    graph: &Graph,
    cid: CallableId,
    caller_file: FileId,
    caller_qn: Option<&str>,
    direct_imports: &HashMap<String, Vec<String>>,
) -> bool {
    let Some(c) = graph.callables.get(&cid) else {
        return true;
    };
    if c.qualified_name.contains('<') && c.qualified_name.contains(" as ") {
        return true;
    }
    if c.attributes.iter().any(|a| a == "macro") {
        return true;
    }
    if let Some(cut) = c.qualified_name.find("::tests::") {
        let prefix = &c.qualified_name[..cut + "::tests::".len()];
        let caller_shares = caller_qn.is_some_and(|q| q.starts_with(prefix));
        // A glob (`use a::b::tests::*;`) or an exact single-item import
        // (`use a::b::tests::name;`) of this candidate's own tests
        // module makes it legitimately callable even from a file whose
        // own qualified name shares none of that prefix — the `::tests`
        // rule exists to catch an *accidental* same-simple-name fan-out,
        // not to override an import the caller wrote on purpose.
        let glob = format!("{prefix}*");
        // An import path is stored verbatim (`cross_file.rs`'s "use" arm
        // does not resolve `crate::`), so `use crate::builder::tests::*;`
        // is recorded as literally `"crate::builder::tests::*"` while the
        // candidate's own qualified name carries the *resolved* crate
        // root (`"vmm::builder::tests::..."`). Rewrite a leading `crate`
        // segment with the caller's own crate root (its qualified name's
        // first segment) before comparing, or this glob never matches —
        // measured on llmitm-v5: `default_kernel_cmdline` stayed short 4
        // of 17 edges until this rewrite, all four from files that
        // `use crate::builder::tests::*;`.
        let crate_root = caller_qn.and_then(|q| q.split("::").next());
        let imported = direct_imports.values().flatten().any(|p| {
            let rewritten = match (crate_root, p.as_str()) {
                (Some(root), "crate") => root.to_string(),
                (Some(root), rest) if rest.starts_with("crate::") => {
                    format!("{root}{}", &rest["crate".len()..])
                }
                _ => p.clone(),
            };
            rewritten == c.qualified_name || rewritten == glob
        });
        if !caller_shares && !imported {
            return false;
        }
    }
    // The normalized `vis` enum, not the raw `visibility` string: a
    // trait item's `visibility` is empty whether it is truly private or
    // merely inherited from a trait declared elsewhere, and the two
    // must not be conflated. `rust.rs` (~514-528) records `Vis::Unknown`
    // — deliberately, "an absent token here means inherited, not
    // private" — for exactly this case, so only `Vis::Private` is the
    // signal this rule may act on. `Vis::Unknown` (trait items) and
    // `Vis::Internal` (`pub(crate)`) both stay reachable.
    if c.vis == cgg_core::Vis::Private && c.file != caller_file {
        let candidate_module = c
            .qualified_name
            .rfind("::")
            .map(|i| &c.qualified_name[..i])
            .unwrap_or("");
        let caller_module = caller_qn
            .and_then(|q| q.rfind("::").map(|i| &q[..i]))
            .unwrap_or("");
        let is_ancestor = candidate_module.is_empty()
            || caller_module == candidate_module
            || caller_module.starts_with(&format!("{candidate_module}::"));
        if !is_ancestor {
            return false;
        }
    }
    true
}

/// Callables sharing one qualified name across several files — copies
/// (an Erlang module vendored in test data, a Zig `assert` repeated in a
/// test case, OCaml locals named alike) — narrowed by evidence to one: the
/// copy in the module the file imports (its path, segment-aligned), else
/// the caller's own file, else the one in its directory. `None`: copies
/// with no evidence singling one out — ambiguous, so nothing binds. Candidates under
/// distinct qualified names pass through; when some candidate sits in an
/// imported module exactly, only those stay.
fn narrow_copies(
    graph: &Graph,
    cids: &[CallableId],
    caller_file: FileId,
    module_files: &[String],
    exact_only: bool,
) -> Option<Vec<CallableId>> {
    let mut uniq: Vec<CallableId> = Vec::new();
    for c in cids {
        if !uniq.contains(c) {
            uniq.push(*c);
        }
    }
    let path_of = |c: &CallableId| {
        graph
            .callables
            .get(c)
            .and_then(|n| graph.files.get(&n.file))
            .map(|f| f.path.to_string_lossy().to_ascii_lowercase())
    };
    let in_module = |c: &CallableId| {
        path_of(c).is_some_and(|p| {
            let stem = match p.rsplit_once('.') {
                Some((st, ext)) if !ext.contains('/') => st,
                _ => p.as_str(),
            };
            module_files.iter().any(|m| {
                [stem, p.as_str()]
                    .iter()
                    .any(|x| *x == m || x.ends_with(&format!("/{m}")))
            })
        })
    };
    let exact: Vec<CallableId> = uniq.iter().copied().filter(in_module).collect();
    // Copies of the very module the call names (`cerl:f` in three
    // `cerl.erl` files), as opposed to same-named callables elsewhere.
    let module_copies = !exact.is_empty();
    if !exact.is_empty() || exact_only {
        uniq = exact;
    }
    let caller_dir = graph
        .files
        .get(&caller_file)
        .and_then(|f| f.path.parent().map(|p| p.to_path_buf()));
    let mut out: Vec<CallableId> = Vec::new();
    let mut seen_qn: Vec<&str> = Vec::new();
    for c in &uniq {
        let Some(n) = graph.callables.get(c) else {
            continue;
        };
        let qn = n.qualified_name.as_str();
        if seen_qn.contains(&qn) {
            continue;
        }
        seen_qn.push(qn);
        let group: Vec<CallableId> = uniq
            .iter()
            .copied()
            .filter(|x| {
                graph
                    .callables
                    .get(x)
                    .is_some_and(|m| m.qualified_name == qn)
            })
            .collect();
        if group.len() == 1 {
            out.push(group[0]);
            continue;
        }
        // A copy is a file: the callables one file declares under the
        // name (overloads, clauses of one function) stay together.
        let file_of = |x: &CallableId| graph.callables.get(x).map(|m| m.file);
        let mut files: Vec<FileId> = Vec::new();
        for f in group.iter().filter_map(file_of) {
            if !files.contains(&f) {
                files.push(f);
            }
        }
        let near: Vec<FileId> = files
            .iter()
            .copied()
            .filter(|f| {
                graph
                    .files
                    .get(f)
                    .and_then(|f| f.path.parent())
                    .map(|p| p.to_path_buf())
                    == caller_dir
            })
            .collect();
        // A copy kept as test data (`test/…_SUITE_data/src/cerl.erl`) is
        // called from beside it, which `near` covers; from anywhere else,
        // the one copy that is not test code.
        let is_test =
            |f: &FileId| graph.files.get(f).is_some_and(|f| f.test_role.is_some());
        let shipped: Vec<FileId> = if module_copies {
            files.iter().copied().filter(|f| !is_test(f)).collect()
        } else {
            Vec::new()
        };
        // One copy the evidence singles out; several in the caller's own
        // directory are as ambiguous as several anywhere, and stay unbound.
        let chosen = if let [one] = files.as_slice() {
            Some(*one)
        } else if files.contains(&caller_file) {
            Some(caller_file)
        } else if let [one] = near.as_slice() {
            Some(*one)
        } else if let [one] = shipped.as_slice() {
            Some(*one)
        } else {
            None
        };
        if let Some(f) = chosen {
            out.extend(group.iter().copied().filter(|x| file_of(x) == Some(f)));
        }
    }
    (!out.is_empty()).then_some(out)
}

/// A lookup by a bare qualified name (`f` for a language whose qualified
/// names carry no module).
enum BareQn {
    Found(Vec<CallableId>),
    /// Several callables carry the name and nothing tells them apart.
    Ambiguous,
    Missing,
}

/// The bare-name fallback through a module alias. One callable carrying
/// the name binds as before; several are copies, narrowed by the aliased
/// module's path, the caller's file and directory (`narrow_copies`) —
/// never picked by the order they were indexed in.
#[allow(clippy::too_many_arguments)]
fn bare_qn_lookup(
    graph: &Graph,
    lang: &str,
    name: &str,
    modules: &[&str],
    caller_file: FileId,
    by_qn: &HashMap<(String, String), CallableId>,
    by_simple: &HashMap<(String, String), Vec<CallableId>>,
    reexports: &HashMap<(String, String), String>,
    qn_collisions: &std::collections::HashSet<(String, String)>,
) -> BareQn {
    if !qn_collisions.contains(&(lang.to_string(), name.to_string())) {
        return match lookup_with_reexports(lang, name, by_qn, reexports) {
            Some(cid) => BareQn::Found(vec![cid]),
            None => BareQn::Missing,
        };
    }
    let copies: Vec<CallableId> = by_simple
        .get(&(lang.to_string(), name.to_string()))
        .map(|v| {
            v.iter()
                .copied()
                .filter(|c| {
                    graph
                        .callables
                        .get(c)
                        .is_some_and(|n| n.qualified_name == name)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut module_files: Vec<String> = Vec::new();
    for m in modules {
        let m = m.trim().to_ascii_lowercase();
        if !m.is_empty() {
            module_files.push(m.replace('.', "/"));
            module_files.push(m);
        }
    }
    match narrow_copies(graph, &copies, caller_file, &module_files, false) {
        Some(cids) => BareQn::Found(cids),
        None => BareQn::Ambiguous,
    }
}

/// What a bare call's scope is judged by: the call, where it is written,
/// and the calling file's import tables.
#[derive(Clone, Copy)]
struct BareScope<'s> {
    r: &'s cgg_core::RefRecord,
    caller: Option<CallableId>,
    caller_file: FileId,
    caller_qn: Option<&'s str>,
    module_aliases: &'s HashMap<String, String>,
    direct_imports: &'s HashMap<String, Vec<String>>,
    unqualified_prefixes: &'s [String],
    bases_by_owner: &'s HashMap<(String, String), Vec<String>>,
    /// Scala: the objects the file wildcard-imports, with their bases
    /// (computed once per file).
    scala_wild: &'s std::collections::BTreeSet<String>,
}

/// The type a C# extension method extends, from its signature
/// (`static Box<P> Wrap(this P _)` → `P`), bare of namespace and
/// type arguments, and whether that type is one of the method's own type
/// parameters (`static T Or<T>(this T x, …)` extends every type).
/// `None` for a method whose first parameter is not `this`.
fn csharp_extension_target(sig: &str) -> Option<(String, bool)> {
    let open = sig.find('(')?;
    let rest = sig[open + 1..].trim_start().strip_prefix("this ")?;
    let ty = rest.trim_start();
    let end = ty
        .char_indices()
        .find(|(_, c)| c.is_whitespace() || matches!(c, '<' | '[' | '?' | ',' | ')'))
        .map_or(ty.len(), |(i, _)| i);
    let ty = &ty[..end];
    let bare = ty.rsplit('.').next().unwrap_or(ty).to_string();
    if bare.is_empty() {
        return None;
    }
    // The method's own type parameters: `Name<T, U>(` just before `(`.
    let head = &sig[..open];
    let generic = head.ends_with('>')
        && head.rfind('<').is_some_and(|lt| {
            head[lt + 1..head.len() - 1]
                .split(',')
                .any(|p| p.trim() == bare)
        });
    Some((bare, generic))
}

/// `start` and every type it inherits from through recorded bases (bare/// `start` and every type it inherits from through recorded bases (bare
/// names, depth 8).
fn with_ancestors(
    lang: &str,
    start: Vec<String>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
) -> std::collections::BTreeSet<String> {
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut frontier = start;
    for _ in 0..8 {
        let mut next = Vec::new();
        for t in std::mem::take(&mut frontier) {
            if !seen.insert(t.clone()) {
                continue;
            }
            if let Some(bases) = bases_by_owner.get(&(lang.to_string(), t)) {
                next.extend(bases.iter().cloned());
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    seen
}

/// FINDINGS A6. Whether a bare call — no receiver, no `this`/`self::`
/// qualifier — written inside `caller_qn` can name `cand` unqualified.
///
/// * Python, Rust, Go, JS/TS, PHP, F# have no implicit-receiver call
///   form: a bare name reaches functions, never a method (`collect()` is
///   not `Dates::collect`; an F# static member needs its type).
/// * Perl resolves a bare sub name in the caller's own package; another
///   package's sub needs a qualifier or an import (`via_import`: the
///   candidate came from a module the file imports, Exporter's way in).
/// * Go resolves a bare name in the calling package — the files of its
///   directory — unless the file dot-imports another package.
/// * Java, C#: a bare name reaches constructors, members a static import
///   names, and the members of the calling class, the classes enclosing
///   it, their recorded bases, and — by dispatch — the overrides in
///   classes derived from them; never a method of an unrelated class.
///   Scala the same (WS14), with an object's members also reached through
///   `import a.Owner._` / `import a.Owner.m` (its qualified names carry
///   no package, so the owner is matched by its name).
/// * Unchanged where the bare-name scope depends on types cgg does not
///   have: Kotlin (a lambda with receiver, `apply {}`/`with(x) {}`),
///   Groovy (closure delegates — the Gradle DSL, Spock), C++
///   (argument-dependent lookup; namespace
///   functions are recorded like members); and Swift, Objective-C,
///   whose plugins record no base types, so an inherited member cannot be
///   told from an unrelated one.
/// * Dart, Ruby (WS8): a bare name reaches library-level / top-level
///   functions and the members of the calling class and the types it
///   inherits or mixes in (the bases its methods record); never a method
///   of an unrelated class.
///
/// A call written with a dot whose receiver the plugin could not capture
/// (`dot_call`) is not a bare call. Every other language is unchanged.
fn bare_call_reaches(
    graph: &Graph,
    lang: &str,
    scope: &BareScope<'_>,
    cand: CallableId,
    via_import: bool,
) -> bool {
    let BareScope {
        r,
        caller,
        caller_file,
        caller_qn,
        module_aliases,
        direct_imports,
        unqualified_prefixes,
        bases_by_owner,
        scala_wild,
    } = *scope;
    if r.dot_call {
        return true;
    }
    let Some(c) = graph.callables.get(&cand) else {
        return true;
    };
    // A definition nested in the calling function is in its lexical
    // scope, whatever its qualified name says (a Rust `fn` inside a
    // method is named under the method's type).
    if let Some(outer) = caller.and_then(|id| graph.callables.get(&id))
        && c.file == outer.file
        && outer.start_byte <= c.start_byte
        && c.end_byte <= outer.end_byte
    {
        return true;
    }
    // A constructor is not a member call: `new Foo(…)` / `Foo(…)` names
    // its class, and the candidates here carry the call's name.
    let member = matches!(
        c.kind,
        CallableKind::Method | CallableKind::Destructor | CallableKind::Property
    ) || (lang == "rust" && rust_type_owned(&c.qualified_name));
    match lang {
        "python" | "rust" | "javascript" | "typescript" | "php" | "fsharp" | "lua" => {
            !member
        }
        // Dart: a bare call is a library-level function, or (implicit
        // `this`) a member of the calling class or a type it inherits —
        // never a method of an unrelated class. Ruby the same: a bare
        // `helper` / `helper(x)` is `self`'s method — the class's, a
        // superclass's or an included module's — or a top-level one.
        "dart" | "ruby" => match owner_from_qn(&c.qualified_name) {
            // A top-level definition (Ruby records `def` as a method
            // whatever its scope).
            None => true,
            Some(_) if !member => true,
            Some(co) => caller_qn.and_then(owner_from_qn).is_some_and(|own| {
                with_ancestors(lang, vec![own.to_string()], bases_by_owner).contains(co)
            }),
        },
        "go" => {
            let dir = |f: &FileId| graph.files.get(f).and_then(|r| r.path.parent());
            !member
                && (module_aliases.contains_key(".") || dir(&c.file) == dir(&caller_file))
        }
        "perl" => {
            via_import
                || perl_package(&c.qualified_name) == caller_qn.map_or("", perl_package)
        }
        // Elixir (WS14): a bare call reaches the calling module's own
        // functions and those of a module it `import`s or `use`s — never
        // another module's function that happens to share the name
        // (`from` under `import Ecto.Query` is not a workspace `from`).
        "elixir" => {
            let module = |qn: &str| qn.rsplit_once("::").map(|(m, _)| m.to_string());
            let Some(target) = module(&c.qualified_name) else {
                return true;
            };
            caller_qn.and_then(module).as_deref() == Some(target.as_str())
                || unqualified_prefixes.contains(&target)
        }
        "java" | "csharp" | "scala" => {
            // Scala records every `def` as a function; a member is one
            // with an owner in its qualified name (`Owner::m`).
            let member = if lang == "scala" {
                c.qualified_name.contains("::")
            } else {
                member
            };
            if !member {
                // Scala 2 has no top-level `def`: one with no owner is a
                // package object's member (in a `package.scala`, reached
                // through an import of its package) or a member the
                // grammar lost its class for (a parse error), never
                // visible tree-wide.
                return lang != "scala"
                    || via_import
                        && graph.files.get(&c.file).is_some_and(|f| {
                            f.path.file_stem().is_some_and(|s| s == "package")
                        });
            }
            // Named by the file's static imports: `import static a.b.C.m;`,
            // `import static a.b.C.*;`, C# `using static A.C;`, a Kotlin
            // import of an object member.
            if direct_imports
                .get(&r.name)
                .is_some_and(|v| v.contains(&c.qualified_name))
                || crate::names::split_last_segment(&c.qualified_name).is_some_and(
                    |(owner, _)| unqualified_prefixes.iter().any(|p| p == owner),
                )
            {
                return true;
            }
            // Scala: `import a.b.Owner._` / `import a.b.Owner.m` brings an
            // object's members into scope; its qualified names carry no
            // package (`Owner::m`), so the owner is matched by name.
            if lang == "scala"
                && let Some(owner) = owner_from_qn(&c.qualified_name)
            {
                let owner_last = owner.rsplit("::").next().unwrap_or(owner);
                // The objects the file imports members of, and every type
                // they inherit from: `import DataTypeErrors.toSQLType`
                // reaches the `toSQLType` its trait `DataTypeErrorsBase`
                // declares.
                let mut direct: Vec<String> = Vec::new();
                if let Some(v) = direct_imports.get(&r.name) {
                    for path in v {
                        if let Some(o) = path
                            .rsplit_once('.')
                            .and_then(|(o, _)| o.rsplit('.').next())
                        {
                            direct.push(o.to_string());
                        }
                    }
                }
                let seen = if direct.is_empty() {
                    std::collections::BTreeSet::new()
                } else {
                    with_ancestors(lang, direct, bases_by_owner)
                };
                if scala_wild.contains(owner_last) {
                    return true;
                }
                if seen.contains(owner_last) {
                    return true;
                }
            }
            let Some(caller) = caller_qn else {
                return false;
            };
            let Some((owner_path, _)) =
                crate::names::split_last_segment(&c.qualified_name)
            else {
                return true;
            };
            // The calling class or one enclosing it, or a local function
            // of the caller itself.
            if caller == owner_path
                || caller
                    .strip_prefix(owner_path)
                    .is_some_and(|r| r.starts_with('.') || r.starts_with("::"))
            {
                return true;
            }
            // A base of the calling class (or of an enclosing one).
            let Some(target) = owner_from_qn(&c.qualified_name) else {
                return true;
            };
            // The calling class by its bare name is NOT enough: a partial
            // class split across files keeps its qualified name (matched
            // above), while another class of the same bare name — every
            // generated `_Fields` — is unrelated (FINDINGS R17). Below, a
            // class counts only when reached through a recorded base.
            let mut enclosing: Vec<String> = Vec::new();
            let mut cut = caller;
            while let Some((prefix, _)) = crate::names::split_last_segment(cut) {
                if let Some(o) = owner_from_qn(cut) {
                    enclosing.push(o.to_string());
                }
                cut = prefix;
            }
            // The classes reached from `start` through at least one
            // recorded base (bare names, depth 8). A start class is in the
            // result only if a base chain leads back to it.
            let ancestors = |start: Vec<String>| {
                let mut seen: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                let mut reached: std::collections::BTreeSet<String> =
                    std::collections::BTreeSet::new();
                let mut frontier = start;
                for _ in 0..8 {
                    let mut next = Vec::new();
                    for t in std::mem::take(&mut frontier) {
                        if !seen.insert(t.clone()) {
                            continue;
                        }
                        if let Some(bases) = bases_by_owner.get(&(lang.to_string(), t)) {
                            for b in bases {
                                reached.insert(b.clone());
                                next.push(b.clone());
                            }
                        }
                    }
                    if next.is_empty() {
                        break;
                    }
                    frontier = next;
                }
                reached
            };
            // Inherited: the candidate's class is a base of the caller's
            // class or of an enclosing one.
            if ancestors(enclosing.clone()).contains(target) {
                return true;
            }
            // Dispatched: the candidate's class derives from the caller's
            // (or an enclosing) class, so the bare call — to a method the
            // base declares, often abstractly — reaches the override.
            let below = ancestors(vec![target.to_string()]);
            enclosing.iter().any(|e| below.contains(e))
        }
        _ => true,
    }
}

/// A Rust qualified name whose owner is a type (`Type::f`, `<T as
/// Trait>::f`) rather than a module (`krate::module::f`).
fn rust_type_owned(qn: &str) -> bool {
    let Some((prefix, _)) = crate::names::split_last_segment(qn) else {
        return false;
    };
    prefix.ends_with('>')
        || prefix
            .rsplit("::")
            .next()
            .is_some_and(|o| o.starts_with(|c: char| c.is_uppercase()))
}

/// The package a Perl sub belongs to: its qualified name minus the sub,
/// with the implicit `main` spelled as the empty package.
fn perl_package(qn: &str) -> &str {
    match qn.rsplit_once("::") {
        Some(("main", _)) | None => "",
        Some((p, _)) => p,
    }
}

fn try_resolve_ref(
    graph: &Graph,
    caller_file: FileId,
    lang: &str,
    r: &cgg_core::RefRecord,
    // The calling file's import bindings judged against the workspace
    // (FINDINGS A1). `external_out` is set when the call is refused
    // because it is written through a dependency.
    heads: &crate::heads::FileHeads,
    vocab: &crate::heads::WorkspaceVocab,
    external_out: &mut Option<String>,
    direct_imports: &HashMap<String, Vec<String>>,
    module_aliases: &HashMap<String, String>,
    unqualified_prefixes: &[String],
    scoped_simple: &ScopedSimple<'_>,
    // The imported modules as path forms (see `narrow_copies`).
    module_files: &[String],
    by_qn: &HashMap<(String, String), CallableId>,
    by_simple: &HashMap<(String, String), Vec<CallableId>>,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    by_full_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    std_owner_trait_impls: &HashMap<(String, String, String), Vec<CallableId>>,
    reexports: &HashMap<(String, String), String>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
    // The reverse of `bases_by_owner` (see `derived_index`).
    derived_by_base: &HashMap<(String, String), Vec<String>>,
    // The names a Zig `@import("name")` of a package can resolve to in the
    // tree: every Zig file's stem and every directory holding Zig files.
    zig_packages: &std::collections::HashSet<String>,
    known_owners: &std::collections::HashSet<(String, String)>,
    stub_ids: &std::collections::HashSet<CallableId>,
    signatures: &HashMap<CallableId, &str>,
    var_types: &HashMap<String, String>,
    caller_qn: Option<&str>,
    // The callable the call is written in.
    caller_id: Option<CallableId>,
    fanout_cap: usize,
    // First path segments seen in Rust qualified names (see its
    // construction in `resolve`) — used to tell an unresolved workspace
    // path from an external crate before falling into step 5's fan-out.
    rust_path_heads: &RustPathVocab,
    workspace_owner_names: &std::collections::HashSet<String>,
    // Languages that record every class declaration (see its
    // construction in `resolve`).
    class_record_langs: &std::collections::BTreeSet<String>,
    // A Java anonymous class method's bases, which are not its owner's.
    own_bases: &HashMap<CallableId, Vec<String>>,
    kotlin_extensions: &KotlinExtensions,
    // Module-scoped resolution (FINDINGS A4/A5): the run's import→file
    // index and the calling file's module bindings.
    modix: &crate::modules::ModuleIndex<'_>,
    file_mods: &crate::modules::FileModules,
    // Library scope for languages whose imports name files (WS14).
    libix: &crate::libscope::LibIndex<'_>,
    // PHP namespaces and `use` tables (WS14).
    phpix: &crate::libscope::PhpIndex,
    // Scala: wildcard-imported objects and their bases (WS14).
    scala_wild: &std::collections::BTreeSet<String>,
    // Qualified names more than one callable carries; see `resolve`.
    qn_collisions: &std::collections::HashSet<(String, String)>,
    // `(file, qualified name)` → the overload set it declares.
    overloads: &HashMap<(FileId, &str), Vec<CallableId>>,
    // Set when the call goes through a workspace module that provides
    // nothing it can name: the import as written.
    refused_module: &mut Option<String>,
    // Set when the binding came from an exact lookup inside the module.
    module_certain: &mut bool,
    // Set to the candidate count when the fan-out cap rejected a
    // non-empty set, so the caller can record the drop instead of
    // leaving the site looking uncalled.
    capped: &mut u32,
    // Set when the call names a class cgg knows but that declares no
    // initializer, so the caller can say that rather than "no candidate".
    no_ctor: &mut bool,
) -> Option<Vec<CallableId>> {
    // A paren-less member site (`b.size`) binds only through a receiver
    // typed by `type_hints` (rewritten to a type name), `this`/`super`, or
    // as a bare name; through an untyped value nothing can bind it (it is
    // never fanned out by name), so it is not looked up at all.
    if r.paren_less {
        let rh = r.receiver_hint.trim();
        if !rh.is_empty()
            && !matches!(rh, "this" | "super" | "self")
            && !rh.starts_with(char::is_uppercase)
            && !r.typed_receiver
        {
            return None;
        }
    }
    // Descriptor / interface-definition languages (Smithy, Protobuf,
    // GraphQL). Their references are shape/message/type names that are
    // effectively unique identifiers within the model, so a global
    // by-simple-name match is both safe and the right resolution — it
    // links references across files in the same namespace/package (and
    // same-file edges already emitted by the intra-file linker are
    // deduplicated by the caller). Bounded to ≤4 candidates to stay
    // conservative if a name genuinely collides.
    if matches!(
        lang,
        "smithy" | "proto" | "graphql" | "openapi" | "asyncapi"
    ) {
        if let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone()))
            && !cids.is_empty()
            && cids.len() <= 4
        {
            return Some(cids.clone());
        }
        return None;
    }

    // `super.m()` / `super().m()` names the caller's base types, nothing
    // else. When the caller's class declares its bases, the call is the
    // first `m` up that chain (`resolve_via_bases` never looks at the
    // class itself). Where the plugin records every class, a chain that
    // finds none has left the tree (`super.onCreate()` in a class
    // extending a framework type): the call is into the dependency,
    // refused and recorded by the caller as `super-base-out-of-graph`,
    // where name matching bound it to every unrelated `onCreate` in the
    // workspace. Elsewhere a miss, and a class with no recorded bases,
    // keep the name-matching reading below.
    if is_super_receiver(&r.receiver_hint)
        && let Some(own) = caller_qn.and_then(owner_from_qn)
    {
        let own = own.split('<').next().unwrap_or(own).trim();
        // In an anonymous class body `super` is the anonymous class's
        // base: its own member, or one that base inherits.
        if let Some(bases) = caller_id.and_then(|c| own_bases.get(&c)) {
            for b in bases {
                if let Some(cids) = by_owner_method
                    .get(&(lang.to_string(), b.clone(), r.name.clone()))
                    .filter(|v| !v.is_empty())
                {
                    return Some(cids.clone());
                }
                if let Some(cids) =
                    resolve_via_bases(lang, b, &r.name, by_owner_method, bases_by_owner)
                {
                    return Some(cids);
                }
            }
            return None;
        }
        match bases_by_owner.get(&(lang.to_string(), own.to_string())) {
            Some(b) if !b.is_empty() => {
                let found = resolve_via_bases(
                    lang,
                    own,
                    &r.name,
                    by_owner_method,
                    bases_by_owner,
                );
                if found.is_some() || class_record_langs.contains(lang) {
                    return found;
                }
            }
            // A recorded class that names no base extends the language's
            // root type (`Object`, `object`): nothing in the tree.
            Some(_) if class_record_langs.contains(lang) => return None,
            _ => {}
        }
    }

    // Step 0b: a Kotlin extension function in scope. `expr.ext()` calls an
    // extension declared in the calling file or brought in by `import
    // pkg.ext`, whatever the receiver expression is — Kotlin needs no more
    // to find it, and the receiver's type, which cgg usually does not
    // know, only decides between it and a member of the same name. A
    // known receiver type that declares the member keeps it (members win)
    // through the steps below; otherwise the extension is the reading.
    // Before receivers other than a plain identifier were recorded, these
    // calls were read as bare names and reached the extension by name.
    if lang == "kotlin"
        && !r.receiver_hint.is_empty()
        && !is_super_receiver(&r.receiver_hint)
        && let Some(cids) = kotlin_extension_in_scope(
            r,
            caller_file,
            kotlin_extensions,
            direct_imports,
            by_qn,
            by_owner_method,
        )
    {
        return Some(cids);
    }

    // Step 0: a call written through a dependency — a module, package,
    // namespace or class the file imports from outside the tree, or a
    // bare name imported from one (`external_out` says which). Every step below would match it by
    // NAME against the workspace (`requests.post` -> `app.util.post`,
    // `StringUtils.isBlank` -> a local `StringUtils`), which is a
    // coincidence of spelling, not a call. The exact-path steps cannot
    // succeed for it either: nothing in the tree lives under that path.
    if let Some(module) = heads.external_head(lang, r, vocab) {
        // C++: a namespace opened through a macro (`namespace
        // KWSYS_NAMESPACE {`, `ASMJIT_BEGIN_SUB_NAMESPACE(a64)`) never
        // appears in a qualified name, so its head looks undeclared. A name
        // the file's in-tree include closure declares may be that
        // namespace's member, and keeps the reading cgg had before.
        let declared_by_include = lang == "cpp" && direct_imports.contains_key(&r.name);
        // Java/Kotlin/C# resolve a bare name to a member of the calling
        // class (or an inherited one) before any import, so a static or
        // function import of the same name does not reach it there — and
        // the call is that member, not whatever else carries the name.
        let member_of_caller = if r.receiver_hint.is_empty()
            && matches!(lang, "java" | "kotlin" | "scala" | "csharp")
        {
            caller_qn.and_then(owner_from_qn).and_then(|own| {
                by_owner_method
                    .get(&(lang.to_string(), own.to_string(), r.name.clone()))
                    .filter(|v| !v.is_empty())
                    .cloned()
                    .or_else(|| {
                        resolve_via_bases(
                            lang,
                            own,
                            &r.name,
                            by_owner_method,
                            bases_by_owner,
                        )
                    })
            })
        } else {
            None
        };
        if member_of_caller.is_some() {
            return member_of_caller;
        }
        if !declared_by_include {
            *external_out = Some(module);
            return None;
        }
    }

    // Step 0c: a call written through an import of a workspace module
    // (`svc.f()`, `legacy.run()`, `helpers.f()`, `pkg.mod.Cls().m()`,
    // `jobs.NewRunner().M()`) is looked up in the file that module is,
    // through its re-exports — never by name across the tree. A module
    // that provides nothing the call can name is a refusal, recorded by
    // the caller; it does not fall through to the name matching below.
    if matches!(lang, "javascript" | "typescript" | "python" | "go" | "lua")
        && !heads.locals.shadows_receiver_head(r)
        && let Some(scoped) = file_mods.resolve(
            modix,
            lang,
            caller_file,
            r,
            caller_id
                .and_then(|id| graph.callables.get(&id))
                .map(|c| c.signature_hint.as_str()),
            constructor_names(lang),
            &|owner: &str, method: &str| {
                resolve_via_bases(lang, owner, method, by_owner_method, bases_by_owner)
            },
        )
    {
        match scoped {
            crate::modules::Scoped::Bound { cids, certain } => {
                *module_certain = certain;
                return Some(cids);
            }
            crate::modules::Scoped::Refused(module) => {
                *refused_module = Some(module);
                return None;
            }
            crate::modules::Scoped::Ambiguous => return None,
            crate::modules::Scoped::NoCtor => {
                *no_ctor = true;
                return None;
            }
            crate::modules::Scoped::External(module) => {
                *external_out = Some(module);
                return None;
            }
        }
    }
    // WS14: a Dart call through the library scope — the file's own
    // library, its imports' exported namespaces (`show`/`hide`), an
    // import prefix — binds there, or is refused / external; never
    // matched by name across the tree.
    if lang == "dart"
        && !heads.locals.shadows_receiver_head(r)
        && let Some(scoped) = libix.resolve_dart(caller_file, r, caller_qn)
    {
        match scoped {
            crate::modules::Scoped::Bound { cids, certain } => {
                *module_certain = certain && cids.len() == 1;
                return Some(cids);
            }
            crate::modules::Scoped::Refused(module) => {
                *refused_module = Some(module);
                return None;
            }
            crate::modules::Scoped::Ambiguous => return None,
            crate::modules::Scoped::NoCtor => {
                *no_ctor = true;
                return None;
            }
            crate::modules::Scoped::External(module) => {
                *external_out = Some(module);
                return None;
            }
        }
    }
    // WS14: a Zig call through file-level `@import`s and aliases
    // (`std.debug.print()`) is the function the file the path ends in
    // declares; through a dependency, external.
    if lang == "zig"
        && !heads.locals.shadows_receiver_head(r)
        && let Some(scoped) = libix.resolve_zig(caller_file, r)
    {
        match scoped {
            crate::modules::Scoped::Bound { cids, certain } => {
                *module_certain = certain && cids.len() == 1;
                return Some(cids);
            }
            crate::modules::Scoped::External(module) => {
                *external_out = Some(module);
                return None;
            }
            crate::modules::Scoped::Refused(module) => {
                *refused_module = Some(module);
                return None;
            }
            crate::modules::Scoped::Ambiguous => return None,
            crate::modules::Scoped::NoCtor => return None,
        }
    }
    // WS14b: a Perl method call on the caller's invocant (`$self->m()`)
    // is the caller's package's `m` or one it inherits; inherited from a
    // base outside the tree, external.
    if lang == "perl"
        && let Some(scoped) = libix.resolve_perl(r, caller_qn)
    {
        match scoped {
            crate::modules::Scoped::Bound { cids, certain } => {
                *module_certain = certain && cids.len() == 1;
                return Some(cids);
            }
            crate::modules::Scoped::External(module) => {
                *external_out = Some(module);
                return None;
            }
            crate::modules::Scoped::Refused(module) => {
                *refused_module = Some(module);
                return None;
            }
            crate::modules::Scoped::Ambiguous | crate::modules::Scoped::NoCtor => {
                return None;
            }
        }
    }
    // WS14: PHP namespaces and `use` imports. `X::m()` is the method of
    // the class `X` names through them (a dependency's class is
    // external); `parent::m()` the caller's base's; `f()` the namespace's
    // function, else the global one.
    if lang == "php"
        && let Some(scoped) = phpix.resolve(caller_file, r, caller_qn)
    {
        match scoped {
            crate::modules::Scoped::Bound { cids, certain } => {
                *module_certain = certain && cids.len() == 1;
                return Some(cids);
            }
            crate::modules::Scoped::External(module) => {
                *external_out = Some(module);
                return None;
            }
            crate::modules::Scoped::Refused(module) => {
                *refused_module = Some(module);
                return None;
            }
            crate::modules::Scoped::Ambiguous => return None,
            crate::modules::Scoped::NoCtor => return None,
        }
    }
    // Erlang `m:f(…)` names module `m` (FINDINGS R14): its `f`, the copy
    // the evidence singles out when the module is vendored more than once
    // — never another module's `f`.
    if lang == "erlang" && !r.receiver_hint.trim().is_empty() {
        let m = r.receiver_hint.trim();
        let simple = r.name.rsplit(':').next().unwrap_or(&r.name);
        if m.chars().all(|c| c.is_alphanumeric() || c == '_') {
            let qn = format!("{m}:{simple}");
            let exact: Vec<CallableId> = by_simple
                .get(&(lang.to_string(), simple.to_string()))
                .map(|v| {
                    v.iter()
                        .copied()
                        .filter(|c| {
                            graph
                                .callables
                                .get(c)
                                .is_some_and(|n| n.qualified_name == qn)
                        })
                        .collect()
                })
                .unwrap_or_default();
            if exact.is_empty() {
                return None;
            }
            let found = narrow_copies(
                graph,
                &exact,
                caller_file,
                &[m.to_ascii_lowercase()],
                false,
            )?;
            // One function per arity: the call's argument count picks it,
            // and the first clause stands for the function.
            let found = crate::intra_file::first_clause_per_arity(graph, &found, r.arity);
            *module_certain = found.len() == 1;
            return Some(found);
        }
    }
    // JS/TS qualified names carry no module (`spawnJob`, `Runner.start`):
    // every lookup of one below by exact name is a lookup by bare name,
    // answered by whichever same-named callable was indexed last.
    let bare_qns = matches!(lang, "javascript" | "typescript");

    // WS14b: a Scala bare call is a member of the type around it or one
    // it inherits, its bases resolved through the declaring file's
    // imports and package (not by simple name); a companion object's
    // members are not the class's.
    if lang == "scala"
        && (r.receiver_hint.is_empty() || r.receiver_hint == "this")
        && !r.dot_call
    {
        match libix.resolve_scala_bare(caller_file, r) {
            Some(crate::modules::Scoped::Bound { cids, certain }) => {
                *module_certain = certain;
                return Some(cids);
            }
            Some(crate::modules::Scoped::External(module))
                if r.receiver_hint.is_empty() =>
            {
                *external_out = Some(module);
                return None;
            }
            _ => {}
        }
    }
    // Step 1: direct import match — `foo()` where `foo` was
    // imported.
    if r.receiver_hint.is_empty() {
        let bare_scope = BareScope {
            r,
            caller: caller_id,
            caller_file,
            caller_qn,
            module_aliases,
            direct_imports,
            unqualified_prefixes,
            bases_by_owner,
            scala_wild,
        };
        // Java/C#: a bare name is a member of the calling class or one it
        // inherits before any static import (see `member_of_calling_class`).
        // Ruby (WS8): a bare `helper` is `self`'s — the class's, a
        // superclass's or an included module's.
        if matches!(lang, "java" | "csharp" | "ruby")
            && let Some(caller) = caller_qn
            && let Some(member) = member_of_calling_class(
                graph,
                lang,
                &r.name,
                caller,
                caller_file,
                by_owner_method,
                bases_by_owner,
                derived_by_base,
                fanout_cap,
                capped,
            )
        {
            return member;
        }
        // An exact Java/C# lookup reaches the overload set it lands in.
        let with_overloads = |cid: CallableId| -> Vec<CallableId> {
            if !matches!(lang, "java" | "csharp") {
                return vec![cid];
            }
            graph
                .callables
                .get(&cid)
                .and_then(|c| overloads.get(&(c.file, c.qualified_name.as_str())))
                .cloned()
                .unwrap_or_else(|| vec![cid])
        };
        if let Some(qns) = direct_imports.get(&r.name) {
            let mut cids: Vec<CallableId> = Vec::new();
            for qn in qns {
                if let Some(cid) = lookup_with_reexports(lang, qn, by_qn, reexports) {
                    for c in with_overloads(cid) {
                        if !cids.contains(&c) {
                            cids.push(c);
                        }
                    }
                }
            }
            if !cids.is_empty() {
                return Some(cids);
            }
        }
        // Also check module_aliases for bare calls — handles
        // Kotlin/Java `import com.example.Foo` + `Foo()` constructor
        // and `import com.example.helper` + `helper()` top-level fn.
        if let Some(target) = module_aliases.get(&r.name) {
            // Try target.name (the full path IS the callable)
            if let Some(cid) = lookup_with_reexports(lang, target, by_qn, reexports) {
                return Some(vec![cid]);
            }
            // Zig `const assert = std.debug.assert;` (FINDINGS R19): the
            // alias names a declaration of a module, followed through the
            // file's other aliases to its `@import`. The declaration found
            // in that module's file is the call, at the confidence of an
            // exact lookup. A module the tree does not hold (`std` outside
            // the Zig repository itself) is a dependency: the call is
            // external, never matched by name to a workspace `assert`.
            if lang == "zig"
                && let Some((path, member)) = target.rsplit_once('.')
            {
                let mut full = path.to_string();
                for _ in 0..4 {
                    let head = full.split('.').next().unwrap_or("").to_string();
                    match module_aliases.get(&head) {
                        Some(m) if m.split('.').next() != Some(head.as_str()) => {
                            full = format!("{m}{}", &full[head.len()..]);
                        }
                        _ => break,
                    }
                }
                let head = full.split('.').next().unwrap_or("");
                let lower = full.to_ascii_lowercase();
                let files = [lower.replace('.', "/"), lower.clone()];
                let same: Vec<CallableId> = by_simple
                    .get(&(lang.to_string(), member.to_string()))
                    .map(|v| {
                        v.iter()
                            .copied()
                            .filter(|c| {
                                graph
                                    .callables
                                    .get(c)
                                    .is_some_and(|n| n.qualified_name == member)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(found) =
                    narrow_copies(graph, &same, caller_file, &files, true)
                {
                    *module_certain = true;
                    return Some(found);
                }
                if let Some(import) = module_aliases.get(head) {
                    let base = import.strip_suffix(".zig").unwrap_or(import);
                    let base = base.rsplit('/').next().unwrap_or(base);
                    if !zig_packages.contains(base) {
                        *external_out = Some(import.clone());
                        return None;
                    }
                }
            }
            // Try just the name itself as a qualified name. Several
            // callables carrying it are narrowed by evidence (see
            // `narrow_copies`), never picked by order.
            // Zig `const f = link.g;`: `g` in the file `link` aliases
            // (`const link = @import("link.zig")`).
            let (name, module): (&str, String) = match (lang, target.rsplit_once('.')) {
                ("zig", Some((path, member))) => {
                    let head = path.split('.').next().unwrap_or(path);
                    let expanded = module_aliases
                        .get(head)
                        .map(|m| format!("{m}{}", &path[head.len()..]))
                        .unwrap_or_else(|| path.to_string());
                    (member, expanded)
                }
                _ => (r.name.as_str(), target.clone()),
            };
            if !bare_qns {
                match bare_qn_lookup(
                    graph,
                    lang,
                    name,
                    &[target.as_str(), module.as_str()],
                    caller_file,
                    by_qn,
                    by_simple,
                    reexports,
                    qn_collisions,
                ) {
                    BareQn::Found(cids) => return Some(cids),
                    BareQn::Ambiguous => return None,
                    BareQn::Missing => {}
                }
            }
        }
        // Scala (WS14): `import a.b.Obj.m` / `import a.b.Obj._` bring an
        // object's members — declared on it or inherited from its traits —
        // into scope. Scala qualified names carry no package (`Obj::m`), so
        // the object is found by its name and its member by the owner
        // index, then up its declared bases.
        if lang == "scala" {
            // A member the calling class declares or inherits outranks a
            // wildcard or explicit import of the name (SLS 2: inherited
            // definitions bind before imports): `get(p)` inside a `Params`
            // subclass is `Params.get`, not an imported `functions.get`.
            if let Some(own) = caller_qn.and_then(owner_from_qn) {
                let own_last = own.rsplit("::").next().unwrap_or(own);
                let inherited = by_owner_method
                    .get(&(lang.to_string(), own_last.to_string(), r.name.clone()))
                    .filter(|v| !v.is_empty())
                    .cloned()
                    .or_else(|| {
                        resolve_via_bases(
                            lang,
                            own_last,
                            &r.name,
                            by_owner_method,
                            bases_by_owner,
                        )
                    });
                if let Some(ids) = inherited {
                    return Some(ids);
                }
            }
            // `(object, the package it is imported from)`.
            let mut owners: Vec<(&str, &str)> = Vec::new();
            if let Some(paths) = direct_imports.get(&r.name) {
                for path in paths {
                    if let Some((obj_path, _)) = path.rsplit_once('.')
                        && let Some((pkg, o)) = obj_path.rsplit_once('.')
                    {
                        owners.push((o, pkg));
                    }
                }
            }
            for p in unqualified_prefixes {
                if let Some((pkg, o)) = p.rsplit_once('.') {
                    owners.push((o, pkg));
                }
            }
            let mut found: Vec<CallableId> = Vec::new();
            for (o, pkg) in owners {
                // The object the import names lives in that package: a
                // same-named class elsewhere (`case class Cast` beside
                // `object Cast`) is not it.
                let dir = pkg.replace('.', "/").to_ascii_lowercase();
                let in_pkg = |c: &CallableId| {
                    graph
                        .callables
                        .get(c)
                        .and_then(|n| graph.files.get(&n.file))
                        .is_some_and(|f| {
                            f.path.to_string_lossy().to_ascii_lowercase().contains(&dir)
                        })
                };
                let ids = by_owner_method
                    .get(&(lang.to_string(), o.to_string(), r.name.clone()))
                    .map(|v| v.iter().copied().filter(in_pkg).collect::<Vec<_>>())
                    .filter(|v| !v.is_empty())
                    .or_else(|| {
                        resolve_via_bases(
                            lang,
                            o,
                            &r.name,
                            by_owner_method,
                            bases_by_owner,
                        )
                    })
                    .unwrap_or_default();
                for id in ids {
                    if !found.contains(&id) {
                        found.push(id);
                    }
                }
            }
            if !found.is_empty() && found.len() <= fanout_cap.max(1) {
                return Some(found);
            }
        }
        // Step 1b: bare-name lookup via unqualified-import prefixes
        // (Haskell `import Data.Map` + `lookup`, OCaml `open Foo` + `bar`,
        // Elixir `import Foo` + `bar`, F# `open System.IO` + `File`, …).
        for prefix in unqualified_prefixes {
            for joiner in [".", "::"] {
                let qn = format!("{prefix}{joiner}{}", r.name);
                if let Some(cid) = lookup_with_reexports(lang, &qn, by_qn, reexports) {
                    return Some(with_overloads(cid));
                }
            }
        }
        // Zig resolves a bare name in the file's own containers first; a
        // name the file declares is never another file's (FINDINGS R13).
        if lang == "zig"
            && by_simple
                .get(&(lang.to_string(), r.name.clone()))
                .is_some_and(|v| {
                    v.iter().any(|c| {
                        graph
                            .callables
                            .get(c)
                            .is_some_and(|n| n.file == caller_file)
                    })
                })
        {
            return None;
        }
        // Step 1c: scoped by-simple lookup — restricted to callables
        // defined in files whose path matches one of this file's
        // import prefixes. This carries Haskell/OCaml/Dart over the
        // gap where the plugin omits the module prefix from
        // qualified_name. Cap candidates at 8 to bound noise.
        if let Some(cids) = scoped_simple.get_capped(&r.name, 8) {
            // FINDINGS A6: only what a bare name can reach from here.
            let reach: Vec<CallableId> = cids
                .iter()
                .copied()
                .filter(|c| bare_call_reaches(graph, lang, &bare_scope, *c, true))
                .collect();
            if !reach.is_empty() {
                // Copies of one qualified name in several matched files are
                // narrowed by evidence, never all bound; with none, the call
                // is ambiguous and refused (recorded by the caller). Dart
                // imports name files, so only a candidate in one of them
                // counts (FINDINGS R16).
                return narrow_copies(
                    graph,
                    &reach,
                    caller_file,
                    module_files,
                    lang == "dart",
                );
            }
        }
        // Step 1d: global by-simple fallback. Last resort — only when
        // the file has at least one import and the simple name is
        // unique-ish (≤3 candidates). Skip stdlib-ish names.
        let has_imports = !direct_imports.is_empty()
            || !module_aliases.is_empty()
            || !unqualified_prefixes.is_empty();
        if has_imports && !r.paren_less {
            let is_stdlib = cgg_core::stdlib::stdlib_names(lang)
                .is_some_and(|s| s.contains(r.name.as_str()));
            if !is_stdlib
                && let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone()))
                && !cids.is_empty()
                && cids.len() <= 3
            {
                let cids: Vec<CallableId> = if lang == "rust" {
                    cids.iter()
                        .copied()
                        .filter(|c| {
                            rust_candidate_reachable(
                                graph,
                                *c,
                                caller_file,
                                caller_qn,
                                direct_imports,
                            )
                        })
                        .collect()
                } else {
                    cids.clone()
                };
                // FINDINGS A6: a bare name never reaches a method of an
                // unrelated type, nor another Perl package's sub. Decided
                // after the ≤3 test, which stays a property of the name.
                let cids: Vec<CallableId> = cids
                    .into_iter()
                    .filter(|c| bare_call_reaches(graph, lang, &bare_scope, *c, false))
                    .collect();
                if !cids.is_empty() {
                    return Some(cids);
                }
            }
        }
    } else {
        // Step 2: attribute call `mod.fn()` where `mod` is aliased.
        // receiver_hint is the full receiver expression (e.g., "mod"
        // or "mod.sub"). Take its first segment to match module alias.
        let first = r.receiver_hint.split(['.', ':']).next().unwrap_or("");
        if let Some(module) = module_aliases
            .get(first)
            .filter(|_| !heads.locals.shadows_receiver_head(r))
        {
            // Rebuild the full target path. For `mod.fn()` with alias
            // `mod=helpers` -> `helpers.fn`. For `mod.sub.fn()` ->
            // `helpers.sub.fn`.
            let rest = r.receiver_hint.strip_prefix(first).unwrap_or("");
            let qn = format!("{module}{rest}.{}", r.name);
            if let Some(cid) = lookup_with_reexports(lang, &qn, by_qn, reexports) {
                return Some(vec![cid]);
            }
            // Rust path joiner.
            let qn2 = format!("{module}{}::{}", rest.replace('.', "::"), r.name);
            if let Some(cid) = lookup_with_reexports(lang, &qn2, by_qn, reexports) {
                return Some(vec![cid]);
            }
            // Definitions in some languages don't carry a module prefix,
            // so try the bare name — only when exactly one callable
            // carries it. Several is a choice `by_qn` would make by
            // indexing order (a decoy in `src/zzz/` won). JS/TS module
            // bindings were settled by step 0c.
            if !bare_qns {
                let path = format!("{module}{rest}");
                match bare_qn_lookup(
                    graph,
                    lang,
                    &r.name,
                    &[module.as_str(), path.as_str()],
                    caller_file,
                    by_qn,
                    by_simple,
                    reexports,
                    qn_collisions,
                ) {
                    BareQn::Found(cids) => return Some(cids),
                    BareQn::Ambiguous => return None,
                    BareQn::Missing => {}
                }
            }
        }

        // Step 3: qualified-path call `foo::bar::baz()` (Rust) or
        // `foo.bar.baz()` (Python / Go / C#). The receiver_hint is
        // already the joined path. Try both the Rust and the dotted
        // form.
        let mut rh_buf = r.receiver_hint.trim().to_string();
        let rh = rh_buf.as_str();
        if !rh.is_empty() && !bare_qns {
            // Direct paths in both joiners.
            let direct_dot = format!("{rh}.{}", r.name);
            if let Some(cid) = lookup_with_reexports(lang, &direct_dot, by_qn, reexports)
            {
                return Some(vec![cid]);
            }
            let direct = format!("{rh}::{}", r.name);
            if let Some(cid) = lookup_with_reexports(lang, &direct, by_qn, reexports) {
                // `by_qn` keeps one callable per qualified name. C++
                // often has two: the real definition and a test stub,
                // or two overloads the signature key did not separate.
                // A typed call `kernel->add_module` must reach every
                // one of them; step 4's owner-method index has the full
                // set. Returning the single `by_qn` survivor here would
                // send every such call at whichever file was indexed
                // last.
                let several = lang == "cpp"
                    && by_owner_method
                        .get(&(lang.to_string(), rh.to_string(), r.name.clone()))
                        .is_some_and(|v| v.len() > 1);
                if !several {
                    return Some(vec![cid]);
                }
            }
            // Step 3b: Rust intra-crate retry — when `mod::fn()` lives
            // inside `crate::other::Type::method`, the qualified name
            // we want is `crate::mod::fn`. Walk every prefix of the
            // caller's qualified name, prepending it to `<rh>::<name>`,
            // until we hit a match. Shortest prefix first (just the
            // crate) is the most common hit. Limit to `::` joiner —
            // dot-joined languages don't have this resolution rule.
            if lang == "rust"
                && let Some(qn) = caller_qn
            {
                let segs: Vec<&str> = qn.split("::").collect();
                // Try crate-only first, then progressively longer
                // prefixes. Stop before the last segment (that's
                // the callable's own name).
                for i in 1..segs.len() {
                    let prefix = segs[..i].join("::");
                    let candidate = format!("{prefix}::{rh}::{}", r.name);
                    if let Some(cid) =
                        lookup_with_reexports(lang, &candidate, by_qn, reexports)
                    {
                        return Some(vec![cid]);
                    }
                }
            }
            // If the head segment is imported as something else, rewrite.
            // e.g., `use foo as f; f::bar()` -> receiver=f, name=bar -> foo::bar.
            if let Some(first) = rh.split(['.', ':']).next()
                && let Some(qns) = direct_imports.get(first)
            {
                for base in qns {
                    let rest = rh.strip_prefix(first).unwrap_or("");
                    let rewritten_colon =
                        format!("{base}{}::{}", rest.replace('.', "::"), r.name);
                    if let Some(cid) =
                        lookup_with_reexports(lang, &rewritten_colon, by_qn, reexports)
                    {
                        return Some(vec![cid]);
                    }
                    let rewritten_dot = format!("{base}{rest}.{}", r.name);
                    if let Some(cid) =
                        lookup_with_reexports(lang, &rewritten_dot, by_qn, reexports)
                    {
                        return Some(vec![cid]);
                    }
                }
            }
        }

        // Step 3c: path-headed receivers (Rust only). A leading
        // `crate`/`super`/`self` segment is never rewritten above — step
        // 3's direct lookup and step 3b's prefix-prepend both leave the
        // keyword embedded literally in the candidate qualified name
        // (`crate::x::y` is not how anything is keyed in `by_qn`), so a
        // call written relative to the caller's own module never
        // resolves. Resolve the head against the caller's own qualified
        // name and retry the exact lookup; if that still misses, keep
        // the rewritten path so steps 4 and 5 below reason about it
        // rather than the keyword.
        //
        // Gated on `rh_buf.contains("::")` — a BARE `self` (no `::` at
        // all) is `self.method()`, the ordinary field-expression method
        // receiver, not a `self::`-relative module path, and must reach
        // step 4/5 completely untouched: rewriting it here (as an
        // earlier version of this fix did) turns it into a lowercase
        // non-"self" path, which both silences the `rh != "self"` guards
        // below and sends every `self.foo()` call in an `impl Trait for
        // T` block into step 5's fan-out. Measured false-positive: 88 of
        // 218 edges added on llmitm-v5 were exactly this — e.g.
        // `vmm/src/devices/virtio/rng/event_handler.rs:99`, base leaves
        // `self.method()` unresolved, this bug bound it to an unrelated
        // same-named method on another type. A bare `crate`/`super`
        // receiver has no such collision (`crate.x`/`super.x` are not
        // valid Rust), but SPECS.md scopes this whole step to
        // `receiver_hint contains "::"` and that is honoured uniformly
        // here rather than special-cased per keyword.
        if lang == "rust" && rh_buf.contains("::") {
            let head = rh_buf.split("::").next().unwrap_or("").to_string();
            let rewritten_head = caller_qn.and_then(|qn| {
                let segs: Vec<&str> = qn.split("::").collect();
                match head.as_str() {
                    // `crate::x` -> `<crate_root>::x`.
                    "crate" => segs.first().map(|s| (*s).to_string()),
                    // `self::x` -> `<caller's own module>::x`.
                    "self" if segs.len() >= 2 => Some(segs[..segs.len() - 1].join("::")),
                    // `super::x` -> `<caller's module, one level up>::x`.
                    "super" if segs.len() >= 3 => Some(segs[..segs.len() - 2].join("::")),
                    _ => None,
                }
            });
            if let Some(new_head) = rewritten_head {
                let rest = rh_buf.strip_prefix(head.as_str()).unwrap_or("").to_string();
                let rewritten = format!("{new_head}{rest}");
                let candidate = format!("{rewritten}::{}", r.name);
                if let Some(cid) =
                    lookup_with_reexports(lang, &candidate, by_qn, reexports)
                {
                    return Some(vec![cid]);
                }
                rh_buf = rewritten;
            }
        }
        let rh = rh_buf.as_str();
        // `State<AppState>` (see `deref_wrapper_split`): steps 4/4c judge
        // the wrapper; its argument is tried only after the wrapper misses.
        let deref_inner = if lang == "rust" {
            cgg_core::rust_types::deref_wrapper_split(rh)
        } else {
            None
        };
        let rh = deref_inner.map_or(rh, |(outer, _)| outer);

        // Step 4: Type-qualified method call (Issue 2).
        // When receiver_hint is a type name (e.g. "MermaidFormatter"),
        // look the owning type up directly in the (owner, method) index —
        // O(1) — instead of scanning every qualified name. `type_hints`
        // rewrites a typed local/param receiver to its type name before
        // this runs, so `reg.commit()` with `reg: Registry` arrives here
        // as receiver_hint = "Registry". A path-headed receiver
        // (`rh.contains("::")`) is tried here too, regardless of case:
        // `cgg_format::NodeIds` starts lowercase but its LAST segment,
        // tried as `owner` below, is the type that matters.
        //
        // Deliberately NOT gated on `is_external_rust_head` for ENTRY —
        // that was tried and measured wrong: `kvm_bindings::CpuId::try_from(..)`
        // (`vcpu.rs:218`) has an external HEAD (`kvm_bindings` is a real
        // external crate) but its owner, `CpuId`, is a type THIS crate
        // implements `TryFrom` for locally — `<kvm_bindings::CpuId as
        // TryFrom<Cpuid>>::try_from`. Blocking step 4 outright for an
        // external head loses this legitimate case. But a bare-owner
        // match (`CpuId`, ignoring the `kvm_bindings::` head) is exactly
        // how `kvm_ioctls::Kvm::new()` — a DIFFERENT external crate's
        // constructor — wrongly bound to a local, unrelated
        // `vmm::vstate::kvm::Kvm::new`: two types named `Kvm` sharing a
        // bare owner is a coincidence, not a call. The rule below (last
        // amendment): when the head is external, only a FULL-PATH owner
        // match counts — the candidate's qualified name must contain
        // the receiver's exact path as its owner
        // (`by_full_owner_method`, keyed unreduced). No bare-owner
        // fan-out, no base-class search: an external head's only
        // legitimate match is a local trait impl written for that exact
        // external type. An in-workspace head keeps every existing
        // owner-list trick (last segment, import-aliased owner, base
        // classes) unchanged.
        let relaxed_path_headed = lang == "rust" && rh.contains("::");
        if !rh.is_empty()
            && rh != "self"
            && rh != "Self"
            && rh != "cls"
            && (relaxed_path_headed
                // A receiver the plugin or type propagation read as a type
                // (a cast, a construction, a typed value) is one whatever
                // its case: Go's `&server{}`, a lowercase C struct.
                || r.typed_receiver
                // A PEP 8 private class (`_CollectErrors`) is still a
                // type: the plugin types `x = _CollectErrors()` as one,
                // and rejecting it here left the call bound to nothing.
                || (if lang == "python" { rh.trim_start_matches('_') } else { rh })
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_uppercase()))
        {
            let external = lang == "rust"
                && rh.contains("::")
                && is_external_rust_head(
                    rh.split("::").next().unwrap_or(""),
                    rust_path_heads,
                    workspace_owner_names,
                    direct_imports,
                    module_aliases,
                    caller_qn,
                );
            if external {
                if let Some(cids) = by_full_owner_method.get(&(
                    lang.to_string(),
                    rh.to_string(),
                    r.name.clone(),
                )) && !cids.is_empty()
                {
                    return Some(cids.clone());
                }
            } else {
                // The owner key in the index is the *bare* type name. A
                // receiver can arrive as a multi-segment path (`Utils::Platforms`,
                // `a.b.Thing`), so also try its last segment as the owner —
                // this is what the previous suffix-scan matched and must not
                // regress. Additionally canonicalize an aliased receiver type
                // through the file's import map (Issue 7): `use a::b::Engine as
                // Motor` means a receiver typed `Motor` owns whatever `Engine`
                // owns, so try the alias target's bare type name too.
                let mut owners: Vec<&str> = vec![rh];
                // Normalised so a generic owner written at the call site
                // (`AddressSpace::<T>::new_memory`, turbofish) is tried as
                // its bare type name, matching how `by_owner_method`'s own
                // keys are normalised (`owner_from_qn` -> `normalize_owner`).
                let last_seg = rh
                    .rsplit([':', '.'])
                    .next()
                    .map(crate::names::normalize_owner)
                    .filter(|s| !s.is_empty());
                if let Some(last) = last_seg
                    && last != rh
                {
                    owners.push(last);
                }
                if let Some(paths) = direct_imports.get(rh) {
                    for p in paths {
                        let last = p.rsplit("::").next().unwrap_or(p);
                        if last != rh {
                            owners.push(last);
                        }
                    }
                }
                // c12: an enum-variant (or associated-const) receiver arrives
                // as one path — `TrustKind::Network` — and the tries above
                // both target `Network`, the variant, which owns no methods.
                // The real owner is the receiver's OWN prefix: strip the last
                // `::`-segment off and try that. If the remaining prefix is
                // itself multi-segment (`a::b::Type::Variant` -> `a::b::Type`),
                // also try its bare last segment (`Type`) — mirroring the
                // rh/last_seg pair above, one level in. Rust-only: dot-joined
                // languages don't have this shape, and PHP shares the `::`
                // joiner but not the enum-variant idiom, so it is gated on
                // the language rather than the joiner.
                if lang == "rust"
                    && let Some(idx) = rh.rfind("::")
                {
                    let owner_prefix = &rh[..idx];
                    if !owner_prefix.is_empty() {
                        owners.push(owner_prefix);
                        if let Some(last) = owner_prefix.rsplit("::").next()
                            && last != owner_prefix
                        {
                            owners.push(last);
                        }
                    }
                }
                // C#: a bare type name (`Path`, or a local typed `Path`)
                // reaches only types in namespaces the file can see — its
                // own and enclosing ones, its `using`s, every `global
                // using`. A same-named class elsewhere in the tree that the
                // file never imports is not the callee: `Path.Combine`
                // under `using System.IO` is `System.IO.Path`'s.
                let cs_scoped = lang == "csharp" && !rh.contains('.');
                let mut cs_hidden = false;
                // An inherited method is declared on a base, not on the
                // class the receiver names. Tried after every direct owner
                // match below, so a subclass override always wins.
                for owner in &owners {
                    if by_owner_method
                        .get(&(lang.to_string(), (*owner).to_string(), r.name.clone()))
                        .is_none_or(|c| c.is_empty())
                        && let Some(cids) = resolve_via_bases(
                            lang,
                            owner,
                            &r.name,
                            by_owner_method,
                            bases_by_owner,
                        )
                    {
                        // Bases are recorded by bare name, so the base a
                        // chain reaches may be one of many same-named
                        // copies (a sample's generated `ICalculator`, once
                        // per sample). The copy the file can see, nearest
                        // the caller, is the one it derives from.
                        let cids: Vec<CallableId> = if cs_scoped {
                            let seen: Vec<CallableId> = cids
                                .iter()
                                .copied()
                                .filter(|c| {
                                    graph.callables.get(c).is_none_or(|n| {
                                        heads.cs_candidate_visible(
                                            vocab,
                                            n.file,
                                            &n.qualified_name,
                                        )
                                    })
                                })
                                .collect();
                            if seen.is_empty() { cids } else { seen }
                        } else {
                            cids
                        };
                        return Some(
                            nearest_class_copies(graph, &cids, caller_file)
                                .unwrap_or(cids),
                        );
                    }
                }
                // A class the calling file declares is the one a receiver
                // of that name means there (a test's local `TestScript`),
                // never a same-named class of another file. Python only:
                // its classes are declared whole in one file (a C#
                // `partial` class or a Rust type's `impl` blocks span
                // files), and its receiver types keep the qualifier they
                // were written with — a Java local `ObjectNode.Builder b`
                // is recorded as a bare `Builder`, which a file's own
                // nested `Builder` would wrongly claim.
                let own_file_class = lang == "python";
                let mut own_class_miss = false;
                for owner in &owners {
                    if let Some(cids) = by_owner_method.get(&(
                        lang.to_string(),
                        (*owner).to_string(),
                        r.name.clone(),
                    )) && !cids.is_empty()
                    {
                        // Only for a type written unqualified: `ObjectNode.Builder`
                        // names another class's `Builder`, whatever the
                        // calling file's own `Builder` is.
                        if own_file_class
                            && *owner == rh
                            && heads.own_classes.contains(*owner)
                            && !cids.iter().any(|c| {
                                graph
                                    .callables
                                    .get(c)
                                    .is_some_and(|n| n.file == caller_file)
                            })
                        {
                            // The file's own class does not declare it: an
                            // inherited member, or nothing the tree has.
                            if let Some(inherited) = resolve_via_bases(
                                lang,
                                owner,
                                &r.name,
                                by_owner_method,
                                bases_by_owner,
                            ) {
                                return Some(inherited);
                            }
                            own_class_miss = true;
                            continue;
                        }
                        if cs_scoped {
                            let seen: Vec<CallableId> = cids
                                .iter()
                                .copied()
                                .filter(|c| {
                                    graph.callables.get(c).is_none_or(|n| {
                                        heads.cs_candidate_visible(
                                            vocab,
                                            n.file,
                                            &n.qualified_name,
                                        )
                                    })
                                })
                                .collect();
                            if seen.is_empty() {
                                cs_hidden = true;
                                continue;
                            }
                            return Some(seen);
                        }
                        return Some(cids.clone());
                    }
                }
                if cs_hidden {
                    *external_out = Some(rh.to_string());
                    return None;
                }
                if own_class_miss {
                    return None;
                }
                if lang == "rust"
                    && !r.path_receiver
                    && !cgg_core::stdlib::stdlib_names(lang)
                        .is_some_and(|std| std.contains(r.name.as_str()))
                    && let Some(cids) = std_owner_trait_impls.get(&(
                        lang.to_string(),
                        crate::names::normalize_owner(rh).to_string(),
                        r.name.clone(),
                    ))
                    && !cids.is_empty()
                {
                    return Some(cids.clone());
                }
            }
        }

        // Step 4a: Rust `self.m()` the intra-file linker could not place —
        // the method lives in another `impl` block of the caller's own
        // type, in another file. An exact owner lookup on that type (and
        // its traits' defaults), never a name match: impl blocks live in
        // the type's crate, so only that crate's `Owner::m` qualifies.
        if lang == "rust"
            && rh == "self"
            && let Some(cq) = caller_qn
            && let Some(owner) = owner_from_qn(cq)
        {
            let owner = owner.split('<').next().unwrap_or(owner).trim();
            let krate = cq.split("::").next().unwrap_or("");
            let same_crate = |c: &CallableId| {
                graph
                    .callables
                    .get(c)
                    .is_some_and(|n| n.qualified_name.split("::").next() == Some(krate))
            };
            let own: Vec<CallableId> = by_owner_method
                .get(&(lang.to_string(), owner.to_string(), r.name.clone()))
                .map(|v| v.iter().copied().filter(same_crate).collect())
                .unwrap_or_default();
            if !own.is_empty() {
                return Some(own);
            }
            if let Some(cids) =
                resolve_via_bases(lang, owner, &r.name, by_owner_method, bases_by_owner)
            {
                return Some(cids);
            }
            return None;
        }

        // Step 4b: an external wrapper with one type argument — axum's
        // `State<AppState>`, actix's `web::Data<AppState>` — dereferences
        // to it, so a method the wrapper lacks is the argument's. Still a
        // typed lookup (the owner index), never name matching, and never
        // when the wrapper is a workspace type cgg can see the methods of.
        if let Some((outer, inner)) = deref_inner {
            let outer_last = outer.rsplit("::").next().unwrap_or(outer);
            let inner_last = inner.rsplit("::").next().unwrap_or(inner);
            // A workspace wrapper qualifies when it declares `Deref` to its
            // own type argument (`impl<T> Deref for Cfg<T> { type Target = T }`).
            let derefs_to_arg = bases_by_owner
                .get(&(lang.to_string(), outer_last.to_string()))
                .is_some_and(|b| b.iter().any(|x| x == DEREF_TO_ARG));
            if (!workspace_owner_names.contains(outer_last) || derefs_to_arg)
                && let Some(cids) = by_owner_method.get(&(
                    lang.to_string(),
                    inner_last.to_string(),
                    r.name.clone(),
                ))
                && !cids.is_empty()
            {
                return Some(cids.clone());
            }
        }

        // Step 4c: an unresolved path-headed receiver (Rust) whose head
        // names no crate cgg ever indexed — after substituting through
        // this file's own imports, and always for `std`/`core`/`alloc` —
        // cannot be a local call: `serde_json::de::from_str`,
        // `toml::from_str`, `std::io::Error::from`. Left alone, step 5
        // below treats the lowercase head as if it were a local variable
        // name and fans out to any same-named method in the workspace
        // (`RollupLevel::from_str`), which is how a call into an
        // external crate becomes a false cross-file edge. A receiver
        // whose head IS in the workspace vocabulary (a `super`/`self`/
        // `crate` path step 3c could not fully resolve, e.g. because
        // `caller_qn` was unknown) still falls through to step 5
        // unchanged.
        //
        // A one-segment path receiver (`serde_json::from_str`, receiver
        // `serde_json`) carries no `::` of its own; `path_receiver` is
        // what tells it apart from a variable (`parser.from_str`). Left
        // to step 5 it fanned out to every workspace `from_str`.
        //
        // A file outside any Cargo crate has crate root `crate`: with no
        // manifest there is no crate graph to judge a one-segment head by.
        if lang == "rust"
            && (rh.contains("::")
                || r.path_receiver
                    && !matches!(rh, "self" | "super" | "crate")
                    && !caller_qn.is_some_and(|q| q.starts_with("crate::")))
            && is_external_rust_head(
                rh.split("::").next().unwrap_or(""),
                rust_path_heads,
                workspace_owner_names,
                direct_imports,
                module_aliases,
                caller_qn,
            )
        {
            return None;
        }

        // Step 4d (C#, WS8): `x.M()` on a receiver typed `T` — a variable
        // of that type, or a construction `new T()` — that neither `T`
        // nor its bases declare is an extension method: a static
        // `M(this U …)` whose extended type `U` is `T`, a base of `T`, or
        // a type parameter of `M`. Without this a typed receiver lost
        // every extension call it makes.
        if lang == "csharp"
            && !r.paren_less
            && (rh.starts_with(char::is_uppercase) || r.typed_receiver)
        {
            let recv = rh.rsplit('.').next().unwrap_or(rh);
            let recv = recv.split('<').next().unwrap_or(recv).trim();
            let family = with_ancestors(lang, vec![recv.to_string()], bases_by_owner);
            if let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone())) {
                let ext: Vec<CallableId> = cids
                    .iter()
                    .copied()
                    .filter(|c| {
                        signatures
                            .get(c)
                            .and_then(|sig| csharp_extension_target(sig))
                            .is_some_and(|(t, generic)| generic || family.contains(&t))
                    })
                    .collect();
                if !ext.is_empty() && ext.len() <= fanout_cap {
                    return Some(ext);
                }
            }
        }

        // Step 4e (Solidity): `x.f()` on a receiver typed `T` that `T`
        // does not declare — a library function attached by `using L for
        // T;` (or `using L for *;`) in the calling file: `L.f(x)`.
        // Without this a typed receiver lost every library call it makes,
        // which an untyped one only reached by name fan-out.
        if lang == "solidity"
            && !r.paren_less
            && (rh.starts_with(char::is_uppercase) || r.typed_receiver)
        {
            let recv = rh.rsplit('.').next().unwrap_or(rh).trim();
            let mut lib_fns: Vec<CallableId> = Vec::new();
            for lib in heads.libraries_for(recv) {
                if let Some(cids) = by_owner_method.get(&(
                    lang.to_string(),
                    lib.to_string(),
                    r.name.clone(),
                )) {
                    for c in cids {
                        if !lib_fns.contains(c) {
                            lib_fns.push(*c);
                        }
                    }
                }
            }
            if !lib_fns.is_empty() && lib_fns.len() <= fanout_cap {
                return Some(lib_fns);
            }
        }

        // Step 5: Trait/interface method dispatch.
        // When receiver_hint is a variable name (lowercase) and the method
        // name exists on definitions in other files, find all callables
        // with that simple_name. This handles `formatter.render()` where
        // formatter is a trait object — we emit edges to all implementors.
        // Only applies when the method name is NOT a common stdlib method
        // (to avoid matching `vec.is_empty()` to `WalkOutcome::is_empty`).
        // A paren-less site (`b.size`) through an untyped receiver is a
        // field read far more often than a parameterless method: it binds
        // only where the receiver is typed, never by name fan-out.
        if !rh.is_empty()
            && !r.paren_less
            // A typed receiver is never fanned out by name: its type's
            // own lookup (step 4) was the answer, hit or miss.
            && !r.typed_receiver
            && rh != "self"
            && rh != "Self"
            && rh != "cls"
            && rh.chars().next().is_some_and(|c| c.is_lowercase())
        {
            // Skip if the method name is in the stdlib manifest for this language
            let _sp_fan = cgg_core::profile::span("xfile::fanout");
            let is_stdlib_method = cgg_core::stdlib::stdlib_names(lang)
                .is_some_and(|std| std.contains(r.name.as_str()));
            if !is_stdlib_method
                && let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone()))
                && !cids.is_empty()
            {
                // Above the cap the fan-out is too speculative to emit.
                // The caller records *that* it was dropped, with the
                // count — silence here reads as "no call at this site",
                // which understates the caller set rather than widening
                // it, and that is the failure mode impact analysis
                // cannot tolerate.
                // Prefer concrete implementations. A Protocol member
                // or an @abstractmethod is a declaration, not a target.
                let concrete: Vec<CallableId> = cids
                    .iter()
                    .copied()
                    .filter(|c| !stub_ids.contains(c))
                    .collect();
                let cids = if concrete.is_empty() {
                    cids.clone()
                } else {
                    concrete
                };
                // Drop candidates whose signature cannot accept this
                // call's keywords. One-sided and evidence-based: only a
                // keyword that is provably not a parameter eliminates a
                // candidate, so narrowing fan-out can never turn a real
                // edge into a missing one.
                let fits: Vec<CallableId> = cids
                    .iter()
                    .copied()
                    .filter(|c| {
                        signatures
                            .get(c)
                            .is_none_or(|sig| signature_accepts(sig, &r.kwargs))
                    })
                    .collect();
                let cids = if fits.is_empty() { cids } else { fits };
                // Over the cap, a set that is entirely one workspace trait's
                // implementations is not a guess among unrelated methods:
                // it is a dispatch through that trait, and the call's
                // target is the trait's declaration. Bind there (one
                // edge; `--dynamic-dispatch` carries it on to the impls)
                // rather than drop the site — dropping left the
                // declaration in-degree 0 however many sites called it.
                //
                // Only for a receiver that is a variable or a field path
                // (`src`, `s.vec_index`): the result of a call chain
                // (`s.parse().context(..)`) has a type cgg never saw, and
                // the method is as likely an external extension trait's
                // (anyhow's `Context`) as the workspace trait's.
                if lang == "rust"
                    && cids.len() > fanout_cap
                    && rh
                        .chars()
                        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
                    && let Some(decl) = homogeneous_trait_declaration(
                        graph,
                        &cids,
                        &r.name,
                        by_owner_method,
                    )
                {
                    return Some(vec![decl]);
                }
                // A receiver recovered from a macro token tree is one the
                // extractor could not type the way it does an ordinary
                // call. Measured: every false addition from this path sat
                // on a multi-destination site (13 of 15 sampled) and every
                // single-destination one was right (85 of 85), so it binds
                // only when exactly one candidate remains.
                if r.from_macro_arg && cids.len() > 1 {
                    // One narrowing first: a trait's own declaration
                    // (`Trait::m`, bodiless) never outranks an impl of
                    // that same trait (`<T as Trait>::m`) among the
                    // candidates — measured, 20 of 47 two-way sites here
                    // were exactly that pair, and the impl was the call.
                    let impl_traits: Vec<String> = cids
                        .iter()
                        .filter_map(|c| graph.callables.get(c))
                        .filter_map(|c| trait_of_impl_qn(&c.qualified_name))
                        .collect();
                    let narrowed: Vec<CallableId> = cids
                        .iter()
                        .copied()
                        .filter(|c| {
                            graph.callables.get(c).is_none_or(|n| {
                                !is_trait_declaration_of(&n.qualified_name, &impl_traits)
                            })
                        })
                        .collect();
                    if narrowed.len() == 1 {
                        return Some(narrowed);
                    }
                    *capped = cids.len() as u32;
                    return None;
                }
                // The cap is decided on this set — exactly as base does —
                // *before* the visibility filter runs. A set base would
                // have suppressed as over-cap must stay suppressed: it
                // must never shrink into an under-cap set that fans out
                // to survivors, which are wrong for the same reason the
                // ones filtered away are. The filter is only ever allowed
                // to narrow a set base was already going to emit.
                if cids.len() > fanout_cap {
                    *capped = cids.len() as u32;
                    return None;
                }
                // Drop a Rust candidate this fan-out cannot actually
                // reach: a private item outside the caller's module tree,
                // or a `mod tests` helper the caller does not share.
                let cids: Vec<CallableId> = if lang == "rust" {
                    cids.into_iter()
                        .filter(|c| {
                            rust_candidate_reachable(
                                graph,
                                *c,
                                caller_file,
                                caller_qn,
                                direct_imports,
                            )
                        })
                        .collect()
                } else if lang == "kotlin" {
                    // Likewise a Kotlin candidate the call cannot reach: an
                    // extension out of the caller's scope, or a top-level
                    // function that is not an extension at all.
                    cids.into_iter()
                        .filter(|c| {
                            kotlin_extensions.receiver_call_target(
                                graph,
                                *c,
                                caller_file,
                                direct_imports,
                            )
                        })
                        .collect()
                } else {
                    cids
                };
                if !cids.is_empty() {
                    return Some(cids);
                }
            }
        }
    }

    // Step 6: instantiation — `Widget(3)` enters `Widget.__init__`.
    // Last, so a function of the same name always wins. When the
    // class declares no initializer, an inherited one still counts.
    if r.receiver_hint.is_empty() {
        for ctor in constructor_names(lang) {
            if let Some(cids) = by_owner_method.get(&(
                lang.to_string(),
                r.name.clone(),
                (*ctor).to_string(),
            )) && !cids.is_empty()
            {
                return Some(cids.clone());
            }
            if let Some(cids) =
                resolve_via_bases(lang, &r.name, ctor, by_owner_method, bases_by_owner)
            {
                return Some(cids);
            }
        }
        // A known class with no initializer of its own: there is no
        // callable to point at, which is a different fact from "cgg
        // has never heard of this name".
        if !constructor_names(lang).is_empty()
            && known_owners.contains(&(lang.to_string(), r.name.clone()))
        {
            *no_ctor = true;
        }

        // Step 7: calling an instance — `agent("prompt")` where
        // `agent` is an object enters `type(agent).__call__`. In the
        // audited service this was the single most load-bearing edge
        // in the system and it was invisible.
        if let Some(ty) = var_types.get(&r.name) {
            for call_op in call_operator_names(lang) {
                if let Some(cids) = by_owner_method.get(&(
                    lang.to_string(),
                    ty.clone(),
                    (*call_op).to_string(),
                )) && !cids.is_empty()
                {
                    return Some(cids.clone());
                }
                if let Some(cids) =
                    resolve_via_bases(lang, ty, call_op, by_owner_method, bases_by_owner)
                {
                    return Some(cids);
                }
            }
        }
    }

    // c3 amendment: a macro-argument reference's `receiver_hint` — when
    // set — came from raw token-tree adjacency (`refs_from_token_tree`),
    // not a typed expression: a type alias's bare name
    // (`type Limiter = DefaultLogRateLimiter; limiter.check(..)` inside
    // `assert!`), a file-wide `var_types` guess keyed on a common bare
    // name that resolved to the wrong binding, or similar. When every
    // qualified step above found nothing, base's plain bare-name path
    // (no receiver_hint at all, since it never attached one) would still
    // have found these through step 1d's global fallback. Retry the same
    // way, but stricter — a crate-wide UNIQUE simple name, not step 1d's
    // ≤3 — since throwing away receiver information entirely is a bigger
    // uncertainty jump than an ordinary bare call ever risks.
    if r.from_macro_arg && !r.receiver_hint.is_empty() {
        let is_stdlib = cgg_core::stdlib::stdlib_names(lang)
            .is_some_and(|s| s.contains(r.name.as_str()));
        if !is_stdlib
            && let Some(cids) = by_simple.get(&(lang.to_string(), r.name.clone()))
            && cids.len() == 1
        {
            return Some(cids.clone());
        }
    }

    None
}

/// Resolve a receiver's head segment to the crate it actually names,
/// substituting through the file's own `use` imports first.
/// `arg_parser::Arguments` in a file with `use utils::arg_parser;` names
/// the crate `utils`, not a (nonexistent) crate literally called
/// `arg_parser` — `arg_parser` is a nested module, never the first
/// segment of any qualified name in `by_qn`. This is the file-scoped,
/// well-defined alternative to indexing every segment of every
/// qualified name globally (reverted: that approach could not tell a
/// module/type segment from text INSIDE a trait-impl wrapper's generic
/// argument — see `rust_path_heads`'s doc comment).
fn workspace_head<'a>(
    head: &'a str,
    direct_imports: &'a HashMap<String, Vec<String>>,
    module_aliases: &'a HashMap<String, String>,
) -> &'a str {
    if let Some(target) = module_aliases.get(head) {
        return target.split("::").next().unwrap_or(target);
    }
    if let Some(targets) = direct_imports.get(head)
        && let Some(first) = targets.first()
    {
        return first.split("::").next().unwrap_or(first);
    }
    head
}

/// A Rust path-headed receiver's head is external — cannot be a local
/// call — when, after `workspace_head` substitution, it is `std`/
/// `core`/`alloc` or is not the first segment of any Rust `by_qn` key
/// (a crate cgg actually indexed). Shared by step 4's relaxed owner
/// lookup and step 4c so both apply the identical rule.
fn is_external_rust_head(
    raw_head: &str,
    rust_path_heads: &RustPathVocab,
    workspace_owner_names: &std::collections::HashSet<String>,
    direct_imports: &HashMap<String, Vec<String>>,
    module_aliases: &HashMap<String, String>,
    caller_qn: Option<&str>,
) -> bool {
    let head = crate::names::normalize_owner(raw_head);
    let effective = crate::names::normalize_owner(workspace_head(
        head,
        direct_imports,
        module_aliases,
    ));
    if matches!(effective, "std" | "core" | "alloc") {
        return true;
    }
    // `use crate::channel::pool;` makes `pool`'s effective head `crate`
    // — relative path keywords are in-workspace by definition.
    if matches!(effective, "crate" | "self" | "super") {
        return false;
    }
    // A receiver's first segment is not always a crate: an enum-variant
    // or associated-const receiver leads with the TYPE (`TrustKind` in
    // `TrustKind::Network`). Such a head is in-workspace whenever the
    // workspace declares a type by that name, and treating it as
    // external would refuse the prefix-owner tries that resolve it.
    // `use a::b::*;` makes every child of `a::b` nameable bare.
    let via_glob = || {
        direct_imports.get("*").is_some_and(|globs| {
            globs.iter().any(|g| {
                g.strip_suffix("::*").is_some_and(|base| {
                    rust_path_heads
                        .module_paths
                        .contains(&format!("{base}::{head}"))
                })
            })
        })
    };
    !rust_path_heads.crate_roots.contains(effective)
        && !workspace_owner_names.contains(effective)
        && rust_module_head_path(head, caller_qn, rust_path_heads).is_none()
        && !via_glob()
}

/// What cgg knows about Rust path vocabulary in this workspace: the
/// crate roots it indexed and every module path beneath them. Built
/// sorted-insensitive (a set, only ever probed), so `--jobs` cannot
/// change an answer.
#[derive(Default)]
struct RustPathVocab {
    crate_roots: std::collections::HashSet<String>,
    module_paths: std::collections::HashSet<String>,
}

/// The `::`-segments of a Rust qualified name up to (not including) the
/// first one carrying `<` — a generic owner (`Type<T>`) or a trait-impl
/// wrapper (`<A as b::Trait>`), whose embedded paths are not modules.
fn rust_plain_segments(qn: &str) -> Vec<&str> {
    qn.split("::").take_while(|s| !s.contains('<')).collect()
}

/// The full module path a relative one-segment `head` names from inside
/// `caller_qn`, when the caller's module or one of its ancestors
/// declares a child by that name. Innermost first: that is the scope
/// rustc consults.
fn rust_module_head_path(
    head: &str,
    caller_qn: Option<&str>,
    vocab: &RustPathVocab,
) -> Option<String> {
    if head.is_empty() || matches!(head, "self" | "super" | "crate" | "Self") {
        return None;
    }
    let segs = rust_plain_segments(caller_qn?);
    (1..segs.len()).rev().find_map(|i| {
        let candidate = format!("{}::{head}", segs[..i].join("::"));
        vocab.module_paths.contains(&candidate).then_some(candidate)
    })
}

/// Look up `qn` in the callable index, following Rust `pub use`
/// re-export chains up to a small depth cap so malformed graphs can't
/// loop.
///
/// A miss in the caller's language retries the qualified name under
/// each sibling in [`cgg_core::language_family`]. That is how a C++
/// file `#include`ing a C header reaches the `.c` definition: the
/// include records the name, and the only copy of it is indexed under
/// `"c"`. The same-language entry wins when both exist, and it is the
/// single `(language, qualified_name)` slot — the later callable, the
/// same one a same-language lookup would return. Simple-name fan-out
/// is not consulted here; widening that index mixes every C function
/// into every C++ duck-typed call.
fn lookup_with_reexports(
    lang: &str,
    qn: &str,
    by_qn: &HashMap<(String, String), CallableId>,
    reexports: &HashMap<(String, String), String>,
) -> Option<CallableId> {
    if let Some(cid) = lookup_in_language(lang, qn, by_qn, reexports) {
        return Some(cid);
    }
    for sib in cgg_core::language_family(lang) {
        if *sib == lang {
            continue;
        }
        if let Some(cid) = by_qn.get(&((*sib).to_string(), qn.to_string())).copied() {
            return Some(cid);
        }
    }
    None
}

fn lookup_in_language(
    lang: &str,
    qn: &str,
    by_qn: &HashMap<(String, String), CallableId>,
    reexports: &HashMap<(String, String), String>,
) -> Option<CallableId> {
    let mut current = qn.to_string();
    for _ in 0..8 {
        if let Some(cid) = by_qn.get(&(lang.to_string(), current.clone())).copied() {
            return Some(cid);
        }
        if let Some(next) = reexports.get(&(lang.to_string(), current.clone())) {
            current = next.clone();
            continue;
        }
        return None;
    }
    None
}

/// The innermost callable whose byte range contains `byte`.
///
/// `by_span` maps `(file, start_byte, end_byte)` to the callable id and
/// is built once per run. This used to finish with
/// `graph.callables.values().find(...)` — a scan of *every callable in
/// the graph*, per reference. On Zig's compiler that is 572,840
/// references against 344,808 callables, and it was 449s of a 128s
/// wall-clock run (the span nests across 8 threads). Nothing else in
/// the reference loop came close: the actual resolution was 1.8s.
fn enclosing_callable_id(
    by_span: &HashMap<(FileId, u32, u32), CallableId>,
    facts: &FileFacts,
    byte: u32,
) -> Option<CallableId> {
    let mut best: Option<(&cgg_core::DefRecord, u32)> = None;
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
    let (d, _) = best?;
    by_span
        .get(&(facts.file, d.start_byte, d.end_byte))
        .copied()
}

#[cfg(test)]
mod tests {
    /// `resolve` at the default fan-out cap.
    fn resolve_default(g: &Graph, f: &[FileFacts]) -> CrossFileOutput {
        resolve(g, f, DEFAULT_FANOUT_CAP, false, false)
    }

    use super::*;
    use cgg_core::{
        DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
        graph::{CallableKind, CallableNode, FileRecord as GraphFileRecord},
    };
    use std::path::PathBuf;

    /// The lazy step-1c lookup answers exactly what the eager map it
    /// replaced held: every same-named definition in a file whose path
    /// contains a fragment, in file then declaration order, and nothing
    /// when there are none or more than the cap.
    #[test]
    fn scoped_lookup_matches_the_eager_scan() {
        let files: Vec<(FileId, String)> = vec![
            (FileId::new(0), "/r/text/pandoc.hs".into()),
            (FileId::new(1), "/r/other/x.hs".into()),
            (FileId::new(2), "/r/text/pandoc/shared.hs".into()),
        ];
        let c = |i: u32| CallableId::new(i);
        let mut def_cids: HashMap<FileId, Vec<(&str, CallableId)>> = HashMap::new();
        def_cids.insert(FileId::new(0), vec![("f", c(0)), ("g", c(1)), ("f", c(2))]);
        def_cids.insert(FileId::new(1), vec![("f", c(3))]);
        def_cids.insert(FileId::new(2), vec![("f", c(4)), ("h", c(5))]);
        let index = ScopedIndex::build(&files, &def_cids);
        let s = ScopedSimple::new(&index, vec!["text/pandoc".into()]);
        assert_eq!(s.get_capped("f", 8), Some(vec![c(0), c(2), c(4)]));
        assert_eq!(s.get_capped("h", 8), Some(vec![c(5)]));
        assert_eq!(s.get_capped("f", 2), None, "over the cap");
        assert_eq!(s.get_capped("missing", 8), None);
        assert_eq!(ScopedSimple::empty().get_capped("f", 8), None);
        let none = ScopedSimple::new(&index, Vec::new());
        assert_eq!(none.get_capped("f", 8), None, "no fragments, no scope");
    }

    fn mk_file(id: u32, path: &str, lang: &str) -> GraphFileRecord {
        GraphFileRecord {
            id: FileId::new(id),
            path: PathBuf::from(path),
            language: lang.into(),
            detected_via: "ext:.py".into(),
            blake3: "0".repeat(64),
            size_bytes: 10,
            lines: 1,
            parse_ms: 0.0,
            parse_status: "ok".into(),
            ..Default::default()
        }
    }

    fn mk_callable(
        id: u32,
        simple: &str,
        qn: &str,
        file: u32,
        lang: &str,
        byte_range: (u32, u32),
    ) -> CallableNode {
        CallableNode {
            id: CallableId::new(id),
            qualified_name: qn.into(),
            simple_name: simple.into(),
            kind: CallableKind::Function,
            language: lang.into(),
            file: FileId::new(file),
            start_line: 1,
            end_line: 1,
            start_byte: byte_range.0,
            end_byte: byte_range.1,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            synthetic: false,
            trait_impl_target: None,
            ..Default::default()
        }
    }

    fn mk_def(
        simple: &str,
        qn: &str,
        variant: DefVariant,
        byte_range: (u32, u32),
    ) -> DefRecord {
        DefRecord {
            simple_name: simple.into(),
            qualified_name: qn.into(),
            variant,
            start_line: 1,
            end_line: 1,
            start_byte: byte_range.0,
            end_byte: byte_range.1,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: vec![],
            ..Default::default()
        }
    }

    fn facts_for(
        file: u32,
        path: &str,
        lang: &str,
        defs: Vec<DefRecord>,
        refs: Vec<RefRecord>,
        imports: Vec<ImportRecord>,
    ) -> FileFacts {
        FileFacts {
            file: FileId::new(file),
            path: PathBuf::from(path),
            language: lang.into(),
            definitions: defs,
            references: refs,
            imports,
            local_types: Vec::new(),
            ..Default::default()
        }
    }

    /// A value reference sitting at module scope — `callback=handler`
    /// in a decorator, `event.listen(cls, "...", cls.hook)` — has no
    /// enclosing callable, so it cannot become an edge. It must still
    /// be recorded: dropping it silently let the *target* reach the
    /// `High` dead-code band with nothing on record to doubt it.
    #[test]
    fn module_scope_value_ref_is_recorded_not_dropped() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "cli.py", "python"));
        g.add_callable(mk_callable(
            0,
            "_validate_key",
            "cli._validate_key",
            0,
            "python",
            (0, 40),
        ));

        // The ref sits at byte 200, outside `_validate_key`'s (0, 40)
        // span and inside no other callable — i.e. module scope.
        let facts = facts_for(
            0,
            "cli.py",
            "python",
            vec![mk_def(
                "_validate_key",
                "cli._validate_key",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![RefRecord {
                name: "_validate_key".into(),
                receiver_hint: cgg_core::VALUE_REF_HINT.into(),
                site_line: 42,
                site_byte: 200,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, std::slice::from_ref(&facts));

        assert!(
            out.edges.is_empty(),
            "a module-scope ref has no source callable, so it must not \
             manufacture an edge"
        );
        assert_eq!(out.unresolved.len(), 1, "the ref must still be recorded");
        let u = &out.unresolved[0];
        assert_eq!(u.name, "_validate_key");
        // A value ref with no enclosing callable is a DIFFERENT fact from
        // an ordinary call site with no enclosing callable: overloading
        // `NoEnclosingCallable` for both drowned the metrics bucket in
        // argument-position value refs (VERIFIED.md §1a / §3.8).
        assert_eq!(u.reason, UnresolvedReason::ValueRefNoEnclosing);
        assert!(
            u.receiver_hint.is_empty(),
            "VALUE_REF_HINT is plumbing, not a receiver type — the \
             dead-code correlation reads this field as a type and would \
             reject the site"
        );
    }

    /// A value ref with two same-name candidates in scope must land in
    /// the distinct `ValueRefAmbiguous` reason, not `AmbiguousInFile` —
    /// the latter is what every other consumer (the `ambiguous-in-file`
    /// metrics bucket, the dead-code correlation) reads as an ordinary
    /// ambiguous call. Measured on cgg's own corpus: 940 of 972
    /// `ambiguous-in-file` entries were value refs like this one.
    #[test]
    fn value_ref_with_two_candidates_gets_its_own_reason() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "app.py", "python"));
        g.add_callable(mk_callable(0, "caller", "app.caller", 0, "python", (0, 20)));
        g.add_callable(mk_callable(
            1,
            "handler",
            "app.handler",
            0,
            "python",
            (100, 140),
        ));
        g.add_callable(mk_callable(
            2,
            "handler",
            "app.Other.handler",
            0,
            "python",
            (200, 240),
        ));

        let facts = facts_for(
            0,
            "app.py",
            "python",
            vec![
                mk_def("caller", "app.caller", DefVariant::FreeFunction, (0, 20)),
                mk_def(
                    "handler",
                    "app.handler",
                    DefVariant::FreeFunction,
                    (100, 140),
                ),
                mk_def(
                    "handler",
                    "app.Other.handler",
                    DefVariant::InherentMethod,
                    (200, 240),
                ),
            ],
            vec![RefRecord {
                name: "handler".into(),
                receiver_hint: cgg_core::VALUE_REF_HINT.into(),
                site_line: 5,
                site_byte: 10,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, std::slice::from_ref(&facts));

        assert!(
            out.edges.is_empty(),
            "an ambiguous value ref must not manufacture an edge"
        );
        assert_eq!(out.unresolved.len(), 1);
        let u = &out.unresolved[0];
        assert_eq!(u.name, "handler");
        assert_eq!(
            u.reason,
            UnresolvedReason::ValueRefAmbiguous { candidates: 2 },
            "must NOT be AmbiguousInFile — that reason is read by other \
             consumers as an ordinary ambiguous call site"
        );
    }

    /// A value ref that uniquely names a *closure*, never a free
    /// function or method, must not become a `Via::Reference` edge.
    #[test]
    fn value_ref_to_a_closure_emits_no_edge() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "app.py", "python"));
        g.add_callable(mk_callable(0, "caller", "app.caller", 0, "python", (0, 20)));
        let mut closure =
            mk_callable(1, "on_click", "app.on_click", 0, "python", (100, 140));
        closure.kind = CallableKind::Closure;
        g.add_callable(closure);

        let facts = facts_for(
            0,
            "app.py",
            "python",
            vec![
                mk_def("caller", "app.caller", DefVariant::FreeFunction, (0, 20)),
                mk_def(
                    "on_click",
                    "app.on_click",
                    DefVariant::NamedClosure,
                    (100, 140),
                ),
            ],
            vec![RefRecord {
                name: "on_click".into(),
                receiver_hint: cgg_core::VALUE_REF_HINT.into(),
                site_line: 5,
                site_byte: 10,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, std::slice::from_ref(&facts));

        assert!(
            out.edges.is_empty(),
            "a value ref uniquely naming a closure must not become an edge"
        );
    }

    #[test]
    fn python_from_import_direct_call() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "helpers.py", "python"));
        g.add_file(mk_file(1, "main.py", "python"));
        g.add_callable(mk_callable(
            0,
            "greet",
            "helpers.greet",
            0,
            "python",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "process",
            "main.process",
            1,
            "python",
            (30, 120),
        ));

        let main_facts = facts_for(
            1,
            "main.py",
            "python",
            vec![mk_def(
                "process",
                "main.process",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "greet".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![ImportRecord {
                kind: "from-import".into(),
                path: "helpers".into(),
                alias: "greet, compute".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let helpers_facts = facts_for(
            0,
            "helpers.py",
            "python",
            vec![mk_def(
                "greet",
                "helpers.greet",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[helpers_facts, main_facts]);
        assert_eq!(out.edges.len(), 1, "expected one cross-file edge");
        assert_eq!(out.edges[0].src, CallableId::new(1));
        assert_eq!(out.edges[0].dst, CallableId::new(0));
        // An unambiguous `from helpers import greet` binding is not a
        // guess — it scored `medium` until 0.6.6, which let a same-file
        // method with a colliding name outrank it at `high`.
        assert_eq!(out.edges[0].confidence, Confidence::High);
        assert_eq!(out.edges[0].resolver.as_str(), "cross-file:imports");
    }

    #[test]
    fn python_module_alias_attribute_call() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "helpers.py", "python"));
        g.add_file(mk_file(1, "main.py", "python"));
        g.add_callable(mk_callable(
            0,
            "compute",
            "helpers.compute",
            0,
            "python",
            (0, 40),
        ));
        g.add_callable(mk_callable(1, "top", "main.top", 1, "python", (30, 120)));

        let main_facts = facts_for(
            1,
            "main.py",
            "python",
            vec![mk_def(
                "top",
                "main.top",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "compute".into(),
                receiver_hint: "h".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![ImportRecord {
                kind: "import".into(),
                path: "helpers".into(),
                alias: "h".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let helpers_facts = facts_for(
            0,
            "helpers.py",
            "python",
            vec![mk_def(
                "compute",
                "helpers.compute",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[helpers_facts, main_facts]);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    #[test]
    fn rust_use_direct_call() {
        // Callables are qualified with the REAL crate name a Cargo.toml
        // crate produces (`rust.rs`'s `rust_module_path`), never the
        // literal sentinel `crate`. `main.rs`'s `use crate::util::helper;`
        // still carries that literal sentinel, unrewritten, because the
        // Rust plugin emits `ImportRecord.path` verbatim — resolution is
        // this crate's job. Before the `crate::` substitution fix, this
        // failed: `direct_imports` held the literal string
        // `"crate::util::helper"`, which cannot match the callable's real
        // qualified name `"mycrate::util::helper"` in `by_qn`.
        //
        // Three decoy `helper`s elsewhere are load-bearing: with only one
        // candidate named `helper` in the whole graph, step 1d's global
        // by-simple-name fallback (cap ≤3) finds it regardless of whether
        // the import resolved — exactly how the *old* version of this
        // test "passed for the wrong reason" (VERIFIED §1e). Four
        // same-named candidates exceeds that cap, so the edge can only
        // come from `direct_imports` actually resolving.
        let mut g = Graph::new();
        g.add_file(mk_file(0, "lib.rs", "rust"));
        g.add_file(mk_file(1, "main.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "helper",
            "mycrate::util::helper",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "main",
            "mycrate::main",
            1,
            "rust",
            (30, 120),
        ));
        g.add_callable(mk_callable(
            2,
            "helper",
            "othercrate::a::helper",
            0,
            "rust",
            (200, 240),
        ));
        g.add_callable(mk_callable(
            3,
            "helper",
            "othercrate::b::helper",
            0,
            "rust",
            (300, 340),
        ));
        g.add_callable(mk_callable(
            4,
            "helper",
            "othercrate::c::helper",
            0,
            "rust",
            (400, 440),
        ));

        let main_facts = facts_for(
            1,
            "main.rs",
            "rust",
            vec![mk_def(
                "main",
                "mycrate::main",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "helper".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![
                ImportRecord {
                    kind: "crate-root".into(),
                    path: "mycrate".into(),
                    alias: "".into(),
                    site_line: 1,
                    site_byte: 0,
                },
                ImportRecord {
                    kind: "use".into(),
                    path: "crate::util::helper".into(),
                    alias: "".into(),
                    site_line: 1,
                    site_byte: 0,
                },
            ],
        );
        let lib_facts = facts_for(
            0,
            "lib.rs",
            "rust",
            vec![mk_def(
                "helper",
                "mycrate::util::helper",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![ImportRecord {
                kind: "crate-root".into(),
                path: "mycrate".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, &[lib_facts, main_facts]);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    /// The `pub use` re-export chain: `mycrate::AuditEvent` (the alias a
    /// `pub use audit::AuditEvent;` in `lib.rs` creates) must resolve to
    /// whatever `mycrate::audit::AuditEvent` resolves to, i.e. the real
    /// definition in `audit.rs`. Before the `crate::` substitution fix
    /// this chain was dead whenever the re-exported item's own path used
    /// a leading `crate::` segment (the common case once a module goes
    /// through more than one level): `reexports` stored the literal
    /// target string, which never matched `by_qn`.
    ///
    /// Same decoy trick as `rust_use_direct_call`: three unrelated
    /// `AuditEvent`s push the global by-simple-name count past step 1d's
    /// cap of 3, so only the reexport chain resolving for real can
    /// produce the edge.
    #[test]
    fn rust_pub_use_reexport_chain_through_crate_prefix() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "audit.rs", "rust"));
        g.add_file(mk_file(1, "lib.rs", "rust"));
        g.add_file(mk_file(2, "main.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "AuditEvent",
            "mycrate::audit::AuditEvent",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            2,
            "main",
            "mycrate::main",
            2,
            "rust",
            (30, 120),
        ));
        g.add_callable(mk_callable(
            3,
            "AuditEvent",
            "othercrate::a::AuditEvent",
            0,
            "rust",
            (200, 240),
        ));
        g.add_callable(mk_callable(
            4,
            "AuditEvent",
            "othercrate::b::AuditEvent",
            0,
            "rust",
            (300, 340),
        ));
        g.add_callable(mk_callable(
            5,
            "AuditEvent",
            "othercrate::c::AuditEvent",
            0,
            "rust",
            (400, 440),
        ));

        let audit_facts = facts_for(
            0,
            "audit.rs",
            "rust",
            vec![mk_def(
                "AuditEvent",
                "mycrate::audit::AuditEvent",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![ImportRecord {
                kind: "crate-root".into(),
                path: "mycrate".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let lib_facts = facts_for(
            1,
            "lib.rs",
            "rust",
            vec![],
            vec![],
            vec![
                ImportRecord {
                    kind: "crate-root".into(),
                    path: "mycrate".into(),
                    alias: "".into(),
                    site_line: 1,
                    site_byte: 0,
                },
                ImportRecord {
                    kind: "pub-use".into(),
                    path: "crate::audit::AuditEvent".into(),
                    alias: "".into(),
                    site_line: 1,
                    site_byte: 0,
                },
            ],
        );
        let main_facts = facts_for(
            2,
            "main.rs",
            "rust",
            vec![mk_def(
                "main",
                "mycrate::main",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "AuditEvent".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![
                ImportRecord {
                    kind: "crate-root".into(),
                    path: "mycrate".into(),
                    alias: "".into(),
                    site_line: 1,
                    site_byte: 0,
                },
                ImportRecord {
                    kind: "use".into(),
                    path: "crate::AuditEvent".into(),
                    alias: "".into(),
                    site_line: 1,
                    site_byte: 0,
                },
            ],
        );

        let out = resolve_default(&g, &[audit_facts, lib_facts, main_facts]);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    /// A call site already bound by the intra-file pass must not also be
    /// re-resolved cross-file. `main.py` defines its own `mk` (same file
    /// as the caller) AND imports a same-named `mk` from `b.py`; the
    /// intra-file pass would have already bound the site to the local
    /// `mk`, recorded here directly in `graph.edges` the way the driver
    /// merges intra-file output before calling `cross_file::resolve`.
    /// Cross-file resolution must see that the site is already bound and
    /// skip it — not add a second edge to `b.mk` at the same site.
    #[test]
    fn site_already_bound_intra_file_is_not_resolved_cross_file() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "b.py", "python"));
        g.add_file(mk_file(1, "main.py", "python"));
        g.add_callable(mk_callable(0, "mk", "b.mk", 0, "python", (0, 40)));
        g.add_callable(mk_callable(1, "mk", "main.mk", 1, "python", (130, 170)));
        g.add_callable(mk_callable(2, "top", "main.top", 1, "python", (30, 120)));

        // The intra-file pass already bound the call site at byte 60
        // (inside `top`) to the local `mk` (callable 1) — this is what
        // `graph.edges` holds by the time `cross_file::resolve` runs.
        g.edges.push(CallEdge {
            src: CallableId::new(2),
            dst: CallableId::new(1),
            site_line: 5,
            site_byte: 60,
            confidence: Confidence::High,
            via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
            weight: 1,
        });

        let main_facts = facts_for(
            1,
            "main.py",
            "python",
            vec![
                mk_def("top", "main.top", DefVariant::FreeFunction, (30, 120)),
                mk_def("mk", "main.mk", DefVariant::FreeFunction, (130, 170)),
            ],
            vec![RefRecord {
                name: "mk".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![ImportRecord {
                kind: "from-import".into(),
                path: "b".into(),
                alias: "mk".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let b_facts = facts_for(
            0,
            "b.py",
            "python",
            vec![mk_def("mk", "b.mk", DefVariant::FreeFunction, (0, 40))],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[b_facts, main_facts]);

        assert!(
            out.edges.is_empty(),
            "the site is already bound intra-file (to `main.mk`); cross-file \
             resolution must not add a second edge to `b.mk` at the same site, \
             got: {:?}",
            out.edges
        );
    }

    /// c4: a bare cross-file call that falls to the step-1d global
    /// by-simple-name fallback must not fan out into a private helper
    /// of an unrelated `::tests::` module, must not fan out into a
    /// private item whose module is not an ancestor of the caller's,
    /// and must still reach a private item that IS an ancestor module
    /// and any `pub` candidate. Mirrors VERIFIED.md §3.4's worked
    /// example: caller `x::a::f`, candidates private `x::b::tests::h`
    /// (dropped), private `x::h` (kept — ancestor), `pub x::c::h`
    /// (kept).
    #[test]
    fn rust_private_candidates_filtered_by_visibility_and_tests_module() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "b/tests_helpers.rs", "rust"));
        g.add_file(mk_file(1, "other.rs", "rust"));
        g.add_file(mk_file(2, "c.rs", "rust"));
        g.add_file(mk_file(3, "a.rs", "rust"));

        // Private helper living under an unrelated `::tests::` module —
        // unreachable from ordinary code in x::a.
        let mut tests_h = mk_callable(10, "h", "x::b::tests::h", 0, "rust", (0, 10));
        tests_h.visibility = String::new();
        tests_h.vis = cgg_core::Vis::Private;
        g.add_callable(tests_h);

        // Private item directly in `x`, an ancestor of the caller's own
        // module `x::a` — Rust visibility reaches into descendant
        // modules, so this one IS reachable.
        let mut ancestor_h = mk_callable(11, "h", "x::h", 1, "rust", (0, 10));
        ancestor_h.visibility = String::new();
        ancestor_h.vis = cgg_core::Vis::Private;
        g.add_callable(ancestor_h);

        // Public item in an unrelated module — always reachable.
        let mut pub_h = mk_callable(12, "h", "x::c::h", 2, "rust", (0, 10));
        pub_h.visibility = "pub".into();
        g.add_callable(pub_h);

        g.add_callable(mk_callable(1, "f", "x::a::f", 3, "rust", (30, 120)));

        let caller_facts = facts_for(
            3,
            "a.rs",
            "rust",
            vec![mk_def("f", "x::a::f", DefVariant::FreeFunction, (30, 120))],
            vec![RefRecord {
                name: "h".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            // An unrelated import so step-1d's `has_imports` gate is open —
            // it never resolves "h" itself.
            vec![ImportRecord {
                kind: "use".into(),
                path: "x::a::unused".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, std::slice::from_ref(&caller_facts));

        let dsts: std::collections::HashSet<CallableId> =
            out.edges.iter().map(|e| e.dst).collect();
        assert!(
            !dsts.contains(&CallableId::new(10)),
            "a private helper under an unrelated ::tests:: module must \
             not be a cross-file candidate: {dsts:?}"
        );
        assert!(
            dsts.contains(&CallableId::new(11)),
            "a private item in an ancestor module of the caller must \
             still resolve: {dsts:?}"
        );
        assert!(
            dsts.contains(&CallableId::new(12)),
            "a pub item must still resolve: {dsts:?}"
        );
        assert_eq!(
            dsts.len(),
            2,
            "exactly the two reachable candidates: {dsts:?}"
        );
    }

    /// c4 (amendment): the visibility filter must only ever SHRINK a
    /// candidate set base was going to emit — never turn an over-cap
    /// set (which base suppresses outright) into an under-cap set by
    /// dropping unreachable candidates first. Seven raw candidates for
    /// `obj.h()`, six of them private helpers under another file's
    /// `::tests::` module (so the c4 filter alone would leave only
    /// one survivor, well under the cap of 5) — but the cap decision
    /// must see all seven, exactly as it would with the filter absent,
    /// and suppress the site entirely.
    #[test]
    fn fanout_cap_is_decided_before_the_visibility_filter_not_after() {
        let mut g = Graph::new();
        for i in 0..6u32 {
            g.add_file(mk_file(i, &format!("t{i}/tests_helpers.rs"), "rust"));
            let mut c = mk_callable(
                20 + i,
                "h",
                &format!("x::t{i}::tests::h"),
                i,
                "rust",
                (0, 10),
            );
            c.visibility = String::new();
            c.vis = cgg_core::Vis::Private;
            g.add_callable(c);
        }
        g.add_file(mk_file(6, "c.rs", "rust"));
        let mut survivor = mk_callable(26, "h", "x::c::h", 6, "rust", (0, 10));
        survivor.visibility = "pub".into();
        g.add_callable(survivor);

        g.add_file(mk_file(99, "a.rs", "rust"));
        g.add_callable(mk_callable(1, "f", "x::a::f", 99, "rust", (30, 120)));

        let caller_facts = facts_for(
            99,
            "a.rs",
            "rust",
            vec![mk_def("f", "x::a::f", DefVariant::FreeFunction, (30, 120))],
            vec![RefRecord {
                name: "h".into(),
                receiver_hint: "obj".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, std::slice::from_ref(&caller_facts));

        assert!(
            out.edges.is_empty(),
            "an over-cap site must stay suppressed even though the \
             visibility filter alone would leave only one candidate: \
             {:?}",
            out.edges
        );
        assert_eq!(out.unresolved.len(), 1);
        assert_eq!(
            out.unresolved[0].reason,
            UnresolvedReason::FanoutCapExceeded { candidates: 7 },
            "the cap must be judged on the pre-filter count of 7, not \
             the post-filter count of 1"
        );
    }

    // c1: path-headed receivers (VERIFIED §1f, §3.1). `super::util::helper()`
    // called from `crate_x::a::b::f` must resolve to `crate_x::a::util::helper`
    // — `super` rewritten to the caller's module one level up (`crate_x::a`),
    // not left embedded literally in a candidate qualified name that can
    // never match `by_qn`.
    //
    // A second, unrelated `helper` (`crate_x::other::decoy::helper`) is
    // deliberately included: without it, step 5's by-simple fan-out finds
    // the right target anyway because "helper" happens to be a unique
    // simple name in a 2-function fixture ("115 got exactly one edge
    // (unique simple name, luck)" — VERIFIED §1f). With a second `helper`
    // in the graph, the unfixed code's fan-out is ambiguous (width 2) and
    // emits both; the fix's exact `by_qn` lookup after rewriting `super`
    // is unaffected by the decoy and still emits exactly one edge.
    #[test]
    fn kotlin_extension_signatures_are_recognised() {
        assert!(is_kotlin_extension("fun String.shout(): String", "shout"));
        assert!(is_kotlin_extension(
            "@Suppress(\"x\") private fun <T> List<T>.firstUpper(n: Int)",
            "firstUpper"
        ));
        assert!(is_kotlin_extension("suspend fun CoroutineScope.go()", "go"));
        assert!(is_kotlin_extension("fun Foo?.orEmpty()", "orEmpty"));
        // Near misses: a plain function, a member, a parameter of a
        // function type with a receiver.
        assert!(!is_kotlin_extension(
            "fun shout(s: String): String",
            "shout"
        ));
        assert!(!is_kotlin_extension("override fun toString()", "toString"));
        assert!(!is_kotlin_extension(
            "fun run(block: Foo.() -> Unit)",
            "run"
        ));
        assert!(!is_kotlin_extension("", "x"));
    }

    #[test]
    fn rust_super_receiver_rewritten_via_caller_module() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "a/util.rs", "rust"));
        g.add_file(mk_file(1, "a/b.rs", "rust"));
        g.add_file(mk_file(2, "other/decoy.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "helper",
            "crate_x::a::util::helper",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "f",
            "crate_x::a::b::f",
            1,
            "rust",
            (30, 120),
        ));
        g.add_callable(mk_callable(
            2,
            "helper",
            "crate_x::other::decoy::helper",
            2,
            "rust",
            (0, 40),
        ));

        let util_facts = facts_for(
            0,
            "a/util.rs",
            "rust",
            vec![mk_def(
                "helper",
                "crate_x::a::util::helper",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let b_facts = facts_for(
            1,
            "a/b.rs",
            "rust",
            vec![mk_def(
                "f",
                "crate_x::a::b::f",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "helper".into(),
                receiver_hint: "super::util".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );
        let decoy_facts = facts_for(
            2,
            "other/decoy.rs",
            "rust",
            vec![mk_def(
                "helper",
                "crate_x::other::decoy::helper",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[util_facts, b_facts, decoy_facts]);
        assert_eq!(
            out.edges.len(),
            1,
            "super::util::helper() from crate_x::a::b::f must resolve to \
             exactly one edge (crate_x::a::util::helper), not fan out to \
             every same-named helper() in the workspace. Got: {:?}",
            out.edges
        );
        assert_eq!(out.edges[0].dst, CallableId::new(0));
        // The exact-match confidence upgrade (`resolve`'s `confidence`
        // match arm, ~1169-1181) only grants `High` via
        // `direct_imports`/`module_aliases`, which this path does not
        // go through — it stays `Medium` here even though the match is
        // exact. Promoting it is the separate "+115 sites ... to
        // exact-High" improvement VERIFIED §3.1 describes; out of scope
        // for this fix, which only changes `try_resolve_ref`.
        assert_eq!(out.edges[0].confidence, Confidence::Medium);
    }

    // c1 (c): an unresolved path-headed receiver whose head names no
    // crate or module cgg ever indexed (`other_crate`, standing in for
    // `serde_json`/`toml`/`cc` in the real corpus) must not fall into
    // step 5's by-simple fan-out and claim the one local `Foo::from_str`
    // as its target — that is precisely how `serde_json::from_str` became
    // a false edge into `RollupLevel::from_str` (VERIFIED §1f).
    #[test]
    fn rust_external_crate_head_produces_no_edge() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "foo.rs", "rust"));
        g.add_file(mk_file(1, "caller.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "from_str",
            "crate_x::Foo::from_str",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "run",
            "crate_x::caller::run",
            1,
            "rust",
            (30, 120),
        ));

        let foo_facts = facts_for(
            0,
            "foo.rs",
            "rust",
            vec![mk_def(
                "from_str",
                "crate_x::Foo::from_str",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let caller_facts = facts_for(
            1,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "crate_x::caller::run",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "from_str".into(),
                receiver_hint: "other_crate::sub".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, &[foo_facts, caller_facts]);
        assert!(
            out.edges.is_empty(),
            "other_crate::sub::from_str(..) must not resolve to the local \
             Foo::from_str just because it is the only from_str in the \
             workspace — other_crate is not a crate cgg ever indexed, so \
             this can only be an external call. Got edges: {:?}",
            out.edges
        );
    }

    // A one-segment path receiver (`serde_json::from_str(..)`, receiver
    // `serde_json`) has no `::` of its own, so step 4c never saw it and
    // step 5 treated the crate name as a variable: every external
    // `serde_json::from_str` fanned out to the workspace's `FromStr`
    // impls (450 false edges on one 23-crate workspace).
    #[test]
    fn rust_one_segment_external_path_receiver_produces_no_edge() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "foo.rs", "rust"));
        g.add_file(mk_file(1, "caller.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "from_str",
            "crate_x::Foo::from_str",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "run",
            "crate_x::caller::run",
            1,
            "rust",
            (30, 120),
        ));
        let foo_facts = facts_for(
            0,
            "foo.rs",
            "rust",
            vec![mk_def(
                "from_str",
                "crate_x::Foo::from_str",
                DefVariant::InherentMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let mk_ref = |path_receiver| RefRecord {
            name: "from_str".into(),
            receiver_hint: "serde_json".into(),
            path_receiver,
            site_line: 5,
            site_byte: 60,
            ..Default::default()
        };
        let caller = |r| {
            facts_for(
                1,
                "caller.rs",
                "rust",
                vec![mk_def(
                    "run",
                    "crate_x::caller::run",
                    DefVariant::FreeFunction,
                    (30, 120),
                )],
                vec![r],
                vec![],
            )
        };
        let out = resolve_default(&g, &[foo_facts.clone(), caller(mk_ref(true))]);
        assert!(
            out.edges.is_empty(),
            "serde_json::from_str bound locally: {:?}",
            out.edges
        );
        // The same text as a VALUE receiver (`serde_json.from_str()`, a
        // variable) is still duck-typed exactly as before.
        let out = resolve_default(&g, &[foo_facts, caller(mk_ref(false))]);
        assert_eq!(
            out.edges.len(),
            1,
            "value receivers keep step 5: {:?}",
            out.edges
        );
    }

    // F11: `mod admin_jobs;` + `admin_jobs::CascadeRunner::spawn()` with no
    // `use` names a child module of the caller's crate. Step 4c judged the
    // head external (it is not a crate root) and returned before the
    // owner index — which holds the right callee — was consulted.
    #[test]
    fn rust_in_workspace_module_head_is_not_external() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "admin_jobs.rs", "rust"));
        g.add_file(mk_file(1, "main.rs", "rust"));
        let qn = "srv::admin_jobs::<CascadeRunner as Job>::spawn";
        g.add_callable(mk_callable(0, "spawn", qn, 0, "rust", (0, 40)));
        g.add_callable(mk_callable(1, "main", "srv::main", 1, "rust", (30, 120)));
        let jobs = facts_for(
            0,
            "admin_jobs.rs",
            "rust",
            vec![mk_def("spawn", qn, DefVariant::TraitMethod, (0, 40))],
            vec![],
            vec![],
        );
        let main = facts_for(
            1,
            "main.rs",
            "rust",
            vec![mk_def(
                "main",
                "srv::main",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "spawn".into(),
                receiver_hint: "admin_jobs::CascadeRunner".into(),
                path_receiver: true,
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );
        let out = resolve_default(&g, &[jobs, main]);
        assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
    }

    // A value typed as an instantiated container (`user: Option<&User>`)
    // reaches a local extension trait implemented FOR that container,
    // keyed under the impl's base owner. A path call never does.
    #[test]
    fn rust_value_typed_as_container_reaches_its_extension_trait_impl() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "user.rs", "rust"));
        g.add_file(mk_file(1, "caller.rs", "rust"));
        let qn = "k::user::<Option<&User> as UserHelper>::person_id";
        g.add_callable(mk_callable(0, "person_id", qn, 0, "rust", (0, 40)));
        g.add_callable(mk_callable(
            1,
            "run",
            "k::caller::run",
            1,
            "rust",
            (30, 120),
        ));
        let defs = facts_for(
            0,
            "user.rs",
            "rust",
            vec![mk_def("person_id", qn, DefVariant::TraitMethod, (0, 40))],
            vec![],
            vec![],
        );
        let caller = |path_receiver| {
            facts_for(
                1,
                "caller.rs",
                "rust",
                vec![mk_def(
                    "run",
                    "k::caller::run",
                    DefVariant::FreeFunction,
                    (30, 120),
                )],
                vec![RefRecord {
                    name: "person_id".into(),
                    receiver_hint: "Option".into(),
                    path_receiver,
                    site_line: 5,
                    site_byte: 60,
                    ..Default::default()
                }],
                vec![],
            )
        };
        let out = resolve_default(&g, &[defs.clone(), caller(false)]);
        assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
        let out = resolve_default(&g, &[defs, caller(true)]);
        assert!(
            out.edges.is_empty(),
            "Option::person_id(..) path: {:?}",
            out.edges
        );
    }

    // F6: six `VectorIndex` impls of `score_set` exceed the cap of 5, but
    // they are one trait's implementations — the site is a dispatch
    // through that trait and binds to its declaration rather than being
    // dropped. One inherent method in the set, or a call-chain receiver,
    // keeps the cap.
    #[test]
    fn over_cap_homogeneous_trait_set_binds_to_the_declaration() {
        let build = |extra_inherent: bool, hint: &str| {
            let mut g = Graph::new();
            g.add_file(mk_file(0, "idx.rs", "rust"));
            g.add_file(mk_file(1, "caller.rs", "rust"));
            let mut defs = vec![];
            let mut add = |id: u32, qn: String, variant, target: Option<&str>| {
                let mut c =
                    mk_callable(id, "score_set", &qn, 0, "rust", (id * 50, id * 50 + 40));
                c.trait_impl_target = target.map(str::to_string);
                g.add_callable(c);
                defs.push(mk_def("score_set", &qn, variant, (id * 50, id * 50 + 40)));
            };
            add(
                0,
                "k::VectorIndex::score_set".into(),
                DefVariant::TraitMethod,
                None,
            );
            for i in 1..=6 {
                add(
                    i,
                    format!("k::<B{i} as VectorIndex>::score_set"),
                    DefVariant::TraitMethod,
                    Some("VectorIndex"),
                );
            }
            if extra_inherent {
                add(
                    7,
                    "k::Cache::score_set".into(),
                    DefVariant::InherentMethod,
                    None,
                );
            }
            g.add_callable(mk_callable(
                20,
                "run",
                "k::caller::run",
                1,
                "rust",
                (30, 120),
            ));
            let idx = facts_for(0, "idx.rs", "rust", defs, vec![], vec![]);
            let caller = facts_for(
                1,
                "caller.rs",
                "rust",
                vec![mk_def(
                    "run",
                    "k::caller::run",
                    DefVariant::FreeFunction,
                    (30, 120),
                )],
                vec![RefRecord {
                    name: "score_set".into(),
                    receiver_hint: hint.into(),
                    site_line: 5,
                    site_byte: 60,
                    ..Default::default()
                }],
                vec![],
            );
            (g, vec![idx, caller])
        };
        let (g, f) = build(false, "s.vec_index");
        let out = resolve_default(&g, &f);
        assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
        assert_eq!(out.edges[0].dst, CallableId::new(0));
        let (g, f) = build(true, "s.vec_index");
        assert!(resolve_default(&g, &f).edges.is_empty());
        let (g, f) = build(false, "s.index()");
        assert!(resolve_default(&g, &f).edges.is_empty());
    }

    // F7: `--guess-unique-names` binds an unplaced site to the one callable
    // carrying its name, at `low`, under a distinct resolver tag — and only
    // when asked. A stdlib name, a typed receiver and an external path
    // never guess.
    #[test]
    fn unique_name_guess_is_opt_in_low_and_guarded() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "req.rs", "rust"));
        g.add_file(mk_file(1, "caller.rs", "rust"));
        for (id, name, qn) in [
            (0, "with_purpose", "k::req::CompletionRequest::with_purpose"),
            (1, "iter", "k::req::Bag::iter"),
        ] {
            let mut c = mk_callable(id, name, qn, 0, "rust", (id * 50, id * 50 + 40));
            c.visibility = "pub".into();
            g.add_callable(c);
        }
        g.add_callable(mk_callable(
            5,
            "run",
            "k::caller::run",
            1,
            "rust",
            (30, 400),
        ));
        let defs = facts_for(
            0,
            "req.rs",
            "rust",
            vec![
                mk_def(
                    "with_purpose",
                    "k::req::CompletionRequest::with_purpose",
                    DefVariant::InherentMethod,
                    (0, 40),
                ),
                mk_def(
                    "iter",
                    "k::req::Bag::iter",
                    DefVariant::InherentMethod,
                    (50, 90),
                ),
            ],
            vec![],
            vec![],
        );
        let r = |name: &str, hint: &str, path_receiver, byte| RefRecord {
            name: name.into(),
            receiver_hint: hint.into(),
            path_receiver,
            site_line: 5,
            site_byte: byte,
            ..Default::default()
        };
        let caller = facts_for(
            1,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "k::caller::run",
                DefVariant::FreeFunction,
                (30, 400),
            )],
            vec![
                r("with_purpose", "{ build() }", false, 60),
                r("iter", "bag", false, 70),
                r("with_purpose", "Other", false, 80),
                r("with_purpose", "ext_crate", true, 90),
            ],
            vec![],
        );
        let facts = [defs, caller];
        assert!(
            resolve(&g, &facts, DEFAULT_FANOUT_CAP, false, false)
                .edges
                .is_empty()
        );
        let out = resolve(&g, &facts, DEFAULT_FANOUT_CAP, false, true);
        assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
        assert_eq!(out.edges[0].site_byte, 60);
        assert_eq!(out.edges[0].confidence, Confidence::Low);
        assert_eq!(out.edges[0].resolver.as_str(), "cross-file:name-unique");
    }

    // An external wrapper with one argument (`State<AppState>`) derefs to
    // it: a method the wrapper lacks is looked up on the argument. A
    // `Sender<Msg>` does not, and a renamed crate / glob-imported module
    // head is in-workspace.
    #[test]
    fn rust_deref_wrapper_and_glob_and_renamed_heads() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "state.rs", "rust"));
        g.add_file(mk_file(1, "caller.rs", "rust"));
        let defs = [
            (0, "pool", "k::state::AppState::pool"),
            (1, "send", "k::state::Msg::send"),
            (2, "run", "k::jobs::Runner::run"),
            (3, "go", "k::util::go"),
        ];
        let mut def_records = vec![];
        for (id, name, qn) in defs {
            g.add_callable(mk_callable(
                id,
                name,
                qn,
                0,
                "rust",
                (id * 50, id * 50 + 40),
            ));
            def_records.push(mk_def(
                name,
                qn,
                DefVariant::InherentMethod,
                (id * 50, id * 50 + 40),
            ));
        }
        g.add_callable(mk_callable(9, "main", "app::main", 1, "rust", (30, 400)));
        let r = |name: &str, hint: &str, path_receiver, byte| RefRecord {
            name: name.into(),
            receiver_hint: hint.into(),
            path_receiver,
            site_line: 5,
            site_byte: byte,
            ..Default::default()
        };
        let imp = |path: &str, alias: &str| ImportRecord {
            kind: "use".into(),
            path: path.into(),
            alias: alias.into(),
            site_line: 1,
            site_byte: 0,
        };
        let caller = facts_for(
            1,
            "caller.rs",
            "rust",
            vec![mk_def(
                "main",
                "app::main",
                DefVariant::FreeFunction,
                (30, 400),
            )],
            vec![
                r("pool", "State<AppState>", false, 60),
                r("send", "Sender<Msg>", false, 70),
                r("run", "jobs::Runner", true, 80),
                r("go", "u", true, 90),
            ],
            vec![imp("k::*", "*"), imp("k", "u")],
        );
        let facts = [
            facts_for(0, "state.rs", "rust", def_records, vec![], vec![]),
            caller,
        ];
        let out = resolve_default(&g, &facts);
        let mut hit: Vec<u32> = out.edges.iter().map(|e| e.site_byte).collect();
        hit.sort();
        assert_eq!(hit, [60, 80, 90], "{:?}", out.edges);
    }

    // A crate made only of re-exports contributes no qualified name, but
    // its crate-root record still makes it a workspace crate; and a file
    // outside any Cargo crate (`crate::…`) is never judged by one-segment
    // path heads.
    #[test]
    fn rust_reexport_only_crate_and_manifestless_files_are_not_external() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "extra.rs", "rust"));
        g.add_file(mk_file(1, "main.rs", "rust"));
        g.add_file(mk_file(2, "salvo_lib.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "max_size",
            "salvo_extra::size_limiter::max_size",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(1, "main", "app::main", 1, "rust", (30, 400)));
        let crate_root = |c: &str| ImportRecord {
            kind: "crate-root".into(),
            path: c.into(),
            alias: String::new(),
            site_line: 1,
            site_byte: 0,
        };
        let mut reexport = crate_root("salvo_extra::size_limiter");
        reexport.kind = "pub-use".into();
        let r = RefRecord {
            name: "max_size".into(),
            receiver_hint: "size_limiter".into(),
            path_receiver: true,
            site_line: 5,
            site_byte: 60,
            ..Default::default()
        };
        let mut imp = crate_root("salvo::size_limiter");
        imp.kind = "use".into();
        let facts = [
            facts_for(
                0,
                "extra.rs",
                "rust",
                vec![mk_def(
                    "max_size",
                    "salvo_extra::size_limiter::max_size",
                    DefVariant::FreeFunction,
                    (0, 40),
                )],
                vec![],
                vec![crate_root("salvo_extra")],
            ),
            facts_for(
                1,
                "main.rs",
                "rust",
                vec![mk_def(
                    "main",
                    "app::main",
                    DefVariant::FreeFunction,
                    (30, 400),
                )],
                vec![r],
                vec![crate_root("app"), imp],
            ),
            facts_for(
                2,
                "salvo_lib.rs",
                "rust",
                vec![],
                vec![],
                vec![crate_root("salvo"), reexport],
            ),
        ];
        let out = resolve_default(&g, &facts);
        assert_eq!(out.edges.len(), 1, "{:?} {:?}", out.edges, out.unresolved);
    }

    // A wasm-bindgen extern method is its `this` type's method, and a
    // `self.m()` whose impl block sits in another file of the same crate
    // binds by exact owner — never to another crate's same-named owner.
    #[test]
    fn rust_wasm_bindgen_methods_and_cross_file_self_calls() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "sys.rs", "rust"));
        g.add_file(mk_file(1, "conn.rs", "rust"));
        g.add_file(mk_file(2, "caller.rs", "rust"));
        let mut tx = mk_callable(
            0,
            "transaction",
            "sys::storage::transaction",
            0,
            "rust",
            (0, 40),
        );
        tx.attributes = vec!["#[wasm_bindgen(method, catch)]".into()];
        tx.signature_hint = "pub fn transaction(this: &DurableObjectStorage) -> R".into();
        g.add_callable(tx);
        g.add_callable(mk_callable(
            1,
            "is_ready",
            "k::conn::Connection::is_ready",
            1,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            2,
            "is_ready",
            "other::Connection::is_ready",
            1,
            "rust",
            (50, 90),
        ));
        g.add_callable(mk_callable(
            3,
            "handle",
            "k::handlers::Connection::handle",
            2,
            "rust",
            (30, 400),
        ));
        let r = |name: &str, hint: &str, byte| RefRecord {
            name: name.into(),
            receiver_hint: hint.into(),
            site_line: 5,
            site_byte: byte,
            ..Default::default()
        };
        let facts = [
            facts_for(0, "sys.rs", "rust", vec![], vec![], vec![]),
            facts_for(1, "conn.rs", "rust", vec![], vec![], vec![]),
            facts_for(
                2,
                "caller.rs",
                "rust",
                vec![mk_def(
                    "handle",
                    "k::handlers::Connection::handle",
                    DefVariant::InherentMethod,
                    (30, 400),
                )],
                vec![
                    r("transaction", "DurableObjectStorage", 60),
                    r("is_ready", "self", 70),
                ],
                vec![],
            ),
        ];
        let out = resolve_default(&g, &facts);
        let mut got: Vec<(u32, CallableId)> =
            out.edges.iter().map(|e| (e.site_byte, e.dst)).collect();
        got.sort();
        assert_eq!(
            got,
            [(60, CallableId::new(0)), (70, CallableId::new(1))],
            "{:?}",
            out.edges
        );
    }

    /// A chained call — `classifier().classify(..)` — gives its inner
    /// and outer references the SAME `site_byte`: tree-sitter starts
    /// the outer `call_expression` at the same byte as the receiver
    /// expression it wraps. Keying the intra-bound skip on
    /// `(src, site_byte)` alone conflated the two: once intra-file
    /// bound the inner `inner()` at that byte, the outer `outer()` ref
    /// at the identical byte was skipped too, even though intra-file
    /// never bound IT and cross-file could. The key must include the
    /// referenced name so the two refs at one site are told apart.
    #[test]
    fn chained_call_same_site_byte_different_names_both_resolve() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "b.py", "python"));
        g.add_file(mk_file(1, "main.py", "python"));
        g.add_callable(mk_callable(0, "outer", "b.outer", 0, "python", (0, 40)));
        g.add_callable(mk_callable(
            1,
            "inner",
            "main.inner",
            1,
            "python",
            (130, 170),
        ));
        g.add_callable(mk_callable(2, "top", "main.top", 1, "python", (30, 120)));

        // Intra-file already bound the `inner` ref at byte 60 — but not
        // `outer`, which shares the same byte in a chained call.
        g.edges.push(CallEdge {
            src: CallableId::new(2),
            dst: CallableId::new(1),
            site_line: 5,
            site_byte: 60,
            confidence: Confidence::High,
            via: Via::Direct,
            resolver: ResolverId::new("intra-file"),
            weight: 1,
        });

        let main_facts = facts_for(
            1,
            "main.py",
            "python",
            vec![
                mk_def("top", "main.top", DefVariant::FreeFunction, (30, 120)),
                mk_def("inner", "main.inner", DefVariant::FreeFunction, (130, 170)),
            ],
            vec![
                RefRecord {
                    name: "inner".into(),
                    receiver_hint: "".into(),
                    site_line: 5,
                    site_byte: 60,
                    ..Default::default()
                },
                RefRecord {
                    name: "outer".into(),
                    receiver_hint: "".into(),
                    site_line: 5,
                    site_byte: 60,
                    ..Default::default()
                },
            ],
            vec![ImportRecord {
                kind: "from-import".into(),
                path: "b".into(),
                alias: "outer".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let b_facts = facts_for(
            0,
            "b.py",
            "python",
            vec![mk_def(
                "outer",
                "b.outer",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[b_facts, main_facts]);

        assert_eq!(
            out.edges.len(),
            1,
            "the `inner` ref is already bound intra-file and must stay \
             suppressed, but the `outer` ref at the identical site_byte \
             names a different callable and must still resolve; got: {:?}",
            out.edges
        );
        assert_eq!(
            out.edges[0].dst,
            CallableId::new(0),
            "the resolved edge must be to `b.outer`, not a re-resolution of `inner`"
        );
    }

    /// c4 (second amendment): a trait item's `visibility` string is
    /// empty whether it is truly private or merely inherited from a
    /// trait declared elsewhere — `rust.rs` records the latter as
    /// `Vis::Unknown`, deliberately, precisely so a reader does not
    /// conflate the two. The filter must key off the normalized `vis`
    /// enum and treat only `Vis::Private` as private; `Vis::Unknown`
    /// (trait items, inherited visibility) must stay reachable even
    /// from another file with no ancestor relationship to the caller.
    #[test]
    fn trait_method_with_unknown_vis_is_not_treated_as_private() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "device.rs", "rust"));
        g.add_file(mk_file(1, "a.rs", "rust"));

        let mut kick =
            mk_callable(30, "kick", "x::b::VirtioDevice::kick", 0, "rust", (0, 10));
        kick.kind = CallableKind::Method;
        kick.visibility = String::new();
        kick.vis = cgg_core::Vis::Unknown;
        g.add_callable(kick);

        g.add_callable(mk_callable(1, "f", "x::a::f", 1, "rust", (30, 120)));

        let caller_facts = facts_for(
            1,
            "a.rs",
            "rust",
            vec![mk_def("f", "x::a::f", DefVariant::FreeFunction, (30, 120))],
            // A method call (`dev.kick()`): the visibility filter is the
            // same one step 5's fan-out applies. A *bare* `kick()` cannot
            // name a trait method in Rust at all (FINDINGS A6), so it no
            // longer reaches the filter this test is about.
            vec![RefRecord {
                name: "kick".into(),
                receiver_hint: "dev".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            // An unrelated import so step-1d's `has_imports` gate is open.
            vec![ImportRecord {
                kind: "use".into(),
                path: "x::a::unused".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, std::slice::from_ref(&caller_facts));

        let dsts: std::collections::HashSet<CallableId> =
            out.edges.iter().map(|e| e.dst).collect();
        assert!(
            dsts.contains(&CallableId::new(30)),
            "a trait item recorded with Vis::Unknown must stay reachable \
             from another file, unlike a genuinely private item: {dsts:?}"
        );
    }

    /// c12: an enum-variant receiver (`TrustKind::Network.untrusted_input()`)
    /// arrives at `try_resolve_ref` as `receiver_hint = "TrustKind::Network"`.
    /// Step 4's existing owner tries are the full hint (`"TrustKind::Network"`,
    /// no such qualified owner) and its bare last segment (`"Network"`, the
    /// *variant*, not a type with methods) — both miss, so the site was lost
    /// even though `TrustKind::untrusted_input` is defined and unambiguous.
    /// The receiver's own prefix (`"TrustKind"`, the enum type) must also be
    /// tried as an owner.
    #[test]
    fn rust_enum_variant_receiver_resolves_to_owner_method() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "m.rs", "rust"));
        g.add_file(mk_file(1, "main.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "untrusted_input",
            "m::TrustKind::untrusted_input",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(1, "main", "crate::main", 1, "rust", (30, 120)));

        let main_facts = facts_for(
            1,
            "main.rs",
            "rust",
            vec![mk_def(
                "main",
                "crate::main",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "untrusted_input".into(),
                receiver_hint: "TrustKind::Network".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );
        let m_facts = facts_for(
            0,
            "m.rs",
            "rust",
            vec![mk_def(
                "untrusted_input",
                "m::TrustKind::untrusted_input",
                DefVariant::InherentMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[m_facts, main_facts]);
        assert_eq!(out.edges.len(), 1, "expected one cross-file edge");
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    // c1 amendment (verifier): a BARE `self` receiver (`self.method()`,
    // from a field_expression, no `::` anywhere in it) must be left
    // completely untouched by step 3c and must not fall into step 5's
    // fan-out. An earlier version of this fix rewrote bare `self` using
    // the caller's own qualified name, which produced a lowercase
    // non-"self" string that slipped past the `rh != "self"` guards in
    // steps 4 and 5 — measured on llmitm-v5: 88 of 218 added edges were
    // exactly this, e.g.
    // `vmm/src/devices/virtio/rng/event_handler.rs:99`, where base
    // correctly leaves `self.method()` unresolved (cross-file resolution
    // never owns a bare `self` receiver — that's the intra-file linker's
    // job) and the bug bound it to an unrelated same-named method two
    // files away.
    #[test]
    fn rust_bare_self_receiver_is_never_cross_file_resolved() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "caller.rs", "rust"));
        g.add_file(mk_file(1, "a.rs", "rust"));
        g.add_file(mk_file(2, "b.rs", "rust"));
        // The caller sits inside a trait impl — the exact shape the
        // regression was found in (`impl Trait for T`).
        g.add_callable(mk_callable(0, "m", "x::<T as Tr>::m", 0, "rust", (30, 120)));
        // Two unrelated types elsewhere both happen to define a method
        // with the same simple name as the one `self.process()` calls.
        g.add_callable(mk_callable(
            1,
            "process_rate_limiter_event",
            "a::TypeA::process",
            1,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            2,
            "process_rate_limiter_event",
            "b::TypeB::process",
            2,
            "rust",
            (0, 40),
        ));

        let caller_facts = facts_for(
            0,
            "caller.rs",
            "rust",
            vec![mk_def(
                "m",
                "x::<T as Tr>::m",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "process_rate_limiter_event".into(),
                receiver_hint: "self".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );
        let a_facts = facts_for(
            1,
            "a.rs",
            "rust",
            vec![mk_def(
                "process_rate_limiter_event",
                "a::TypeA::process",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let b_facts = facts_for(
            2,
            "b.rs",
            "rust",
            vec![mk_def(
                "process_rate_limiter_event",
                "b::TypeB::process",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[caller_facts, a_facts, b_facts]);
        assert!(
            out.edges.is_empty(),
            "self.process() must not resolve cross-file to either \
             TypeA::process or TypeB::process — a bare `self` receiver \
             is not a path. Got edges: {:?}",
            out.edges
        );
    }

    // c1 amendment (verifier), narrowed in the third amendment:
    // `std::io::Error::from(..)` must not resolve to an unrelated local
    // `<Foo as From<E>>::from` just because "Foo" happens to be the
    // only local `from` in the graph — measured false edge on
    // llmitm-v5: `uffd_utils.rs:184` `std::io::Error::from(errno)`
    // bound to `<StartMicrovmError as From<..>>::from`, caused by
    // `names::owner_from_qn` mis-filing `StartMicrovmError` under the
    // bare owner "Error" (the nested-generic-in-trait-position bug,
    // now fixed — see `names::tests`). With that fixed there is no
    // "Error"-keyed entry to find at all, so this resolves correctly
    // without needing step 4's entry gated on the receiver's head being
    // a known crate (that gate was tried and reverted: it also blocked
    // `kvm_bindings::CpuId::try_from`, a real external head with a
    // genuinely local trait impl).
    #[test]
    fn rust_std_from_call_does_not_bind_to_local_from_impl() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "caller.rs", "rust"));
        g.add_file(mk_file(1, "errors.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "run",
            "crate_x::caller::run",
            0,
            "rust",
            (30, 120),
        ));
        g.add_callable(mk_callable(
            1,
            "from",
            "<Foo as From<SomeError>>::from",
            1,
            "rust",
            (0, 40),
        ));

        let caller_facts = facts_for(
            0,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "crate_x::caller::run",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "from".into(),
                receiver_hint: "std::io::Error".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );
        let errors_facts = facts_for(
            1,
            "errors.rs",
            "rust",
            vec![mk_def(
                "from",
                "<Foo as From<SomeError>>::from",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[caller_facts, errors_facts]);
        assert!(
            out.edges.is_empty(),
            "std::io::Error::from(..) must not resolve to an unrelated \
             local <Foo as From<..>>::from just because it is the only \
             from() in the graph. Got edges: {:?}",
            out.edges
        );
    }

    /// c13: a local `impl From<Foo> for String` must not make
    /// `by_owner_method` answer for every bare `String::from(..)` call in
    /// the tree. Verified on /home/dev/Documents/Github/llmitm-v5:
    /// `String::from("root")` at vmm/src/builder.rs:1192 resolved to
    /// `clippy_tracing::<String as From<StripVisitor>>::from` because the
    /// index was keyed on the bare owner name `String`.
    #[test]
    fn std_type_local_trait_impl_not_captured_by_bare_owner_name() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "lib.rs", "rust"));
        g.add_file(mk_file(1, "main.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "from",
            "m::<String as From<Foo>>::from",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(1, "main", "crate::main", 1, "rust", (30, 120)));

        let main_facts = facts_for(
            1,
            "main.rs",
            "rust",
            vec![mk_def(
                "main",
                "crate::main",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "from".into(),
                receiver_hint: "String".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );
        let lib_facts = facts_for(
            0,
            "lib.rs",
            "rust",
            vec![mk_def(
                "from",
                "m::<String as From<Foo>>::from",
                DefVariant::TraitMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[lib_facts, main_facts]);
        assert_eq!(
            out.edges.len(),
            0,
            "a bare `String` receiver must not be captured by a local \
             trait impl for a std type — got edges: {:?}",
            out.edges
        );
    }

    /// c13 companion: the explicit trait-impl path must still resolve —
    /// the fix removes the bare-owner shortcut, not the exact match.
    #[test]
    fn std_type_local_trait_impl_resolves_via_exact_receiver_path() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "lib.rs", "rust"));
        g.add_file(mk_file(1, "main.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "from",
            "m::<String as From<Foo>>::from",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(1, "main", "crate::main", 1, "rust", (30, 120)));

        let main_facts = facts_for(
            1,
            "main.rs",
            "rust",
            vec![mk_def(
                "main",
                "crate::main",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "from".into(),
                receiver_hint: "m::<String as From<Foo>>".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );
        let lib_facts = facts_for(
            0,
            "lib.rs",
            "rust",
            vec![mk_def(
                "from",
                "m::<String as From<Foo>>::from",
                DefVariant::TraitMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[lib_facts, main_facts]);
        assert_eq!(
            out.edges.len(),
            1,
            "exact trait-impl path must still resolve"
        );
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    // c1 second amendment (verifier): `rust_path_heads` used to hold only
    // the FIRST segment of each `by_qn` key, so a receiver written
    // relative to a NESTED module (`arg_parser::Arguments` where the real
    // qn is `mycrate::utils::arg_parser::Arguments::single_value` —
    // `arg_parser` is never the first segment) was wrongly treated as
    // external and lost — 57 true edges on llmitm-v5. Every module/type
    // segment on the path, not just the crate root, must be a known
    // head.
    #[test]
    fn rust_nested_module_head_resolves_via_its_own_import() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "utils/arg_parser.rs", "rust"));
        g.add_file(mk_file(1, "caller.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "single_value",
            "mycrate::utils::arg_parser::Arguments::single_value",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "run",
            "mycrate::caller::run",
            1,
            "rust",
            (30, 120),
        ));

        let arg_facts = facts_for(
            0,
            "utils/arg_parser.rs",
            "rust",
            vec![mk_def(
                "single_value",
                "mycrate::utils::arg_parser::Arguments::single_value",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let caller_facts = facts_for(
            1,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "mycrate::caller::run",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "single_value".into(),
                receiver_hint: "arg_parser::Arguments".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![ImportRecord {
                kind: "use".into(),
                path: "mycrate::utils::arg_parser".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, &[arg_facts, caller_facts]);
        assert_eq!(
            out.edges.len(),
            1,
            "arg_parser::Arguments::single_value() must resolve via the \
             \"use mycrate::utils::arg_parser;\" import in this file — \
             \"arg_parser\" is never the FIRST segment of any qualified \
             name by itself. Got: {:?}",
            out.edges
        );
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    // Companion negative case for the same fix: a receiver whose head is
    // genuinely absent from the workspace — at any depth, not just as a
    // first segment — must still be refused rather than fanning out.
    #[test]
    fn rust_unknown_nested_head_still_produces_no_edge() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "utils/arg_parser.rs", "rust"));
        g.add_file(mk_file(1, "caller.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "single_value",
            "mycrate::utils::arg_parser::Arguments::single_value",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "run",
            "mycrate::caller::run",
            1,
            "rust",
            (30, 120),
        ));

        let arg_facts = facts_for(
            0,
            "utils/arg_parser.rs",
            "rust",
            vec![mk_def(
                "single_value",
                "mycrate::utils::arg_parser::Arguments::single_value",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let caller_facts = facts_for(
            1,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "mycrate::caller::run",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                // "Thing" collides with nothing local; "other_crate" is
                // not a workspace crate at any depth.
                name: "single_value".into(),
                receiver_hint: "other_crate::Thing".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, &[arg_facts, caller_facts]);
        assert!(
            out.edges.is_empty(),
            "other_crate::Thing::single_value() must not resolve just \
             because \"single_value\" is the workspace's only method of \
             that name — other_crate is external at every depth. Got: \
             {:?}",
            out.edges
        );
    }

    /// c4 (third amendment, part 1): `rust.rs`'s `record_macro` files
    /// every `macro_rules!` item as `Vis::Private` with no real
    /// module-privacy meaning (a macro's reachability follows
    /// `#[macro_export]` / the importing `use`, not module nesting) and
    /// marks it with the `"macro"` attribute. The visibility filter must
    /// exempt it, or every cross-file macro use is dropped.
    #[test]
    fn a_private_macro_callable_in_another_file_is_kept() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "macros.rs", "rust"));
        g.add_file(mk_file(1, "a.rs", "rust"));

        let mut mac = mk_callable(
            40,
            "check_metric_after_block",
            "x::b::check_metric_after_block",
            0,
            "rust",
            (0, 10),
        );
        mac.visibility = String::new();
        mac.vis = cgg_core::Vis::Private;
        mac.attributes = vec!["macro".to_string()];
        g.add_callable(mac);

        g.add_callable(mk_callable(1, "f", "x::a::f", 1, "rust", (30, 120)));

        let caller_facts = facts_for(
            1,
            "a.rs",
            "rust",
            vec![mk_def("f", "x::a::f", DefVariant::FreeFunction, (30, 120))],
            vec![RefRecord {
                name: "check_metric_after_block".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            // An unrelated import so step-1d's `has_imports` gate is open.
            vec![ImportRecord {
                kind: "use".into(),
                path: "x::a::unused".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, std::slice::from_ref(&caller_facts));

        let dsts: std::collections::HashSet<CallableId> =
            out.edges.iter().map(|e| e.dst).collect();
        assert!(
            dsts.contains(&CallableId::new(40)),
            "a macro_rules! item recorded as Vis::Private must stay \
             reachable from another file: {dsts:?}"
        );
    }

    /// c4 (third amendment, part 2): the `::tests::` rule exists to
    /// catch an *accidental* same-simple-name fan-out, not to override
    /// an import the caller wrote on purpose. A `use a::b::tests::*;`
    /// (or an exact single-item `use`) of that tests module makes its
    /// helpers legitimately callable even though the caller's own
    /// qualified name shares no `::tests::` prefix.
    #[test]
    fn tests_module_helper_is_kept_when_caller_imports_it_with_a_glob() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "b/tests_helpers.rs", "rust"));
        g.add_file(mk_file(1, "a.rs", "rust"));

        let mut helper = mk_callable(
            50,
            "default_kernel_cmdline",
            "x::b::tests::default_kernel_cmdline",
            0,
            "rust",
            (0, 10),
        );
        helper.visibility = "pub".into();
        g.add_callable(helper);

        g.add_callable(mk_callable(1, "f", "x::a::f", 1, "rust", (30, 120)));

        let caller_facts = facts_for(
            1,
            "a.rs",
            "rust",
            vec![mk_def("f", "x::a::f", DefVariant::FreeFunction, (30, 120))],
            vec![RefRecord {
                name: "default_kernel_cmdline".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![ImportRecord {
                kind: "use".into(),
                path: "x::b::tests::*".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, std::slice::from_ref(&caller_facts));

        let dsts: std::collections::HashSet<CallableId> =
            out.edges.iter().map(|e| e.dst).collect();
        assert!(
            dsts.contains(&CallableId::new(50)),
            "a ::tests:: helper the caller explicitly `use`s via a glob \
             must stay reachable even with no shared ::tests:: prefix: \
             {dsts:?}"
        );
    }

    /// c4 (third amendment, part 2 continued): the "use" import arm
    /// stores a path verbatim, so `use crate::builder::tests::*;` is
    /// recorded literally as `"crate::builder::tests::*"` while the
    /// candidate's own qualified name carries the resolved crate root.
    /// Measured on llmitm-v5: `default_kernel_cmdline` stayed short 4
    /// of 17 edges (persist.rs, pci_mngr.rs, device_manager/mod.rs) —
    /// every miss a file that wrote the literal `crate::` form — until
    /// the leading `crate` segment is rewritten to the caller's own
    /// crate root before comparing.
    #[test]
    fn tests_module_helper_is_kept_with_a_literal_crate_glob_import() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "b/tests_helpers.rs", "rust"));
        g.add_file(mk_file(1, "a.rs", "rust"));

        let mut helper = mk_callable(
            51,
            "default_kernel_cmdline",
            "vmm::b::tests::default_kernel_cmdline",
            0,
            "rust",
            (0, 10),
        );
        helper.visibility = "pub".into();
        g.add_callable(helper);

        g.add_callable(mk_callable(1, "f", "vmm::a::f", 1, "rust", (30, 120)));

        let caller_facts = facts_for(
            1,
            "a.rs",
            "rust",
            vec![mk_def(
                "f",
                "vmm::a::f",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "default_kernel_cmdline".into(),
                receiver_hint: "".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            // The literal, unresolved form the Rust plugin actually
            // stores for `use crate::b::tests::*;`.
            vec![ImportRecord {
                kind: "use".into(),
                path: "crate::b::tests::*".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, std::slice::from_ref(&caller_facts));

        let dsts: std::collections::HashSet<CallableId> =
            out.edges.iter().map(|e| e.dst).collect();
        assert!(
            dsts.contains(&CallableId::new(51)),
            "a ::tests:: helper imported via the literal `use crate::...::*;` \
             form must stay reachable: {dsts:?}"
        );
    }

    // c3 second amendment (VERIFIED §1b / §3.3): a ref extracted from
    // inside a macro token tree can carry a `receiver_hint` inferred from
    // raw token adjacency — a type alias's bare name, a file-wide
    // `var_types` guess — that names the WRONG owner. Base (with no
    // receiver_hint at all for these refs) found such calls through the
    // plain bare-name/global-fallback path; qualifying them on a wrong
    // owner loses the edge outright. `out.notices()` inside `assert!` in
    // cgg's own `replay.rs`, typed `Vec` by a file-wide guess when the
    // real type is `RunOutcome`, is the measured real-world case this
    // reproduces in miniature.
    #[test]
    fn macro_arg_ref_retries_bare_on_exactly_one_crate_wide_candidate() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "outcome.rs", "rust"));
        g.add_file(mk_file(1, "replay.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "notices",
            "crate::outcome::RunOutcome::notices",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "test_fn",
            "crate::test_fn",
            1,
            "rust",
            (30, 120),
        ));

        let outcome_facts = facts_for(
            0,
            "outcome.rs",
            "rust",
            vec![mk_def(
                "notices",
                "crate::outcome::RunOutcome::notices",
                DefVariant::InherentMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let replay_facts = facts_for(
            1,
            "replay.rs",
            "rust",
            vec![mk_def(
                "test_fn",
                "crate::test_fn",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "notices".into(),
                // Wrong owner: no `Vec::notices` exists anywhere, so
                // steps 2-5 (qualified) all fail.
                receiver_hint: "Vec".into(),
                site_line: 5,
                site_byte: 60,
                from_macro_arg: true,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, &[outcome_facts, replay_facts]);
        assert_eq!(
            out.edges.len(),
            1,
            "a from_macro_arg ref with a dead-end receiver must retry bare \
             when exactly one crate-wide candidate exists; got {:?}",
            out.edges
        );
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    // The retry MUST stay strictly stricter than step 1d's ≤3: with two
    // crate-wide candidates for the same simple name, throwing away the
    // (wrong) receiver and guessing is no longer safe, so no edge must be
    // manufactured.
    #[test]
    fn macro_arg_ref_retry_declines_when_crate_wide_candidates_are_ambiguous() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "outcome.rs", "rust"));
        g.add_file(mk_file(1, "replay.rs", "rust"));
        g.add_file(mk_file(2, "other.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "notices",
            "crate::outcome::RunOutcome::notices",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "test_fn",
            "crate::test_fn",
            1,
            "rust",
            (30, 120),
        ));
        g.add_callable(mk_callable(
            2,
            "notices",
            "crate::other::Foo::notices",
            2,
            "rust",
            (0, 40),
        ));

        let outcome_facts = facts_for(
            0,
            "outcome.rs",
            "rust",
            vec![mk_def(
                "notices",
                "crate::outcome::RunOutcome::notices",
                DefVariant::InherentMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let other_facts = facts_for(
            2,
            "other.rs",
            "rust",
            vec![mk_def(
                "notices",
                "crate::other::Foo::notices",
                DefVariant::InherentMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let replay_facts = facts_for(
            1,
            "replay.rs",
            "rust",
            vec![mk_def(
                "test_fn",
                "crate::test_fn",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "notices".into(),
                receiver_hint: "Vec".into(),
                site_line: 5,
                site_byte: 60,
                from_macro_arg: true,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, &[outcome_facts, other_facts, replay_facts]);
        assert!(
            out.edges.is_empty(),
            "two crate-wide candidates must not be guessed between; got {:?}",
            out.edges
        );
    }

    // c1 third amendment: `io::Error::from(x)` via `use std::io;` must
    // resolve exactly like the un-aliased `std::io::Error::from` case —
    // step 4c's import-alias substitution must reach the "std"
    // blocklist (step 4 itself finds nothing here since the local
    // `Foo` owner does not match "io::Error"/"Error" either way).
    #[test]
    fn rust_aliased_std_from_call_does_not_bind_to_local_from_impl() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "caller.rs", "rust"));
        g.add_file(mk_file(1, "errors.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "run",
            "mycrate::caller::run",
            0,
            "rust",
            (30, 120),
        ));
        g.add_callable(mk_callable(
            1,
            "from",
            "<Foo as From<SomeError>>::from",
            1,
            "rust",
            (0, 40),
        ));

        let caller_facts = facts_for(
            0,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "mycrate::caller::run",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "from".into(),
                receiver_hint: "io::Error".into(),
                site_line: 5,
                site_byte: 60,
                ..Default::default()
            }],
            vec![ImportRecord {
                kind: "use".into(),
                path: "std::io".into(),
                alias: "".into(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let errors_facts = facts_for(
            1,
            "errors.rs",
            "rust",
            vec![mk_def(
                "from",
                "<Foo as From<SomeError>>::from",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        let out = resolve_default(&g, &[caller_facts, errors_facts]);
        assert!(
            out.edges.is_empty(),
            "io::Error::from(..) via \"use std::io;\" must not resolve \
             to an unrelated local <Foo as From<..>>::from — the import \
             target's crate root is \"std\". Got edges: {:?}",
            out.edges
        );
    }

    // c1 third amendment: two `TryFrom` impls in each direction between
    // the same pair of types must never conflate — this is the
    // `names::owner_from_qn` nested-generic-in-trait-position fix,
    // exercised end to end through the resolver rather than as a
    // `names.rs` unit test.
    #[test]
    fn rust_try_from_pair_resolves_to_its_own_direction_only() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "conv.rs", "rust"));
        g.add_file(mk_file(1, "caller.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "try_from",
            "mycrate::conv::<A as TryFrom<B>>::try_from",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "try_from",
            "mycrate::conv::<B as TryFrom<A>>::try_from",
            0,
            "rust",
            (50, 90),
        ));
        g.add_callable(mk_callable(
            2,
            "run",
            "mycrate::caller::run",
            1,
            "rust",
            (100, 140),
        ));

        let conv_facts = facts_for(
            0,
            "conv.rs",
            "rust",
            vec![
                mk_def(
                    "try_from",
                    "mycrate::conv::<A as TryFrom<B>>::try_from",
                    DefVariant::FreeFunction,
                    (0, 40),
                ),
                mk_def(
                    "try_from",
                    "mycrate::conv::<B as TryFrom<A>>::try_from",
                    DefVariant::FreeFunction,
                    (50, 90),
                ),
            ],
            vec![],
            vec![],
        );
        let caller_facts = facts_for(
            1,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "mycrate::caller::run",
                DefVariant::FreeFunction,
                (100, 140),
            )],
            vec![RefRecord {
                name: "try_from".into(),
                receiver_hint: "A".into(),
                site_line: 5,
                site_byte: 110,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, &[conv_facts, caller_facts]);
        assert_eq!(
            out.edges.len(),
            1,
            "A::try_from(..) must resolve to exactly the impl owned by \
             A, never both directions. Got: {:?}",
            out.edges
        );
        assert_eq!(out.edges[0].dst, CallableId::new(0));
    }

    // c1 fourth (final) amendment (verifier): when a receiver's head is
    // external, step 4 must accept a candidate only by FULL-PATH owner
    // match, never a bare-last-segment match. `ext_crate::Kvm::new()`
    // must not bind to a local `mycrate::vstate::kvm::Kvm::new` just
    // because both owners reduce to bare "Kvm" — measured false edge on
    // llmitm-v5: `kvm_ioctls::Kvm::new()` bound to local
    // `vmm::vstate::kvm::Kvm::new`. It DOES bind to a local trait impl
    // written for that exact external type
    // (`mycrate::<ext_crate::Kvm as Tr>::m`, full owner
    // "ext_crate::Kvm" — matches the receiver verbatim).
    #[test]
    fn rust_external_head_owner_match_requires_full_path() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "kvm.rs", "rust"));
        g.add_file(mk_file(1, "trait_impl.rs", "rust"));
        g.add_file(mk_file(2, "caller.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "new",
            "mycrate::vstate::kvm::Kvm::new",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "m",
            "mycrate::<ext_crate::Kvm as Tr>::m",
            1,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            2,
            "run",
            "mycrate::caller::run",
            2,
            "rust",
            (100, 200),
        ));

        let kvm_facts = facts_for(
            0,
            "kvm.rs",
            "rust",
            vec![mk_def(
                "new",
                "mycrate::vstate::kvm::Kvm::new",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let trait_impl_facts = facts_for(
            1,
            "trait_impl.rs",
            "rust",
            vec![mk_def(
                "m",
                "mycrate::<ext_crate::Kvm as Tr>::m",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );

        // `ext_crate::Kvm::new()` — bare-owner "Kvm" collides with the
        // local Kvm::new, but the FULL owner "ext_crate::Kvm" only
        // matches the trait impl, whose method is "m", not "new". So
        // this call must resolve to NOTHING.
        let caller_no_match = facts_for(
            2,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "mycrate::caller::run",
                DefVariant::FreeFunction,
                (100, 200),
            )],
            vec![RefRecord {
                name: "new".into(),
                receiver_hint: "ext_crate::Kvm".into(),
                site_line: 5,
                site_byte: 110,
                ..Default::default()
            }],
            vec![],
        );
        let out = resolve_default(
            &g,
            &[kvm_facts.clone(), trait_impl_facts.clone(), caller_no_match],
        );
        assert!(
            out.edges.is_empty(),
            "ext_crate::Kvm::new() must not bind to the local Kvm::new \
             just because both owners reduce to bare \"Kvm\" — \
             ext_crate is external. Got: {:?}",
            out.edges
        );

        // `ext_crate::Kvm::m()` — the SAME full owner, but now the
        // method matches the trait impl exactly. Must resolve to
        // exactly that one edge.
        let caller_match = facts_for(
            2,
            "caller.rs",
            "rust",
            vec![mk_def(
                "run",
                "mycrate::caller::run",
                DefVariant::FreeFunction,
                (100, 200),
            )],
            vec![RefRecord {
                name: "m".into(),
                receiver_hint: "ext_crate::Kvm".into(),
                site_line: 5,
                site_byte: 110,
                ..Default::default()
            }],
            vec![],
        );
        let out = resolve_default(&g, &[kvm_facts, trait_impl_facts, caller_match]);
        assert_eq!(
            out.edges.len(),
            1,
            "ext_crate::Kvm::m() must bind to mycrate::<ext_crate::Kvm \
             as Tr>::m — its full owner matches the receiver verbatim. \
             Got: {:?}",
            out.edges
        );
        assert_eq!(out.edges[0].dst, CallableId::new(1));
    }

    // Step 5, macro-argument receiver: two inherent methods of the same
    // name on unrelated types is a guess and binds nothing; a trait's
    // declaration beside an impl of that trait narrows to the impl.
    #[test]
    fn macro_arg_receiver_fanout_binds_only_a_single_survivor() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "a.rs", "rust"));
        g.add_file(mk_file(1, "b.rs", "rust"));
        g.add_file(mk_file(2, "t.rs", "rust"));
        g.add_callable(mk_callable(0, "m", "mycrate::a::A::m", 0, "rust", (0, 40)));
        g.add_callable(mk_callable(1, "m", "mycrate::b::B::m", 1, "rust", (0, 40)));
        g.add_callable(mk_callable(2, "t", "mycrate::t::t", 2, "rust", (30, 120)));
        let a = facts_for(
            0,
            "a.rs",
            "rust",
            vec![mk_def(
                "m",
                "mycrate::a::A::m",
                DefVariant::InherentMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let b = facts_for(
            1,
            "b.rs",
            "rust",
            vec![mk_def(
                "m",
                "mycrate::b::B::m",
                DefVariant::InherentMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let t = facts_for(
            2,
            "t.rs",
            "rust",
            vec![mk_def(
                "t",
                "mycrate::t::t",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "m".into(),
                receiver_hint: "x".into(),
                site_line: 5,
                site_byte: 60,
                from_macro_arg: true,
                ..Default::default()
            }],
            vec![],
        );
        let out = resolve_default(&g, &[a, b, t]);
        assert!(
            out.edges.is_empty(),
            "two inherent candidates must not be guessed between; got {:?}",
            out.edges
        );

        let mut g = Graph::new();
        g.add_file(mk_file(0, "tr.rs", "rust"));
        g.add_file(mk_file(1, "i.rs", "rust"));
        g.add_file(mk_file(2, "t.rs", "rust"));
        g.add_callable(mk_callable(
            0,
            "m",
            "mycrate::tr::Tr::m",
            0,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(
            1,
            "m",
            "mycrate::i::<Foo as Tr>::m",
            1,
            "rust",
            (0, 40),
        ));
        g.add_callable(mk_callable(2, "t", "mycrate::t::t", 2, "rust", (30, 120)));
        let tr = facts_for(
            0,
            "tr.rs",
            "rust",
            vec![mk_def(
                "m",
                "mycrate::tr::Tr::m",
                DefVariant::TraitMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let i = facts_for(
            1,
            "i.rs",
            "rust",
            vec![mk_def(
                "m",
                "mycrate::i::<Foo as Tr>::m",
                DefVariant::TraitMethod,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let t = facts_for(
            2,
            "t.rs",
            "rust",
            vec![mk_def(
                "t",
                "mycrate::t::t",
                DefVariant::FreeFunction,
                (30, 120),
            )],
            vec![RefRecord {
                name: "m".into(),
                receiver_hint: "x".into(),
                site_line: 5,
                site_byte: 60,
                from_macro_arg: true,
                ..Default::default()
            }],
            vec![],
        );
        let out = resolve_default(&g, &[tr, i, t]);
        assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
        assert_eq!(
            out.edges[0].dst,
            CallableId::new(1),
            "the impl, not the declaration"
        );
    }

    /// A C++ file that `#include`s a C header must resolve a bare call
    /// to the `.c` body, not the header prototype. Both share one
    /// `(language, qualified_name)` slot; the later callable — the body,
    /// when the header is indexed first, as `include/` sorts before
    /// `src/` — is the one a same-language lookup would return.
    #[test]
    fn cpp_file_resolves_call_to_c_definition_via_include() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "quicklz.h", "c"));
        g.add_file(mk_file(1, "quicklz.c", "c"));
        g.add_file(mk_file(2, "Player.cpp", "cpp"));

        // Header prototype first, body second. A sibling index that
        // keeps the first id would bind the call to the prototype.
        g.add_callable(mk_callable(
            0,
            "qlz_decompress",
            "qlz_decompress",
            0,
            "c",
            (0, 60),
        ));
        g.add_callable(mk_callable(
            1,
            "qlz_decompress",
            "qlz_decompress",
            1,
            "c",
            (100, 500),
        ));
        let mut caller = mk_callable(
            2,
            "decompress",
            "Player::decompress",
            2,
            "cpp",
            (5000, 6000),
        );
        caller.kind = CallableKind::Method;
        g.add_callable(caller);

        let h_facts = facts_for(
            0,
            "quicklz.h",
            "c",
            vec![mk_def(
                "qlz_decompress",
                "qlz_decompress",
                DefVariant::FreeFunction,
                (0, 60),
            )],
            vec![],
            vec![],
        );
        let c_facts = facts_for(
            1,
            "quicklz.c",
            "c",
            vec![mk_def(
                "qlz_decompress",
                "qlz_decompress",
                DefVariant::FreeFunction,
                (100, 500),
            )],
            vec![],
            vec![ImportRecord {
                kind: "include".into(),
                path: "quicklz.h".into(),
                alias: String::new(),
                site_line: 1,
                site_byte: 0,
            }],
        );
        let cpp_facts = facts_for(
            2,
            "Player.cpp",
            "cpp",
            vec![mk_def(
                "decompress",
                "Player::decompress",
                DefVariant::InherentMethod,
                (5000, 6000),
            )],
            vec![RefRecord {
                name: "qlz_decompress".into(),
                receiver_hint: String::new(),
                site_line: 110,
                site_byte: 5500,
                ..Default::default()
            }],
            vec![ImportRecord {
                kind: "include".into(),
                path: "quicklz.h".into(),
                alias: String::new(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, &[h_facts, c_facts, cpp_facts]);
        let dsts: Vec<_> = out
            .edges
            .iter()
            .filter(|e| e.src == CallableId::new(2))
            .map(|e| e.dst)
            .collect();
        assert_eq!(
            dsts,
            vec![CallableId::new(1)],
            "the include must land on the C body, not the prototype; \
             unresolved: {:?}",
            out.unresolved
                .iter()
                .filter(|u| u.name == "qlz_decompress")
                .map(|u| &u.reason)
                .collect::<Vec<_>>()
        );
    }

    /// Including some other header must not make every C function of
    /// the same simple name visible. That is what widening `by_simple`
    /// would do: the file has an import, step 1d's cap is 3, and one
    /// C definition would become an edge.
    #[test]
    fn cpp_include_does_not_import_every_c_function() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "other.h", "c"));
        g.add_file(mk_file(1, "quicklz.c", "c"));
        g.add_file(mk_file(2, "Player.cpp", "cpp"));

        g.add_callable(mk_callable(0, "helper", "helper", 0, "c", (0, 40)));
        g.add_callable(mk_callable(
            1,
            "qlz_decompress",
            "qlz_decompress",
            1,
            "c",
            (100, 500),
        ));
        let mut caller = mk_callable(
            2,
            "decompress",
            "Player::decompress",
            2,
            "cpp",
            (5000, 6000),
        );
        caller.kind = CallableKind::Method;
        g.add_callable(caller);

        let other = facts_for(
            0,
            "other.h",
            "c",
            vec![mk_def(
                "helper",
                "helper",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let c_facts = facts_for(
            1,
            "quicklz.c",
            "c",
            vec![mk_def(
                "qlz_decompress",
                "qlz_decompress",
                DefVariant::FreeFunction,
                (100, 500),
            )],
            vec![],
            vec![],
        );
        let cpp_facts = facts_for(
            2,
            "Player.cpp",
            "cpp",
            vec![mk_def(
                "decompress",
                "Player::decompress",
                DefVariant::InherentMethod,
                (5000, 6000),
            )],
            vec![RefRecord {
                name: "qlz_decompress".into(),
                receiver_hint: String::new(),
                site_line: 110,
                site_byte: 5500,
                ..Default::default()
            }],
            vec![ImportRecord {
                kind: "include".into(),
                path: "other.h".into(),
                alias: String::new(),
                site_line: 1,
                site_byte: 0,
            }],
        );

        let out = resolve_default(&g, &[other, c_facts, cpp_facts]);
        let dsts: Vec<_> = out
            .edges
            .iter()
            .filter(|e| e.src == CallableId::new(2))
            .map(|e| e.dst)
            .collect();
        assert!(
            dsts.is_empty(),
            "qlz_decompress is not included by Player.cpp; edges: {dsts:?}"
        );
    }

    /// A typed-looking C++ call fans out over C++ methods only. A C
    /// function of the same simple name is not a candidate.
    #[test]
    fn cpp_fanout_ignores_c_functions() {
        let mut g = Graph::new();
        g.add_file(mk_file(0, "util.c", "c"));
        g.add_file(mk_file(1, "Widget.cpp", "cpp"));
        g.add_file(mk_file(2, "Player.cpp", "cpp"));

        g.add_callable(mk_callable(0, "update", "update", 0, "c", (0, 40)));
        let mut method = mk_callable(1, "update", "Widget::update", 1, "cpp", (0, 80));
        method.kind = CallableKind::Method;
        g.add_callable(method);
        let mut caller = mk_callable(2, "tick", "Player::tick", 2, "cpp", (100, 200));
        caller.kind = CallableKind::Method;
        g.add_callable(caller);

        let c_facts = facts_for(
            0,
            "util.c",
            "c",
            vec![mk_def(
                "update",
                "update",
                DefVariant::FreeFunction,
                (0, 40),
            )],
            vec![],
            vec![],
        );
        let widget = facts_for(
            1,
            "Widget.cpp",
            "cpp",
            vec![mk_def(
                "update",
                "Widget::update",
                DefVariant::InherentMethod,
                (0, 80),
            )],
            vec![],
            vec![],
        );
        let player = facts_for(
            2,
            "Player.cpp",
            "cpp",
            vec![mk_def(
                "tick",
                "Player::tick",
                DefVariant::InherentMethod,
                (100, 200),
            )],
            vec![RefRecord {
                name: "update".into(),
                receiver_hint: "obj".into(),
                site_line: 4,
                site_byte: 150,
                ..Default::default()
            }],
            vec![],
        );

        let out = resolve_default(&g, &[c_facts, widget, player]);
        let dsts: Vec<_> = out
            .edges
            .iter()
            .filter(|e| e.src == CallableId::new(2))
            .map(|e| e.dst)
            .collect();
        assert_eq!(
            dsts,
            vec![CallableId::new(1)],
            "fan-out must stay inside C++; edges: {dsts:?}"
        );
    }

    // ---- FINDINGS A1: external module / package heads ----------------

    /// One target callable per entry `(path, qualified name, packages)`
    /// plus a caller file whose single callable `caller_qn` (with the
    /// given bases) holds `refs`; returns the destination qualified names
    /// and the reasons of every unresolved record.
    fn ext_case(
        lang: &str,
        targets: &[(&str, &str, &[&str])],
        caller: (&str, &str, &[&str], &[&str]),
        refs: Vec<RefRecord>,
        imports: Vec<ImportRecord>,
        extra: Vec<FileFacts>,
    ) -> (Vec<String>, Vec<UnresolvedReason>) {
        let simple = |qn: &str| qn.rsplit(['.', ':']).next().unwrap().to_string();
        let mut g = Graph::new();
        let mut facts = Vec::new();
        for (i, (path, qn, pk)) in targets.iter().enumerate() {
            let id = i as u32;
            g.add_file(mk_file(id, path, lang));
            g.add_callable(mk_callable(id, &simple(qn), qn, id, lang, (0, 40)));
            let mut f = facts_for(
                id,
                path,
                lang,
                vec![mk_def(&simple(qn), qn, DefVariant::FreeFunction, (0, 40))],
                vec![],
                vec![],
            );
            f.packages = pk.iter().map(|s| s.to_string()).collect();
            // The Go plugin always records the package clause; the
            // module-scoped lookup keys a package's names by it.
            if lang == "go" {
                f.imports.push(ImportRecord {
                    kind: "package-root".into(),
                    path: qn.split('.').next().unwrap_or("").into(),
                    alias: String::new(),
                    site_line: 1,
                    site_byte: 0,
                });
            }
            facts.push(f);
        }
        let (cpath, cqn, cpk, cbases) = caller;
        let cid = targets.len() as u32;
        g.add_file(mk_file(cid, cpath, lang));
        g.add_callable(mk_callable(cid, &simple(cqn), cqn, cid, lang, (30, 400)));
        let mut def = mk_def(&simple(cqn), cqn, DefVariant::InherentMethod, (30, 400));
        def.base_types = cbases.iter().map(|s| s.to_string()).collect();
        let mut f = facts_for(cid, cpath, lang, vec![def], refs, imports);
        f.packages = cpk.iter().map(|s| s.to_string()).collect();
        facts.push(f);
        facts.extend(extra);
        let out = resolve_default(&g, &facts);
        let dsts = out
            .edges
            .iter()
            .filter(|e| e.src == CallableId::new(cid))
            .map(|e| g.callables[&e.dst].qualified_name.clone())
            .collect();
        (dsts, out.unresolved.into_iter().map(|u| u.reason).collect())
    }

    fn call(name: &str, recv: &str) -> RefRecord {
        RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            site_line: 5,
            site_byte: 60,
            ..Default::default()
        }
    }

    fn import(kind: &str, path: &str, alias: &str) -> ImportRecord {
        ImportRecord {
            kind: kind.into(),
            path: path.into(),
            alias: alias.into(),
            site_line: 1,
            site_byte: 0,
        }
    }

    fn external(module: &str) -> UnresolvedReason {
        UnresolvedReason::ExternalModule {
            module: module.into(),
        }
    }

    #[test]
    fn python_external_module_head_is_never_name_matched() {
        // `import requests; requests.post()` must not reach a workspace
        // `post` the file never imported — it was HIGH before.
        let t: &[(&str, &str, &[&str])] = &[("app/util.py", "app.util.post", &[])];
        let caller = ("app/main.py", "app.main.run", &[][..], &[][..]);
        let (dsts, why) = ext_case(
            "python",
            t,
            caller,
            vec![call("post", "requests")],
            vec![import("import", "requests", "")],
            vec![],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("requests")]);
        // Near miss: the same call through a workspace module alias.
        let (dsts, _) = ext_case(
            "python",
            t,
            caller,
            vec![call("post", "u")],
            vec![import("import", "app.util", "u")],
            vec![],
        );
        assert_eq!(dsts, vec!["app.util.post".to_string()]);
    }

    #[test]
    fn python_name_imported_from_a_dependency_does_not_bind_bare() {
        let t: &[(&str, &str, &[&str])] = &[("app/a.py", "app.a.post", &[])];
        let caller = ("app/main.py", "app.main.run", &[][..], &[][..]);
        let (dsts, why) = ext_case(
            "python",
            t,
            caller,
            vec![call("post", "")],
            vec![import("from-import", "requests", "post")],
            vec![],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("requests")]);
        let (dsts, _) = ext_case(
            "python",
            t,
            caller,
            vec![call("post", "")],
            vec![import("from-import", "app.a", "post")],
            vec![],
        );
        assert_eq!(dsts, vec!["app.a.post".to_string()]);
    }

    #[test]
    fn js_dependency_head_refused_workspace_package_kept() {
        let caller = ("src/main.js", "main", &[][..], &[][..]);
        let (dsts, why) = ext_case(
            "javascript",
            &[("src/util.js", "post", &[])],
            caller,
            vec![call("post", "axios")],
            vec![import("from-import", "axios", "default as axios")],
            vec![],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("axios")]);
        // A bare specifier naming a package.json in the tree is the
        // workspace's (a monorepo package).
        let (dsts, _) = ext_case(
            "javascript",
            &[("packages/net/index.js", "post", &["@acme/net"])],
            caller,
            vec![call("post", "net")],
            vec![import("from-import", "@acme/net", "default as net")],
            vec![],
        );
        assert_eq!(dsts, vec!["post".to_string()]);
    }

    #[test]
    fn go_import_outside_every_module_is_external() {
        let caller = ("main.go", "main.run", &["example.com/app"][..], &[][..]);
        let (dsts, why) = ext_case(
            "go",
            &[("util/util.go", "util.IDs.NewString", &["example.com/app"])],
            caller,
            vec![call("NewString", "uuid")],
            vec![import("import", "github.com/google/uuid", "")],
            vec![],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("github.com/google/uuid")]);
        // Same-module package: resolves exactly as before.
        let (dsts, _) = ext_case(
            "go",
            &[(
                "internal/jobs/jobs.go",
                "jobs.SpawnJob",
                &["example.com/app"],
            )],
            caller,
            vec![call("SpawnJob", "jobs")],
            vec![import("import", "example.com/app/internal/jobs", "")],
            vec![],
        );
        assert_eq!(dsts, vec!["jobs.SpawnJob".to_string()]);
    }

    #[test]
    fn java_type_imported_from_a_dependency_is_not_the_local_class() {
        let t: &[(&str, &str, &[&str])] = &[(
            "src/com/acme/util/StringUtils.java",
            "com.acme.util.StringUtils.isBlank",
            &["com.acme.util"],
        )];
        let caller = (
            "src/com/acme/App.java",
            "com.acme.App.run",
            &["com.acme"][..],
            &[][..],
        );
        let (dsts, why) = ext_case(
            "java",
            t,
            caller,
            vec![call("isBlank", "StringUtils")],
            vec![import(
                "import",
                "org.apache.commons.lang3.StringUtils",
                "StringUtils",
            )],
            vec![],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("org.apache.commons.lang3")]);
        let (dsts, _) = ext_case(
            "java",
            t,
            caller,
            vec![call("isBlank", "StringUtils")],
            vec![import("import", "com.acme.util.StringUtils", "StringUtils")],
            vec![],
        );
        assert_eq!(dsts, vec!["com.acme.util.StringUtils.isBlank".to_string()]);
    }

    #[test]
    fn java_package_qualified_receiver_outside_the_tree_is_external() {
        let caller = (
            "src/com/acme/App.java",
            "com.acme.App.run",
            &["com.acme"][..],
            &[][..],
        );
        let (dsts, why) = ext_case(
            "java",
            &[(
                "src/com/acme/util/Helpers.java",
                "com.acme.util.Helpers.compare",
                &["com.acme.util"],
            )],
            caller,
            vec![call("compare", "java.lang.Integer")],
            vec![],
            vec![],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("java.lang")]);
        // A fully-qualified workspace class keeps resolving.
        let (dsts, _) = ext_case(
            "java",
            &[(
                "src/com/acme/jobs/Runner.java",
                "com.acme.jobs.Runner.spawnJob",
                &["com.acme.jobs"],
            )],
            caller,
            vec![call("spawnJob", "com.acme.jobs.Runner")],
            vec![],
            vec![],
        );
        assert_eq!(dsts, vec!["com.acme.jobs.Runner.spawnJob".to_string()]);
    }

    #[test]
    fn java_static_import_does_not_hide_an_inherited_member() {
        // `import static org.junit.Assert.check;` but the caller's base
        // class declares `check`: a member wins over a static import.
        let (dsts, _) = ext_case(
            "java",
            &[(
                "src/com/acme/Base.java",
                "com.acme.Base.check",
                &["com.acme"],
            )],
            (
                "src/com/acme/Child.java",
                "com.acme.Child.run",
                &["com.acme"],
                &["Base"],
            ),
            vec![call("check", "")],
            vec![import("from-import", "org.junit.Assert.check", "check")],
            vec![],
        );
        assert_eq!(dsts, vec!["com.acme.Base.check".to_string()]);
    }

    #[test]
    fn csharp_type_name_reaches_only_visible_namespaces() {
        let t: &[(&str, &str, &[&str])] =
            &[("Util.cs", "Acme.Util.Path.Combine", &["Acme.Util"])];
        let caller = ("App.cs", "Acme.App.Run", &["Acme"][..], &[][..]);
        // `using System.IO;` + `Path.Combine` is System.IO.Path's.
        let (dsts, why) = ext_case(
            "csharp",
            t,
            caller,
            vec![call("Combine", "Path")],
            vec![import("using", "System.IO", "")],
            vec![],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("Path")]);
        // Imported by a `using`, or by a `global using` anywhere.
        let (dsts, _) = ext_case(
            "csharp",
            t,
            caller,
            vec![call("Combine", "Path")],
            vec![import("using", "Acme.Util", "")],
            vec![],
        );
        assert_eq!(dsts, vec!["Acme.Util.Path.Combine".to_string()]);
        let mut globals =
            facts_for(9, "GlobalUsings.cs", "csharp", vec![], vec![], vec![]);
        globals.imports = vec![import("using", "global using Acme.Util", "")];
        let (dsts, _) = ext_case(
            "csharp",
            t,
            caller,
            vec![call("Combine", "Path")],
            vec![],
            vec![globals],
        );
        assert_eq!(dsts, vec!["Acme.Util.Path.Combine".to_string()]);
    }

    #[test]
    fn csharp_fully_qualified_system_receiver_is_external() {
        let (dsts, why) = ext_case(
            "csharp",
            &[("Util.cs", "Acme.Util.Path.GetTempPath", &["Acme.Util"])],
            ("App.cs", "Acme.App.Run", &["Acme"], &[]),
            vec![call("GetTempPath", "System.IO.Path")],
            vec![import("using", "System", "")],
            vec![],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("System")]);
    }

    #[test]
    fn cpp_namespace_declared_nowhere_is_external() {
        let t: &[(&str, &str, &[&str])] = &[("src/util.cpp", "acme::split", &[])];
        let caller = ("src/main.cpp", "main", &[][..], &[][..]);
        let path_call = |recv: &str| RefRecord {
            path_receiver: true,
            ..call("split", recv)
        };
        let (dsts, why) =
            ext_case("cpp", t, caller, vec![path_call("boost")], vec![], vec![]);
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("boost")]);
        // A workspace namespace, directly or through a namespace alias.
        let (dsts, _) =
            ext_case("cpp", t, caller, vec![path_call("acme")], vec![], vec![]);
        assert_eq!(dsts, vec!["acme::split".to_string()]);
        let (dsts, _) = ext_case(
            "cpp",
            t,
            caller,
            vec![path_call("a")],
            vec![import("namespace-alias", "acme", "a")],
            vec![],
        );
        assert_eq!(dsts, vec!["acme::split".to_string()]);
        // A variable receiver is not a scope path, whatever its name.
        let (dsts, _) = ext_case(
            "cpp",
            t,
            caller,
            vec![call("split", "boost")],
            vec![],
            vec![],
        );
        assert_eq!(dsts, vec!["acme::split".to_string()]);
    }

    #[test]
    fn cpp_macro_namespace_member_reached_through_an_include_is_kept() {
        // `namespace KWSYS_NAMESPACE {` is spelled `cmsys::` at the call
        // site; no qualified name carries `cmsys`.
        let t: &[(&str, &str, &[&str])] =
            &[("src/kwsys/Tools.hxx", "KWSYS_NAMESPACE::Tools::Exists", &[])];
        let caller = ("src/main.cpp", "main", &[][..], &[][..]);
        let path_call = |recv: &str| RefRecord {
            path_receiver: true,
            ..call("Exists", recv)
        };
        // A class on the path (`Tools`) is the tree's: kept.
        let (dsts, _) = ext_case(
            "cpp",
            t,
            caller,
            vec![path_call("cmsys::Tools")],
            vec![],
            vec![],
        );
        assert_eq!(dsts, vec!["KWSYS_NAMESPACE::Tools::Exists".to_string()]);
        // The member is declared in the file's include closure: kept.
        let (dsts, _) = ext_case(
            "cpp",
            t,
            caller,
            vec![path_call("cmsys")],
            vec![import("include", "kwsys/Tools.hxx", "")],
            vec![],
        );
        assert_eq!(dsts, vec!["KWSYS_NAMESPACE::Tools::Exists".to_string()]);
        // Neither: the head is undeclared and the call refused.
        let (dsts, why) =
            ext_case("cpp", t, caller, vec![path_call("cmsys")], vec![], vec![]);
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("cmsys")]);
    }

    // ----------------------------------------------------------------
    // FINDINGS A4/A5: calls through a workspace module bind inside it.
    // FINDINGS A6: a bare call never reaches an unrelated method.
    // ----------------------------------------------------------------

    /// One file of a module-scoped case.
    #[derive(Clone)]
    struct MFile<'a> {
        path: &'a str,
        /// `(qualified name, kind, signature)` of each callable.
        defs: Vec<(&'a str, CallableKind, &'a str)>,
        imports: Vec<ImportRecord>,
        packages: Vec<&'a str>,
        bases: Vec<&'a str>,
    }

    fn mfile<'a>(
        path: &'a str,
        defs: &[&'a str],
        imports: Vec<ImportRecord>,
    ) -> MFile<'a> {
        MFile {
            path,
            defs: defs
                .iter()
                .map(|q| (*q, CallableKind::Function, ""))
                .collect(),
            imports,
            packages: vec![],
            bases: vec![],
        }
    }

    /// Resolve `refs`, written inside the caller's first callable, over
    /// `files` + `caller`. Returns `(dst qualified name, dst path)` per
    /// edge and every unresolved reason.
    fn module_case(
        lang: &str,
        files: Vec<MFile<'_>>,
        caller: MFile<'_>,
        refs: Vec<RefRecord>,
    ) -> (Vec<(String, String)>, Vec<UnresolvedReason>) {
        module_case_with(lang, files, caller, refs, |_| {})
    }

    /// [`module_case`] with the caller's facts adjusted before resolving
    /// (its locals, say).
    fn module_case_with(
        lang: &str,
        files: Vec<MFile<'_>>,
        caller: MFile<'_>,
        refs: Vec<RefRecord>,
        tweak: impl FnOnce(&mut FileFacts),
    ) -> (Vec<(String, String)>, Vec<UnresolvedReason>) {
        let mut tweak = Some(tweak);
        module_case_each(lang, files, caller, refs, |is_caller, _, f| {
            if is_caller && let Some(t) = tweak.take() {
                t(f);
            }
        })
    }

    /// [`module_case`] with every file's facts adjusted before resolving
    /// (`(is the caller, file index, facts)`).
    fn module_case_each(
        lang: &str,
        files: Vec<MFile<'_>>,
        caller: MFile<'_>,
        refs: Vec<RefRecord>,
        mut tweak: impl FnMut(bool, usize, &mut FileFacts),
    ) -> (Vec<(String, String)>, Vec<UnresolvedReason>) {
        let mut g = Graph::new();
        let mut facts = Vec::new();
        let mut all: Vec<MFile<'_>> = files;
        all.push(caller);
        let n = all.len();
        let mut caller_id = CallableId::new(0);
        for (fi, f) in all.into_iter().enumerate() {
            let fid = fi as u32;
            let mut rec = mk_file(fid, f.path, lang);
            rec.test_role =
                cgg_core::classify_test_file(std::path::Path::new(f.path), lang);
            g.add_file(rec);
            let mut defs = Vec::new();
            for (di, (qn, kind, sig)) in f.defs.iter().enumerate() {
                let id = fid * 100 + di as u32;
                let simple = qn.rsplit(['.', ':']).next().unwrap();
                // The caller's first callable encloses every site.
                let range = if fi == n - 1 && di == 0 {
                    (0, 1000)
                } else {
                    (2000 + di as u32 * 10, 2005 + di as u32 * 10)
                };
                let mut c = mk_callable(id, simple, qn, fid, lang, range);
                c.kind = *kind;
                c.signature_hint = (*sig).to_string();
                g.add_callable(c);
                if fi == n - 1 && di == 0 {
                    caller_id = CallableId::new(id);
                }
                let variant = match kind {
                    CallableKind::Method => DefVariant::InherentMethod,
                    _ => DefVariant::FreeFunction,
                };
                let mut d = mk_def(simple, qn, variant, range);
                d.signature_hint = (*sig).to_string();
                d.base_types = f.bases.iter().map(|b| b.to_string()).collect();
                defs.push(d);
            }
            let mut ff = facts_for(
                fid,
                f.path,
                lang,
                defs,
                if fi == n - 1 { refs.clone() } else { vec![] },
                f.imports,
            );
            ff.packages = f.packages.iter().map(|p| p.to_string()).collect();
            tweak(fi == n - 1, fi, &mut ff);
            facts.push(ff);
        }
        let out = resolve_default(&g, &facts);
        let dsts = out
            .edges
            .iter()
            .filter(|e| e.src == caller_id)
            .map(|e| {
                let c = &g.callables[&e.dst];
                (
                    c.qualified_name.clone(),
                    g.files[&c.file].path.to_string_lossy().into_owned(),
                )
            })
            .collect();
        (dsts, out.unresolved.into_iter().map(|u| u.reason).collect())
    }

    #[test]
    fn lua_calls_through_a_require_bind_in_the_required_file_only() {
        // `util.decode()` is the member of the table util.lua defines;
        // `cjson.decode()` is a dependency's (no file of the tree is
        // `cjson`), never the workspace `decode`; a name util.lua does not
        // provide is refused, not matched elsewhere.
        let files = vec![
            mfile(
                "lib/util.lua",
                &["M.decode", "M:encode", "M.sub.deep"],
                vec![],
            ),
            mfile("other.lua", &["N.decode", "N.missing"], vec![]),
        ];
        let caller = mfile(
            "app.lua",
            &["run"],
            vec![
                import("require", "util", "util"),
                import("require", "cjson", "cjson"),
            ],
        );
        let refs = vec![
            call("decode", "util"),
            RefRecord {
                site_byte: 61,
                ..call("encode", "util")
            },
            RefRecord {
                site_byte: 62,
                ..call("decode", "cjson")
            },
            RefRecord {
                site_byte: 63,
                ..call("missing", "util")
            },
            RefRecord {
                site_byte: 64,
                ..call("deep", "util")
            },
        ];
        let (dsts, why) = module_case("lua", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("M.decode", "lib/util.lua"),
                at("M:encode", "lib/util.lua")
            ]
        );
        assert!(why.contains(&external("cjson")), "{why:?}");
        assert!(why.contains(&refused("util")), "{why:?}");
    }

    #[test]
    fn csharp_extension_targets_are_read_from_the_signature() {
        assert_eq!(
            csharp_extension_target("internal static Faker<P> GetFaker(this P _) =>"),
            Some(("P".to_string(), false))
        );
        assert_eq!(
            csharp_extension_target("public static T Or<T>(this T x, T y)"),
            Some(("T".to_string(), true))
        );
        assert_eq!(
            csharp_extension_target(
                "static int N(this System.Collections.IList<int> xs)"
            ),
            Some(("IList".to_string(), false))
        );
        assert_eq!(csharp_extension_target("static int N(int x)"), None);
    }

    fn refused(module: &str) -> UnresolvedReason {
        UnresolvedReason::NotInImportedModule {
            module: module.into(),
        }
    }

    fn at(qn: &str, path: &str) -> (String, String) {
        (qn.to_string(), path.to_string())
    }

    /// The A4 near-miss: two modules export `spawnJob`; a call through
    /// the namespace import of one binds to that one only, whichever file
    /// was indexed last — and a named import of the other binds there.
    #[test]
    fn js_module_binding_picks_the_imported_file_in_either_order() {
        for reversed in [false, true] {
            let mut files = vec![
                mfile("src/aaa/svc.js", &["spawnJob"], vec![]),
                mfile("src/zzz/other.js", &["spawnJob"], vec![]),
            ];
            if reversed {
                files.reverse();
            }
            let caller = mfile(
                "src/main.js",
                &["go"],
                vec![
                    import("import", "./aaa/svc", "svc"),
                    import("from-import", "./zzz/other", "spawnJob as z"),
                ],
            );
            let (dsts, why) = module_case(
                "javascript",
                files,
                caller,
                vec![call("spawnJob", "svc"), call("z", "")],
            );
            assert_eq!(
                dsts,
                vec![
                    at("spawnJob", "src/aaa/svc.js"),
                    at("spawnJob", "src/zzz/other.js")
                ],
                "reversed={reversed}"
            );
            assert!(why.is_empty(), "{why:?}");
        }
    }

    /// `module.exports = { run, other: impl2 }`, a module that is
    /// `module.exports = require('./legacy')`, and a decoy defining the
    /// same names.
    #[test]
    fn js_require_follows_commonjs_exports() {
        let files = vec![
            mfile(
                "src/legacy.js",
                &["run", "impl2"],
                vec![import("local-export", "", "run, impl2 as other")],
            ),
            mfile(
                "src/proxy.js",
                &[],
                vec![import("re-export", "./legacy", "*")],
            ),
            mfile("src/zzz/decoy.js", &["run", "other"], vec![]),
        ];
        let caller = mfile(
            "src/main.js",
            &["go"],
            vec![
                import("import", "./legacy", "legacy"),
                import("import", "./proxy", "proxy"),
            ],
        );
        let (dsts, _) = module_case(
            "javascript",
            files,
            caller,
            vec![call("other", "legacy"), call("run", "proxy")],
        );
        assert_eq!(
            dsts,
            vec![at("impl2", "src/legacy.js"), at("run", "src/legacy.js")]
        );
    }

    /// A barrel `index.js`: `export *`, `export { helper as h }`. A name
    /// the barrel does not export is refused, not bound to the decoy.
    #[test]
    fn js_barrel_reexports_are_followed_and_a_missing_name_refused() {
        let files = vec![
            mfile("src/lib/impl.js", &["f"], vec![]),
            mfile("src/lib/util.js", &["helper"], vec![]),
            mfile(
                "src/lib/index.js",
                &[],
                vec![
                    import("re-export", "./impl", "*"),
                    import("re-export", "./util", "helper as h"),
                ],
            ),
            mfile("src/decoy.js", &["f", "h", "missing"], vec![]),
        ];
        let caller = mfile(
            "src/main.js",
            &["go"],
            vec![import("import", "./lib", "lib")],
        );
        let (dsts, why) = module_case(
            "javascript",
            files,
            caller,
            vec![
                call("f", "lib"),
                call("h", "lib"),
                RefRecord {
                    site_byte: 70,
                    ..call("missing", "lib")
                },
            ],
        );
        assert_eq!(
            dsts,
            vec![at("f", "src/lib/impl.js"), at("helper", "src/lib/util.js")]
        );
        assert_eq!(why, vec![refused("./lib")]);
    }

    /// A module whose file is not in the tree: refused, never matched by
    /// name to a same-named function elsewhere.
    #[test]
    fn js_module_without_a_file_is_refused() {
        let files = vec![mfile("src/zzz/decoy.ts", &["gone"], vec![])];
        let caller = mfile(
            "src/main.ts",
            &["go"],
            vec![import("import", "./not-analyzed", "m")],
        );
        let (dsts, why) =
            module_case("typescript", files, caller, vec![call("gone", "m")]);
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![refused("./not-analyzed")]);
    }

    fn py(
        path: &'static str,
        module: &'static str,
        defs: &[(&'static str, CallableKind)],
    ) -> MFile<'static> {
        MFile {
            path,
            defs: defs.iter().map(|(q, k)| (*q, *k, "")).collect(),
            imports: vec![],
            packages: vec![module],
            bases: vec![],
        }
    }

    /// FINDINGS A5, Python: `pkg.sub.jobs.Runner().m()` and `from .
    /// import helpers; helpers.f()` bind in the module only; a package
    /// re-export (`from .core import helper` in `__init__.py`) is
    /// followed; a name the module does not provide is refused.
    #[test]
    fn python_module_head_binds_inside_the_module() {
        let method = CallableKind::Method;
        let func = CallableKind::Function;
        let mut init = py("pkg/__init__.py", "pkg", &[]);
        init.imports = vec![import("from-import", ".core", "helper")];
        let files = vec![
            init,
            py("pkg/core.py", "pkg.core", &[("pkg.core.helper", func)]),
            py(
                "pkg/helpers.py",
                "pkg.helpers",
                &[("pkg.helpers.helper_fn", func)],
            ),
            py(
                "pkg/sub/jobs.py",
                "pkg.sub.jobs",
                &[("pkg.sub.jobs.Runner.start_runner", method)],
            ),
            py(
                "other/decoy.py",
                "other.decoy",
                &[
                    ("other.decoy.helper", func),
                    ("other.decoy.helper_fn", func),
                    ("other.decoy.missing", func),
                    ("other.decoy.Runner.start_runner", method),
                ],
            ),
        ];
        let mut caller = py("pkg/main.py", "pkg.main", &[("pkg.main.go", func)]);
        caller.imports = vec![
            import("import", "pkg", ""),
            import("from-import", ".", "helpers"),
        ];
        let refs = vec![
            call("helper", "pkg"),
            RefRecord {
                site_byte: 61,
                ..call("start_runner", "pkg.sub.jobs.Runner()")
            },
            RefRecord {
                site_byte: 62,
                ..call("helper_fn", "helpers")
            },
            RefRecord {
                site_byte: 63,
                ..call("missing", "pkg")
            },
        ];
        let (dsts, why) = module_case("python", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("pkg.core.helper", "pkg/core.py"),
                at("pkg.sub.jobs.Runner.start_runner", "pkg/sub/jobs.py"),
                at("pkg.helpers.helper_fn", "pkg/helpers.py"),
            ]
        );
        assert_eq!(why, vec![refused("pkg")]);
    }

    /// FINDINGS A5, Go: the import path decides between two packages of
    /// one name; a value a package function built (`t, err :=
    /// util.Open()`) and a parameter of a package type reach that
    /// package's type, by the function's declared result.
    #[test]
    fn go_import_path_decides_between_same_named_packages() {
        let pkg = |path: &'static str, ip: &str| MFile {
            path,
            defs: vec![
                ("util.F", CallableKind::Function, "func F() int"),
                ("util.T.M", CallableKind::Method, "func (t *T) M() int"),
                (
                    "util.Open",
                    CallableKind::Function,
                    "func Open(p string) (*T, error)",
                ),
            ],
            imports: vec![
                import("package-root", "util", ""),
                import("package-path", "", ip),
            ],
            packages: vec!["example.com/app"],
            bases: vec![],
        };
        for reversed in [false, true] {
            let mut files = vec![
                pkg("internal/a/util/util.go", "example.com/app/internal/a/util"),
                pkg("internal/b/util/util.go", "example.com/app/internal/b/util"),
            ];
            if reversed {
                files.reverse();
            }
            let caller = MFile {
                path: "main.go",
                defs: vec![("main.main", CallableKind::Function, "func main()")],
                imports: vec![
                    import("package-root", "main", ""),
                    import("import", "example.com/app/internal/a/util", ""),
                ],
                packages: vec!["example.com/app"],
                bases: vec![],
            };
            let refs = vec![
                call("F", "util"),
                RefRecord {
                    site_byte: 61,
                    ..call("M", "util.Open()")
                },
                RefRecord {
                    site_byte: 62,
                    ..call("M", "util.T")
                },
            ];
            let (dsts, why) = module_case("go", files, caller, refs);
            let a = "internal/a/util/util.go";
            assert_eq!(
                dsts,
                vec![at("util.F", a), at("util.T.M", a), at("util.T.M", a)],
                "reversed={reversed}"
            );
            assert!(why.is_empty(), "{why:?}");
        }
    }

    /// FINDINGS A6, PHP: a bare `collect()` is a function call; the
    /// method `Dates::collect` is not in scope for it, while a free
    /// function of the same name is.
    #[test]
    fn php_bare_call_reaches_functions_never_methods() {
        let files = vec![
            MFile {
                path: "src/Dates.php",
                defs: vec![
                    ("Dates::helper", CallableKind::Method, ""),
                    ("Dates::collect", CallableKind::Method, ""),
                ],
                imports: vec![],
                packages: vec![],
                bases: vec![],
            },
            mfile("src/helpers.php", &["helper"], vec![]),
        ];
        let caller = MFile {
            path: "src/App.php",
            defs: vec![("App::run", CallableKind::Method, "")],
            imports: vec![import("use", "App\\Util\\Dates", "")],
            packages: vec![],
            bases: vec![],
        };
        let refs = vec![
            call("helper", ""),
            RefRecord {
                site_byte: 61,
                ..call("collect", "")
            },
        ];
        let (dsts, _) = module_case("php", files, caller, refs);
        assert_eq!(dsts, vec![at("helper", "src/helpers.php")]);
    }

    /// FINDINGS A6, Java: a bare `g()` reaches the calling class's own
    /// base, never the same-named method of an unrelated class.
    #[test]
    fn java_bare_call_reaches_bases_not_unrelated_classes() {
        let m = CallableKind::Method;
        let files = vec![
            MFile {
                path: "src/p/Base.java",
                defs: vec![("p.Base.g", m, "")],
                imports: vec![],
                packages: vec!["p"],
                bases: vec![],
            },
            MFile {
                path: "src/p/Other.java",
                defs: vec![("p.Other.g", m, ""), ("p.Other.h", m, "")],
                imports: vec![],
                packages: vec!["p"],
                bases: vec![],
            },
        ];
        let caller = MFile {
            path: "src/p/A.java",
            defs: vec![("p.A.f", m, "")],
            imports: vec![import("import", "java.util.List", "")],
            packages: vec!["p"],
            bases: vec!["Base"],
        };
        let refs = vec![
            call("g", ""),
            RefRecord {
                site_byte: 61,
                ..call("h", "")
            },
        ];
        let (dsts, _) = module_case("java", files, caller, refs);
        assert_eq!(dsts, vec![at("p.Base.g", "src/p/Base.java")]);
    }

    /// FINDINGS A6, Go: a bare call reaches its own package — the files
    /// of the calling directory — never a same-named function of another
    /// package (`setRun` in `secret/set` is not `alias/set`'s).
    #[test]
    fn go_bare_call_stays_in_its_package() {
        let go_file = |path: &'static str| MFile {
            path,
            defs: vec![("set.setRun", CallableKind::Function, "func setRun() error")],
            imports: vec![import("package-root", "set", "")],
            packages: vec!["example.com/app"],
            bases: vec![],
        };
        for reversed in [false, true] {
            let mut files = vec![
                go_file("pkg/cmd/alias/set/set.go"),
                go_file("pkg/cmd/secret/set/set.go"),
            ];
            if reversed {
                files.reverse();
            }
            let caller = MFile {
                path: "pkg/cmd/secret/set/set_test.go",
                defs: vec![("set.TestSet", CallableKind::Function, "func TestSet()")],
                imports: vec![
                    import("package-root", "set", ""),
                    import("import", "testing", ""),
                ],
                packages: vec!["example.com/app"],
                bases: vec![],
            };
            let (dsts, _) = module_case("go", files, caller, vec![call("setRun", "")]);
            assert_eq!(
                dsts,
                vec![at("set.setRun", "pkg/cmd/secret/set/set.go")],
                "reversed={reversed}"
            );
        }
    }

    /// A workspace package imported by name (`@std/assert`) is its own
    /// directory: its entry module (Deno's `mod.ts` here) and that
    /// module's re-exports, not every file whose path ends in `assert`.
    #[test]
    fn js_workspace_package_resolves_through_its_root() {
        let pkg = |path: &'static str,
                   defs: &[&'static str],
                   mut imports: Vec<ImportRecord>| {
            // The plugin records the package each file belongs to.
            imports.push(import("package-name", "", "@std/assert"));
            MFile {
                path,
                defs: defs
                    .iter()
                    .map(|d| (*d, CallableKind::Function, ""))
                    .collect(),
                imports,
                packages: vec!["@std/assert"],
                bases: vec![],
            }
        };
        let files = vec![
            pkg("assert/assert.ts", &["assert"], vec![]),
            pkg("assert/equals.ts", &["assertEquals"], vec![]),
            pkg(
                "assert/mod.ts",
                &[],
                vec![import("re-export", "./equals.ts", "*")],
            ),
            mfile("expect/_assert_equals.ts", &["assertEquals"], vec![]),
        ];
        let caller = mfile(
            "collections/filter_keys_test.ts",
            &["test"],
            vec![import("from-import", "@std/assert", "assertEquals")],
        );
        let (dsts, why) =
            module_case("typescript", files, caller, vec![call("assertEquals", "")]);
        assert_eq!(dsts, vec![at("assertEquals", "assert/equals.ts")]);
        assert!(why.is_empty(), "{why:?}");
    }

    /// `useThing().get()`: a call on what an imported *function* returned
    /// is not a path into the module, so module-scoped resolution leaves
    /// it to the resolver's other readings rather than refusing it.
    #[test]
    fn a_call_on_an_imported_functions_result_is_not_refused() {
        let files = vec![mfile("src/hooks.js", &["useThing"], vec![])];
        let caller = mfile(
            "src/main.js",
            &["go"],
            vec![import("from-import", "./hooks", "useThing")],
        );
        let (_, why) =
            module_case("javascript", files, caller, vec![call("get", "useThing()")]);
        assert!(
            !why.iter()
                .any(|w| matches!(w, UnresolvedReason::NotInImportedModule { .. })),
            "{why:?}"
        );
    }

    /// A6 keeps what Java lets a bare name reach through a static import:
    /// `import static q.TestUtils.*;` and `import static q.TestUtils.single;`
    /// name members of another class, and those calls stay bound — the
    /// single import exactly, though a same-named method exists elsewhere.
    #[test]
    fn java_static_imports_reach_members_of_another_class() {
        let m = CallableKind::Method;
        let files = vec![
            MFile {
                path: "src/q/TestUtils.java",
                defs: vec![("q.TestUtils.helper", m, ""), ("q.TestUtils.single", m, "")],
                imports: vec![],
                packages: vec!["q"],
                bases: vec![],
            },
            MFile {
                path: "src/r/Other.java",
                defs: vec![("r.Other.single", m, "")],
                imports: vec![],
                packages: vec!["r"],
                bases: vec![],
            },
        ];
        let caller = MFile {
            path: "src/p/B.java",
            defs: vec![("p.B.f", m, "")],
            imports: vec![
                import("from-import", "q.TestUtils", "*"),
                import("from-import", "q.TestUtils.single", "single"),
            ],
            packages: vec!["p"],
            bases: vec![],
        };
        let refs = vec![
            call("helper", ""),
            RefRecord {
                site_byte: 61,
                ..call("single", "")
            },
        ];
        let (dsts, _) = module_case("java", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("q.TestUtils.helper", "src/q/TestUtils.java"),
                at("q.TestUtils.single", "src/q/TestUtils.java"),
            ]
        );
    }

    /// A6 keeps dispatch: a bare `step()` in a base class reaches the
    /// override in a class derived from it (the base often declares it
    /// abstractly), never the same-named method of an unrelated class.
    #[test]
    fn java_bare_call_reaches_overrides_in_derived_classes() {
        let m = CallableKind::Method;
        let files = vec![
            MFile {
                path: "src/p/Impl.java",
                defs: vec![("p.Impl.step", m, "")],
                imports: vec![],
                packages: vec!["p"],
                bases: vec!["Base"],
            },
            MFile {
                path: "src/p/Other.java",
                defs: vec![("p.Other.step", m, "")],
                imports: vec![],
                packages: vec!["p"],
                bases: vec![],
            },
        ];
        let caller = MFile {
            path: "src/p/Base.java",
            defs: vec![("p.Base.run", m, "")],
            imports: vec![import("import", "java.util.List", "")],
            packages: vec!["p"],
            bases: vec![],
        };
        let (dsts, _) = module_case("java", files, caller, vec![call("step", "")]);
        assert_eq!(dsts, vec![at("p.Impl.step", "src/p/Impl.java")]);
    }

    /// `module.exports = new SettingsHelpers(…)` with the class required
    /// from another file: a member call through the module is the
    /// class's method, followed through the import.
    #[test]
    fn js_commonjs_instance_export_reaches_its_class() {
        let files = vec![
            MFile {
                path: "svc/helpers/settings-helpers.js",
                defs: vec![("SettingsHelpers.getEmail", CallableKind::Method, "")],
                imports: vec![import("local-export", "", "SettingsHelpers as default")],
                packages: vec![],
                bases: vec![],
            },
            mfile(
                "svc/helpers/index.js",
                &[],
                vec![
                    import("import", "./settings-helpers", "SettingsHelpers"),
                    import("local-export", "", "SettingsHelpers as default"),
                ],
            ),
            mfile("other/decoy.js", &["getEmail"], vec![]),
        ];
        let caller = mfile(
            "svc/email/wrapper.js",
            &["go"],
            vec![import("import", "../helpers", "settingsHelpers")],
        );
        let (dsts, why) = module_case(
            "javascript",
            files,
            caller,
            vec![call("getEmail", "settingsHelpers")],
        );
        assert_eq!(
            dsts,
            vec![at(
                "SettingsHelpers.getEmail",
                "svc/helpers/settings-helpers.js"
            )]
        );
        assert!(why.is_empty(), "{why:?}");
    }

    /// A path alias that matches the same module in two projects
    /// (`web/` and its copy `web-old/`) resolves to the one nearest the
    /// importing file, not to neither.
    #[test]
    fn js_alias_prefers_the_module_nearest_the_importer() {
        let files = vec![
            mfile("web/src/backend/Api.ts", &["getItems"], vec![]),
            mfile("web-old/src/backend/Api.js", &["getItems"], vec![]),
        ];
        let caller = mfile(
            "web/src/pages/List.tsx",
            &["render"],
            vec![import("import", "@/backend/Api", "Api")],
        );
        let (dsts, _) =
            module_case("typescript", files, caller, vec![call("getItems", "Api")]);
        assert_eq!(dsts, vec![at("getItems", "web/src/backend/Api.ts")]);
    }

    // ---- WS4 review classes (R1–R8): refuse only when decisive. ----

    fn go_pkg_file(
        path: &'static str,
        pkg: &'static str,
        ip: &'static str,
        defs: Vec<(&'static str, CallableKind, &'static str)>,
        mut extra: Vec<ImportRecord>,
    ) -> MFile<'static> {
        let mut imports = vec![
            import("package-root", pkg, ""),
            import("package-path", "", ip),
        ];
        imports.append(&mut extra);
        MFile {
            path,
            defs,
            imports,
            packages: vec!["example.com/app"],
            bases: vec![],
        }
    }

    fn go_caller(imports: Vec<ImportRecord>) -> MFile<'static> {
        let mut all = vec![import("package-root", "main", "")];
        all.extend(imports);
        MFile {
            path: "main.go",
            defs: vec![("main.main", CallableKind::Function, "func main()")],
            imports: all,
            packages: vec!["example.com/app"],
            bases: vec![],
        }
    }

    fn refusals(why: &[UnresolvedReason]) -> usize {
        why.iter()
            .filter(|w| matches!(w, UnresolvedReason::NotInImportedModule { .. }))
            .count()
    }

    /// R2 (Go): `ttl.CacheDefault.Int()` goes through a package-level
    /// variable, so a miss keeps the other readings; `ttl.Missing()` —
    /// the package itself as the receiver — is still refused.
    #[test]
    fn go_package_variable_receiver_is_not_refused() {
        let files = vec![go_pkg_file(
            "ttl/ttl.go",
            "ttl",
            "example.com/app/ttl",
            vec![(
                "ttl.Duration.Int",
                CallableKind::Method,
                "func (d Duration) Int() int",
            )],
            vec![],
        )];
        let caller = go_caller(vec![import("import", "example.com/app/ttl", "")]);
        let refs = vec![
            call("Int", "ttl.CacheDefault"),
            RefRecord {
                site_byte: 61,
                ..call("Missing", "ttl")
            },
        ];
        let (dsts, why) = module_case("go", files, caller, refs);
        // Duck typing (the pre-WS4 reading) still finds `Duration.Int`.
        assert_eq!(dsts, vec![at("ttl.Duration.Int", "ttl/ttl.go")]);
        assert_eq!(refusals(&why), 0, "{why:?}");
    }

    /// R3 (Go): a value a package function returns is typed by the
    /// function's declared result: a type of another workspace package is
    /// looked up there, one of a dependency is filed as external.
    #[test]
    fn go_result_type_of_another_package_or_a_dependency() {
        let files = vec![
            go_pkg_file(
                "idv/idv.go",
                "idv",
                "example.com/app/idv",
                vec![("idv.Provider.Verify", CallableKind::Method, "")],
                vec![],
            ),
            go_pkg_file(
                "svc/svc.go",
                "svc",
                "example.com/app/svc",
                vec![
                    (
                        "svc.GetIdv",
                        CallableKind::Function,
                        "func GetIdv() idv.Provider",
                    ),
                    (
                        "svc.GetStore",
                        CallableKind::Function,
                        "func GetStore() (oss.Store, error)",
                    ),
                    ("svc.Local.Verify", CallableKind::Method, ""),
                ],
                vec![
                    import("import", "example.com/app/idv", ""),
                    import("import", "github.com/ext/oss", ""),
                ],
            ),
        ];
        let caller = go_caller(vec![import("import", "example.com/app/svc", "")]);
        let refs = vec![
            call("Verify", "svc.GetIdv()"),
            RefRecord {
                site_byte: 61,
                ..call("Verify", "svc.GetStore()")
            },
        ];
        let (dsts, why) = module_case("go", files, caller, refs);
        assert_eq!(dsts, vec![at("idv.Provider.Verify", "idv/idv.go")]);
        assert_eq!(why, vec![external("github.com/ext/oss")]);
    }

    /// R6 (Go): a bare call is the calling package's own, however many
    /// other packages declare the name.
    #[test]
    fn go_bare_call_binds_in_its_own_package_first() {
        let mut files = Vec::new();
        for d in ["a", "b", "c", "d", "e", "f", "g", "h", "i"] {
            let path: &'static str = match d {
                "a" => "cmd/a/list/list.go",
                "b" => "cmd/b/list/list.go",
                "c" => "cmd/c/list/list.go",
                "d" => "cmd/d/list/list.go",
                "e" => "cmd/e/list/list.go",
                "f" => "cmd/f/list/list.go",
                "g" => "cmd/g/list/list.go",
                "h" => "cmd/h/list/list.go",
                _ => "cmd/i/list/list.go",
            };
            files.push(go_pkg_file(
                path,
                "list",
                "example.com/app/cmd/x/list",
                vec![(
                    "list.listRun",
                    CallableKind::Function,
                    "func listRun() error",
                )],
                vec![],
            ));
        }
        let caller = MFile {
            path: "cmd/a/list/list_test.go",
            defs: vec![("list.TestList", CallableKind::Function, "func TestList()")],
            imports: vec![
                import("package-root", "list", ""),
                import("import", "testing", ""),
            ],
            packages: vec!["example.com/app"],
            bases: vec![],
        };
        let (dsts, _) = module_case("go", files, caller, vec![call("listRun", "")]);
        assert_eq!(dsts, vec![at("list.listRun", "cmd/a/list/list.go")]);
    }

    /// R2 (Python): `wif._refresher.stop()` goes through a module-level
    /// instance — not refused; `wif.missing()` still is.
    #[test]
    fn python_module_level_value_receiver_is_not_refused() {
        let files = vec![
            py("pkg/__init__.py", "pkg", &[]),
            py(
                "pkg/wif.py",
                "pkg.wif",
                &[("pkg.wif._Refresher.stop", CallableKind::Method)],
            ),
        ];
        let mut caller = py("main.py", "main", &[("main.go", CallableKind::Function)]);
        caller.imports = vec![import("from-import", "pkg", "wif")];
        let refs = vec![
            call("stop", "wif._refresher"),
            RefRecord {
                site_byte: 61,
                ..call("stop", "wif")
            },
        ];
        let (_, why) = module_case("python", files, caller, refs);
        assert_eq!(why, vec![refused("pkg.wif")]);
    }

    /// R2/R4 (JS/TS): a property path (`welcome.api.isActive()`), a
    /// default export cgg cannot name, and a builder chain off `new X()`
    /// keep the other readings; `plain.missing()` through a namespace
    /// binding is still refused.
    #[test]
    fn js_values_and_chains_are_not_refused() {
        let files = vec![
            mfile("src/welcome.js", &["Api.isActive"], vec![]),
            mfile(
                "src/resize.ts",
                &["createResizeObserver"],
                vec![import("local-export", "", "<value> as default")],
            ),
            mfile("src/plain.js", &["run"], vec![]),
            mfile(
                "src/builder.js",
                &["Builder.withTeam", "Builder.build"],
                vec![],
            ),
            mfile("src/zzz/decoy.js", &["missing"], vec![]),
        ];
        let caller = mfile(
            "src/main.js",
            &["go"],
            vec![
                import("import", "./welcome", "welcome"),
                import("from-import", "./resize", "default as resizeObserver"),
                import("import", "./plain", "plain"),
                import("from-import", "./builder", "Builder"),
            ],
        );
        let refs = vec![
            call("isActive", "welcome.api"),
            RefRecord {
                site_byte: 61,
                ..call("subscribe", "resizeObserver")
            },
            RefRecord {
                site_byte: 62,
                ..call("build", "new Builder({}).withTeam(1)")
            },
            RefRecord {
                site_byte: 63,
                ..call("missing", "plain")
            },
        ];
        let (_, why) = module_case("javascript", files, caller, refs);
        assert_eq!(refusals(&why), 1, "{why:?}");
        assert!(why.contains(&refused("./plain")), "{why:?}");
    }

    /// R5: an alias whose suffix matches the module in two places binds to
    /// the one that provides the name; and a package's root comes from the
    /// files that belong to it, not from another package's tsconfig key of
    /// the same name.
    #[test]
    fn js_alias_copies_and_package_roots() {
        let mut lib = mfile(
            "packages/lib/markdown.ts",
            &["markdownToSafeHTML"],
            vec![import("package-name", "", "@calcom/lib")],
        );
        lib.packages = vec!["@calcom/lib"];
        let mut atoms = mfile(
            "packages/atoms/lib/markdown.ts",
            &["markdownToSafeHTML"],
            vec![import("package-name", "", "@calcom/atoms")],
        );
        atoms.packages = vec!["@calcom/atoms", "@calcom/lib"];
        let files = vec![
            lib,
            atoms,
            mfile("web/src/support/ui.tsx", &["renderWithProviders"], vec![]),
            mfile(
                "web/test/support/ui.tsx",
                &["mockGetBoundingClientRect"],
                vec![],
            ),
        ];
        let caller = mfile(
            "web/src/card.spec.tsx",
            &["t"],
            vec![
                import("from-import", "@calcom/lib/markdown", "markdownToSafeHTML"),
                import("from-import", "@/support/ui", "mockGetBoundingClientRect"),
            ],
        );
        let refs = vec![
            call("markdownToSafeHTML", ""),
            RefRecord {
                site_byte: 61,
                ..call("mockGetBoundingClientRect", "")
            },
        ];
        let (dsts, why) = module_case("typescript", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("markdownToSafeHTML", "packages/lib/markdown.ts"),
                at("mockGetBoundingClientRect", "web/test/support/ui.tsx"),
            ]
        );
        assert!(why.is_empty(), "{why:?}");
    }

    /// R8: copies of one qualified name. The copy in the imported module
    /// wins (Erlang `-import(lists, …)` is not `lists1`); copies with no
    /// evidence for one bind nothing (two same-directory OCaml `k`).
    #[test]
    fn copies_of_a_name_need_evidence() {
        let files = vec![
            mfile("stdlib/lists.erl", &["lists:filter"], vec![]),
            mfile("test_data/a/lists1.erl", &["lists1:filter"], vec![]),
            mfile("test_data/b/lists1.erl", &["lists1:filter"], vec![]),
        ];
        let caller = mfile(
            "tools/xref.erl",
            &["xref:run"],
            vec![import("import", "lists", "")],
        );
        let (dsts, _) = module_case("erlang", files, caller, vec![call("filter", "")]);
        assert_eq!(dsts, vec![at("lists:filter", "stdlib/lists.erl")]);

        let files = vec![
            mfile("src/fiber/src/scheduler.ml", &["k"], vec![]),
            mfile("src/fiber/src/stream.ml", &["k"], vec![]),
        ];
        let caller = mfile(
            "src/fiber/src/core.ml",
            &["run"],
            vec![import("open", "Fiber", "")],
        );
        let (dsts, _) = module_case("ocaml", files, caller, vec![call("k", "")]);
        assert!(dsts.is_empty(), "{dsts:?}");
    }

    /// R8 through a module alias: `Type.fromInterned()` with `const Type =
    /// @import("Type.zig")` is `Type.zig`'s copy; `matcher.expect()` for a
    /// module no workspace file is binds none of the copies.
    #[test]
    fn module_alias_copies_need_evidence() {
        let files = vec![
            mfile("src/Type.zig", &["fromInterned"], vec![]),
            mfile("src/Value.zig", &["fromInterned"], vec![]),
        ];
        let caller = mfile(
            "src/Sema.zig",
            &["field"],
            vec![import("import", "Type.zig", "Type")],
        );
        let (dsts, _) =
            module_case("zig", files, caller, vec![call("fromInterned", "Type")]);
        assert_eq!(dsts, vec![at("fromInterned", "src/Type.zig")]);

        let files = vec![
            mfile("lib/a/widget_tester.dart", &["expect"], vec![]),
            mfile("lib/b/shell_test.dart", &["expect"], vec![]),
        ];
        let caller = mfile(
            "test/x_test.dart",
            &["main"],
            vec![import("import", "package:matcher/matcher.dart", "matcher")],
        );
        let (dsts, _) =
            module_case("dart", files, caller, vec![call("expect", "matcher")]);
        assert!(dsts.is_empty(), "{dsts:?}");
    }

    /// `module.exports = { doExport: require('./exporter') }` then
    /// `m.doExport()` calls the exporter module's default; and a module
    /// that defines the name somewhere (`this.searchByName = …` in a
    /// constructor function) is not proof of absence, so the call is not
    /// refused.
    #[test]
    fn js_namespace_member_called_and_nested_definitions() {
        let files = vec![
            mfile(
                "src/exporter/exporter.js",
                &["exports"],
                vec![import("local-export", "", "exports as default")],
            ),
            mfile(
                "src/exporter/index.js",
                &[],
                vec![import("re-export", "./exporter", "* as doExport")],
            ),
            mfile(
                "src/Card.js",
                &["CardLibrary.searchByName"],
                vec![import("local-export", "", "<value> as default")],
            ),
            mfile("src/zzz/decoy.js", &["doExport", "searchByName"], vec![]),
        ];
        let caller = mfile(
            "src/backup.js",
            &["backup"],
            vec![
                import("import", "./exporter", "exporter"),
                import("import", "./Card", "card"),
            ],
        );
        let refs = vec![
            call("doExport", "exporter"),
            RefRecord {
                site_byte: 61,
                ..call("searchByName", "card")
            },
        ];
        let (dsts, why) = module_case("javascript", files, caller, refs);
        assert!(
            dsts.contains(&at("exports", "src/exporter/exporter.js")),
            "{dsts:?}"
        );
        assert_eq!(refusals(&why), 0, "{why:?}");
    }

    // ----------------------------------------------------------------
    // Round 2 (R9–R16): shadows, overload sets, exports maps, external
    // labels, Zig own declarations, Erlang `m:f`, F#/Dart bare calls.
    // ----------------------------------------------------------------

    /// R9 (Go): a parameter or local named like an import binding is the
    /// receiver inside its scope — `c.JSON()` with `c *app.RequestContext`
    /// in a file importing `c ".../client"` is not refused as a miss in
    /// `client`. Outside that scope the import is the receiver again.
    #[test]
    fn go_local_shadowing_an_import_alias_is_not_refused() {
        let files = vec![
            go_pkg_file(
                "pkg/app/client/client.go",
                "client",
                "example.com/app/pkg/app/client",
                vec![("client.Do", CallableKind::Function, "func Do()")],
                vec![],
            ),
            go_pkg_file(
                "pkg/app/ctx.go",
                "app",
                "example.com/app/pkg/app",
                vec![(
                    "app.RequestContext.JSON",
                    CallableKind::Method,
                    "func (c *RequestContext) JSON(code int)",
                )],
                vec![],
            ),
        ];
        let caller = go_caller(vec![
            import("import", "example.com/app/pkg/app/client", "c"),
            import("import", "example.com/app/pkg/app", ""),
        ]);
        let refs = vec![
            call("JSON", "c"),
            RefRecord {
                site_byte: 61,
                ..call("JSON", "c")
            },
        ];
        let (dsts, why) = module_case_with("go", files, caller, refs, |f| {
            // `c` is a local over bytes 0..61 only.
            for (ty, at) in [(LOCAL_SHADOW_MARK, 0), ("", 61)] {
                f.local_types.push(cgg_core::LocalType {
                    var_name: "c".into(),
                    type_name: ty.into(),
                    scope_byte: at,
                });
            }
        });
        // Inside the scope: the pre-WS4 reading (the method by name).
        assert_eq!(
            dsts,
            vec![at("app.RequestContext.JSON", "pkg/app/ctx.go")],
            "{why:?}"
        );
        // Near miss: past the scope `c` is the package again, and the
        // package has no `JSON`.
        assert_eq!(why, vec![refused("example.com/app/pkg/app/client")]);
    }

    const LOCAL_SHADOW_MARK: &str = "<local>";

    /// R10 (Java): a bare call to an overloaded method of the caller's
    /// base class, or of a class imported `import static q.C.*`, binds the
    /// whole overload set — never an unrelated class's same-named method.
    #[test]
    fn java_overload_sets_through_bases_and_static_wildcards() {
        let m = CallableKind::Method;
        let files = vec![
            MFile {
                path: "src/p/BaseQueryTest.java",
                defs: vec![
                    ("p.BaseQueryTest.testQuery", m, "void testQuery(String sql)"),
                    (
                        "p.BaseQueryTest.testQuery",
                        m,
                        "void testQuery(String sql, int n)",
                    ),
                ],
                imports: vec![],
                packages: vec!["p"],
                bases: vec![],
            },
            MFile {
                path: "src/q/CookieUtil.java",
                defs: vec![
                    (
                        "q.CookieUtil.add",
                        m,
                        "static void add(StringBuilder b, String n, long v)",
                    ),
                    (
                        "q.CookieUtil.add",
                        m,
                        "static void add(StringBuilder b, String n, String v)",
                    ),
                ],
                imports: vec![],
                packages: vec!["q"],
                bases: vec![],
            },
            MFile {
                path: "src/r/Other.java",
                defs: vec![("r.Other.testQuery", m, ""), ("r.Other.add", m, "")],
                imports: vec![],
                packages: vec!["r"],
                bases: vec![],
            },
        ];
        let caller = MFile {
            path: "src/p/ArraysQueryTest.java",
            defs: vec![("p.ArraysQueryTest.testArrays", m, "")],
            imports: vec![import("from-import", "q.CookieUtil", "*")],
            packages: vec!["p"],
            bases: vec!["BaseQueryTest"],
        };
        let refs = vec![
            call("testQuery", ""),
            RefRecord {
                site_byte: 61,
                ..call("add", "")
            },
        ];
        let (dsts, _) = module_case("java", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("p.BaseQueryTest.testQuery", "src/p/BaseQueryTest.java"),
                at("p.BaseQueryTest.testQuery", "src/p/BaseQueryTest.java"),
                at("q.CookieUtil.add", "src/q/CookieUtil.java"),
                at("q.CookieUtil.add", "src/q/CookieUtil.java"),
            ]
        );
    }

    /// R11: a package's `exports` map names the file a sub-path specifier
    /// reaches (`@std/assert/instance-of` → `instance_of.ts`), though no
    /// file is spelled like it.
    #[test]
    fn js_exports_map_subpath_names_its_file() {
        let files = vec![
            MFile {
                path: "assert/instance_of.ts",
                defs: vec![("assertInstanceOf", CallableKind::Function, "")],
                imports: vec![
                    import("package-name", "", "@std/assert"),
                    import("package-export", "", "@std/assert/instance-of"),
                ],
                packages: vec!["@std/assert"],
                bases: vec![],
            },
            mfile("zzz/instance_of.ts", &["assertInstanceOf"], vec![]),
        ];
        let caller = mfile(
            "expect/_matchers.ts",
            &["toBeInstanceOf"],
            vec![import(
                "from-import",
                "@std/assert/instance-of",
                "assertInstanceOf",
            )],
        );
        let (dsts, why) = module_case(
            "typescript",
            files,
            caller,
            vec![call("assertInstanceOf", "")],
        );
        assert_eq!(
            dsts,
            vec![at("assertInstanceOf", "assert/instance_of.ts")],
            "{why:?}"
        );
    }

    /// R12: a bare specifier names a package unless a path alias says
    /// otherwise. `next/navigation` is a dependency even when the tree
    /// has a `next/` directory and a `navigation.ts` elsewhere — external,
    /// never that file's `redirect`. A workspace package re-exporting a
    /// dependency (`export default dayjs` of `import dayjs from "dayjs"`)
    /// labels the call with the dependency. An alias (`@/lib/…`) still
    /// matches by suffix.
    #[test]
    fn js_bare_specifiers_name_packages_not_lookalike_files() {
        let files = vec![
            mfile("src/next/config.ts", &["config"], vec![]),
            mfile("src/lib/navigation.ts", &["redirect"], vec![]),
            // A lookalike of the dependency's function, never its target.
            mfile("src/zzz/dayjs.ts", &["dayjs"], vec![]),
            MFile {
                path: "packages/dayjs/index.ts",
                defs: vec![],
                imports: vec![
                    import("package-name", "", "@scope/dayjs"),
                    import("from-import", "dayjs", "default as dayjs"),
                    import("local-export", "", "dayjs as default"),
                ],
                packages: vec!["@scope/dayjs"],
                bases: vec![],
            },
        ];
        let caller = mfile(
            "src/app/page.ts",
            &["page"],
            vec![
                import("from-import", "next/navigation", "redirect"),
                import("from-import", "@scope/dayjs", "default as dayjs"),
                import("from-import", "@/lib/navigation", "redirect as go"),
            ],
        );
        let refs = vec![
            call("redirect", ""),
            RefRecord {
                site_byte: 61,
                ..call("dayjs", "")
            },
            RefRecord {
                site_byte: 62,
                ..call("go", "")
            },
        ];
        let (dsts, why) = module_case("typescript", files, caller, refs);
        assert_eq!(
            dsts,
            vec![at("redirect", "src/lib/navigation.ts")],
            "{why:?}"
        );
        assert!(why.contains(&external("next/navigation")), "{why:?}");
        assert!(why.contains(&external("dayjs")), "{why:?}");
        assert_eq!(refusals(&why), 0, "{why:?}");
    }

    /// R12 (Go): in module mode an import path no `go.mod` covers is a
    /// dependency, even with a package directory of the same name.
    #[test]
    fn go_stdlib_import_named_like_a_workspace_directory_is_external() {
        let files = vec![go_pkg_file(
            "internal/reflect/reflect.go",
            "reflect",
            "example.com/app/internal/reflect",
            vec![(
                "reflect.DeepEqual",
                CallableKind::Function,
                "func DeepEqual()",
            )],
            vec![],
        )];
        let caller = go_caller(vec![import("import", "reflect", "")]);
        let (dsts, why) =
            module_case("go", files, caller, vec![call("DeepEqual", "reflect")]);
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(why, vec![external("reflect")]);
    }

    /// JS: a module whose value cgg cannot read (`module.exports = {
    /// ...spread }`, `export default router` of a local variable) is no
    /// proof a name is absent; `module.exports = require('./m')` carries
    /// `./m`'s value, class instance included; a relative module cgg did
    /// not analyze (`types.d.ts`) refuses only `mod.f()`.
    #[test]
    fn js_unreadable_values_are_not_proof_of_absence() {
        let m = CallableKind::Method;
        let files = vec![
            mfile(
                "core/utils/index.js",
                &[],
                vec![import("local-export", "", "<value> as default")],
            ),
            mfile(
                "core/router.js",
                &[],
                vec![import("local-export", "", "router as default")],
            ),
            MFile {
                path: "core/importer/import-manager.js",
                defs: vec![("ImportManager.getExtensions", m, "")],
                imports: vec![import("local-export", "", "ImportManager as default")],
                packages: vec![],
                bases: vec![],
            },
            mfile(
                "core/importer/index.js",
                &[],
                vec![import("re-export", "./import-manager", "*, default")],
            ),
            mfile("zzz/decoy.js", &["addTable", "get"], vec![]),
        ];
        let caller = mfile(
            "core/migrations/run.js",
            &["run"],
            vec![
                import("from-import", "../utils", "addTable"),
                import("import", "../router", "router"),
                import("import", "../importer", "ImportManager"),
                import("from-import", "../types", "Props"),
            ],
        );
        let refs = vec![
            call("addTable", ""),
            RefRecord {
                site_byte: 61,
                ..call("get", "router")
            },
            RefRecord {
                site_byte: 62,
                ..call("getExtensions", "ImportManager")
            },
            RefRecord {
                site_byte: 63,
                ..call("render", "Props")
            },
        ];
        let (dsts, why) = module_case("javascript", files, caller, refs);
        assert!(
            dsts.contains(&at(
                "ImportManager.getExtensions",
                "core/importer/import-manager.js"
            )),
            "{dsts:?}"
        );
        assert_eq!(refusals(&why), 0, "{why:?}");
    }

    /// The near miss of the above: a module whose exports cgg reads in
    /// full, a default class without a constructor, and a namespace call
    /// into an unanalyzed relative module are still decisive.
    #[test]
    fn js_readable_modules_stay_decisive() {
        let m = CallableKind::Method;
        let files = vec![
            mfile("core/utils/index.js", &["other"], vec![]),
            MFile {
                path: "core/templates/email.js",
                defs: vec![("Email.getPayload", m, "")],
                imports: vec![import("local-export", "", "Email as default")],
                packages: vec![],
                bases: vec![],
            },
            mfile("zzz/decoy.js", &["addTable", "dbget"], vec![]),
        ];
        let caller = mfile(
            "core/migrations/run.js",
            &["run"],
            vec![
                import("from-import", "../utils", "addTable"),
                import("from-import", "../templates/email", "default as Email"),
                import("import", "../gone", "gone"),
            ],
        );
        let refs = vec![
            call("addTable", ""),
            RefRecord {
                site_byte: 61,
                ..call("Email", "")
            },
            RefRecord {
                site_byte: 62,
                ..call("dbget", "gone")
            },
        ];
        let (dsts, why) = module_case("javascript", files, caller, refs);
        assert!(dsts.is_empty(), "{dsts:?}");
        assert!(why.contains(&refused("../utils")), "{why:?}");
        assert!(why.contains(&refused("../gone")), "{why:?}");
        assert!(
            why.iter()
                .any(|w| matches!(w, UnresolvedReason::ClassWithoutExplicitInit)),
            "{why:?}"
        );
    }

    /// R13 (Zig): `const addRunArtifact = link.addRunArtifact;` names the
    /// file `link` is; a name the calling file declares itself is never
    /// bound to another file's copy.
    #[test]
    fn zig_member_aliases_and_own_declarations() {
        let files = vec![
            mfile("test/link/link.zig", &["addRunArtifact"], vec![]),
            mfile("lib/std/Build.zig", &["addRunArtifact", "pack"], vec![]),
            mfile("lib/std/crypto/ml_dsa.zig", &["pack"], vec![]),
        ];
        let caller = mfile(
            "test/link/macho.zig",
            &["testAll", "pack"],
            vec![
                import("import", "link.zig", "link"),
                import("import", "std", "std"),
                import("alias", "link.addRunArtifact", "addRunArtifact"),
            ],
        );
        let refs = vec![
            call("addRunArtifact", ""),
            RefRecord {
                site_byte: 61,
                ..call("pack", "")
            },
        ];
        let (dsts, _) = module_case("zig", files, caller, refs);
        assert!(
            dsts.contains(&at("addRunArtifact", "test/link/link.zig")),
            "{dsts:?}"
        );
        assert!(
            !dsts
                .iter()
                .any(|(q, p)| q == "pack" && p != "test/link/macho.zig"),
            "{dsts:?}"
        );
        assert!(
            !dsts.contains(&at("addRunArtifact", "lib/std/Build.zig")),
            "{dsts:?}"
        );
    }

    /// R17 (Java): a bare call in a nested class reaches its own class by
    /// qualified owner — never every same-named nested class (`_Fields`)
    /// elsewhere. Near miss: the caller's own overload set still fans out.
    #[test]
    fn java_bare_member_is_the_qualified_calling_class() {
        let m = CallableKind::Method;
        let files = vec![MFile {
            path: "gen/Supervisor.java",
            defs: vec![
                ("gen.Supervisor.A._Fields.find", m, ""),
                ("gen.Supervisor.B._Fields.find", m, ""),
                ("gen.Supervisor.A._Fields.put", m, ""),
            ],
            imports: vec![],
            packages: vec!["gen"],
            bases: vec![],
        }];
        let caller = MFile {
            path: "gen/Nimbus.java",
            defs: vec![
                ("gen.Nimbus.A._Fields.orThrow", m, ""),
                ("gen.Nimbus.A._Fields.find", m, ""),
                ("gen.Nimbus.B._Fields.find", m, ""),
                ("gen.Nimbus.A._Fields.put", m, "void put(int a)"),
                ("gen.Nimbus.A._Fields.put", m, "void put(String a)"),
            ],
            imports: vec![],
            packages: vec!["gen"],
            bases: vec![],
        };
        let refs = vec![
            call("find", ""),
            RefRecord {
                site_byte: 61,
                ..call("put", "")
            },
        ];
        let (dsts, _) = module_case("java", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("gen.Nimbus.A._Fields.find", "gen/Nimbus.java"),
                at("gen.Nimbus.A._Fields.put", "gen/Nimbus.java"),
                at("gen.Nimbus.A._Fields.put", "gen/Nimbus.java"),
            ]
        );
    }

    /// R18 (C#/Java): copies of one class (a sample vendored several
    /// times) — the caller's copy, the one beside it, wins. Near miss: two
    /// copies equally far from the caller are ambiguous, refused and
    /// recorded, never both bound.
    #[test]
    fn class_copies_narrow_to_the_callers_copy() {
        let m = CallableKind::Method;
        let copy = |path: &'static str| MFile {
            path,
            defs: vec![("Samples.Form1.InitializeComponent", m, "")],
            imports: vec![],
            packages: vec!["Samples"],
            bases: vec![],
        };
        let caller = |path: &'static str| MFile {
            path,
            defs: vec![("Samples.Form1.Form1", CallableKind::Constructor, "")],
            imports: vec![],
            packages: vec!["Samples"],
            bases: vec![],
        };
        let files = vec![copy("a/Form1.Designer.cs"), copy("b/Form1.Designer.cs")];
        let (dsts, _) = module_case(
            "csharp",
            files,
            caller("a/Form1.cs"),
            vec![call("InitializeComponent", "")],
        );
        assert_eq!(
            dsts,
            vec![at(
                "Samples.Form1.InitializeComponent",
                "a/Form1.Designer.cs"
            )]
        );
        let files = vec![copy("a/Form1.Designer.cs"), copy("b/Form1.Designer.cs")];
        let (dsts, unresolved) = module_case(
            "csharp",
            files,
            caller("c/Form1.cs"),
            vec![call("InitializeComponent", "")],
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert!(!unresolved.is_empty(), "a refusal is recorded");
    }

    /// R20 (Java): a bare call to a method a derived class overrides is a
    /// virtual call — the declaration and its overrides, as the bare-name
    /// fan-out bound them before, never the declaration alone (which left
    /// every override looking uncalled). Over the fan-out cap it is
    /// refused and recorded. Near miss: an inherited overload set nothing
    /// overrides binds whole (R10).
    #[test]
    fn java_bare_call_to_an_overridden_member_reaches_the_overrides() {
        let m = CallableKind::Method;
        let decl = || MFile {
            path: "src/p/ActionSetDecl.java",
            defs: vec![("p.ActionSet.addOne", m, "")],
            imports: vec![],
            packages: vec!["p"],
            bases: vec![],
        };
        let sub = |path: &'static str, qn: &'static str| MFile {
            path,
            defs: vec![(qn, m, "")],
            imports: vec![],
            packages: vec!["p"],
            bases: vec!["ActionSet"],
        };
        let caller = || MFile {
            path: "src/p/ActionSet.java",
            defs: vec![("p.ActionSet.add", m, "")],
            imports: vec![],
            packages: vec!["p"],
            bases: vec![],
        };
        let files = vec![sub("src/p/Empty.java", "p.Empty.addOne"), decl()];
        let (dsts, _) = module_case("java", files, caller(), vec![call("addOne", "")]);
        assert_eq!(
            dsts,
            vec![
                at("p.ActionSet.addOne", "src/p/ActionSetDecl.java"),
                at("p.Empty.addOne", "src/p/Empty.java"),
            ]
        );

        // Near miss: from a subclass, `this` is never a sibling — the
        // inherited declaration, not the sibling's override.
        let files = vec![sub("src/p/Empty.java", "p.Empty.addOne"), decl()];
        let child = MFile {
            path: "src/p/Child.java",
            defs: vec![("p.Child.run", m, "")],
            imports: vec![],
            packages: vec!["p"],
            bases: vec!["ActionSet"],
        };
        let (dsts, _) = module_case("java", files, child, vec![call("addOne", "")]);
        assert_eq!(
            dsts,
            vec![at("p.ActionSet.addOne", "src/p/ActionSetDecl.java")]
        );

        let mut files = vec![decl()];
        for (path, qn) in [
            ("src/p/S1.java", "p.S1.addOne"),
            ("src/p/S2.java", "p.S2.addOne"),
            ("src/p/S3.java", "p.S3.addOne"),
            ("src/p/S4.java", "p.S4.addOne"),
            ("src/p/S5.java", "p.S5.addOne"),
        ] {
            files.push(sub(path, qn));
        }
        let (dsts, unresolved) =
            module_case("java", files, caller(), vec![call("addOne", "")]);
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(
            unresolved,
            vec![UnresolvedReason::FanoutCapExceeded { candidates: 6 }]
        );

        let files = vec![MFile {
            path: "src/p/PlainBase.java",
            defs: vec![
                ("p.PlainBase.helper", m, "void helper(int a)"),
                ("p.PlainBase.helper", m, "void helper(String a)"),
            ],
            imports: vec![],
            packages: vec!["p"],
            bases: vec![],
        }];
        let caller = MFile {
            path: "src/p/Plain.java",
            defs: vec![("p.Plain.run", m, "")],
            imports: vec![],
            packages: vec!["p"],
            bases: vec!["PlainBase"],
        };
        let (dsts, _) = module_case("java", files, caller, vec![call("helper", "")]);
        assert_eq!(
            dsts,
            vec![
                at("p.PlainBase.helper", "src/p/PlainBase.java"),
                at("p.PlainBase.helper", "src/p/PlainBase.java"),
            ]
        );
    }

    /// R19 (Zig): `const assert = std.debug.assert;` where `std` is not in
    /// the tree is a call into the dependency — external, never a
    /// workspace `assert`. Near miss: where the module is in the tree
    /// (`lib/std/testing.zig`), the alias is followed to it.
    #[test]
    fn zig_alias_of_a_dependency_is_external_and_of_the_tree_is_followed() {
        let files = vec![mfile(
            "src/clients/java/src/jni_tests.zig",
            &["assert"],
            vec![],
        )];
        let caller = mfile(
            "src/vsr.zig",
            &["run"],
            vec![
                import("import", "std", "std"),
                import("alias", "std.debug.assert", "assert"),
            ],
        );
        let (dsts, unresolved) =
            module_case("zig", files, caller, vec![call("assert", "")]);
        assert!(dsts.is_empty(), "{dsts:?}");
        assert_eq!(unresolved, vec![external("std")]);

        let files = vec![
            mfile("lib/std/std.zig", &["init"], vec![]),
            mfile("lib/std/testing.zig", &["expect"], vec![]),
            mfile("lib/other/expect.zig", &["expect"], vec![]),
        ];
        let caller = mfile(
            "test/behavior.zig",
            &["checkIt"],
            vec![
                import("import", "std", "std"),
                import("alias", "std.testing.expect", "expect"),
            ],
        );
        let (dsts, _) = module_case("zig", files, caller, vec![call("expect", "")]);
        assert_eq!(dsts, vec![at("expect", "lib/std/testing.zig")]);
    }

    /// R14 (Erlang): `m:f(…)` binds module `m`'s `f` — the shipped copy
    /// when test data keeps another, the clause of the call's arity —
    /// never another module's `f`.
    #[test]
    fn erlang_remote_call_binds_its_module_only() {
        let files = vec![
            MFile {
                path: "lib/asn1/src/asn1ct_gen_per.erl",
                defs: vec![
                    (
                        "asn1ct_gen_per:gen_dec_prim",
                        CallableKind::Function,
                        "gen_dec_prim(A) -> a",
                    ),
                    (
                        "asn1ct_gen_per:gen_dec_prim",
                        CallableKind::Function,
                        "gen_dec_prim(A, B) -> b",
                    ),
                ],
                imports: vec![],
                packages: vec![],
                bases: vec![],
            },
            mfile(
                "lib/dialyzer/test/r9c_SUITE_data/src/asn1/asn1ct_gen_per.erl",
                &["asn1ct_gen_per:gen_dec_prim"],
                vec![],
            ),
            mfile(
                "lib/asn1/src/asn1ct_gen_ber_bin_v2.erl",
                &["asn1ct_gen_ber_bin_v2:gen_dec_prim"],
                vec![],
            ),
        ];
        let caller = mfile(
            "lib/ssl/src/user.erl",
            &["user:run"],
            vec![import("include", "x.hrl", "")],
        );
        let refs = vec![RefRecord {
            arity: Some(2),
            ..call("asn1ct_gen_per:gen_dec_prim", "asn1ct_gen_per")
        }];
        let (dsts, _) = module_case("erlang", files, caller, refs);
        assert_eq!(
            dsts,
            vec![at(
                "asn1ct_gen_per:gen_dec_prim",
                "lib/asn1/src/asn1ct_gen_per.erl"
            )]
        );
    }

    /// R15 (F#): a bare call is never a type's member, wherever that type
    /// is declared; the opened module's function is.
    #[test]
    fn fsharp_bare_call_never_reaches_a_member() {
        let files = vec![
            MFile {
                path: "tests/Resolver/Generators.fs",
                defs: vec![("PackageTypes.PackageName", CallableKind::Method, "")],
                imports: vec![],
                packages: vec![],
                bases: vec![],
            },
            mfile("src/Core/Domain.fs", &["Paket.Domain.PackageName"], vec![]),
        ];
        let caller = mfile(
            "tests/Resolver/PropertyTests.fs",
            &["PropertyTests.check"],
            vec![import("open", "Paket.Domain", "")],
        );
        let (dsts, _) =
            module_case("fsharp", files, caller, vec![call("PackageName", "")]);
        assert!(
            !dsts.contains(&at(
                "PackageTypes.PackageName",
                "tests/Resolver/Generators.fs"
            )),
            "{dsts:?}"
        );
    }

    /// R16 (Dart): an import scopes bare names to the file it names. A
    /// same-named function in some other file whose path merely contains
    /// `fixtures` (`shell/common/fixtures/…`) is not it; the imported
    /// file's own function is. WS14: a name a prefixed import brings in
    /// is reached only through the prefix, and a name no workspace import
    /// provides comes from a dependency (`package:test`) — refused as
    /// external, never matched by name.
    #[test]
    fn dart_bare_call_needs_an_imported_file() {
        let files = vec![
            mfile(
                "shell/common/fixtures/shell_test.dart",
                &["expect", "load"],
                vec![],
            ),
            mfile("tools/pkg/configs/test/fixtures.dart", &["load"], vec![]),
        ];
        for (prefix, want) in [
            ("", vec![at("load", "tools/pkg/configs/test/fixtures.dart")]),
            ("fixtures", vec![]),
        ] {
            let caller = mfile(
                "tools/pkg/configs/test/loader_test.dart",
                &["main"],
                vec![
                    import("import", "package:test/test.dart", ""),
                    import("import", "fixtures.dart", prefix),
                ],
            );
            let mut refs = vec![
                call("expect", ""),
                RefRecord {
                    site_byte: 61,
                    ..call("load", "")
                },
            ];
            if !prefix.is_empty() {
                refs.push(RefRecord {
                    site_byte: 71,
                    ..call("load", prefix)
                });
            }
            let (dsts, why) = module_case("dart", files.clone(), caller, refs);
            let mut want = want;
            if !prefix.is_empty() {
                want.push(at("load", "tools/pkg/configs/test/fixtures.dart"));
            }
            assert_eq!(dsts, want, "prefix {prefix:?}: {dsts:?}");
            assert!(why.contains(&external("package:test/test.dart")), "{why:?}");
        }
    }

    /// WS14 (Dart): `package:<name>/x.dart` names `<root>/lib/x.dart` of
    /// the pubspec declaring `<name>`, and an import reaches what that
    /// library exports, filtered by `show` / `hide`.
    #[test]
    fn dart_package_uris_exports_and_combinators() {
        let pkg = |path: &'static str, defs: &[&'static str], imports| MFile {
            packages: vec!["pkg_a"],
            ..mfile(path, defs, imports)
        };
        let files = vec![
            pkg(
                "pkg_a/lib/pkg_a.dart",
                &[],
                vec![
                    import("package-root", "pkg_a", ""),
                    import("export", "src/helpers.dart", ""),
                    import("show", "src/helpers.dart", "helperOne"),
                    import("export", "src/other.dart", ""),
                ],
            ),
            pkg(
                "pkg_a/lib/src/helpers.dart",
                &["helperOne", "helperTwo"],
                vec![import("package-root", "pkg_a", "")],
            ),
            pkg(
                "pkg_a/lib/src/other.dart",
                &["otherFn"],
                vec![import("package-root", "pkg_a", "")],
            ),
            // A decoy every import misses.
            mfile("tool/helpers.dart", &["helperTwo"], vec![]),
        ];
        let caller = mfile(
            "app/test/main_test.dart",
            &["main"],
            vec![import("import", "package:pkg_a/pkg_a.dart", "")],
        );
        let refs = vec![
            call("helperOne", ""),
            RefRecord {
                site_byte: 21,
                ..call("otherFn", "")
            },
            RefRecord {
                site_byte: 41,
                ..call("helperTwo", "")
            },
        ];
        let (dsts, _) = module_case("dart", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("helperOne", "pkg_a/lib/src/helpers.dart"),
                at("otherFn", "pkg_a/lib/src/other.dart"),
            ],
            "{dsts:?}"
        );
    }

    /// WS14 (goal a): a bare call through a parameter or local is the
    /// value's, never another file's same-named function; the same name
    /// out of the binding's scope still resolves as before.
    #[test]
    fn a_local_binding_is_never_matched_across_files() {
        let files = vec![mfile("src/util.js", &["cb"], vec![])];
        let caller = mfile(
            "src/main.js",
            &["main"],
            vec![import("from-import", "./other", "x")],
        );
        let refs = vec![
            call("cb", ""),
            RefRecord {
                site_byte: 600,
                ..call("cb", "")
            },
        ];
        let (dsts, why) = module_case_with("javascript", files, caller, refs, |f| {
            f.locals = vec![cgg_core::LocalBinding {
                name: "cb".into(),
                start_byte: 0,
                end_byte: 100,
                decl_byte: 0,
            }];
        });
        // Byte 0 is inside the binding; byte 600 (still in `main`) is not.
        assert_eq!(dsts, vec![at("cb", "src/util.js")], "{dsts:?}");
        assert!(why.contains(&UnresolvedReason::ShadowedByLocal), "{why:?}");
    }

    /// WS14 (PHP): `X::m()` is the method of the class `X` names through
    /// the file's namespace and `use` table; a dependency's class of the
    /// same short name is external; `parent::m()` is the base's; `f()` is
    /// the namespace's function, else the global one.
    #[test]
    fn php_namespaces_and_use_tables_decide_the_class() {
        let pkg = |path: &'static str, ns: &'static str, defs: &[&'static str]| {
            mfile(path, defs, vec![import("package", ns, "")])
        };
        let files = vec![
            pkg(
                "src/Util/Uuid.php",
                "Acme\\Util",
                &["Uuid::fromString", "helper"],
            ),
            pkg("src/Other/Uuid.php", "Acme\\Other", &["Uuid::fromString"]),
            pkg("src/Other/Base.php", "Acme\\Other", &["Base::boot"]),
        ];
        let caller = MFile {
            bases: vec!["Base"],
            ..mfile(
                "app/Main.php",
                &["Main::boot"],
                vec![
                    import("package", "App", ""),
                    import("use", "Acme\\Util\\Uuid", ""),
                    import("use", "Vendor\\Lib\\Uuid", "VUuid"),
                    import("use", "Acme\\Util\\helper", ""),
                    import("use", "Acme\\Other\\Base", ""),
                ],
            )
        };
        let refs = vec![
            call("fromString", "Uuid"),
            RefRecord {
                site_byte: 21,
                ..call("fromString", "VUuid")
            },
            RefRecord {
                site_byte: 41,
                ..call("helper", "")
            },
            RefRecord {
                site_byte: 61,
                ..call("boot", "parent")
            },
        ];
        let (dsts, why) = module_case("php", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("Uuid::fromString", "src/Util/Uuid.php"),
                at("helper", "src/Util/Uuid.php"),
                at("Base::boot", "src/Other/Base.php"),
            ],
            "{dsts:?}"
        );
        assert!(why.contains(&external("vendor\\lib\\uuid")), "{why:?}");
    }

    /// WS14 (Zig): a path through file-level `@import`s reaches the
    /// function the last file declares; a module a build script declares
    /// is the tree's, whatever its root file is called (`@import("xev")`
    /// → `src/main.zig`); a module the tree does not hold is external.
    #[test]
    fn zig_import_paths_and_build_modules() {
        let files = vec![
            mfile(
                "build.zig",
                &["build"],
                vec![import("zig-module", "src/main.zig", "xev")],
            ),
            mfile(
                "src/main.zig",
                &[],
                vec![import("import", "loop.zig", "loop")],
            ),
            mfile("src/loop.zig", &["run"], vec![]),
            mfile("src/other/loop.zig", &["run"], vec![]),
        ];
        let caller = mfile(
            "examples/basic.zig",
            &["main"],
            vec![
                import("import", "xev", "xev"),
                import("import", "std", "std"),
            ],
        );
        let refs = vec![
            call("run", "xev.loop"),
            RefRecord {
                site_byte: 21,
                ..call("run", "std.loop")
            },
        ];
        let (dsts, why) = module_case("zig", files, caller, refs);
        assert_eq!(dsts, vec![at("run", "src/loop.zig")], "{dsts:?}");
        assert!(why.contains(&external("std")), "{why:?}");

        // `pub const checksum = @import("vsr/checksum.zig").checksum;` in
        // `vsr.zig`: `vsr.checksum()` is that file's function, not a
        // same-named one elsewhere.
        let files = vec![
            mfile(
                "src/vsr.zig",
                &[],
                vec![import(
                    "import-member",
                    "vsr/checksum.zig#checksum",
                    "checksum",
                )],
            ),
            mfile("src/vsr/checksum.zig", &["checksum"], vec![]),
            mfile("src/other/checksum.zig", &["checksum"], vec![]),
        ];
        let caller = mfile(
            "src/testing/storage.zig",
            &["check"],
            vec![import("import", "../vsr.zig", "vsr")],
        );
        let (dsts, _) = module_case("zig", files, caller, vec![call("checksum", "vsr")]);
        assert_eq!(
            dsts,
            vec![at("checksum", "src/vsr/checksum.zig")],
            "{dsts:?}"
        );
    }

    /// WS14b (Zig): a receiver typed as a container (`s.close()` with
    /// `s: *Surface`, `Surface = @import("surface.zig").Surface`) reaches
    /// the member that container declares, not a same-named method of
    /// another container in the same or another file; a name the
    /// container does not declare is refused. A call on a type
    /// function's result through a dependency is external.
    #[test]
    fn zig_container_typed_receiver_reaches_its_member() {
        let files = vec![
            // `pub const Surface = struct { fn close }` and
            // `pub const Window = struct { fn close }` in one file.
            mfile("src/surface.zig", &["close", "close"], vec![]),
            mfile("src/other.zig", &["close", "missing", "init"], vec![]),
        ];
        let caller = mfile(
            "src/split_tree.zig",
            &["closeActive"],
            vec![
                import("import-member", "surface.zig#Surface", "Surface"),
                import("import", "std", "std"),
            ],
        );
        let refs = vec![
            call("close", "Surface"),
            RefRecord {
                site_byte: 30,
                ..call("missing", "Surface")
            },
            RefRecord {
                site_byte: 40,
                ..call("init", "std.array_list.Managed(T)")
            },
        ];
        let spans = |is_caller: bool, fi: usize, f: &mut FileFacts| {
            if !is_caller && fi == 0 {
                // Defs of a non-caller file sit at 2000.. and 2010...
                f.type_spans = vec![
                    cgg_core::TypeSpan {
                        name: "Surface".into(),
                        start_byte: 1990,
                        end_byte: 2008,
                        ..Default::default()
                    },
                    cgg_core::TypeSpan {
                        name: "Window".into(),
                        start_byte: 2008,
                        end_byte: 2020,
                        ..Default::default()
                    },
                ];
            }
        };
        let (dsts, why) =
            module_case_each("zig", files.clone(), caller.clone(), refs, spans);
        assert_eq!(dsts.len(), 1, "{dsts:?} {why:?}");
        assert_eq!(dsts[0].1, "src/surface.zig", "{dsts:?}");
        assert!(why.contains(&refused("Surface")), "{why:?}");
        assert!(why.contains(&external("std")), "{why:?}");
        // Near miss: without the container spans the same call is not
        // bound to either `close` by name.
        let (dsts, _) = module_case("zig", files, caller, vec![call("close", "Surface")]);
        assert!(dsts.is_empty(), "{dsts:?}");
    }

    /// WS14b (Perl): `$self->m()` on the caller's invocant is the caller
    /// package's `m`, else the first up its bases (depth-first); with
    /// the chain leaving the tree it is external. A same-named sub of an
    /// unrelated package is never the callee, and a receiver that is not
    /// the invocant keeps the old reading.
    #[test]
    fn perl_invocant_call_reaches_the_package_and_its_bases() {
        let files = vec![
            mfile(
                "lib/My/Domain.pm",
                &["My::Domain::is_public", "My::Domain::base_only"],
                vec![import("perl-package", "My::Domain", "")],
            ),
            mfile(
                "lib/My/Other.pm",
                &[
                    "My::Other::is_public",
                    "My::Other::base_only",
                    "My::Other::render",
                ],
                vec![import("perl-package", "My::Other", "")],
            ),
        ];
        let caller = mfile(
            "lib/My/Domain/Void.pm",
            &["My::Domain::Void::status"],
            vec![
                import("perl-package", "My::Domain::Void", ""),
                import("perl-isa", "My::Domain", "My::Domain::Void"),
                import("perl-isa", "Ext::Base", "My::Domain::Void"),
            ],
        );
        let inv = |name: &str, at: u32| RefRecord {
            invocant: true,
            site_byte: at,
            ..call(name, "$self")
        };
        let refs = vec![inv("base_only", 10), inv("render", 20)];
        let (dsts, why) = module_case("perl", files.clone(), caller.clone(), refs);
        assert_eq!(
            dsts,
            vec![at("My::Domain::base_only", "lib/My/Domain.pm")],
            "{dsts:?}"
        );
        assert!(why.contains(&external("Ext::Base")), "{why:?}");
        // Near miss: not the invocant — not bound through the bases.
        let (dsts, _) = module_case("perl", files, caller, vec![call("base_only", "$x")]);
        assert!(
            !dsts.contains(&at("My::Domain::base_only", "lib/My/Domain.pm"))
                || dsts.len() > 1,
            "{dsts:?}"
        );
    }

    /// WS14b (PHP): a global function declared in several files none of
    /// which the caller's include chain reaches binds to the one
    /// unconditional definition when every other copy is a conditional
    /// fallback (`if (!function_exists(…))`); with two unconditional
    /// copies it stays refused.
    #[test]
    fn php_unconditional_definition_beats_guarded_fallbacks() {
        let files = vec![
            mfile("inc/cache.php", &["get_multi", "dup"], vec![]),
            mfile("inc/compat.php", &["get_multi"], vec![]),
            mfile("admin/noop.php", &["dup"], vec![]),
        ];
        let caller = mfile("lib/client.php", &["run"], vec![]);
        let refs = vec![
            call("get_multi", ""),
            RefRecord {
                site_byte: 70,
                ..call("dup", "")
            },
        ];
        let (dsts, _) =
            module_case_each("php", files, caller, refs, |is_caller, fi, f| {
                if !is_caller && fi == 1 {
                    f.definitions[0].conditional = true;
                }
            });
        assert_eq!(dsts, vec![at("get_multi", "inc/cache.php")], "{dsts:?}");
    }

    /// WS14b (Scala): a bare call inherited through `extends QueryTest`
    /// reaches the `QueryTest` the caller's file imports, not a same-named
    /// trait of another package, nor the companion object's member.
    #[test]
    fn scala_base_is_resolved_through_the_declaring_files_imports() {
        let ts = |name: &str, kind: &str, pkg: &str, bases: &[&str], s: u32, e: u32| {
            cgg_core::TypeSpan {
                name: name.into(),
                start_byte: s,
                end_byte: e,
                kind: kind.into(),
                bases: bases.iter().map(|b| b.to_string()).collect(),
                package: pkg.into(),
                decls: vec![],
            }
        };
        let files = vec![
            // trait QueryTest { def check }  /  object QueryTest { def check }
            mfile(
                "core/QueryTest.scala",
                &["QueryTest::check", "QueryTest::check"],
                vec![],
            ),
            mfile("connect/QueryTest.scala", &["QueryTest::check"], vec![]),
        ];
        let caller = mfile(
            "hive/Suite.scala",
            &["Suite::run"],
            vec![import("import", "org.ex.sql.QueryTest", "")],
        );
        let (dsts, _) = module_case_each(
            "scala",
            files,
            caller,
            vec![call("check", "")],
            |is_caller, fi, f| {
                f.type_spans = match (is_caller, fi) {
                    (true, _) => vec![ts(
                        "Suite",
                        "class",
                        "org.ex.sql.hive",
                        &["QueryTest"],
                        0,
                        1000,
                    )],
                    (false, 0) => vec![
                        ts("QueryTest", "trait", "org.ex.sql", &[], 1990, 2008),
                        ts("QueryTest", "object", "org.ex.sql", &[], 2008, 2020),
                    ],
                    _ => vec![ts(
                        "QueryTest",
                        "class",
                        "org.ex.sql.connect",
                        &[],
                        1990,
                        2008,
                    )],
                };
            },
        );
        assert_eq!(
            dsts,
            vec![at("QueryTest::check", "core/QueryTest.scala")],
            "{dsts:?}"
        );

        // N1: a bare name an explicit import brings in from a package the
        // tree does not declare is external, not a workspace `render`.
        let files = vec![mfile("ui/Other.scala", &["Other::render"], vec![])];
        let caller = mfile(
            "ui/Page.scala",
            &["Page::show"],
            vec![import(
                "import",
                "org.json4s.JsonMethods.{compact,render}",
                "",
            )],
        );
        let (dsts, why) = module_case_each(
            "scala",
            files,
            caller,
            vec![call("render", "")],
            |is_caller, _, f| {
                f.type_spans = if is_caller {
                    vec![ts("Page", "class", "org.ex.ui", &[], 0, 1000)]
                } else {
                    vec![ts("Other", "class", "org.ex.ui", &[], 1990, 2008)]
                };
            },
        );
        assert!(dsts.is_empty(), "{dsts:?}");
        assert!(
            why.contains(&external("org.json4s.JsonMethods.render")),
            "{why:?}"
        );
    }

    /// WS14 (Scala): a bare call reaches the calling class, its bases and
    /// an object `import a.Owner._` names — never an unrelated class's
    /// member (`assert` is not `SubmitRestProtocolMessage::assert`).
    #[test]
    fn scala_bare_call_reaches_bases_and_imported_objects_only() {
        let files = vec![
            mfile("core/Message.scala", &["Message::assert"], vec![]),
            mfile("core/Helpers.scala", &["Helpers::helper"], vec![]),
            mfile("core/Base.scala", &["Base::log"], vec![]),
        ];
        let caller = MFile {
            bases: vec!["Base"],
            ..mfile(
                "core/Suite.scala",
                &["Suite::test"],
                vec![import("import", "org.acme.Helpers._", "")],
            )
        };
        let refs = vec![
            call("assert", ""),
            RefRecord {
                site_byte: 21,
                ..call("helper", "")
            },
            RefRecord {
                site_byte: 41,
                ..call("log", "")
            },
        ];
        let (dsts, _) = module_case("scala", files, caller, refs);
        assert_eq!(
            dsts,
            vec![
                at("Helpers::helper", "core/Helpers.scala"),
                at("Base::log", "core/Base.scala"),
            ],
            "{dsts:?}"
        );
    }

    /// WS14 (Elixir): a bare call reaches the calling module and the
    /// modules it `import`s or `use`s (options after the module name
    /// ignored) — not another module's same-named function.
    #[test]
    fn elixir_bare_call_reaches_its_module_and_imports() {
        let files = vec![
            mfile(
                "lib/query_builder.ex",
                &["Stats.QueryBuilder::from"],
                vec![],
            ),
            mfile("lib/helpers.ex", &["App.Helpers::h"], vec![]),
        ];
        let caller = mfile(
            "lib/goals.ex",
            &["App.Goals::a"],
            vec![
                import("import", "Ecto.Query", ""),
                import("import", "App.Helpers, only: [h: 1]", ""),
            ],
        );
        let refs = vec![
            call("from", ""),
            RefRecord {
                site_byte: 21,
                ..call("h", "")
            },
        ];
        let (dsts, _) = module_case("elixir", files, caller, refs);
        assert_eq!(
            dsts,
            vec![at("App.Helpers::h", "lib/helpers.ex")],
            "{dsts:?}"
        );
    }

    /// Copies are files: the overloads or clauses one file declares
    /// under a name stay together; from outside the tests the one copy
    /// that is not test data is chosen.
    #[test]
    fn copies_are_files_and_test_data_is_not_shipped() {
        let files = vec![
            mfile("lib/compiler/src/cerl.erl", &["cerl:fold_literal"], vec![]),
            mfile(
                "lib/dialyzer/test/opaque_SUITE_data/src/cerl.erl",
                &["cerl:fold_literal"],
                vec![],
            ),
            mfile(
                "lib/dialyzer/test/options1_SUITE_data/src/cerl.erl",
                &["cerl:fold_literal"],
                vec![],
            ),
        ];
        let caller = mfile("lib/dialyzer/src/dataflow.erl", &["dataflow:run"], vec![]);
        let (dsts, _) = module_case(
            "erlang",
            files,
            caller,
            vec![call("cerl:fold_literal", "cerl")],
        );
        assert_eq!(
            dsts,
            vec![at("cerl:fold_literal", "lib/compiler/src/cerl.erl")]
        );
    }
}
