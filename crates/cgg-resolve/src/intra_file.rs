//! Intra-file linker.
//!
//! Ported from codescope's `get_containing_def_for_ref`, but byte-based
//! and strongly typed:
//!
//! * For every call site reference in a file, locate the **smallest**
//!   enclosing definition (the callable whose byte range tightly
//!   contains the ref's site byte).
//! * Match the ref's simple name against definitions in the same file.
//!   * **Exactly one match** → `CallEdge` with
//!     `confidence=high` / `resolver="intra-file"`.
//!   * **Zero matches** → `AuditUnresolvedCall` with
//!     `reason="no-candidate-in-scope"`.
//!   * **Two or more matches** → `AuditUnresolvedCall` with
//!     `reason="ambiguous-in-file"` (Task 6's scope-aware resolvers
//!     collapse these).
//!
//! Cycles — including self-calls (`fn f() { f(); }`) — are emitted
//! normally; no deduplication or cycle-breaking is performed.

use cgg_core::{
    DefRecord, DefVariant, FileFacts, RefRecord,
    audit::{AuditUnresolvedCall, UnresolvedReason},
    graph::{CallEdge, CallableKind, Confidence, Via},
    ids::{CallableId, FileId, ResolverId},
};

use crate::names::owner_from_qn;

/// Map from a file's definition index (matching `FileFacts.definitions`
/// order) to the final `CallableId` assigned when inserting into the
/// graph. Keyed outside this module by the driver.
pub type DefIdMap = std::collections::HashMap<(FileId, u32), CallableId>;

/// Outcome of linking a single file.
#[derive(Debug, Default)]
pub struct LinkOutcome {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<AuditUnresolvedCall>,
}

/// Languages where a bare `name(...)` can never reach an instance
/// method — there is no implicit-receiver call form, so the only things
/// in scope are module-level.
///
/// Deliberately an allowlist. Ruby's `foo` inside a class *is*
/// `self.foo`, and Java and C# resolve a bare name to an instance method
/// of the enclosing type, so filtering those would lose real edges.
fn bare_call_is_never_a_method(language: &str) -> bool {
    matches!(
        language,
        "python" | "rust" | "go" | "javascript" | "typescript" | "php"
    )
}

/// Whether a definition is a member of a type rather than a free
/// function.
/// A same-name candidate: its index in `definitions`, and the record.
type Candidate<'a> = (u32, &'a DefRecord);
/// A byte range, `start..end`.
type Span = (u32, u32);

fn is_method_variant(v: DefVariant) -> bool {
    matches!(
        v,
        DefVariant::InherentMethod
            | DefVariant::TraitMethod
            | DefVariant::TraitDefaultMethod
            | DefVariant::StaticMethod
            | DefVariant::ClassMethod
            | DefVariant::Constructor
            | DefVariant::Destructor
            | DefVariant::Property
    )
}

/// `async` replaces the scope-derived variant (`AsyncFunction` for an
/// `async fn` in an `impl` as much as for a free one), so the variant
/// alone cannot say "method". The qualified name can: an owner that is a
/// type — capitalised, or a `<T as Trait>` wrapper — makes it one.
fn is_async_method(d: &DefRecord) -> bool {
    if d.variant != DefVariant::AsyncFunction {
        return false;
    }
    let qn = d.qualified_name.as_str();
    let prefix = qn
        .strip_suffix(d.simple_name.as_str())
        .unwrap_or(qn)
        .trim_end_matches([':', '.']);
    if prefix.ends_with('>') {
        return true;
    }
    prefix
        .rsplit(['.', ':'])
        .next()
        .is_some_and(|owner| owner.starts_with(|c: char| c.is_uppercase()))
}

/// Run the intra-file linker over a single file.
///
/// `def_ids` must contain entries for every `(facts.file, idx)` pair
/// where `idx` is a valid index into `facts.definitions`. Overload sets
/// are bounded by the default fan-out cap; [`link_file_with_cap`] takes
/// the run's `--fanout-cap`.
pub fn link_file(facts: &FileFacts, def_ids: &DefIdMap) -> LinkOutcome {
    link_file_with_cap(facts, def_ids, crate::cross_file::DEFAULT_FANOUT_CAP)
}

