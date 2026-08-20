//! Content-derived, stable id allocation.
//!
//! Replaces the old per-run sequential `u32` counters. An id here is a
//! blake3 hash of a node's *identity* — a file's relative path, or a
//! callable's `(language, file, owner, qualified_name)` tuple — so the
//! same node gets the same id on every run, and adding or removing an
//! unrelated node elsewhere in the tree never perturbs it.
//!
//! # Collision handling
//!
//! The default candidate id is the first 52 bits of the identity's blake3
//! hash. On the rare collision (another node already claimed that exact
//! 52-bit value), we escalate by pulling *more bits from the same
//! blake3 output* via `finalize_xof()` — first 64 bits, then 96, doubling
//! until a free slot is found.
//!
//! This escalation is a pure function of the colliding item's own hash
//! stream. It never compares to, or depends on the identity of, whichever
//! other node it collided with — no lexicographic tie-break, no
//! "insertion order decides who keeps the short form". That property is
//! what makes ids stable under unrelated edits: if node A collided with
//! node B in one run, and a later run adds/removes some unrelated node C
//! before A or B is ever hashed, A and B still escalate exactly as they
//! did before, because nothing about their own escalation reads the
//! *other* colliding party, only their own extended hash stream and the
//! current `seen` set. Compare that to a scheme that says "whichever
//! sorts first keeps the short id" — that tie-break's outcome depends on
//! which nodes exist at all, so an unrelated addition could flip who
//! gets escalated.
use std::collections::HashSet;

/// How many bits of the blake3 hash are used for the initial candidate
/// id. 52 bits leaves headroom under `u64` for the escalation doubling
/// (52 -> 64 is still one u64) while giving a huge birthday-bound margin
/// for realistic corpus sizes (millions of nodes).
const DEFAULT_BITS: u32 = 52;

/// Mask hashing output down to `bits` bits (never more than 64).
fn mask_to_bits(v: u64, bits: u32) -> u64 {
    if bits >= 64 {
        v
    } else {
        v & ((1u64 << bits) - 1)
    }
}

/// Pull `bits` bits (<=64) out of a blake3 XOF stream, reading enough
/// bytes to cover them.
fn xof_bits(xof: &mut blake3::OutputReader, bits: u32) -> u64 {
    let nbytes = (bits as usize).div_ceil(8).clamp(1, 8);
    let mut buf = [0u8; 8];
    xof.fill(&mut buf[..nbytes]);
    let v = u64::from_le_bytes(buf);
    mask_to_bits(v, bits)
}

/// Given a hasher already fed with the identity bytes, find a value not
/// already in `seen`, drawing successive windows from the item's own
/// blake3 XOF stream. Inserts the chosen value into `seen` before
/// returning it.
///
/// Every draw is the same width, so **an id never exceeds 52 bits**.
/// That is deliberate: `cgg-node` binds ids as N-API `i64`, which
/// reaches JavaScript as a `number`, and a `number` only represents
/// integers exactly up to 2^53-1. An earlier revision widened to 64
/// bits on collision, which put every escalated id past that bound and
/// silently rounded it, so Node reported a different id than the CLI
/// for the same callable. Staying inside 52 bits keeps all four front
/// ends agreeing by construction.
///
/// Redrawing is a pure function of this item's own hash stream. It
/// never reads, compares to, or depends on the identity of whichever
/// other node it collided with — no lexicographic tie-break, no
/// "whoever sorts first keeps the short form". That is what keeps ids
/// stable under unrelated edits: a scheme that tie-breaks by comparing
/// the two colliding parties would flip its answer whenever the set of
/// nodes changed.
fn allocate(hasher: blake3::Hasher, seen: &mut HashSet<u64>) -> u64 {
    let mut xof = hasher.finalize_xof();
    let mut guard = 0u32;
    loop {
        let candidate = xof_bits(&mut xof, DEFAULT_BITS);
        if seen.insert(candidate) {
            return candidate;
        }
        // Reached only when two callables are genuinely
        // indistinguishable to cgg — same file, same qualified name,
        // same signature — or on a true 52-bit hash collision, which
        // is vanishingly rare. Either way the next window is a fresh,
        // deterministic draw.
        guard += 1;
        assert!(
            guard < 1_000_000,
            "stable id allocator exhausted its hash stream without finding a \
             free slot; this should be statistically impossible and indicates a bug"
        );
    }
}

