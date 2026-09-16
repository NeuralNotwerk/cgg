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
pub fn resolve(graph: &Graph, facts: &[FileFacts], fanout_cap: usize) -> CrossFileOutput {
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
    for c in graph.callables.values() {
        by_qn.insert((c.language.clone(), c.qualified_name.clone()), c.id);
        by_simple
            .entry((c.language.clone(), c.simple_name.clone()))
            .or_default()
            .push(c.id);
        if let Some(owner) = owner_from_qn(&c.qualified_name)
            && (!is_std_type_local_trait_impl(&c.language, &c.qualified_name)
                || workspace_inherent_owners
                    .contains(&(c.language.clone(), owner.to_string())))
        {
            by_owner_method
                .entry((c.language.clone(), owner.to_string(), c.simple_name.clone()))
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
    let mut rust_path_heads: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for (lang_key, qn) in by_qn.keys() {
        if lang_key == "rust"
            && let Some(head) = qn.split("::").next()
        {
            rust_path_heads.insert(head.to_string());
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
    // member or an `@abstractmethod`. They carry no body worth entering,
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
                c.attributes.iter().any(|a| a.contains("abstractmethod"))
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
    // chain when a method is inherited rather than declared. Recorded on
    // methods rather than types, because cgg's model has no node for a
    // type — any method of the class carries the same base list.
    let mut bases_by_owner: HashMap<(String, String), Vec<String>> = HashMap::new();
    for f in facts {
        for d in &f.definitions {
            if d.base_types.is_empty() {
                continue;
            }
            let Some(owner) = owner_from_qn(&d.qualified_name) else {
                continue;
            };
            let slot = bases_by_owner
                .entry((f.language.clone(), owner.to_string()))
                .or_default();
            for b in &d.base_types {
                // Store the bare type name: the index is keyed that way,
                // and a base is written as `generic.ObjectListView` or
                // `Handler<T>` as often as plainly.
                let bare = b.split(['<', '[']).next().unwrap_or(b).trim();
                let bare = bare.rsplit(['.', ':', '\\']).next().unwrap_or(bare);
                if !bare.is_empty() && !slot.iter().any(|x| x == bare) {
                    slot.push(bare.to_string());
                }
            }
        }
    }

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
    let mut files_by_lang: HashMap<&str, Vec<(FileId, String)>> = HashMap::new();
    for f in facts {
        files_by_lang
            .entry(f.language.as_str())
            .or_default()
            .push((f.file, f.path.to_string_lossy().to_ascii_lowercase()));
    }

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
                    if conflicted.contains(lt.var_name.as_str()) {
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

            for imp in &facts.imports {
                match imp.kind.as_str() {
                    "from-import" => {
                        // Python: imp.path is module; imp.alias is the
                        // items list ("greet, compute" or "greet as g").
                        // JS/TS: imp.path is relative path; items are
                        // exported names from that file.
                        let module = imp.path.trim();
                        for item in imp.alias.split(',') {
                            let (src, alias) = match item.split_once(" as ") {
                                Some((s, a)) => (s.trim(), a.trim()),
                                None => (item.trim(), item.trim()),
                            };
                            if src.is_empty() {
                                continue;
                            }
                            let qn = format!("{module}.{src}");
                            direct_imports
                                .entry(alias.to_string())
                                .or_default()
                                .push(qn);
                            // For JS/TS where definitions don't carry a
                            // module prefix, also try the bare name.
                            if module.starts_with('.') || module.starts_with('/') {
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
                        direct_imports
                            .entry("__using_static__".into())
                            .or_default()
                            .push(full);
                    }
                    "include" if matches!(lang.as_str(), "c" | "cpp" | "objc") => {
                        // C/C++: `#include "helpers.h"` — all definitions
                        // from the included file become available in this
                        // TU. We resolve the path relative to the current
                        // file and transitively chase includes up to 8
                        // levels deep.
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
                                    if let Some(last) = prefix.rsplit('.').next() {
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
                            | "julia" => {
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
            let mut scoped_simple: HashMap<String, Vec<CallableId>> = HashMap::new();
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
                for target in module_aliases.values() {
                    path_fragments.push(target.replace('.', "/").to_ascii_lowercase());
                }
                path_fragments.retain(|f| !f.is_empty());
                path_fragments.sort();
                path_fragments.dedup();
                let candidates = files_by_lang
                    .get(lang.as_str())
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                for (fid, fpath) in candidates {
                    if !path_fragments
                        .iter()
                        .any(|frag| fpath.contains(frag.as_str()))
                    {
                        continue;
                    }
                    if let Some(target) = facts_by_id.get(fid) {
                        for d in &target.definitions {
                            if let Some(cid) = by_qn
                                .get(&(lang.clone(), d.qualified_name.clone()))
                                .copied()
                            {
                                scoped_simple
                                    .entry(d.simple_name.clone())
                                    .or_default()
                                    .push(cid);
                            }
                        }
                    }
                }
            }

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

                let _sp_ref = cgg_core::profile::span("xfile::resolve-ref");
                let mut capped = 0u32;
                let mut no_ctor = false;
                let super_recv = is_super_receiver(&r.receiver_hint);
                let resolved = try_resolve_ref(
                    graph,
                    facts.file,
                    &lang,
                    r,
                    &direct_imports,
                    &module_aliases,
                    &unqualified_prefixes,
                    &scoped_simple,
                    &by_qn,
                    &by_simple,
                    &by_owner_method,
                    &by_full_owner_method,
                    &reexports,
                    &bases_by_owner,
                    &known_owners,
                    &stub_ids,
                    &signatures,
                    &var_types,
                    caller_qn,
                    fanout_cap,
                    &rust_path_heads,
                    &workspace_owner_names,
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
                // An unambiguous binding is not a guess. A bare name
                // bound by `from x import y` in this very file, or a
                // module alias resolving to exactly one callable, is as
                // certain as same-file resolution — and while it scored
                // `medium`, a same-file method with a colliding name
                // could outrank the correct target at `high`. Anything
                // with more than one candidate stays `medium`: that is
                // fan-out, and fan-out is a hypothesis.
                let confidence = match &resolved {
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
                                resolver: resolver_id.clone(),
                                weight: 1,
                            });
                        }
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
    let resolved = includer_dir.join(include_path);
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
            include_by_last
                .get(last)?
                .iter()
                .find(|f| f.path.ends_with(include_path))
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
    // Import all definitions from the target.
    for d in &target.definitions {
        direct_imports
            .entry(d.simple_name.clone())
            .or_default()
            .push(d.qualified_name.clone());
    }
    // Transitively follow includes in the target.
    for imp in &target.imports {
        if imp.kind == "include" {
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

fn try_resolve_ref(
    graph: &Graph,
    caller_file: FileId,
    lang: &str,
    r: &cgg_core::RefRecord,
    direct_imports: &HashMap<String, Vec<String>>,
    module_aliases: &HashMap<String, String>,
    unqualified_prefixes: &[String],
    scoped_simple: &HashMap<String, Vec<CallableId>>,
    by_qn: &HashMap<(String, String), CallableId>,
    by_simple: &HashMap<(String, String), Vec<CallableId>>,
    by_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    by_full_owner_method: &HashMap<(String, String, String), Vec<CallableId>>,
    reexports: &HashMap<(String, String), String>,
    bases_by_owner: &HashMap<(String, String), Vec<String>>,
    known_owners: &std::collections::HashSet<(String, String)>,
    stub_ids: &std::collections::HashSet<CallableId>,
    signatures: &HashMap<CallableId, &str>,
    var_types: &HashMap<String, String>,
    caller_qn: Option<&str>,
    fanout_cap: usize,
    // First path segments seen in Rust qualified names (see its
    // construction in `resolve`) — used to tell an unresolved workspace
    // path from an external crate before falling into step 5's fan-out.
    rust_path_heads: &std::collections::HashSet<String>,
    workspace_owner_names: &std::collections::HashSet<String>,
    // Set to the candidate count when the fan-out cap rejected a
    // non-empty set, so the caller can record the drop instead of
    // leaving the site looking uncalled.
    capped: &mut u32,
    // Set when the call names a class cgg knows but that declares no
    // initializer, so the caller can say that rather than "no candidate".
    no_ctor: &mut bool,
) -> Option<Vec<CallableId>> {
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

    // Step 1: direct import match — `foo()` where `foo` was
    // imported.
    if r.receiver_hint.is_empty() {
        if let Some(qns) = direct_imports.get(&r.name) {
            let cids: Vec<_> = qns
                .iter()
                .filter_map(|qn| lookup_with_reexports(lang, qn, by_qn, reexports))
                .collect();
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
            // Try just the name itself as a qualified name
            if let Some(cid) = lookup_with_reexports(lang, &r.name, by_qn, reexports) {
                return Some(vec![cid]);
            }
        }
        // Step 1b: bare-name lookup via unqualified-import prefixes
        // (Haskell `import Data.Map` + `lookup`, OCaml `open Foo` + `bar`,
        // Elixir `import Foo` + `bar`, F# `open System.IO` + `File`, …).
        for prefix in unqualified_prefixes {
            for joiner in [".", "::"] {
                let qn = format!("{prefix}{joiner}{}", r.name);
                if let Some(cid) = lookup_with_reexports(lang, &qn, by_qn, reexports) {
                    return Some(vec![cid]);
                }
            }
        }
        // Step 1c: scoped by-simple lookup — restricted to callables
        // defined in files whose path matches one of this file's
        // import prefixes. This carries Haskell/OCaml/Dart over the
        // gap where the plugin omits the module prefix from
        // qualified_name. Cap candidates at 8 to bound noise.
        if let Some(cids) = scoped_simple.get(&r.name)
            && !cids.is_empty()
            && cids.len() <= 8
        {
            return Some(cids.clone());
        }
        // Step 1d: global by-simple fallback. Last resort — only when
        // the file has at least one import and the simple name is
        // unique-ish (≤3 candidates). Skip stdlib-ish names.
        let has_imports = !direct_imports.is_empty()
            || !module_aliases.is_empty()
            || !unqualified_prefixes.is_empty();
        if has_imports {
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
        if let Some(module) = module_aliases.get(first) {
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
            // For JS/TS: definitions don't carry a module prefix, so
            // try bare name as fallback when the module is a relative
            // path or a short package-like name that doesn't match
            // any qualified name prefix.
            if let Some(cid) = lookup_with_reexports(lang, &r.name, by_qn, reexports) {
                return Some(vec![cid]);
            }
        }

        // Step 3: qualified-path call `foo::bar::baz()` (Rust) or
        // `foo.bar.baz()` (Python / Go / C#). The receiver_hint is
        // already the joined path. Try both the Rust and the dotted
        // form.
        let mut rh_buf = r.receiver_hint.trim().to_string();
        let rh = rh_buf.as_str();
        if !rh.is_empty() {
            // Direct paths in both joiners.
            let direct_dot = format!("{rh}.{}", r.name);
            if let Some(cid) = lookup_with_reexports(lang, &direct_dot, by_qn, reexports)
            {
                return Some(vec![cid]);
            }
            let direct = format!("{rh}::{}", r.name);
            if let Some(cid) = lookup_with_reexports(lang, &direct, by_qn, reexports) {
                return Some(vec![cid]);
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
                || rh.chars().next().is_some_and(|c| c.is_uppercase()))
        {
            let external = lang == "rust"
                && rh.contains("::")
                && is_external_rust_head(
                    rh.split("::").next().unwrap_or(""),
                    rust_path_heads,
                    workspace_owner_names,
                    direct_imports,
                    module_aliases,
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
                        return Some(cids);
                    }
                }
                for owner in owners {
                    if let Some(cids) = by_owner_method.get(&(
                        lang.to_string(),
                        owner.to_string(),
                        r.name.clone(),
                    )) && !cids.is_empty()
                    {
                        return Some(cids.clone());
                    }
                }
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
        if lang == "rust"
            && rh.contains("::")
            && is_external_rust_head(
                rh.split("::").next().unwrap_or(""),
                rust_path_heads,
                workspace_owner_names,
                direct_imports,
                module_aliases,
            )
        {
            return None;
        }

        // Step 5: Trait/interface method dispatch.
        // When receiver_hint is a variable name (lowercase) and the method
        // name exists on definitions in other files, find all callables
        // with that simple_name. This handles `formatter.render()` where
        // formatter is a trait object — we emit edges to all implementors.
        // Only applies when the method name is NOT a common stdlib method
        // (to avoid matching `vec.is_empty()` to `WalkOutcome::is_empty`).
        if !rh.is_empty()
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
    rust_path_heads: &std::collections::HashSet<String>,
    workspace_owner_names: &std::collections::HashSet<String>,
    direct_imports: &HashMap<String, Vec<String>>,
    module_aliases: &HashMap<String, String>,
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
    // A receiver's first segment is not always a crate: an enum-variant
    // or associated-const receiver leads with the TYPE (`TrustKind` in
    // `TrustKind::Network`). Such a head is in-workspace whenever the
    // workspace declares a type by that name, and treating it as
    // external would refuse the prefix-owner tries that resolve it.
    !rust_path_heads.contains(effective) && !workspace_owner_names.contains(effective)
}

/// Look up `qn` in the callable index, following Rust `pub use`
/// re-export chains up to a small depth cap so malformed graphs can't
/// loop.
fn lookup_with_reexports(
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
        resolve(g, f, DEFAULT_FANOUT_CAP)
    }

    use super::*;
    use cgg_core::{
        DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
        graph::{CallableKind, CallableNode, FileRecord as GraphFileRecord},
    };
    use std::path::PathBuf;

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
            vec![RefRecord {
                name: "kick".into(),
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
}
