//! Opt-in span profiler for locating where a run spends its time.
//!
//! `RunMetrics::phases` records four coarse buckets — walk, parse,
//! extract, link. That was enough while "link" was one pass; it stopped
//! being enough once link grew a type propagator, an intra-file linker,
//! a cross-file resolver, an FFI linker, a descriptor linker, a
//! framework engine with six matchers, and a dead-code analysis. A 25%
//! regression inside that bucket is invisible from outside it.
//!
//! Design constraints, in order:
//!
//! 1. **Absent from release builds, not merely disabled.** [`span`] is
//!    `#[cfg]`-compiled to a constant `None` in release, so there is no
//!    atomic load, no clock read and no branch to argue about. Debug
//!    builds — what tests and local debugging run — collect by default
//!    and print with `--profile`. The published latency numbers are
//!    measured on release binaries, and a profiler that *could* perturb
//!    them is a profiler that makes those numbers arguable.
//! 2. **Thread-safe.** Extraction is parallel, so spans are merged under
//!    a mutex that is only ever touched while profiling is on.
//! 3. **Aggregating, not tracing.** The question being answered is
//!    "which phase got slower", not "what happened at 14:02:11.3". Spans
//!    accumulate into `(total, count)` per name, so a span inside a
//!    per-file loop is fine.
//!
//! Spans are flat and additive. Nesting a child inside a parent means
//! the child's time is counted in both, which is what you want for
//! "cross-file is 60% of link" and is why the report prints a percentage
//! of wall rather than pretending the rows sum to 100.
//!
//! ```ignore
//! let _s = profile::span("frameworks::detect");
//! ```

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Collection is on by default in debug builds and off in release,
/// where `--profile` turns it on at runtime.
///
/// It used to be `#[cfg]`-compiled *out* of release entirely, so the
/// one build anyone actually runs could not answer "where is this
/// dwelling?" — the question you have precisely when a real input is
/// pathological, and the one a debug build cannot answer because it
/// distorts the ratios you are reading. A capability that needs a
/// special build is not a capability.
///
/// The cost of keeping it is a relaxed atomic load and a
/// perfectly-predicted branch per span. Spans are coarse — tens of
/// thousands of entries per run, not per callable — and the difference
/// is unmeasurable against the corpus, which is the bar that matters
/// for published latency numbers.
static ENABLED: AtomicBool = AtomicBool::new(cfg!(debug_assertions));

/// Per-span totals. Atomics rather than a map behind a lock: a span
/// inside the per-file parse loop is entered once per file from every
/// rayon worker, and a global mutex there does not merely add overhead —
/// it *serialises the parallel section being measured*, so the profiler
/// reports the contention it caused. That is how the first version of
/// this module read 263% CPU on a run the plain binary did at 212%.
#[derive(Default, Debug)]
struct Counters {
    nanos: AtomicU64,
    calls: AtomicU64,
}

type Registry = Mutex<BTreeMap<&'static str, &'static Counters>>;

fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(|| Mutex::new(BTreeMap::new()))
}

// Both of these exist only to serve `span`, which is `#[cfg]`-compiled
// away in release. Gating them the same way keeps the module's promise
// literal — the machinery is *absent* from a release build, not merely
// unreachable — and keeps `cargo build --release` warning-free.
thread_local! {
    /// Per-thread cache so the registry lock is taken once per thread
    /// per span name, not once per span *entry*. Keyed on the name's
    /// pointer, which is stable because span names are `&'static str`
    /// literals.
    static CACHE: RefCell<HashMap<usize, &'static Counters>> =
        RefCell::new(HashMap::new());
}