/// The path string an id is keyed on: the file's location **relative to
/// the analysis root**, not the path as it was typed.
///
/// This is load-bearing. `cgg-walk` reports each file under the
/// `display_root` it was given, so the raw path is whatever the caller
/// wrote — and hashing that made an id depend on the invocation rather
/// than on the code. The same file, same content, same name produced
/// four different ids:
///
/// ```text
/// cgg /tmp/smoke2   Ck1v6lk3phz
/// cgg smoke2        Cf83zjoounv
/// cgg .             Cn10c7f0lc7
/// cgg t.py          Cvgb78ftyly
/// ```
///
/// which defeats the point of content-derived ids: two checkouts at
/// different paths, or CI and a laptop, would agree on the graph and
/// disagree on every id in it.
///
/// With one root — the overwhelmingly common case — the root is stripped
/// entirely, so the key is location-independent: the same tree gives the
/// same ids wherever it sits on disk. With several roots the root's
/// canonical directory name is kept as a prefix, because `src/mod.py`
/// and `tests/mod.py` are different files and stripping both to `mod.py`
/// would collide them into the redraw path, which is order-dependent —
/// the very thing the identity key exists to avoid.
pub fn id_path(path: &std::path::Path, roots: &[IdRoot]) -> String {
    for r in roots {
        if let Ok(rest) = path.strip_prefix(&r.display) {
            // A file passed directly as a root strips to nothing; its own
            // name is the whole relative path.
            let rest = if rest.as_os_str().is_empty() {
                std::path::Path::new(path.file_name().unwrap_or_default())
            } else {
                rest
            };
            return format!("{}{}", r.prefix, rest.to_string_lossy());
        }
    }
    // No root matched (defensive: the walker builds these paths from the
    // roots, so this should not happen). Falling back to the raw path is
    // wrong-but-stable rather than a panic.
    path.to_string_lossy().into_owned()
}

/// One analysis root, prepared for `id_path`.
#[derive(Debug, Clone)]
pub struct IdRoot {
    /// The root exactly as the walker will have prefixed file paths with.
    pub display: std::path::PathBuf,
    /// `""` for a single root, `"<dirname>/"` when several roots could
    /// otherwise contribute colliding relative paths.
    pub prefix: String,
}

/// Build the `IdRoot` list for a run. Canonicalisation happens here, once
/// per root rather than once per file, and only to derive the
/// disambiguating directory name — the strip itself is textual, because
/// the walker composes every path from the display root.
pub fn id_roots(paths: &[std::path::PathBuf]) -> Vec<IdRoot> {
    let multi = paths.len() > 1;
    paths
        .iter()
        .map(|p| {
            let prefix = if !multi {
                String::new()
            } else {
                let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
                let dir = if canon.is_file() {
                    canon.parent().map(|d| d.to_path_buf()).unwrap_or(canon)
                } else {
                    canon
                };
                dir.file_name()
                    .map(|n| format!("{}/", n.to_string_lossy()))
                    .unwrap_or_default()
            };
            IdRoot {
                display: p.clone(),
                prefix,
            }
        })
        .collect()
}

/// Stateful allocator for content-derived `FileId`/`CallableId` values.
/// One instance is threaded through a whole `analyze_in_pool` run so the
/// three allocation sites (file/callable merge, exit-node synthesis,
/// entry-node synthesis) share one `seen` universe and never collide with
/// each other.
#[derive(Default)]
pub struct StableIds {
    seen_files: HashSet<u64>,
    seen_callables: HashSet<u64>,
}

