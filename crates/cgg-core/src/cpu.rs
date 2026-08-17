//! Physical-core detection, for choosing a sane default worker count.
//!
//! `std::thread::available_parallelism()` reports *logical* CPUs, which
//! on any SMT machine is double the physical core count. cgg's hot loops
//! are parse and resolve — both compute- and allocator-bound rather than
//! latency-bound — and hyperthread siblings share the execution units
//! those loops saturate. Oversubscribing them buys contention, not
//! throughput.
//!
//! So the default is **half the physical cores, capped at
//! [`MAX_AUTO_JOBS`]**, detected at runtime. Nothing here is hardcoded
//! to a machine: an EPYC 7532 (32 physical / 64 logical) gets 8, a
//! 16-physical-core workstation gets 8, an 8-core laptop gets 4, and a
//! container pinned to 2 CPUs gets 1.
//!
//! The cap exists because the default should be a good guest on a shared
//! machine, not because more threads stop helping — on a large tree they
//! clearly do. `--jobs N` overrides it.
//!
//! # Why the cgroup cap matters
//!
//! Kernel topology describes the *host*, not the share of it this
//! process may use. A container limited to 4 CPUs on a 64-core host
//! still sees 32 physical cores in sysfs. `available_parallelism()` does
//! respect cgroup quotas, so the detected count is capped by it —
//! otherwise CI inside a small container would spawn 16 workers for 4
//! CPUs and thrash.

#[cfg(target_os = "linux")]
use std::collections::HashSet;

/// Physical cores usable by this process.
///
/// Falls back to logical parallelism when topology is unreadable, which
/// only over-estimates on SMT hardware — and the caller halves it, so
/// the result lands back at roughly the physical count anyway.
pub fn physical_cores() -> usize {
    let logical = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let detected = detect_physical().unwrap_or(logical);
    // Topology describes the host; the quota describes our share.
    detected.min(logical).max(1)
}

/// Upper bound on the automatic worker count.
///
/// Most machines running cgg have 4-8 physical cores, so above this the
/// default stops tracking the hardware and behaves like a common
/// desktop. That is a deliberate choice to be a well-behaved guest on a
/// big shared box rather than to win a benchmark: a 64-thread server
/// gets 8 workers by default and leaves the rest for whatever else is
/// running. `--jobs N` overrides it, and on a large tree that is worth
/// doing — see the note on cost below.
const MAX_AUTO_JOBS: usize = 8;

/// Default worker count: half the physical cores, capped, never zero.
///
/// **This is tuned for politeness, not for throughput, and the
/// difference is measurable.** On a 32-physical-core machine the default
/// is 8 rather than 16 or 32, and on a large repository more workers are
/// genuinely faster — Druid measured 18.9s at 8 threads against 8.9s at
/// 32. Anyone analysing a large tree, or on a machine they own, should
/// pass `--jobs`. The default optimises for not monopolising a host that
/// may be shared.
pub fn default_jobs() -> usize {
    (physical_cores() / 2).clamp(1, MAX_AUTO_JOBS)
}

#[cfg(target_os = "linux")]
fn detect_physical() -> Option<usize> {
    // Each core's siblings list is identical for every thread on that
    // core, so the number of distinct lists is the number of cores.
    // `thread_siblings_list` is the direct expression of that and needs
    // no pairing of two separate files.
    let dir = std::fs::read_dir("/sys/devices/system/cpu").ok()?;
    let mut cores: HashSet<String> = HashSet::new();
    for entry in dir.flatten() {
        let p = entry.path().join("topology/thread_siblings_list");
        if let Ok(s) = std::fs::read_to_string(&p) {
            let s = s.trim();
            if !s.is_empty() {
                cores.insert(s.to_string());
            }
        }
    }
    if !cores.is_empty() {
        return Some(cores.len());
    }
    // Older kernels and some VMs expose no topology directory. Fall back
    // to the (physical id, core id) pairs in /proc/cpuinfo.
    let info = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    let mut pairs: HashSet<(String, String)> = HashSet::new();
    let (mut pkg, mut core) = (None, None);
    for line in info.lines() {
        let Some((k, v)) = line.split_once(':') else {
            if line.trim().is_empty()
                && let (Some(p), Some(c)) = (pkg.take(), core.take())
            {
                pairs.insert((p, c));
            }
            continue;
        };
        match k.trim() {
            "physical id" => pkg = Some(v.trim().to_string()),
            "core id" => core = Some(v.trim().to_string()),
            _ => {}
        }
    }
    if let (Some(p), Some(c)) = (pkg, core) {
        pairs.insert((p, c));
    }
    (!pairs.is_empty()).then_some(pairs.len())
}

#[cfg(target_os = "macos")]
fn detect_physical() -> Option<usize> {
    let out = std::process::Command::new("sysctl")
        .args(["-n", "hw.physicalcpu"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn detect_physical() -> Option<usize> {
    // No portable query without a dependency. Assume SMT2, the
    // overwhelmingly common case on machines large enough for it to
    // matter; a non-SMT machine then gets half its cores, which is
    // conservative rather than wrong.
    let logical = std::thread::available_parallelism().ok()?.get();
    Some((logical / 2).max(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_is_sane_and_bounded_by_the_quota() {
        let logical = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        let phys = physical_cores();
        assert!(phys >= 1, "at least one core");
        assert!(
            phys <= logical,
            "physical ({phys}) must never exceed the parallelism this \
             process is allowed ({logical}) — a container quota bounds the \
             host's topology"
        );
    }

    #[test]
    fn default_jobs_is_never_zero() {
        // A single-core machine, or a container pinned to one CPU, must
        // still get a usable worker count rather than a pool of zero.
        assert!(default_jobs() >= 1);
        assert!(default_jobs() <= physical_cores().max(1));
    }
}