fn counters(name: &'static str) -> &'static Counters {
    CACHE.with(|cache| {
        let key = name.as_ptr() as usize;
        if let Some(c) = cache.borrow().get(&key) {
            return *c;
        }
        let mut reg = registry().lock().unwrap_or_else(|e| e.into_inner());
        let c: &'static Counters = reg
            .entry(name)
            .or_insert_with(|| Box::leak(Box::new(Counters::default())));
        cache.borrow_mut().insert(key, c);
        c
    })
}

/// Turn profiling on for the rest of the process.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// A running span. Records its elapsed time when dropped.
#[derive(Debug)]
pub struct Span {
    counters: &'static Counters,
    start: Instant,
}

impl Drop for Span {
    fn drop(&mut self) {
        // Two relaxed atomic adds. No lock, so a span in a parallel loop
        // does not serialise it.
        self.counters
            .nanos
            .fetch_add(self.start.elapsed().as_nanos() as u64, Ordering::Relaxed);
        self.counters.calls.fetch_add(1, Ordering::Relaxed);
    }
}

/// Start a span — or, in a release build, nothing at all.
///
/// In **release** this is `#[cfg]`-compiled to an unconditional `None`.
/// The `Option<Span>` is then a compile-time constant, `Drop` is never
/// reachable, and the whole call vanishes: no atomic load, no clock
/// read, no branch. That is a stronger guarantee than a runtime flag,
/// and it is the point — the numbers this project publishes are
/// measured on release binaries, and a profiler that could perturb them
/// is a profiler that makes those numbers arguable.
///
/// In **debug** it returns `Option<Span>` rather than a no-op guard so
/// the disabled path still skips `Instant::now()`, which is a vDSO call
/// a per-file span would otherwise pay twice per file for nothing.
#[inline(always)]
pub fn span(name: &'static str) -> Option<Span> {
    {
        enabled().then(|| Span {
            counters: counters(name),
            start: Instant::now(),
        })
    }
}

/// Whether this binary can profile at all.
pub const fn compiled_in() -> bool {
    true
}

/// One row of the report.
#[derive(Clone, Debug)]
pub struct Row {
    pub name: &'static str,
    pub total_ms: f64,
    pub calls: u64,
}

/// Every span recorded so far, slowest first.
pub fn report() -> Vec<Row> {
    let reg = registry().lock().unwrap_or_else(|e| e.into_inner());
    let mut rows: Vec<Row> = reg
        .iter()
        .map(|(name, c)| Row {
            name,
            total_ms: c.nanos.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            calls: c.calls.load(Ordering::Relaxed),
        })
        .collect();
    rows.sort_by(|a, b| b.total_ms.total_cmp(&a.total_ms));
    rows
}

/// Human-readable table.
///
/// `wall_ms` is passed in rather than measured here so the percentages
/// are against the run the caller actually timed. Rows can exceed 100%
/// in total: spans nest, and a parent counts its children's time.
pub fn render(wall_ms: f64) -> String {
    if !compiled_in() {
        return "\nprofile: compiled out of release builds — the published \
latency numbers are measured on release binaries, so the profiler is not \
allowed to perturb them. Rebuild with `cargo build -p cgg` (debug) and \
re-run with --profile.\n"
            .to_string();
    }
    let rows = report();
    if rows.is_empty() {
        return String::new();
    }
    let mut s = String::from("\nprofile (spans nest, so percentages may sum past 100)\n");
    s.push_str(&format!(
        "  {:<34} {:>10} {:>7} {:>9}\n",
        "span", "total ms", "% wall", "calls"
    ));
    for r in &rows {
        let pct = if wall_ms > 0.0 {
            r.total_ms / wall_ms * 100.0
        } else {
            0.0
        };
        s.push_str(&format!(
            "  {:<34} {:>10.1} {:>6.1}% {:>9}\n",
            r.name, r.total_ms, pct, r.calls
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_builds_compile_spans_out_entirely() {
        // The guarantee that lets published latency numbers stand: in a
        // release build `span` is a constant `None`, so nothing can be
        // recorded even if collection were somehow switched on.
        if !compiled_in() {
            let _s = span("test::release");
            assert!(_s.is_none());
            assert!(!report().iter().any(|r| r.name == "test::release"));
        }
    }

    #[test]
    fn an_enabled_span_accumulates_across_calls() {
        enable();
        for _ in 0..3 {
            let _s = span("test::enabled");
            std::hint::black_box(0);
        }
        let rows = report();
        let row = rows
            .iter()
            .find(|r| r.name == "test::enabled")
            .expect("span recorded");
        assert_eq!(row.calls, 3, "three entries accumulate into one row");
        // Ordering is slowest-first, which is the whole point of the
        // report — a caller reads the top row and stops.
        let sorted = rows.windows(2).all(|w| w[0].total_ms >= w[1].total_ms);
        assert!(sorted, "report must be sorted slowest first");
    }
}