/// [`link_file`] with an explicit fan-out cap for overload sets.
pub fn link_file_with_cap(
    facts: &FileFacts,
    def_ids: &DefIdMap,
    fanout_cap: usize,
) -> LinkOutcome {
    let mut out = LinkOutcome::default();
    let resolver_id = ResolverId::new("intra-file");

    for rref in &facts.references {
        let enclosing = enclosing_def_index(facts, rref);
        let src = enclosing.and_then(|i| def_ids.get(&(facts.file, i as u32)).copied());

        // Value-reference (Issue 4): `register(handler)` names `handler`
        // as a value. Resolve it by name to a same-file callable and emit
        // a `Via::Reference` edge; if it names no known callable, drop it
        // silently — value refs must never reach the unresolved/external
        // buckets.
        // String-reference (shape E): a literal that *names* a callable.
        // Suppression-and-entry-node only, by explicit contract — it
        // never becomes an edge, and it must never reach the unresolved
        // / external buckets either, where it would be reported as a
        // call into third-party code that does not exist.
        if rref.receiver_hint == cgg_core::STRING_REF_HINT {
            continue;
        }

        if rref.receiver_hint == cgg_core::VALUE_REF_HINT {
            let Some(src_id) = src else { continue };
            let matches: Vec<u32> = facts
                .definitions
                .iter()
                .enumerate()
                .filter(|(_, d)| d.simple_name == rref.name)
                .map(|(i, _)| i as u32)
                .collect();
            if let [cand_idx] = matches.as_slice() {
                let dst_def = &facts.definitions[*cand_idx as usize];
                // Anything but a closure bound to the same simple name is
                // a real target for a value-reference edge — a function,
                // a method, a constructor, a property. See the matching
                // check in `cross_file::resolve`.
                if dst_def.variant.to_callable_kind() != CallableKind::Closure {
                    let dst_id = def_ids[&(facts.file, *cand_idx)];
                    out.edges.push(CallEdge {
                        src: src_id,
                        dst: dst_id,
                        site_line: rref.site_line,
                        site_byte: rref.site_byte,
                        confidence: Confidence::Medium,
                        via: Via::Reference,
                        resolver: resolver_id.clone(),
                        weight: 1,
                    });
                }
            }
            continue;
        }

        // Candidate defs by simple-name match in this file.
        let mut candidates: Vec<(u32, &DefRecord)> = facts
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, d)| d.simple_name == rref.name)
            .map(|(i, d)| (i as u32, d))
            .collect();

        // A bare identifier is not a method call in these languages:
        // `helper(2)` inside a class reaches a module-level `helper`,
        // never `self.helper`. Without this a same-file method with a
        // colliding name captured the call — and, because same-file
        // resolution scores `high` and the cross-file import scores
        // `medium`, the *wrong* target outranked the right one. Only
        // languages with no implicit-self call form are listed; Ruby,
        // Java and C# do resolve a bare name to a method and must not
        // be filtered.
        let mut methods_out_of_scope = 0u32;
        if rref.receiver_hint.is_empty()
            && !rref.dot_call
            && bare_call_is_never_a_method(&facts.language)
        {
            let before = candidates.len();
            candidates
                .retain(|(_, d)| !is_method_variant(d.variant) && !is_async_method(d));
            methods_out_of_scope = (before - candidates.len()) as u32;
        }

        // Python lexical scope for a bare call. A function nested inside
        // another is visible by name only within it, and the innermost
        // definition shadows outer ones: `_scan()` inside the method that
        // defines `_scan` is that local, and `_scan()` anywhere else is
        // the module's. Without this, same-named nested definitions were
        // ambiguous or bound across scopes. Deliberately NOT applied to
        // `obj.name()` calls: a nested function escapes its scope when
        // passed as a value (`XMODEM(getc, putc)` then `self.getc()`),
        // and name lookup is the only thing that finds it there.
        if rref.receiver_hint.is_empty()
            && facts.language == "python"
            && candidates.len() > 1
        {
            let parent_span = |d: &DefRecord| -> Option<(u32, u32)> {
                facts
                    .definitions
                    .iter()
                    .filter(|p| {
                        p.start_byte <= d.start_byte
                            && d.end_byte <= p.end_byte
                            && (p.start_byte, p.end_byte) != (d.start_byte, d.end_byte)
                    })
                    .map(|p| (p.start_byte, p.end_byte))
                    .min_by_key(|(a, b)| b - a)
            };
            let site = rref.site_byte;
            let visible: Vec<(Candidate<'_>, Option<Span>)> = candidates
                .iter()
                .map(|c| (*c, parent_span(c.1)))
                .filter(|(_, parent)| parent.is_none_or(|(a, b)| a <= site && site < b))
                .collect();
            // Innermost enclosing scope first: the smallest parent span,
            // and module level (no parent) last.
            if let Some(best) = visible
                .iter()
                .map(|(_, p)| p.map_or(u32::MAX, |(a, b)| b - a))
                .min()
            {
                let narrowed: Vec<(u32, &DefRecord)> = visible
                    .iter()
                    .filter(|(_, p)| p.map_or(u32::MAX, |(a, b)| b - a) == best)
                    .map(|(c, _)| *c)
                    .collect();
                if !narrowed.is_empty() {
                    candidates = narrowed;
                }
            }
        }

        // Receiver-based narrowing (Issue 1). A call of the form
        // `Foo::bar()` or `obj.bar()` carries a receiver_hint that names
        // the owning type. We resolve it to a target owner and narrow:
        //
        //   * empty receiver            -> no narrowing.
        //   * `self`/`Self`/`cls`/`this`-> owner is the *enclosing*
        //                                  impl/class's owner type.
        //   * anything else             -> owner is the receiver hint.
        //
        // We first try a strict match (the candidate's *owner* segment —
        // the one right before the simple name — equals the target),
        // which alone disambiguates `Parser::new` vs `Cursor::new` and
        // `Self::new`. Only if that yields nothing do we fall back to
        // the looser "any path segment equals owner" match, which keeps
        // module-qualified forms reachable.
        let rh = rref.receiver_hint.as_str();
        // C++ unqualified name lookup inside a member function searches the
        // enclosing class before the enclosing namespace: `get_binary(...)`
        // in `binary_reader::get_bson_binary` is `binary_reader::get_binary`
        // even when a same-named function exists elsewhere in the file.
        // Narrow only when some candidate is such a member; otherwise the
        // set stands.
        if rh.is_empty() && facts.language == "cpp" && candidates.len() > 1 {
            if let Some(owner) = enclosing
                .and_then(|i| owner_from_qn(&facts.definitions[i].qualified_name))
            {
                let members: Vec<(u32, &DefRecord)> = candidates
                    .iter()
                    .copied()
                    .filter(|(_, d)| owner_from_qn(&d.qualified_name) == Some(owner))
                    .collect();
                if !members.is_empty() {
                    candidates = members;
                }
            }
        }
        let is_self = rh == "self" || rh == "Self" || rh == "cls" || rh == "this";
        if is_self {
            // `self`/`Self`-qualified call. The owner is the *enclosing
            // method's* type. We resolve it from the enclosing definition,
            // but that owner is **derived** (and wrong when the enclosing
            // def is a nested function whose owner segment is the outer
            // function, not the class), so we narrow ONLY on an exact
            // owner match and never fall back to the looser segment match —
            // a wrong derived owner must not drop otherwise-valid
            // candidates. When no candidate matches, we leave the set
            // unnarrowed (resolve by name), which is the historical
            // behavior for `self` receivers.
            if let Some(owner) = enclosing
                .and_then(|i| owner_from_qn(&facts.definitions[i].qualified_name))
            {
                let strict: Vec<(u32, &DefRecord)> = candidates
                    .iter()
                    .copied()
                    .filter(|(_, d)| owner_from_qn(&d.qualified_name) == Some(owner))
                    .collect();
                if !strict.is_empty() {
                    candidates = strict;
                }
            }
            // Two impls of one trait on one type (`PartialOrd` and
            // `PartialOrd<i32>`) both provide the method; inside one of
            // them, `self.partial_cmp(other)` is that impl's — its argument
            // types are the ones in scope. One-sided like the rest. A
            // prefix that is a path suffix of the other names the same
            // type: a C++ in-class declaration is `(anonymous)::Target::f`
            // while its out-of-line definition is `Target::f`, and
            // narrowing to one would drop the definition.
            if candidates.len() > 1
                && let Some(prefix) = enclosing.and_then(|i| {
                    crate::names::split_last_segment(&facts.definitions[i].qualified_name)
                        .map(|(p, _)| p)
                })
            {
                let same_impl: Vec<(u32, &DefRecord)> = candidates
                    .iter()
                    .copied()
                    .filter(|(_, d)| {
                        crate::names::split_last_segment(&d.qualified_name)
                            .is_some_and(|(p, _)| same_type_prefix(p, prefix))
                    })
                    .collect();
                if same_impl.len() == 1 {
                    candidates = same_impl;
                }
            }
        } else if !rh.is_empty() {
            // A C++ scope-qualified call (`std::to_string(i)`) names a
            // namespace or class path. A declaration the grammar mis-split
            // under a macro carries its return type in its qualified name
            // (`MACRO std::string to_string(…)` read as the name
            // `std::string to_string`), so its apparent owner, `std`, is
            // not a scope at all; such a candidate qualifies only when the
            // scope it really sits in (its name minus the return type the
            // signature shows) is the one the call names.
            let cpp_path = facts.language == "cpp" && rref.path_receiver;
            if cpp_path {
                candidates.retain(|(_, d)| {
                    is_scope_path_qn(d)
                        || misparsed_scope(d).is_some_and(|s| {
                            let s = without_anonymous(&format!("{s}::"));
                            s == format!("{rh}::") || s.ends_with(&format!("::{rh}::"))
                        })
                });
            }
            // Explicit qualifier names the owner. Try a strict owner match
            // first (disambiguates `Parser::new` vs `Cursor::new`); only
            // if that yields nothing fall back to the looser "any path
            // segment equals the qualifier" match, which keeps
            // module-qualified forms reachable.
            let strict: Vec<(u32, &DefRecord)> = candidates
                .iter()
                .copied()
                .filter(|(_, d)| owner_from_qn(&d.qualified_name) == Some(rh))
                .collect();
            if !strict.is_empty() {
                candidates = strict;
            } else if cpp_path {
                // `ns::f()` is `f` declared in the scope the call spells —
                // `a::b::f`, `ctype<char>::f` — never a member of a class
                // inside it (`ranges::begin` is not
                // `ranges::drop_view::begin`); the one deeper reading is a
                // constructor, `ns::Klass(…)` entering `ns::Klass::Klass`.
                let direct = format!("{rh}::{}", rref.name);
                let ctor = format!("{rh}::{}::{}", rref.name, rref.name);
                candidates.retain(|(_, d)| {
                    // A mis-split candidate already proved its real scope
                    // above; its qualified name ends in `RetType name`.
                    if !is_scope_path_qn(d) {
                        return true;
                    }
                    let qn = without_anonymous(&d.qualified_name);
                    [direct.as_str(), ctor.as_str()].iter().any(|path| {
                        qn == *path
                            || qn.strip_suffix(path).is_some_and(|p| p.ends_with("::"))
                    })
                });
            } else {
                candidates.retain(|(_, d)| {
                    d.qualified_name.split("::").any(|seg| seg == rh)
                        || d.qualified_name.split('.').any(|seg| seg == rh)
                });
            }
        }

        match candidates.as_slice() {
            [] => {
                // Say which kind of "no candidate" this is. A bare call
                // whose only same-name definitions are methods is not a
                // name cgg has never seen — it is a name whose target is
                // somewhere else, and reporting it as though the name
                // does not exist is what P1-2 of the field report was
                // about.
                let reason = if methods_out_of_scope > 0 {
                    UnresolvedReason::NotInScopeForBareCall {
                        methods: methods_out_of_scope,
                    }
                } else {
                    UnresolvedReason::NoCandidateInFile
                };
                out.unresolved.push(AuditUnresolvedCall::new(
                    src,
                    facts.file,
                    rref.site_line,
                    rref.site_byte,
                    rref.name.clone(),
                    rref.receiver_hint.clone(),
                    reason,
                ));
            }
            [(cand_idx, _)] => {
                let Some(src_id) = src else {
                    // No enclosing callable (e.g. ref at module top
                    // level) — the edge has no source; record it as
                    // unresolved with a specific reason so Task 6's
                    // resolver can pick it up.
                    out.unresolved.push(AuditUnresolvedCall::new(
                        None,
                        facts.file,
                        rref.site_line,
                        rref.site_byte,
                        rref.name.clone(),
                        rref.receiver_hint.clone(),
                        UnresolvedReason::NoEnclosingCallable,
                    ));
                    continue;
                };
                let dst_id = def_ids[&(facts.file, *cand_idx)];
                out.edges.push(CallEdge {
                    src: src_id,
                    dst: dst_id,
                    site_line: rref.site_line,
                    site_byte: rref.site_byte,
                    confidence: Confidence::High,
                    via: Via::Direct,
                    resolver: resolver_id.clone(),
                    weight: 1,
                });
            }
            // Every candidate carries one qualified name: an overload set
            // (`matrix4::element(int, int)` and its `const` twin, Java's
            // `add(int)` / `add(String)`). The callee is known; only the
            // overload is not, so the call binds to each at Medium rather
            // than being dropped as ambiguous. Bounded by the fan-out cap
            // like any other guess, so `--fanout-cap 0` still never guesses.
            [(_, first), rest @ ..]
                if src.is_some()
                    && binds_same_name_set(&facts.language)
                    && rest
                        .iter()
                        .all(|(_, d)| d.qualified_name == first.qualified_name) =>
            {
                let (targets, confidence) = if is_clause_language(&facts.language) {
                    clause_targets(&facts.language, &candidates, rref.arity)
                } else {
                    (
                        candidates.iter().map(|(i, _)| *i).collect::<Vec<u32>>(),
                        Confidence::Medium,
                    )
                };
                // One certain callee is not a guess; the cap bounds guesses.
                if confidence != Confidence::High && targets.len() > fanout_cap {
                    out.unresolved.push(AuditUnresolvedCall::new(
                        src,
                        facts.file,
                        rref.site_line,
                        rref.site_byte,
                        rref.name.clone(),
                        rref.receiver_hint.clone(),
                        UnresolvedReason::FanoutCapExceeded {
                            candidates: targets.len() as u32,
                        },
                    ));
                    continue;
                }
                let src_id = src.expect("guarded above");
                for cand_idx in targets {
                    out.edges.push(CallEdge {
                        src: src_id,
                        dst: def_ids[&(facts.file, cand_idx)],
                        site_line: rref.site_line,
                        site_byte: rref.site_byte,
                        confidence,
                        via: Via::Direct,
                        resolver: resolver_id.clone(),
                        weight: 1,
                    });
                }
            }
            _ => {
                let mut rec = AuditUnresolvedCall::new(
                    src,
                    facts.file,
                    rref.site_line,
                    rref.site_byte,
                    rref.name.clone(),
                    rref.receiver_hint.clone(),
                    UnresolvedReason::AmbiguousInFile,
                );
                rec.candidates.file_local = candidates.len() as u32;
                out.unresolved.push(rec);
            }
        }
    }

    out
}

