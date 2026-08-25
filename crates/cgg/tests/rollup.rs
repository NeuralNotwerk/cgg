//! End-to-end coverage for `--rollup`, `--rollup-by` and `--from-graph`.
//!
//! The unit tests in `src/rollup.rs` and `src/replay.rs` cover the
//! folding rules on hand-built graphs. These cover the parts only a real
//! run exercises: that the flags reach the pipeline, that the artifact
//! says what happened, that a rolled-up graph is still valid in every
//! output format, and that a replay of a saved graph and a fresh
//! analysis of the same tree agree byte for byte.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary built")
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// Two packages, three files, calls within and across each.
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "alpha/Cargo.toml",
        "[package]\nname = \"alpha\"\n",
    );
    write(
        tmp.path(),
        "alpha/src/one.rs",
        "pub struct P;\n\
         impl P {\n    pub fn new() -> P { P }\n    pub fn step(&self) { helper(); }\n}\n\
         pub fn helper() {}\n\
         pub fn drive() { let p = P::new(); p.step(); crate::two::far(); }\n",
    );
    write(
        tmp.path(),
        "alpha/src/two.rs",
        "pub fn far() { super::one::helper(); }\n",
    );
    write(
        tmp.path(),
        "beta/Cargo.toml",
        "[package]\nname = \"beta\"\n",
    );
    write(
        tmp.path(),
        "beta/src/lib.rs",
        "pub fn entry() { crate::one::helper(); }\npub fn other() { entry(); }\n",
    );
    // Bulk, so folding actually pays. On a handful of callables the
    // three-line banner and the per-node member tags cost more than the
    // grouping saves, and `--rollup` correctly declines to make the
    // output bigger — which means a small fixture tests the guard, not
    // the feature.
    let mut bulk = String::from("use crate::one::helper;\n");
    for i in 0..60 {
        bulk.push_str(&format!(
            "pub fn generated_worker_number_{i}() {{ helper(); crate::two::far(); }}\n"
        ));
    }
    write(tmp.path(), "alpha/src/bulk.rs", &bulk);
    tmp
}

