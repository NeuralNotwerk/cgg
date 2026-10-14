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

/// Upper bound on the automatic worker count on a machine with fewer
/// than [`WIDE_HOST_THRESHOLD`] physical cores.
///
/// Most machines running cgg have 4-8 physical cores, so below the wide-
/// host threshold the default stops tracking the hardware and behaves
/// like a common desktop. `--jobs N` overrides it, and on a large tree
/// that is worth doing — see the note on cost below.
const MAX_AUTO_JOBS: usize = 8;

/// Physical-core count at and above which the cap widens from
/// [`MAX_AUTO_JOBS`] to [`MAX_AUTO_JOBS_WIDE_HOST`].
///
/// Measured (`l/MY-FINDINGS.md`, this repo's own AB harness): on a
/// 64-physical / 128-logical EPYC host, llmitm-v5 analysed in ~275ms at
/// the old fixed auto-default of 8 workers and ~175ms at 32 workers —
/// 8 was leaving over a third of the wall clock on the table on exactly
/// the kind of big shared box the low cap was written to protect.
/// Below 32 physical cores the machine is small enough that
/// `MAX_AUTO_JOBS` was already the binding constraint only rarely (it
/// only bites once `physical_cores()/2` exceeds 8, i.e. >=16 physical
/// cores), so this only changes behaviour on genuinely wide hosts.
const WIDE_HOST_THRESHOLD: usize = 32;

/// Upper bound on the automatic worker count once
/// [`WIDE_HOST_THRESHOLD`] is met or exceeded.
const MAX_AUTO_JOBS_WIDE_HOST: usize = 32;

/// Pure core: maps a (physical core count, parallelism quota) pair to a
/// worker count. Kept free of any I/O so it can be tested for every
/// core count without needing a matching real host.
///
/// `quota` is `std::thread::available_parallelism()` — logical
/// parallelism, which already reflects any cgroup CPU quota — and always
/// wins over the topology-detected `physical` count when it is smaller,
/// so a container pinned below its host's physical core count still
/// gets a jobs count sized to what it may actually use.
fn jobs_for(physical: usize, quota: usize) -> usize {
    let usable = physical.min(quota).max(1);
    let cap = if usable >= WIDE_HOST_THRESHOLD {
        MAX_AUTO_JOBS_WIDE_HOST
    } else {
        MAX_AUTO_JOBS
    };
    (usable / 2).clamp(1, cap)
}

/// Default worker count: half the physical cores, capped, never zero.
///
/// **This is tuned for politeness on small-to-mid machines, not for
/// throughput, and the difference is measurable.** On a machine under
/// [`WIDE_HOST_THRESHOLD`] physical cores the default stays capped at
/// [`MAX_AUTO_JOBS`] rather than chasing every core — Druid measured
/// 18.9s at 8 threads against 8.9s at 32 on one large repository, and
/// anyone analysing a large tree, or on a machine they own, should still
/// pass `--jobs` explicitly. Once physical cores clears
/// [`WIDE_HOST_THRESHOLD`] the cap widens to
/// [`MAX_AUTO_JOBS_WIDE_HOST`], because at that host size 8 stops being
/// "a well-behaved guest" and starts being most of the machine sitting
/// idle — see [`WIDE_HOST_THRESHOLD`]'s doc comment for the measurement.
/// Bounded throughout by whatever cgroup quota
/// `std::thread::available_parallelism()` already reflects.
pub fn default_jobs() -> usize {
    let quota = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    jobs_for(physical_cores(), quota)
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

    /// SPECS.md c10: "the pure function that maps (physical_cores, quota)
    /// -> jobs, for 4, 8, 16, 32, 64, 128 cores." Quota == physical in
    /// this table, i.e. an unconstrained host at each size.
    #[test]
    fn jobs_for_caps_at_8_below_32_cores_and_32_at_or_above() {
        let cases = [
            // (physical, quota, expected jobs)
            (4, 4, 2),
            (8, 8, 4),
            (16, 16, 8),
            // 24/2=12 would exceed the old cap were it not for the
            // clamp; still below the wide-host threshold so it stays
            // capped at MAX_AUTO_JOBS, not widened.
            (24, 24, 8),
            (32, 32, 16),
            (64, 64, 32),
            // 128/2=64 must still clamp to the wide-host cap of 32, not
            // grow unbounded.
            (128, 128, 32),
        ];
        for (physical, quota, expected) in cases {
            assert_eq!(
                jobs_for(physical, quota),
                expected,
                "jobs_for({physical}, {quota}) should be {expected}"
            );
        }
    }

    /// A cgroup quota below the detected physical count MUST still win:
    /// a container pinned to 4 CPUs on a 64-physical-core host gets a
    /// jobs count sized to its 4, not the host's topology.
    #[test]
    fn jobs_for_is_bounded_by_the_quota_not_just_physical_cores() {
        assert_eq!(
            jobs_for(64, 4),
            2,
            "a 4-CPU quota on a 64-physical-core host must bound usable \
             cores to 4, giving 4/2=2 workers — not 32"
        );
    }

    #[test]
    fn jobs_for_never_zero_even_on_a_single_core() {
        assert_eq!(jobs_for(1, 1), 1);
        assert_eq!(jobs_for(0, 0), 1);
    }
}
