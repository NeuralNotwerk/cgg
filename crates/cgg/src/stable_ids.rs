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
/// already in `seen`, escalating bit width on collision. Inserts the
/// chosen value into `seen` before returning it.
fn allocate(hasher: blake3::Hasher, seen: &mut HashSet<u64>) -> u64 {
    let mut xof = hasher.finalize_xof();

    let mut bits = DEFAULT_BITS;
    loop {
        let candidate = xof_bits(&mut xof, bits);
        if seen.insert(candidate) {
            return candidate;
        }
        if bits >= 64 {
            // We've exhausted a full u64 read from the xof stream at
            // this bit width and still collided. Keep widening the xof
            // read itself (distinct bytes each loop) rather than bit
            // width, which cannot exceed 64. This should be practically
            // unreachable — it requires the `seen` set to already
            // contain the specific 64-bit value derived from this
            // exact byte window of this exact hash stream — but must
            // never silently degrade into a duplicate id.
            let mut guard = 0u32;
            loop {
                let mut buf = [0u8; 8];
                xof.fill(&mut buf);
                let v = u64::from_le_bytes(buf);
                if seen.insert(v) {
                    return v;
                }
                guard += 1;
                assert!(
                    guard < 1_000_000,
                    "stable id allocator exhausted escalation without finding a free slot; \
                     this should be statistically impossible and indicates a bug"
                );
            }
        }
        bits = (bits * 2).min(64);
    }
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

    /// Allocate a `CallableId` keyed on
    /// `(language, file_path, owner_qualified_name, qualified_name)`.
    pub fn callable(
        &mut self,
        language: &str,
        file_path: &str,
        owner_qualified_name: Option<&str>,
        qualified_name: &str,
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
        cgg_core::ids::CallableId::new_u64(allocate(hasher, &mut self.seen_callables))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            a.callable("rust", "src/lib.rs", Some("Widget"), "Widget::new"),
            b.callable("rust", "src/lib.rs", Some("Widget"), "Widget::new"),
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
            let id = ids.callable("rust", "src/generated.rs", None, &qn);
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
            ids.callable("python", "app/models.py", Some("User"), "User.save")
        };

        // Now seed a fresh allocator's `seen` set with that exact value,
        // forcing the same call to collide and escalate.
        let mut seeded = StableIds::new();
        seeded.seen_callables.insert(natural.as_u64());
        let escalated =
            seeded.callable("python", "app/models.py", Some("User"), "User.save");

        assert_ne!(
            escalated, natural,
            "escalation must not silently reuse the colliding value"
        );
        assert_ne!(escalated.as_u64(), natural.as_u64());

        // Deterministic: repeating the exact same seen-set state produces
        // the exact same escalated id.
        let mut seeded_again = StableIds::new();
        seeded_again.seen_callables.insert(natural.as_u64());
        let escalated_again =
            seeded_again.callable("python", "app/models.py", Some("User"), "User.save");
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

    #[test]
    fn files_and_callables_use_independent_seen_sets() {
        // A file and a callable can legitimately land on the same raw
        // hash value without either needing to escalate, because they're
        // different id types tracked in different `seen` sets.
        let mut ids = StableIds::new();
        let f = ids.file("src/lib.rs");
        // Not asserting equality (vanishingly unlikely) — just that nothing
        // panics and both allocate independently of one another.
        let c = ids.callable("rust", "src/lib.rs", None, "lib::main");
        let _ = (f, c);
    }
}