fn run(dir: &Path, args: &[&str]) -> (String, String) {
    let out = cgg().arg(dir).args(args).output().expect("cgg runs");
    assert!(out.status.success(), "cgg failed: {out:?}");
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

// ---------------------------------------------------------------------
// --rollup-by
// ---------------------------------------------------------------------

#[test]
fn rollup_by_file_yields_one_node_per_file() {
    let tmp = fixture();
    let (stdout, stderr) = run(tmp.path(), &["--rollup-by", "file"]);
    assert!(stderr.contains("ROLLED UP to `file`"), "{stderr}");
    for f in ["alpha/src/one.rs", "alpha/src/two.rs", "beta/src/lib.rs"] {
        assert!(stdout.contains(f), "missing {f} in:\n{stdout}");
    }
    // Group nodes are named for the group, so no callable name survives.
    assert!(!stdout.contains("::helper"), "{stdout}");
}

#[test]
fn rollup_by_package_finds_the_manifest_directories() {
    let tmp = fixture();
    let (stdout, stderr) = run(tmp.path(), &["--rollup-by", "package"]);
    assert!(stderr.contains("ROLLED UP to `package`"), "{stderr}");
    assert!(stdout.contains(r#"["alpha"#), "{stdout}");
    assert!(stdout.contains(r#"["beta"#), "{stdout}");
}

#[test]
fn a_rolled_up_diagram_states_that_it_is_one() {
    // The banner has to survive copy-paste of the mermaid block: the
    // group keys read as ordinary module paths, so nothing else in the
    // artifact says it is not the full call graph.
    let tmp = fixture();
    let (stdout, _) = run(tmp.path(), &["--rollup-by", "file"]);
    assert!(stdout.contains("%% cgg: ROLLED UP"), "{stdout}");
    assert!(stdout.contains("not the full call graph"), "{stdout}");
}

#[test]
fn the_member_count_is_visible_in_the_diagram() {
    let tmp = fixture();
    let (stdout, _) = run(tmp.path(), &["--rollup-by", "file"]);
    assert!(stdout.contains("fns"), "member counts missing:\n{stdout}");
}

#[test]
fn intra_group_calls_do_not_become_self_loops() {
    let tmp = fixture();
    let (stdout, _) = run(tmp.path(), &["--rollup-by", "package"]);
    for line in stdout.lines().filter(|l| l.contains("-->")) {
        let parts: Vec<&str> = line.split("-->").collect();
        let src = parts[0].trim();
        let dst = parts[1].rsplit('|').next().unwrap_or(parts[1]).trim();
        assert_ne!(src, dst, "self-loop in a rolled-up graph: {line}");
    }
}

#[test]
fn an_unknown_level_is_rejected_with_the_valid_ones_named() {
    let tmp = fixture();
    cgg()
        .arg(tmp.path())
        .args(["--rollup-by", "modul"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("module"));
}

#[test]
fn every_output_format_renders_a_rolled_up_graph() {
    let tmp = fixture();
    for (fmt, marker) in [
        ("mermaid", "flowchart LR"),
        ("json", "\"rollup\""),
        ("dot", "digraph"),
        ("graphml", "<graphml"),
    ] {
        let (stdout, _) = run(tmp.path(), &["--rollup-by", "file", "-t", fmt]);
        assert!(stdout.contains(marker), "{fmt} output:\n{stdout}");
    }
    // The multiplicity a fold produced has to reach the formats that
    // claim to be faithful to call frequency.
    let (json, _) = run(tmp.path(), &["--rollup-by", "file", "-t", "json"]);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let weights: Vec<u64> = v["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e.get("weight").and_then(|w| w.as_u64()))
        .collect();
    assert!(
        weights.iter().any(|w| *w > 1),
        "a folded edge must carry the call count it stands for: {weights:?}"
    );
    // ...and an unfolded graph must not carry the key at all, so the
    // default JSON output is unchanged from before the field existed.
    let (plain, _) = run(tmp.path(), &["-t", "json"]);
    assert!(
        !plain.contains("\"weight\""),
        "weight leaked into a plain graph"
    );
}

// ---------------------------------------------------------------------
// --rollup (budget)
// ---------------------------------------------------------------------

#[test]
fn a_graph_under_budget_is_left_byte_identical() {
    // The property that makes `--rollup` safe to leave in a wrapper
    // script: under budget, it changes nothing at all.
    let tmp = fixture();
    let (plain, _) = run(tmp.path(), &[]);
    let (budgeted, stderr) = run(tmp.path(), &["--rollup", "10m"]);
    assert_eq!(plain, budgeted, "an unmet budget must not touch the graph");
    assert!(stderr.contains("not needed"), "{stderr}");
}

#[test]
fn a_tight_budget_rolls_up_and_says_so_even_under_quiet() {
    // `-q` silences every other advisory because the artifact still says
    // what it is. This one is the only thing distinguishing a graph of
    // your code from a graph of your directory layout.
    let tmp = fixture();
    let out = cgg()
        .arg(tmp.path())
        .args(["--rollup", "400", "-q"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("ROLLED UP"),
        "-q must not hide it: {stderr:?}"
    );
}

#[test]
fn an_unmeetable_budget_warns_instead_of_pretending() {
    let tmp = fixture();
    let (_, stderr) = run(tmp.path(), &["--rollup", "1"]);
    assert!(stderr.contains("could not be met"), "{stderr}");
    assert!(stderr.contains("OVER"), "{stderr}");
}

#[test]
fn the_budget_is_measured_against_the_selected_format() {
    // A graph is far larger as JSON than as mermaid, so the same budget
    // must engage for one and not the other. Without `rollup_format`
    // reaching the pipeline this passes by accident in mermaid and
    // silently under-rolls every `-t json` run.
    let tmp = fixture();
    // Derive the budget from what each format actually costs rather than
    // hard-coding one. A literal broke the moment the estimator was
    // recalibrated, and it broke by silently testing nothing — both
    // formats rolled up, so the assertion that they differ was the only
    // thing that noticed.
    let size = |args: &[&str]| -> u64 {
        let (_, err) = run(tmp.path(), args);
        err.split("renders to about ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(|| panic!("no size line in: {err}"))
    };
    let mermaid_cost = size(&["--rollup", "999m"]);
    let json_cost = size(&["--rollup", "999m", "-t", "json"]);
    assert!(
        json_cost > mermaid_cost,
        "the same graph must cost more as JSON ({json_cost}) than as          mermaid ({mermaid_cost}), or this test proves nothing"
    );
    // Between the two: mermaid fits, JSON does not.
    let budget = ((mermaid_cost + json_cost) / 2).to_string();
    let (_, mermaid) = run(tmp.path(), &["--rollup", &budget]);
    let (_, json) = run(tmp.path(), &["--rollup", &budget, "-t", "json"]);
    assert!(mermaid.contains("not needed"), "mermaid: {mermaid}");
    assert!(json.contains("ROLLED UP"), "json: {json}");
}

#[test]
fn a_budgeted_run_records_what_it_tried_in_the_audit() {
    let tmp = fixture();
    let out = tmp.path().join("g.mmd");
    cgg()
        .arg(tmp.path())
        .args(["--rollup", "400", "-o"])
        .arg(&out)
        .assert()
        .success();
    let audit = std::fs::read_to_string(tmp.path().join("g.mmd.audit.json")).unwrap();
    // The `json` audit document is a top-level array of events.
    let events: Vec<serde_json::Value> = serde_json::from_str(&audit).unwrap();
    let rolled = events
        .iter()
        .find(|e| e["event"] == "rolled_up")
        .expect("a rolled_up event");
    assert!(rolled["attempts"].as_array().unwrap().len() > 1);
    assert!(
        rolled["nodes_before"].as_u64().unwrap()
            > rolled["nodes_after"].as_u64().unwrap()
    );
}

#[test]
fn rollup_is_deterministic_at_every_thread_count() {
    // Grouping walks `callables` and `edges` in order and never iterates
    // a HashMap. `query.rs` shipped the opposite once, and it stayed
    // invisible until a cap started turning work away — a budget picking
    // a level is the same kind of trigger.
    let tmp = fixture();
    let first = run(tmp.path(), &["--rollup-by", "module", "--jobs", "1"]).0;
    for jobs in ["2", "4", "8"] {
        let got = run(tmp.path(), &["--rollup-by", "module", "--jobs", jobs]).0;
        assert_eq!(got, first, "rolled-up graph differs at --jobs {jobs}");
    }
}

// ---------------------------------------------------------------------
// --from-graph
// ---------------------------------------------------------------------

fn saved(tmp: &Path) -> std::path::PathBuf {
    let p = tmp.join("saved.json");
    cgg()
        .arg(tmp)
        .args(["-t", "json", "-o"])
        .arg(&p)
        .assert()
        .success();
    p
}

#[test]
fn a_replayed_rollup_matches_a_freshly_analyzed_one_byte_for_byte() {
    // The contract that makes `--from-graph` worth having: it is the
    // same pipeline tail over the same graph, so slicing a saved
    // document must not be a second, drifting implementation.
    let tmp = fixture();
    let json = saved(tmp.path());
    for level in ["type", "module", "file", "package", "dir:1", "language"] {
        let direct = run(tmp.path(), &["--rollup-by", level]).0;
        let out = cgg()
            .args(["--from-graph"])
            .arg(&json)
            .args(["--rollup-by", level])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            direct,
            "replay differs from a fresh analysis at `{level}`"
        );
    }
}

#[test]
fn a_replay_can_be_filtered_again() {
    let tmp = fixture();
    let json = saved(tmp.path());
    let out = cgg()
        .args(["--from-graph"])
        .arg(&json)
        .args(["--filter", "helper$", "-n", "1"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("helper"), "{stdout}");
    // `beta::entry` calls `helper` directly, so one hop reaches it;
    // `beta::other` calls `entry`, so it is two hops away and must not.
    assert!(
        stdout.contains("entry"),
        "1 hop must reach a direct caller:\n{stdout}"
    );
    assert!(
        !stdout.contains("::other"),
        "1 hop must not reach a two-hop caller:\n{stdout}"
    );
}

#[test]
fn a_replay_takes_no_paths() {
    let tmp = fixture();
    let json = saved(tmp.path());
    cgg()
        .arg(tmp.path())
        .args(["--from-graph"])
        .arg(&json)
        .assert()
        .failure()
        .stderr(predicate::str::contains("takes no paths"));
}

#[test]
fn a_replay_refuses_options_it_cannot_honour() {
    // Silently emitting an ordinary graph is the failure mode
    // `--write-roots` used to have. Refusing names the reason.
    let tmp = fixture();
    let json = saved(tmp.path());
    for flag in ["--dead-code", "--include-external", "--dynamic-dispatch"] {
        let out = cgg()
            .args(["--from-graph"])
            .arg(&json)
            .arg(flag)
            .output()
            .unwrap();
        assert!(!out.status.success(), "{flag} should be refused");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stderr.contains("replayed graph"),
            "{flag} error was: {stderr}"
        );
    }
}

#[test]
fn the_json_document_carries_its_schema_and_version() {
    let tmp = fixture();
    let (stdout, _) = run(tmp.path(), &["-t", "json"]);
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["schema"], "cgg.graph.v1");
    assert_eq!(v["cgg_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn a_document_from_a_different_schema_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let p = tmp.path().join("g.json");
    std::fs::write(&p, r#"{"schema":"cgg.graph.v99","callables":{}}"#).unwrap();
    cgg()
        .args(["--from-graph"])
        .arg(&p)
        .assert()
        .failure()
        .stderr(predicate::str::contains("cgg.graph.v99"));
}

#[test]
fn replaying_an_already_filtered_document_warns() {
    // The one thing a replay genuinely cannot do. A caller who does not
    // know reads a partial answer as a complete one.
    let tmp = fixture();
    let p = tmp.path().join("narrow.json");
    cgg()
        .arg(tmp.path())
        .args(["--filter", "helper$", "-t", "json", "-o"])
        .arg(&p)
        .assert()
        .success();
    let out = cgg().args(["--from-graph"]).arg(&p).output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("already filtered"), "{stderr}");
}

// ---------------------------------------------------------------------
// --rollup x --node-ids
// ---------------------------------------------------------------------

/// The budget must be measured against the renderer that will actually
/// run, node-id scheme included.
///
/// `--node-ids hash` puts a ten-character base36 id in place of an
/// ordinal, on every node and again on both ends of every edge, which is
/// about a third more document. A budget measured against the numbered
/// form while the hashed form is emitted lets the artifact sail past the
/// budget the user set — silently, because the stderr line reports the
/// figure it measured. So: pick a budget that sits strictly between the
/// two renderings of the same graph, and the schemes must disagree about
/// whether it fits.
#[test]
fn the_rollup_budget_follows_the_node_id_scheme() {
    let tmp = fixture();
    let opts = cgg::RunOptions {
        paths: vec![tmp.path().to_path_buf()],
        ..Default::default()
    };
    let graph = cgg::analyze(&opts).expect("analysis").graph;

    let tokens = |ids| {
        cgg::rollup::estimate_tokens(&cgg::emit::graph_to_string_with(
            &graph,
            cgg::OutputFormat::Mermaid,
            ids,
        ))
    };
    let short = tokens(cgg_format::NodeIds::Short);
    let hash = tokens(cgg_format::NodeIds::Hash);
    assert!(
        short < hash,
        "numbering must be the smaller rendering: {short} vs {hash}"
    );

    // Strictly between the two: the numbered graph fits, the hashed one
    // does not.
    let budget = (short + hash) / 2;
    assert!(
        short < budget && budget < hash,
        "{short} < {budget} < {hash}"
    );
    let budget = budget.to_string();

    let (_, short_err) = run(tmp.path(), &["--rollup", &budget, "--node-ids", "short"]);
    assert!(
        !short_err.contains("ROLLED UP"),
        "the numbered graph fits in {budget} and must not fold:\n{short_err}"
    );

    let (_, hash_err) = run(tmp.path(), &["--rollup", &budget, "--node-ids", "hash"]);
    assert!(
        hash_err.contains("ROLLED UP"),
        "the hashed graph exceeds {budget} and must fold:\n{hash_err}"
    );
}