/// A qualified name that is a scope path ending in the definition's own
/// name (`ns::Type::f`, `f`), as opposed to one whose last segment is not
/// the name at all (`std::string to_string`, a return type swallowed into
/// the declarator).
fn is_scope_path_qn(d: &DefRecord) -> bool {
    let (qn, name) = (d.qualified_name.as_str(), d.simple_name.as_str());
    qn == name || qn.strip_suffix(name).is_some_and(|p| p.ends_with("::"))
}

/// For a qualified name built from a mis-split declarator — `scope::` +
/// `RetType name` — the scope the definition really sits in: the longest
/// prefix whose remainder (`RetType name`) is what the signature shows,
/// starting at a token boundary, right before `name(`.
/// `__libdispatch::__chunk_partitions __partition_chunks` with signature
/// `… __chunk_partitions __partition_chunks(…)` sits in `__libdispatch`;
/// `std::string to_string` with signature `… std::string to_string(…)`
/// sits in the global scope (`string to_string` follows `::` there, not a
/// token boundary). `None` when it cannot be read back.
fn misparsed_scope(d: &DefRecord) -> Option<&str> {
    let (qn, name) = (d.qualified_name.as_str(), d.simple_name.as_str());
    if !qn.ends_with(name) || is_scope_path_qn(d) {
        return None;
    }
    let collapse = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let sig = collapse(&d.signature_hint);
    let declarator_in_sig = |decl: &str| {
        let decl = collapse(decl);
        let needle = format!("{decl}(");
        decl.contains(' ')
            && sig.match_indices(&needle).any(|(at, _)| {
                sig[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|c| c.is_whitespace())
            })
    };
    qn.match_indices("::")
        .map(|(i, _)| (&qn[..i], &qn[i + 2..]))
        .chain(std::iter::once(("", qn)))
        .find(|(_, decl)| declarator_in_sig(decl))
        .map(|(scope, _)| scope)
}

