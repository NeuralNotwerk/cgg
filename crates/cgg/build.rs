//! Stamps the CLI's `--version` with the build's identity.
//!
//! Every build of an unreleased tree used to print the same `cgg 0.11.0`
//! as the official release, so an interim binary could not be told from
//! the published one. `CGG_BUILD_VERSION` is:
//!
//! - `0.11.0` — an official build: `CGG_OFFICIAL_BUILD=1` (the release
//!   workflow), a clean checkout at tag `v0.11.0`, or no git checkout at
//!   all (a crates.io package or `cargo install`, built from released
//!   source);
//! - `0.11.0 (dev g1b88140)` — any other commit, clean;
//! - `0.11.0 (dev g1b88140+d3f9a2c)` — uncommitted changes, where the
//!   `d` part hashes the diff and untracked sources, so two different
//!   interim builds of one commit never print the same identity.
//!
//! The version number stays the second word, so `awk '{print $2}'`
//! readers (`upgrade-local.sh`, `perf-compare.sh`) are unaffected; the
//! library's version (`cgg_core::CGG_VERSION`, Python, Node, C ABI) is
//! untouched.

use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// FNV-1a: a short, dependency-free fingerprint of the uncommitted state.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn main() {
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // Only this repository's own layout counts: a packaged crate is
    // unpacked under `target/package/`, inside the checkout, and must not
    // pick up the surrounding repository's state.
    let root = Path::new(&manifest).join("../..");
    let in_checkout =
        root.join(".git").exists() && root.join("crates/cgg/build.rs").exists();

    println!("cargo:rerun-if-env-changed=CGG_OFFICIAL_BUILD");
    let official_env = std::env::var("CGG_OFFICIAL_BUILD").is_ok_and(|v| v == "1");

    let build = if official_env || !in_checkout {
        version.clone()
    } else {
        for p in [
            ".git/HEAD",
            ".git/index",
            ".git/refs",
            "crates",
            "Cargo.toml",
            "Cargo.lock",
        ] {
            println!("cargo:rerun-if-changed={}", root.join(p).display());
        }
        let head = git(&root, &["rev-parse", "--short=7", "HEAD"])
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| "unknown".into());
        let scope = ["--", "crates", "Cargo.toml", "Cargo.lock"];
        let status = git(
            &root,
            &[
                &["status", "--porcelain", "--untracked-files=all"][..],
                &scope[..],
            ]
            .concat(),
        )
        .unwrap_or_default();
        let dirty = !status.trim().is_empty();
        let tagged = git(&root, &["describe", "--exact-match", "--tags", "HEAD"])
            .is_some_and(|t| t.trim() == format!("v{version}"));
        if tagged && !dirty {
            version.clone()
        } else if dirty {
            let diff = git(&root, &[&["diff", "HEAD"][..], &scope[..]].concat())
                .unwrap_or_default();
            let mut h = fnv(diff.as_bytes()) ^ fnv(status.as_bytes());
            // Untracked files are in `status` by name only; fold in their
            // contents too.
            for line in status.lines().filter(|l| l.starts_with("??")) {
                if let Ok(b) = std::fs::read(root.join(line[3..].trim())) {
                    h ^= fnv(&b).rotate_left(17);
                }
            }
            format!("{version} (dev g{head}+d{:07x})", h & 0xfff_ffff)
        } else {
            format!("{version} (dev g{head})")
        }
    };
    println!("cargo:rustc-env=CGG_BUILD_VERSION={build}");
}
