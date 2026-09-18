//! `--since` shells out to `git` inside the tree being analyzed, and that
//! tree is not trusted. Two things the untrusted side must never get:
//!
//! * **A git option.** The revspec is a positional argument. Before 0.8.5
//!   it was passed bare, so a value beginning with `-` was parsed by git
//!   as an option — `--since=--output=/some/path` made `git diff` write
//!   that file. `since` is also a Python/Node/C option, so a service
//!   forwarding a caller's revspec inherited the injection.
//! * **A command to run.** A checked-out repository carries its own
//!   `.git/config`, and git executes commands it names: `core.fsmonitor`
//!   runs on a plain `git diff`, and a `.gitattributes` `diff=<driver>`
//!   plus `diff.<driver>.command` runs the driver. Pointing `--since` at an
//!   untrusted checkout was arbitrary code execution.
//!
//! Each test builds a repository that tries one of these and asserts the
//! sentinel it would have left behind does not exist.

use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let mut c = Command::new("git");
    c.current_dir(dir);
    // Git exports these to every process it spawns, and they outrank
    // `current_dir`; the repo's own pre-commit hook runs this suite, so
    // without stripping them the fixture would act on the REAL repo.
    for v in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_NAMESPACE",
        "GIT_PREFIX",
    ] {
        c.env_remove(v);
    }
    let out = c.args(args).output().expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with one committed Python file and an uncommitted edit,
/// so `--since HEAD` has exactly one changed callable to find.
fn repo_with_a_change(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main", "."]);
    git(dir, &["config", "user.email", "t@example.com"]);
    git(dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.py"), "def a():\n    pass\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
    std::fs::write(dir.join("a.py"), "def a():\n    return 1\n").unwrap();
}

fn cgg(dir: &Path, since: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cgg"))
        .arg(dir)
        .arg(format!("--since={since}"))
        .arg("-q")
        .output()
        .expect("cgg runs")
}

#[test]
fn a_revspec_beginning_with_a_dash_is_never_a_git_option() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    repo_with_a_change(d);

    // Outside the repo, so a successful injection is unmistakable.
    let sentinel = tempfile::tempdir().unwrap();
    let target = sentinel.path().join("PWNED");

    let out = cgg(d, &format!("--output={}", target.display()));

    assert!(
        !target.exists(),
        "git's --output option was reachable through the revspec"
    );
    assert!(
        !out.status.success(),
        "an invalid revspec is an error, not a silent empty result"
    );
}

#[test]
fn since_does_not_run_commands_from_the_repos_own_config() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    repo_with_a_change(d);

    let sentinel = tempfile::tempdir().unwrap();
    let fsmonitor_hit = sentinel.path().join("HIT_FSMONITOR");
    let driver_hit = sentinel.path().join("HIT_DIFF_DRIVER");

    // Both command-execution paths a checkout can carry: the fsmonitor
    // hook, run by any index-reading command, and a per-path diff
    // driver, run when diffing a file the attributes route to it.
    git(
        d,
        &[
            "config",
            "core.fsmonitor",
            &format!("touch {}; true", fsmonitor_hit.display()),
        ],
    );
    std::fs::write(d.join(".gitattributes"), "a.py diff=evil\n").unwrap();
    git(
        d,
        &[
            "config",
            "diff.evil.command",
            &format!("sh -c 'touch {}; true'", driver_hit.display()),
        ],
    );

    let out = cgg(d, "HEAD");

    assert!(
        !fsmonitor_hit.exists(),
        "the repo's core.fsmonitor command was executed"
    );
    assert!(
        !driver_hit.exists(),
        "the repo's diff driver command was executed"
    );
    // Hardening must not cost the feature: the change is still found.
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("1 callable seed(s)"),
        "--since HEAD still seeds the changed callable: {stderr}"
    );
}