/// `a::(anonymous)::b` → `a::b`: an unnamed namespace's members are found
/// by qualified lookup through the enclosing one.
fn without_anonymous(qn: &str) -> String {
    qn.replace("(anonymous)::", "")
}

/// Whether two owner prefixes name the same type: equal, or equal once
/// `(anonymous)` segments and a trailing type-argument list are dropped
/// and one is a path suffix of the other (`(anonymous)::Target` and
/// `Target`; `cm::PathResolver::(anonymous)::Impl` and
/// `cm::PathResolver::Impl<Policy>`). A Rust `<A as Trait<X>>` segment
/// keeps its arguments — they are what tells two impls apart.
fn same_type_prefix(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if a.contains(" as ") || b.contains(" as ") {
        return false;
    }
    fn norm(p: &str) -> Vec<&str> {
        let mut segs: Vec<&str> = p
            .split("::")
            .flat_map(|s| s.split('.'))
            .filter(|s| *s != "(anonymous)")
            .collect();
        if let Some(last) = segs.last_mut()
            && !last.starts_with('<')
            && let Some(i) = last.find('<')
        {
            *last = &last[..i];
        }
        segs
    }
    let (a, b) = (norm(a), norm(b));
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    !short.is_empty() && long.ends_with(&short)
}

/// Return the index of the smallest definition whose byte range
/// contains `rref.site_byte`. Ties are broken by the smaller byte
/// span — matching codescope's "smallest-enclosing" rule.
fn enclosing_def_index(facts: &FileFacts, rref: &RefRecord) -> Option<usize> {
    let b = rref.site_byte;
    let mut best: Option<(usize, u32)> = None;
    for (i, d) in facts.definitions.iter().enumerate() {
        if d.start_byte <= b && b < d.end_byte {
            let span = d.end_byte - d.start_byte;
            match best {
                None => best = Some((i, span)),
                Some((_, bspan)) if span < bspan => best = Some((i, span)),
                _ => {}
            }
        }
    }
    best.map(|(i, _)| i)
}

