//! Best-effort "is there a newer release?" notice.
//!
//! This is the *only* network access `cgg` ever makes, and it is built
//! to be invisible everywhere it shouldn't appear:
//!
//! * **Interactive only.** Disabled unless stderr is a TTY — so pipes,
//!   redirects, CI, coding agents, and the pre-commit hook never see it.
//! * **Opt-out.** The `--no-update-check` flag, `--quiet`, or any of
//!   `CGG_NO_UPDATE_CHECK` / `DO_NOT_TRACK` / `CI` in the environment
//!   disables it entirely.
//! * **Non-blocking.** [`spawn`] kicks off a background thread that
//!   overlaps the analysis; [`finish`] joins it at the very end. The
//!   common case touches only a local cache file — the network is hit at
//!   most once per 24h.
//! * **Fail-silent.** Any error (offline, DNS, 403, 404 before the first
//!   release, parse) is swallowed; it never changes exit code, stdout,
//!   or the analysis result.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const REPO: &str = "NeuralNotwerk/cgg";
const INTERVAL_SECS: u64 = 24 * 60 * 60;
const HTTP_TIMEOUT_SECS: u64 = 2;

/// Cached result of the last check: when we last looked and the newest
/// version we saw. Lets every run inside the 24h window decide whether to
/// nag from this file alone, with no network call.
#[derive(Serialize, Deserialize, Default)]
struct State {
    /// Unix seconds of the last check attempt (success *or* failure, so a
    /// persistently-offline machine backs off instead of retrying hourly).
    last_check: u64,
    /// Newest version string observed (no leading `v`).
    latest_seen: String,
}

/// Kick off the background version check. Returns `None` — doing nothing,
/// spawning no thread, touching no network — unless this is an
/// interactive run a human would actually want a notice on. `disabled`
/// folds in the `--no-update-check` / `--quiet` flags from the caller.
pub fn spawn(disabled: bool) -> Option<JoinHandle<Option<String>>> {
    let disabled = disabled
        || std::env::var_os("CGG_NO_UPDATE_CHECK").is_some()
        || std::env::var_os("DO_NOT_TRACK").is_some()
        || std::env::var_os("CI").is_some()
        || !std::io::stderr().is_terminal();
    if disabled {
        return None;
    }
    Some(std::thread::spawn(check))
}

/// Join the background check and print the upgrade notice to stderr, if
/// any. Cheap unless the 24h network refresh came due this run, in which
/// case it waits at most `HTTP_TIMEOUT_SECS` (and only after the analysis
/// has already finished). Always called last so the notice trails the
/// run summary.
pub fn finish(handle: Option<JoinHandle<Option<String>>>) {
    if let Some(h) = handle {
        if let Ok(Some(msg)) = h.join() {
            eprintln!("{msg}");
        }
    }
}

/// Body of the background thread: consult the cache, refresh over the
/// network at most once per 24h, persist the result, and return the
/// notice string when a newer version exists.
fn check() -> Option<String> {
    let current = env!("CARGO_PKG_VERSION");
    let path = state_path();
    let now = now_secs();

    // Inside the throttle window: decide from the cache, no network.
    if let Some(ref p) = path {
        if let Some(state) = read_state(p) {
            if now.saturating_sub(state.last_check) < INTERVAL_SECS {
                return notice(current, &state.latest_seen);
            }
        }
    }

    // Refresh. Record the attempt time regardless of outcome so an
    // offline machine doesn't re-hit the network every single run.
    let fetched = fetch_latest();
    if let Some(ref p) = path {
        let latest_seen = fetched.clone().unwrap_or_else(|| current.to_string());
        let _ = write_state(p, &State { last_check: now, latest_seen });
    }
    notice(current, &fetched?)
}

/// GET the repo's latest release tag. `None` on any failure, including a
/// 404 before the first release is published.
fn fetch_latest() -> Option<String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = minreq::get(url)
        .with_header(
            "User-Agent",
            concat!("cgg/", env!("CARGO_PKG_VERSION")),
        )
        .with_header("Accept", "application/vnd.github+json")
        .with_timeout(HTTP_TIMEOUT_SECS)
        .send()
        .ok()?;
    if resp.status_code != 200 {
        return None;
    }
    let body = resp.as_str().ok()?;
    let json: serde_json::Value = serde_json::from_str(body).ok()?;
    let tag = json.get("tag_name")?.as_str()?;
    Some(tag.trim().trim_start_matches('v').to_string())
}

fn notice(current: &str, latest: &str) -> Option<String> {
    if is_newer(latest, current) {
        Some(format!(
            "\ncgg {latest} is available (you have {current}). \
             Update: cargo install --git https://github.com/{REPO} cgg\n\
             (silence this with CGG_NO_UPDATE_CHECK=1)"
        ))
    } else {
        None
    }
}

/// `true` if `a` is a strictly newer dotted-numeric version than `b`.
/// Pre-release / build suffixes are ignored; non-numeric parts read as 0.
fn is_newer(a: &str, b: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.split(['-', '+'])
            .next()
            .unwrap_or(v)
            .split('.')
            .map(|p| p.trim().parse().unwrap_or(0))
            .collect()
    }
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let x = a.get(i).copied().unwrap_or(0);
        let y = b.get(i).copied().unwrap_or(0);
        if x != y {
            return x > y;
        }
    }
    false
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Machine-global cache location (`$XDG_CACHE_HOME/cgg/update-check.json`,
/// not the per-project `.cgg-cache`), so the once-a-day budget is shared
/// across every project on the machine.
fn state_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))?;
    let dir = base.join("cgg");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("update-check.json"))
}

fn read_state(p: &PathBuf) -> Option<State> {
    serde_json::from_slice(&std::fs::read(p).ok()?).ok()
}

/// Write via a temp file + rename so a process exiting mid-write never
/// leaves a half-written cache.
fn write_state(p: &PathBuf, s: &State) -> std::io::Result<()> {
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(s).unwrap_or_default())?;
    std::fs::rename(&tmp, p)
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn newer_detection() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        // pre-release / build suffixes are ignored
        assert!(!is_newer("0.2.0-rc1", "0.2.0"));
        assert!(is_newer("0.2.0", "0.1.9-beta"));
    }
}