impl StableIds {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a `FileId` keyed on the file's relative path. Stable
    /// across content edits — it only changes if the file itself is
    /// renamed or moved.
    pub fn file(&mut self, relative_path: &str) -> cgg_core::ids::FileId {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"file\0");
        hasher.update(relative_path.as_bytes());
        cgg_core::ids::FileId::new_u64(allocate(hasher, &mut self.seen_files))
    }

    /// Allocate a `CallableId` keyed on `(language, file_path,
    /// owner_qualified_name, qualified_name, signature_hint)`.
    ///
    /// `signature_hint` is load-bearing, not decoration. Without it the
    /// key is not unique: a file that declares several callables sharing
    /// a qualified name — overloads, which C++, C#, Java and Erlang have
    /// in quantity — hashes them identically. Over the 113-repo
    /// benchmark corpus that is **309,347 of 1,745,670 callables
    /// (17.7%)**, peaking at 52.8% in one repo, and every one of them
    /// falls through to the redraw path in `allocate`, where the id is
    /// decided by declaration order rather than content. That breaks the
    /// guarantee this module exists to provide: delete the first of two
    /// overloads and the second inherits its id, so a consumer diffing
    /// ids across runs reads "unchanged" while the id now names a
    /// different function. Adding the signature takes that population to
    /// **2.25%** — the residual being callables cgg genuinely cannot
    /// tell apart, where no key can do better.
    ///
    /// `start_byte` was measured as an alternative and rejected. It is
    /// unique corpus-wide, but it churns on movement — one comment line
    /// added at the top of `spdlog`'s busiest header moved 135 of 1,157
    /// ids, destroying the diffability this change exists for — and it
    /// still lets a survivor inherit a deleted sibling's id, because
    /// removing a definition shifts the next one into its byte offset.
    ///
    /// What holds: editing an unrelated file changes nothing, moving
    /// code within a file changes nothing, and removing one overload
    /// leaves its siblings alone.
    pub fn callable(
        &mut self,
        language: &str,
        file_path: &str,
        owner_qualified_name: Option<&str>,
        qualified_name: &str,
        signature_hint: &str,
    ) -> cgg_core::ids::CallableId {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"callable\0");
        hasher.update(language.as_bytes());
        hasher.update(b"\0");
        hasher.update(file_path.as_bytes());
        hasher.update(b"\0");
        hasher.update(owner_qualified_name.unwrap_or("").as_bytes());
        hasher.update(b"\0");
        hasher.update(qualified_name.as_bytes());
        hasher.update(b"\0");
        hasher.update(signature_hint.as_bytes());
        cgg_core::ids::CallableId::new_u64(allocate(hasher, &mut self.seen_callables))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defect this keyed on: `cgg-walk` reports files under the
    /// root as typed, so hashing the raw path made an id a function of
    /// the invocation. `cgg /tmp/x`, `cgg x`, `cgg .` and `cgg t.py` all
    /// describe the same file and produced four different ids.
    #[test]
    fn id_path_is_the_same_however_the_root_was_written() {
        use std::path::{Path, PathBuf};
        let forms = [
            (PathBuf::from("/tmp/smoke2"), "/tmp/smoke2/t.py"),
            (PathBuf::from("smoke2"), "smoke2/t.py"),
            (PathBuf::from("."), "./t.py"),
            (PathBuf::from("t.py"), "t.py"),
        ];
        for (root, file) in forms {
            let roots = vec![IdRoot {
                display: root.clone(),
                prefix: String::new(),
            }];
            assert_eq!(
                id_path(Path::new(file), &roots),
                "t.py",
                "root {root:?} / file {file}"
            );
        }
    }

    /// With one root the root is stripped entirely, so the same tree
    /// gives the same ids wherever it is checked out.
    #[test]
    fn a_single_root_is_stripped_so_ids_are_location_independent() {
        use std::path::{Path, PathBuf};
        let a = id_roots(&[PathBuf::from("/home/alice/repo")]);
        let b = id_roots(&[PathBuf::from("/srv/ci/build/repo")]);
        assert_eq!(
            id_path(Path::new("/home/alice/repo/src/lib.rs"), &a),
            id_path(Path::new("/srv/ci/build/repo/src/lib.rs"), &b),
        );
        assert_eq!(
            id_path(Path::new("/home/alice/repo/src/lib.rs"), &a),
            "src/lib.rs"
        );
    }

    /// Several roots keep a disambiguating directory name, because
    /// `src/m.py` and `tests/m.py` are different files and collapsing
    /// both to `m.py` would push them into the order-dependent redraw.
    #[test]
    fn multiple_roots_do_not_collapse_same_named_files() {
        use std::path::Path;
        let roots = vec![
            IdRoot {
                display: "/mr/src".into(),
                prefix: "src/".into(),
            },
            IdRoot {
                display: "/mr/tests".into(),
                prefix: "tests/".into(),
            },
        ];
        let a = id_path(Path::new("/mr/src/m.py"), &roots);
        let b = id_path(Path::new("/mr/tests/m.py"), &roots);
        assert_ne!(a, b);
        assert_eq!((a.as_str(), b.as_str()), ("src/m.py", "tests/m.py"));
    }

    #[test]
    fn same_inputs_produce_the_same_file_id() {
        let mut a = StableIds::new();
        let mut b = StableIds::new();
        assert_eq!(a.file("src/main.rs"), b.file("src/main.rs"));
    }

    #[test]
    fn same_inputs_produce_the_same_callable_id() {
        let mut a = StableIds::new();
        let mut b = StableIds::new();
        assert_eq!(
            a.callable(
                "rust",
                "src/lib.rs",
                Some("Widget"),
                "Widget::new",
                "fn new()"
            ),
            b.callable(
                "rust",
                "src/lib.rs",
                Some("Widget"),
                "Widget::new",
                "fn new()"
            ),
        );
    }

    #[test]
    fn distinct_inputs_do_not_collide_at_typical_corpus_size() {
        let mut ids = StableIds::new();
        let mut seen = HashSet::new();
        // A few tens of thousands of distinct callables, well beyond
        // what a single-repo run typically extracts.
        for i in 0..50_000u32 {
            let qn = format!("module::function_{i}");
            let id = ids.callable("rust", "src/generated.rs", None, &qn, "");
            assert!(seen.insert(id), "unexpected collision at i={i}");
        }
    }

    #[test]
    fn file_ids_are_distinct_for_distinct_paths() {
        let mut ids = StableIds::new();
        let mut seen = HashSet::new();
        for i in 0..20_000u32 {
            let path = format!("crates/pkg/src/mod_{i}.rs");
            let id = ids.file(&path);
            assert!(seen.insert(id), "unexpected file id collision at i={i}");
        }
    }

    /// Force a collision by pre-seeding `seen_callables` with the exact
    /// 52-bit value the next real allocation would otherwise get, and
    /// confirm the escalation kicks in: the final id differs from the
    /// seeded value, is deterministic given the same seen-set state, and
    /// is not simply the colliding value reused.
    #[test]
    fn callable_collision_escalates_deterministically() {
        // First, learn what the natural (unseeded) id would be.
        let natural = {
            let mut ids = StableIds::new();
            ids.callable("python", "app/models.py", Some("User"), "User.save", "")
        };

        // Now seed a fresh allocator's `seen` set with that exact value,
        // forcing the same call to collide and escalate.
        let mut seeded = StableIds::new();
        seeded.seen_callables.insert(natural.as_u64());
        let escalated =
            seeded.callable("python", "app/models.py", Some("User"), "User.save", "");

        assert_ne!(
            escalated, natural,
            "escalation must not silently reuse the colliding value"
        );
        assert_ne!(escalated.as_u64(), natural.as_u64());

        // Deterministic: repeating the exact same seen-set state produces
        // the exact same escalated id.
        let mut seeded_again = StableIds::new();
        seeded_again.seen_callables.insert(natural.as_u64());
        let escalated_again = seeded_again.callable(
            "python",
            "app/models.py",
            Some("User"),
            "User.save",
            "",
        );
        assert_eq!(escalated, escalated_again);
    }

    /// Same escalation property for `FileId`.
    #[test]
    fn file_collision_escalates_deterministically() {
        let natural = {
            let mut ids = StableIds::new();
            ids.file("src/lib.rs")
        };

        let mut seeded = StableIds::new();
        seeded.seen_files.insert(natural.as_u64());
        let escalated = seeded.file("src/lib.rs");

        assert_ne!(escalated, natural);

        let mut seeded_again = StableIds::new();
        seeded_again.seen_files.insert(natural.as_u64());
        let escalated_again = seeded_again.file("src/lib.rs");
        assert_eq!(escalated, escalated_again);
    }

    /// The case that actually occurs, and that nothing covered before:
    /// two callables in ONE file sharing a qualified name — overloads —
    /// allocated from ONE `StableIds`. On `(language, file, owner, qn)`
    /// alone these hash identically, so the second escalated and its id
    /// became a function of declaration order; deleting the first then
    /// handed its id to the second. Distinct signatures must now yield
    /// distinct ids with no escalation, and removing one must leave the
    /// other's id untouched.
    #[test]
    fn overloads_in_one_file_get_distinct_ids_that_survive_a_sibling_removal() {
        let (f, qn, owner) = ("src/shape.cpp", "Shape::area", Some("Shape"));
        let (sig1, sig2) = ("int area(int w)", "int area(int w, int h)");

        let mut both = StableIds::new();
        let first = both.callable("cpp", f, owner, qn, sig1);
        let second = both.callable("cpp", f, owner, qn, sig2);
        assert_ne!(first, second, "overloads must not share an id");

        // Both fit in 52 bits, so both survive the trip through
        // cgg-node's `number` binding intact.
        assert!(first.as_u64() < 1 << 52, "first exceeds 52 bits: {first}");
        assert!(
            second.as_u64() < 1 << 52,
            "second exceeds 52 bits: {second}"
        );

        // Delete the first overload. The second keeps its own id rather
        // than inheriting the departed one's — the silent-corruption
        // case, where a stale id still resolves but names something else.
        let mut survivor = StableIds::new();
        assert_eq!(
            survivor.callable("cpp", f, owner, qn, sig2),
            second,
            "surviving overload's id must not depend on a sibling"
        );
    }

    /// Two callables cgg genuinely cannot tell apart — same file, same
    /// qualified name, same signature — still get distinct ids, and both
    /// stay inside 52 bits so neither is corrupted by `cgg-node`'s
    /// `number` binding.
    #[test]
    fn indistinguishable_siblings_still_get_distinct_js_safe_ids() {
        let mut ids = StableIds::new();
        let a = ids.callable("erlang", "src/m.erl", None, "m:go", "");
        let b = ids.callable("erlang", "src/m.erl", None, "m:go", "");
        assert_ne!(a, b, "a redraw must not hand out the same value twice");
        for id in [a, b] {
            assert!(
                id.as_u64() < 1 << 52,
                "id {id} exceeds 52 bits and is unsafe as a JS number"
            );
        }
    }

    #[test]
    fn files_and_callables_use_independent_seen_sets() {
        // A file and a callable can legitimately land on the same raw
        // hash value without either needing to escalate, because they're
        // different id types tracked in different `seen` sets.
        let mut ids = StableIds::new();
        let f = ids.file("src/lib.rs");
        // Not asserting equality (vanishingly unlikely) — just that nothing
        // panics and both allocate independently of one another.
        let c = ids.callable("rust", "src/lib.rs", None, "lib::main", "");
        let _ = (f, c);
    }
}