/// Languages where two same-file definitions sharing a qualified name are
/// one callee: overloads (Java, C++, C#, Kotlin, Scala, Swift, Groovy,
/// F#, Solidity, TypeScript overload signatures), multiple-dispatch
/// methods (Julia), or clauses (see [`is_clause_language`]). Everywhere
/// else a repeated name is a different function in another scope or a
/// redefinition — `lookahead1W` twice in one minified bundle, a Python
/// `def` in both arms of an `if` — and binding all of them would be a
/// guess dressed as a fact, so those stay `ambiguous-in-file`.
fn binds_same_name_set(language: &str) -> bool {
    is_clause_language(language)
        || matches!(
            language,
            "java"
                | "cpp"
                | "csharp"
                | "kotlin"
                | "scala"
                | "swift"
                | "groovy"
                | "fsharp"
                | "solidity"
                | "typescript"
                | "julia"
        )
}

/// Languages that define one function as several clauses, each extracted
/// as its own definition under one qualified name. Same-name candidates
/// there are not overloads to guess between: they are one function
/// (Haskell equations), or one function per arity (Erlang/Elixir `f/1`
/// and `f/2`, which the call's argument count tells apart).
fn is_clause_language(language: &str) -> bool {
    matches!(language, "erlang" | "elixir" | "haskell")
}

/// The clauses a call in a clause language binds to: the first clause of
/// each function it can reach. Erlang/Elixir candidates are narrowed to
/// the call's arity when both are known; if that leaves nothing, the
/// arities are unknowable here and every function stays. One function
/// left is a certain callee — `High`; several arities left is a guess.
fn clause_targets(
    language: &str,
    candidates: &[(u32, &DefRecord)],
    arity: Option<u32>,
) -> (Vec<u32>, Confidence) {
    let arity_of = |d: &DefRecord| -> Option<u32> {
        if language == "haskell" {
            // Every equation of a Haskell function has the same arity.
            return Some(0);
        }
        clause_arity(&d.signature_hint, &d.simple_name)
    };
    let mut pool: Vec<(u32, &DefRecord)> = candidates.to_vec();
    if let Some(a) = arity {
        let narrowed: Vec<(u32, &DefRecord)> = pool
            .iter()
            .copied()
            .filter(|(_, d)| arity_of(d) == Some(a))
            .collect();
        if !narrowed.is_empty() {
            pool = narrowed;
        }
    }
    // First clause (lowest byte) per arity; `None` arities group together.
    let mut firsts: Vec<(Option<u32>, u32, u32)> = Vec::new();
    for (idx, d) in &pool {
        let a = arity_of(d);
        match firsts.iter_mut().find(|(fa, _, _)| *fa == a) {
            Some(slot) if d.start_byte < slot.2 => {
                slot.1 = *idx;
                slot.2 = d.start_byte;
            }
            Some(_) => {}
            None => firsts.push((a, *idx, d.start_byte)),
        }
    }
    firsts.sort_by_key(|(_, _, b)| *b);
    let confidence = if firsts.len() == 1 {
        Confidence::High
    } else {
        Confidence::Medium
    };
    (firsts.into_iter().map(|(_, i, _)| i).collect(), confidence)
}

/// The arity a clause's signature declares: `f(A, B) -> …` is 2, Elixir's
/// paren-less `def g, do: …` is 0. `None` when the hint does not show it.
fn clause_arity(hint: &str, name: &str) -> Option<u32> {
    let ident = |c: char| c.is_alphanumeric() || c == '_' || c == '?' || c == '!';
    // The name as a whole word: `f` in `def f(a)` is not the `f` of `def`.
    let at = hint.match_indices(name).map(|(i, _)| i).find(|&i| {
        !hint[..i].chars().next_back().is_some_and(ident)
            && !hint[i + name.len()..]
                .chars()
                .next()
                .is_some_and(|c| ident(c) && c != '?' && c != '!')
    })? + name.len();
    let rest = &hint[at..];
    let Some(body) = rest.strip_prefix('(') else {
        return (!name.is_empty()).then_some(0);
    };
    let (mut depth, mut commas, mut any) = (1u32, 0u32, false);
    for c in body.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(if any { commas + 1 } else { 0 });
                }
            }
            ',' if depth == 1 => commas += 1,
            c if !c.is_whitespace() => any = true,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::{DefRecord, DefVariant, FileFacts, RefRecord};
    use std::path::PathBuf;

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
            attributes: Vec::new(),
            ..Default::default()
        }
    }

    fn mk_ref(name: &str, site_byte: u32) -> RefRecord {
        RefRecord {
            name: name.into(),
            receiver_hint: String::new(),
            site_line: 1,
            site_byte,
            ..Default::default()
        }
    }

    fn facts_with(defs: Vec<DefRecord>, refs: Vec<RefRecord>) -> FileFacts {
        facts_with_lang("rust", defs, refs)
    }

    fn facts_with_lang(
        lang: &str,
        defs: Vec<DefRecord>,
        refs: Vec<RefRecord>,
    ) -> FileFacts {
        FileFacts {
            file: FileId::new(0),
            path: PathBuf::from("t.rs"),
            language: lang.into(),
            definitions: defs,
            references: refs,
            imports: Vec::new(),
            local_types: Vec::new(),
            ..Default::default()
        }
    }

    fn mk_map(facts: &FileFacts) -> DefIdMap {
        facts
            .definitions
            .iter()
            .enumerate()
            .map(|(i, _)| ((facts.file, i as u32), CallableId::new(i as u32)))
            .collect()
    }

    #[test]
    fn single_match_emits_edge() {
        // def foo at bytes 0..100, def bar at bytes 100..200
        // ref at byte 50 (inside foo) calls "bar"
        let defs = vec![
            mk_def("foo", "m::foo", DefVariant::FreeFunction, (0, 100)),
            mk_def("bar", "m::bar", DefVariant::FreeFunction, (100, 200)),
        ];
        let refs = vec![mk_ref("bar", 50)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.unresolved.len(), 0);
        let e = &out.edges[0];
        assert_eq!(e.src, CallableId::new(0));
        assert_eq!(e.dst, CallableId::new(1));
        assert_eq!(e.confidence, Confidence::High);
        assert_eq!(e.resolver.as_str(), "intra-file");
    }

    // Two `PartialOrd` impls on one type: inside one of them,
    // `self.partial_cmp(..)` is that impl's own method.
    #[test]
    fn self_call_prefers_the_callers_own_impl_block() {
        let a = "m::<T as PartialOrd>";
        let b = "m::<T as PartialOrd<i32>>";
        let defs = vec![
            mk_def(
                "partial_cmp",
                &format!("{a}::partial_cmp"),
                DefVariant::TraitMethod,
                (0, 100),
            ),
            mk_def(
                "partial_cmp",
                &format!("{b}::partial_cmp"),
                DefVariant::TraitMethod,
                (100, 200),
            ),
            mk_def(
                "gt",
                &format!("{b}::gt"),
                DefVariant::TraitMethod,
                (200, 300),
            ),
        ];
        let mut r = mk_ref("partial_cmp", 250);
        r.receiver_hint = "self".into();
        let facts = facts_with(defs, vec![r]);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 1, "{:?}", out.edges);
        assert_eq!(out.edges[0].dst, map[&(facts.file, 1)]);
    }

    // C++: the in-class declaration is `(anonymous)::Target::f`, the
    // out-of-line definition `Target::f`. Same type — the own-block
    // tie-break must not drop the definition (cmake `DumpDebugger`).
    #[test]
    fn self_call_keeps_declaration_and_out_of_line_definition() {
        let defs = vec![
            mk_def(
                "f",
                "(anonymous)::Target::f",
                DefVariant::InherentMethod,
                (0, 10),
            ),
            mk_def(
                "dump",
                "(anonymous)::Target::dump",
                DefVariant::InherentMethod,
                (10, 100),
            ),
            mk_def("f", "Target::f", DefVariant::InherentMethod, (100, 200)),
        ];
        let mut r = mk_ref("f", 50);
        r.receiver_hint = "this".into();
        let facts = facts_with(defs, vec![r]);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        // Not narrowed to the declaration alone: both survive (and with
        // two candidates the site is left to the cross-file pass).
        assert!(out.edges.is_empty(), "{:?}", out.edges);
        assert!(same_type_prefix(
            "cm::PathResolver::(anonymous)::Impl",
            "cm::PathResolver::Impl<Policy>"
        ));
        assert!(!same_type_prefix(
            "m::<T as PartialOrd>",
            "m::<T as PartialOrd<i32>>"
        ));
        assert!(!same_type_prefix("a.Outer.Builder", "a.Other.Builder"));
    }

    #[test]
    fn zero_candidates_unresolved_no_candidate() {
        let defs = vec![mk_def("foo", "m::foo", DefVariant::FreeFunction, (0, 100))];
        let refs = vec![mk_ref("baz", 50)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 0);
        assert_eq!(out.unresolved.len(), 1);
        assert_eq!(
            out.unresolved[0].reason,
            UnresolvedReason::NoCandidateInFile
        );
        assert_eq!(out.unresolved[0].name, "baz");
    }

    #[test]
    fn an_overload_set_binds_every_overload_at_medium() {
        // `matrix4::element(int, int)` and its `const` twin: one callee,
        // unknown overload. Dropping it as ambiguous lost every call.
        let defs = vec![
            mk_def("caller", "m::caller", DefVariant::FreeFunction, (0, 50)),
            mk_def(
                "element",
                "matrix4::element",
                DefVariant::InherentMethod,
                (50, 80),
            ),
            mk_def(
                "element",
                "matrix4::element",
                DefVariant::InherentMethod,
                (80, 110),
            ),
        ];
        let facts = facts_with_lang("cpp", defs, vec![mk_ref("element", 10)]);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert!(out.unresolved.is_empty(), "{:?}", out.unresolved);
        let dsts: Vec<_> = out.edges.iter().map(|e| e.dst).collect();
        assert_eq!(dsts, vec![CallableId::new(1), CallableId::new(2)]);
        assert!(out.edges.iter().all(|e| e.confidence == Confidence::Medium));
        // `--fanout-cap 0` means never guess, overloads included.
        let capped = link_file_with_cap(&facts, &map, 0);
        assert!(capped.edges.is_empty());
        assert_eq!(
            capped.unresolved[0].reason,
            UnresolvedReason::FanoutCapExceeded { candidates: 2 }
        );
    }

    #[test]
    fn erlang_clauses_bind_once_to_the_function_of_the_calls_arity() {
        // `f(0) -> …; f(N) -> …` is one function `f/1`; `f(A, B)` is a
        // different function `f/2`. `f(1)` reaches the first clause of
        // `f/1`, certainly, and nothing of `f/2`.
        let clause = |sig: &str, span: (u32, u32)| {
            let mut d = mk_def("f", "m:f", DefVariant::FreeFunction, span);
            d.signature_hint = sig.to_string();
            d
        };
        let defs = vec![
            mk_def("g", "m:g", DefVariant::FreeFunction, (0, 50)),
            clause("f(0) -> zero", (50, 70)),
            clause("f(N) -> N", (70, 90)),
            clause("f(A, B) -> A + B", (90, 110)),
        ];
        let mut r = mk_ref("f", 10);
        r.arity = Some(1);
        let facts = facts_with_lang("erlang", defs, vec![r]);
        let map = mk_map(&facts);
        let out = link_file_with_cap(&facts, &map, 0);
        assert!(out.unresolved.is_empty(), "{:?}", out.unresolved);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].dst, CallableId::new(1));
        assert_eq!(out.edges[0].confidence, Confidence::High);
    }

    #[test]
    fn clause_arity_reads_the_signature() {
        assert_eq!(clause_arity("f(A, {B, C}) -> x", "f"), Some(2));
        assert_eq!(clause_arity("g() -> f(1)", "g"), Some(0));
        assert_eq!(clause_arity("def g, do: f(1)", "g"), Some(0));
        assert_eq!(clause_arity("def f(a), do", "f"), Some(1));
    }

    #[test]
    fn a_repeated_name_in_a_language_without_overloading_stays_ambiguous() {
        // Two `lookahead1W` in one minified bundle are two scopes, not
        // an overload set.
        let defs = vec![
            mk_def("caller", "caller", DefVariant::FreeFunction, (0, 50)),
            mk_def(
                "lookahead1W",
                "lookahead1W",
                DefVariant::FreeFunction,
                (50, 80),
            ),
            mk_def(
                "lookahead1W",
                "lookahead1W",
                DefVariant::FreeFunction,
                (80, 110),
            ),
        ];
        let facts = facts_with_lang("javascript", defs, vec![mk_ref("lookahead1W", 10)]);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert!(out.edges.is_empty());
        assert_eq!(out.unresolved[0].reason, UnresolvedReason::AmbiguousInFile);
    }

    #[test]
    fn ambiguous_name_flags_unresolved() {
        // Two defs named `m` — common in Rust where `impl A { fn m } impl B { fn m }`.
        // Ruby, where a bare name *is* an implicit-self method call, so
        // both candidates are genuinely in scope and the ambiguity is
        // real. In Rust the same source resolves to neither — see
        // `a_bare_call_does_not_reach_a_method`.
        let defs = vec![
            mk_def("caller", "m::caller", DefVariant::FreeFunction, (0, 50)),
            mk_def("m", "m::A::m", DefVariant::InherentMethod, (50, 80)),
            mk_def("m", "m::B::m", DefVariant::InherentMethod, (80, 110)),
        ];
        let facts = facts_with_lang("ruby", defs, vec![mk_ref("m", 10)]);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 0);
        assert_eq!(out.unresolved.len(), 1);
        assert_eq!(out.unresolved[0].reason, UnresolvedReason::AmbiguousInFile);
        // Evidence is recorded for the regression instrument (Issue 9).
        assert_eq!(out.unresolved[0].candidates.file_local, 2);
    }

    /// A bare identifier does not reach a method, and says so.
    ///
    /// `helper(2)` inside a file that also defines `Holder.helper` was
    /// resolving to the method — at `high`, because same-file resolution
    /// outranks the cross-file import that is the real target. The
    /// reason it now records matters as much as the dropped edge:
    /// `no-candidate-in-file` reads as "this name does not exist", which
    /// is the opposite of the truth.
    #[test]
    fn a_bare_call_does_not_reach_a_method() {
        let defs = vec![
            mk_def("caller", "m::caller", DefVariant::FreeFunction, (0, 50)),
            mk_def(
                "helper",
                "m::Holder::helper",
                DefVariant::InherentMethod,
                (50, 80),
            ),
        ];
        let facts = facts_with(defs, vec![mk_ref("helper", 10)]);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(
            out.edges.len(),
            0,
            "a method is not in scope for a bare call"
        );
        assert_eq!(
            out.unresolved[0].reason,
            UnresolvedReason::NotInScopeForBareCall { methods: 1 }
        );
    }

    #[test]
    fn owner_qualifier_disambiguates_same_name() {
        // Both `Parser::new` and `Cursor::new` exist; `Parser::new()`
        // names the owner, so exactly one candidate must win (Issue 1).
        let defs = vec![
            mk_def("build", "m::build", DefVariant::FreeFunction, (0, 50)),
            mk_def("new", "m::Parser::new", DefVariant::Constructor, (50, 80)),
            mk_def("new", "m::Cursor::new", DefVariant::Constructor, (80, 110)),
        ];
        let refs = vec![RefRecord {
            name: "new".into(),
            receiver_hint: "Parser".into(),
            site_line: 1,
            site_byte: 10,
            ..Default::default()
        }];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.unresolved.len(), 0);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].src, CallableId::new(0)); // m::build
        assert_eq!(out.edges[0].dst, CallableId::new(1)); // m::Parser::new
    }

    #[test]
    fn self_qualifier_resolves_to_enclosing_owner() {
        // Inside `Widget::build`, `Self::new()` must bind to
        // `Widget::new`, not the same-named `Gadget::new` (Issue 1).
        let defs = vec![
            mk_def(
                "build",
                "m::Widget::build",
                DefVariant::InherentMethod,
                (0, 50),
            ),
            mk_def("new", "m::Widget::new", DefVariant::Constructor, (50, 80)),
            mk_def("new", "m::Gadget::new", DefVariant::Constructor, (80, 110)),
        ];
        let refs = vec![RefRecord {
            name: "new".into(),
            receiver_hint: "Self".into(),
            site_line: 1,
            site_byte: 10,
            ..Default::default()
        }];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.unresolved.len(), 0);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].src, CallableId::new(0)); // Widget::build
        assert_eq!(out.edges[0].dst, CallableId::new(1)); // Widget::new
    }

    #[test]
    fn smallest_enclosing_wins() {
        // Outer fn at 0..200, inner named closure at 10..50, ref at byte 20.
        // The ref should be attributed to the inner closure (smallest
        // enclosing), not the outer function.
        let defs = vec![
            mk_def("outer", "m::outer", DefVariant::FreeFunction, (0, 200)),
            mk_def(
                "inner",
                "m::outer::inner",
                DefVariant::NamedClosure,
                (10, 50),
            ),
            mk_def("target", "m::target", DefVariant::FreeFunction, (200, 300)),
        ];
        let refs = vec![mk_ref("target", 20)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 1);
        // src must be the closure (id 1), not the outer function (id 0).
        assert_eq!(out.edges[0].src, CallableId::new(1));
        assert_eq!(out.edges[0].dst, CallableId::new(2));
    }

    #[test]
    fn self_call_is_preserved_as_edge() {
        // Recursion: `fn f() { f(); }`
        let defs = vec![mk_def("f", "m::f", DefVariant::FreeFunction, (0, 100))];
        let refs = vec![mk_ref("f", 20)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 1);
        assert_eq!(out.edges[0].src, out.edges[0].dst);
    }

    #[test]
    fn ref_outside_any_def_is_unresolved() {
        // Top-level statement (not inside any callable).
        let defs = vec![mk_def(
            "foo",
            "m::foo",
            DefVariant::FreeFunction,
            (100, 200),
        )];
        let refs = vec![mk_ref("foo", 10)];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(out.edges.len(), 0);
        assert_eq!(out.unresolved.len(), 1);
        assert_eq!(
            out.unresolved[0].reason,
            UnresolvedReason::NoEnclosingCallable
        );
    }

    /// A value ref (`VALUE_REF_HINT`) that uniquely names a same-file
    /// *closure* must not become a `Via::Reference` edge — only a free
    /// function or method is a real target. Mirrors
    /// `cross_file::tests::value_ref_to_a_closure_emits_no_edge`.
    #[test]
    fn value_ref_to_a_same_file_closure_emits_no_edge() {
        let defs = vec![
            mk_def("caller", "m::caller", DefVariant::FreeFunction, (0, 100)),
            mk_def(
                "on_click",
                "m::on_click",
                DefVariant::NamedClosure,
                (100, 200),
            ),
        ];
        let refs = vec![RefRecord {
            name: "on_click".into(),
            receiver_hint: cgg_core::VALUE_REF_HINT.into(),
            site_line: 1,
            site_byte: 50,
            ..Default::default()
        }];
        let facts = facts_with(defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        assert_eq!(
            out.edges.len(),
            0,
            "a value ref uniquely naming a closure must not become an edge"
        );
        assert_eq!(
            out.unresolved.len(),
            0,
            "value refs must never reach the unresolved bucket"
        );
    }

    /// `std::to_string(i)` must not bind to a declaration the grammar
    /// mis-split under a macro into the qualified name `std::string
    /// to_string`; a real `namespace std { void swap(..) }` still binds.
    #[test]
    fn cpp_scope_qualified_call_needs_a_scope_path_target() {
        let defs = vec![
            mk_def("caller", "caller", DefVariant::FreeFunction, (0, 100)),
            mk_def(
                "to_string",
                "std::string to_string",
                DefVariant::FreeFunction,
                (100, 200),
            ),
            mk_def("swap", "std::swap", DefVariant::FreeFunction, (200, 300)),
        ];
        let call = |name: &str| RefRecord {
            name: name.into(),
            receiver_hint: "std".into(),
            site_line: 1,
            site_byte: 50,
            path_receiver: true,
            ..Default::default()
        };
        let facts = facts_with_lang("cpp", defs, vec![call("to_string"), call("swap")]);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        let dsts: Vec<_> = out.edges.iter().map(|e| e.dst).collect();
        assert_eq!(
            dsts,
            vec![map[&(FileId::new(0), 2)]],
            "only std::swap binds"
        );
    }

    /// The mis-split shape with the scope in front of the return type
    /// (`ns::RetType name`) still binds a call qualified by that scope; a
    /// member of a class inside the scope does not, a constructor does.
    #[test]
    fn cpp_scope_qualified_call_reads_the_real_scope() {
        let mut part = mk_def(
            "partition",
            "pd::chunks partition",
            DefVariant::FreeFunction,
            (100, 200),
        );
        part.signature_hint = "EXPORT chunks partition(long n)".into();
        let defs = vec![
            mk_def("caller", "caller", DefVariant::FreeFunction, (0, 100)),
            part,
            mk_def(
                "begin",
                "ranges::drop_view::begin",
                DefVariant::InherentMethod,
                (200, 300),
            ),
            mk_def(
                "Error",
                "dap::Error::Error",
                DefVariant::Constructor,
                (300, 400),
            ),
            mk_def(
                "table",
                "std::ctype<char>::table",
                DefVariant::StaticMethod,
                (400, 500),
            ),
        ];
        let call = |name: &str, recv: &str, byte: u32| RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            site_line: 1,
            site_byte: byte,
            path_receiver: true,
            ..Default::default()
        };
        let refs = vec![
            call("partition", "pd", 10),
            call("begin", "ranges", 20),
            call("Error", "dap", 30),
            call("table", "ctype<char>", 40),
        ];
        let facts = facts_with_lang("cpp", defs, refs);
        let map = mk_map(&facts);
        let out = link_file(&facts, &map);
        let dsts: Vec<_> = out.edges.iter().map(|e| e.dst).collect();
        assert_eq!(
            dsts,
            vec![
                map[&(FileId::new(0), 1)],
                map[&(FileId::new(0), 3)],
                map[&(FileId::new(0), 4)]
            ],
            "pd::partition, the dap::Error constructor and ctype<char>::table \
             bind; ranges::begin does not"
        );
    }

    #[test]
    fn misparsed_scope_reads_the_scope_in_front_of_the_return_type() {
        let def = |qn: &str, name: &str, sig: &str| {
            let mut d = mk_def(name, qn, DefVariant::FreeFunction, (0, 1));
            d.signature_hint = sig.into();
            d
        };
        let cases = [
            (
                "std::string to_string",
                "to_string",
                "TPL std::string to_string(const J& j)",
                Some(""),
            ),
            (
                "__libdispatch::__chunk_partitions __partition_chunks",
                "__partition_chunks",
                "EXPORT __chunk_partitions __partition_chunks(long n) noexcept",
                Some("__libdispatch"),
            ),
            (
                "cpp::enable_if_t<is_big_int_v<T>, int>\ncountr_zero",
                "countr_zero",
                "LIBC_INLINE constexpr cpp::enable_if_t<is_big_int_v<T>, int> countr_zero(T v)",
                Some(""),
            ),
            (
                "__merge::merge_result<A,\n B> __merge_impl",
                "__merge_impl",
                "inline merge_result<A, B> __merge_impl(I f)",
                Some("__merge"),
            ),
            ("ns::f", "f", "void f()", None),
        ];
        for (qn, name, sig, want) in cases {
            assert_eq!(misparsed_scope(&def(qn, name, sig)), want, "{qn}");
        }
    }
}
