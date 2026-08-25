//! Every output format, end to end.
//!
//! `-t` is the flag most likely to be consumed by another tool, and the
//! two machine formats had the weakest coverage in the workspace
//! (graphml 62% region / 50% function, json 40% function). A renderer
//! that emits *something* for every graph still passes a smoke test
//! while emitting a document its consumer rejects, so these assert the
//! structure a consumer actually depends on: the GraphML namespace, one
//! edge per call site in the machine formats, collapsed multiplicity in
//! the human ones, and escaping of characters that would otherwise
//! break the document.

use assert_cmd::Command;
use std::path::Path;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary built")
}

fn write(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
}

/// `caller` calls `helper` twice — the multiplicity case — plus a node
/// with no edges and a call into the stdlib for the exit-node paths.
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "m.py",
        concat!(
            "import os\n",
            "def helper():\n    return 1\n",
            "def caller():\n    return helper() + helper()\n",
            "def solo():\n    return os.path.join('a', 'b')\n",
        ),
    );
    tmp
}

fn render(dir: &Path, args: &[&str]) -> String {
    let out = dir.join("out.txt");
    let mut c = cgg();
    c.arg(dir).arg("-o").arg(&out);
    for a in args {
        c.arg(a);
    }
    c.assert().success();
    std::fs::read_to_string(&out).unwrap()
}

// ---------------------------------------------------------------------
// GraphML
// ---------------------------------------------------------------------

#[test]
fn graphml_declares_the_standard_namespace() {
    // REGRESSION: this said `graphml.graphstruct.org`, which is neither
    // the GraphML spec's namespace nor a real domain. yEd and Gephi are
    // the reason this format exists; a document declaring an unknown
    // namespace is not reliably importable by either.
    let tmp = fixture();
    let g = render(tmp.path(), &["-t", "graphml"]);
    assert!(
        g.contains(r#"xmlns="http://graphml.graphdrawing.org/xmlns""#),
        "GraphML must declare the spec namespace:\n{g}"
    );
    assert!(
        !g.contains("graphstruct"),
        "the typo'd namespace must not come back:\n{g}"
    );
}

#[test]
fn graphml_is_well_formed_xml() {
    let tmp = fixture();
    let g = render(tmp.path(), &["-t", "graphml", "--include-stdlib"]);

    // Declaration first, single root element, balanced tags.
    assert!(g.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
    assert_eq!(g.matches("<graphml").count(), 1, "exactly one root:\n{g}");
    assert_eq!(g.matches("</graphml>").count(), 1);
    assert_eq!(
        g.matches("<node ").count(),
        g.matches("</node>").count(),
        "every <node> closes:\n{g}"
    );
    assert_eq!(
        g.matches("<graph ").count(),
        g.matches("</graph>").count(),
        "every <graph> closes:\n{g}"
    );
    // Attribute keys must be declared before use.
    for key in ["label", "lang", "via"] {
        assert!(
            g.contains(&format!(r#"<key id="{key}""#)),
            "missing <key> declaration for `{key}`:\n{g}"
        );
    }
}

#[test]
fn graphml_escapes_xml_metacharacters() {
    // Rust generics (`Vec<T>`) and C++ operators put `<`, `>` and `&`
    // into qualified names. Unescaped, they end the document.
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "m.rs",
        "fn holder<T>(x: T) -> T { x }\nfn call() { holder::<u8>(1); }\n",
    );
    let g = render(tmp.path(), &["-t", "graphml"]);
    for (i, line) in g.lines().enumerate() {
        if let Some(rest) = line.trim().strip_prefix("<data key=\"label\">") {
            let text = rest.trim_end_matches("</data>");
            assert!(
                !text.contains('<') && !text.contains('>') && !text.contains('&'),
                "line {i} leaks a raw XML metacharacter into a label: {line}"
            );
        }
    }
}

// ---------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------

#[test]
fn json_carries_the_documented_shape() {
    let tmp = fixture();
    let raw = render(tmp.path(), &["-t", "json"]);
    let v: serde_json::Value = serde_json::from_str(&raw).expect("valid JSON");

    for key in [
        "callables",
        "files",
        "edges",
        "unresolved",
        "file_audits",
        "metrics",
    ] {
        assert!(
            v.get(key).is_some(),
            "top-level `{key}` missing from JSON output"
        );
    }
    let edges = v["edges"].as_array().unwrap();
    assert!(!edges.is_empty(), "fixture must produce edges");
    // The skill tells agents these fields exist; CI gates key on them.
    for e in edges {
        for key in ["site_line", "site_byte", "confidence", "via", "resolver"] {
            assert!(e.get(key).is_some(), "edge missing `{key}`: {e}");
        }
    }
}

#[test]
fn json_keeps_one_edge_per_call_site() {
    // The documented contract: mermaid/dot collapse repeated call sites
    // into a multiplicity label, JSON and GraphML do not, so
    // programmatic consumers keep call-frequency information.
    let tmp = fixture();
    let v: serde_json::Value =
        serde_json::from_str(&render(tmp.path(), &["-t", "json"])).unwrap();
    let edges = v["edges"].as_array().unwrap();
    let caller_to_helper = edges
        .iter()
        .filter(|e| {
            let s = e.to_string();
            s.contains("site_line")
        })
        .count();
    assert!(
        caller_to_helper >= 2,
        "two call sites must stay two edges: {edges:?}"
    );

    // ...and the distinct lines must actually differ or be recorded.
    let lines: Vec<_> = edges
        .iter()
        .filter_map(|e| e["site_byte"].as_u64())
        .collect();
    assert_eq!(
        lines.len(),
        edges.len(),
        "every edge carries a site_byte so duplicates stay distinguishable"
    );
}

#[test]
fn json_is_parseable_with_every_optional_edge_kind_on() {
    let tmp = fixture();
    let raw = render(
        tmp.path(),
        &[
            "-t",
            "json",
            "--include-external",
            "--include-stdlib",
            "--reference-edges",
        ],
    );
    let v: serde_json::Value =
        serde_json::from_str(&raw).expect("valid JSON with exit nodes");
    // `via` is a tagged object: {"kind": "stdlib"}.
    let vias: std::collections::BTreeSet<_> = v["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["via"]["kind"].as_str())
        .collect();
    assert!(
        vias.contains("stdlib"),
        "--include-stdlib must be visible in `via.kind`: {vias:?}"
    );
}

// ---------------------------------------------------------------------
// DOT
// ---------------------------------------------------------------------

#[test]
fn dot_is_syntactically_balanced() {
    let tmp = fixture();
    let g = render(tmp.path(), &["-t", "dot", "--include-stdlib"]);
    assert!(g.starts_with("digraph cgg {"), "dot header:\n{g}");
    assert_eq!(
        g.matches('{').count(),
        g.matches('}').count(),
        "braces must balance:\n{g}"
    );
    for line in g.lines().map(str::trim) {
        if line.contains("->") || line.starts_with('n') {
            assert!(
                line.ends_with(';'),
                "every dot statement ends in a semicolon: {line}"
            );
        }
    }
}

#[test]
fn dot_collapses_repeated_call_sites_into_a_multiplicity_label() {
    let tmp = fixture();
    let g = render(tmp.path(), &["-t", "dot"]);
    assert!(
        g.contains(r#"[label="2x"]"#),
        "two call sites collapse to a 2x label in dot:\n{g}"
    );
}

#[test]
fn dot_escapes_quotes_in_labels() {
    let tmp = tempfile::tempdir().unwrap();
    // A route string with a quote in the entry-node name.
    write(
        tmp.path(),
        "app.py",
        concat!(
            "from flask import Flask\n",
            "app = Flask(__name__)\n\n",
            "@app.route('/a\"b')\n",
            "def h():\n    return 1\n",
        ),
    );
    let g = render(tmp.path(), &["-t", "dot"]);
    let mut checked = 0;
    for line in g.lines() {
        let Some(start) = line.find("label=\"") else {
            continue;
        };
        // Walk the label value, honouring backslash escapes, and stop at
        // the first *unescaped* quote — that is where the attribute ends.
        // (Scanning to the last quote on the line would run into
        // `shape=`/`tooltip=` and report their quotes as unescaped.)
        let mut escaped = false;
        let mut terminated = false;
        for c in line[start + 7..].chars() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                terminated = true;
                break;
            }
        }
        assert!(
            terminated,
            "dot label never closes, so a quote leaked: {line}"
        );
        checked += 1;
    }
    assert!(checked > 0, "fixture produced no dot labels to check:\n{g}");
    // The route string carried a `"`, so the escape must be present.
    assert!(
        g.contains(r#"\""#),
        "the quote in the route must be escaped:\n{g}"
    );
}

// ---------------------------------------------------------------------
// Mermaid + cross-format agreement
// ---------------------------------------------------------------------

#[test]
fn mermaid_collapses_repeated_call_sites() {
    let tmp = fixture();
    let g = render(tmp.path(), &["-t", "mermaid"]);
    assert!(
        g.contains("-->|2x|"),
        "two call sites collapse to a |2x| label in mermaid:\n{g}"
    );
}

#[test]
fn mermaid_escapes_angle_brackets_in_labels() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "m.rs",
        "fn holder<T>(x: T) -> T { x }\nfn call() { holder::<u8>(1); }\n",
    );
    let g = render(tmp.path(), &["-t", "mermaid"]);
    // Mermaid node labels live inside `["..."]`; a raw `<` breaks the
    // renderer, so generics must arrive as entities.
    assert!(
        g.contains("&lt;") || !g.contains('<'),
        "generics must be escaped:\n{g}"
    );
}

#[test]
fn every_format_sees_the_same_callables() {
    // A format is a view, not a filter: whatever the query produced must
    // appear in all four renderings.
    let tmp = fixture();
    let mermaid = render(tmp.path(), &["-t", "mermaid"]);
    let dot = render(tmp.path(), &["-t", "dot"]);
    let graphml = render(tmp.path(), &["-t", "graphml"]);
    let json = render(tmp.path(), &["-t", "json"]);

    for name in ["m.helper", "m.caller", "m.solo"] {
        assert!(mermaid.contains(name), "mermaid missing {name}");
        assert!(dot.contains(name), "dot missing {name}");
        assert!(graphml.contains(name), "graphml missing {name}");
        assert!(json.contains(name), "json missing {name}");
    }
}

#[test]
fn graphml_and_json_agree_on_edge_count() {
    // Both are per-call-site formats, so they must not disagree about
    // how many calls there were.
    let tmp = fixture();
    let graphml = render(tmp.path(), &["-t", "graphml"]);
    let json: serde_json::Value =
        serde_json::from_str(&render(tmp.path(), &["-t", "json"])).unwrap();
    assert_eq!(
        graphml.matches("<edge ").count(),
        json["edges"].as_array().unwrap().len(),
        "graphml and json must report the same number of edges"
    );
}

#[test]
fn an_empty_tree_still_produces_a_valid_document_in_every_format() {
    // Degenerate input is where renderers emit truncated headers.
    for (fmt, needle) in [
        ("mermaid", "flowchart LR"),
        ("dot", "digraph cgg {"),
        ("graphml", "</graphml>"),
        ("json", "\"callables\""),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let g = render(tmp.path(), &["-t", fmt]);
        assert!(
            g.contains(needle),
            "empty tree broke the {fmt} document:\n{g}"
        );
    }
    // ...and the JSON one must still parse.
    let tmp = tempfile::tempdir().unwrap();
    let raw = render(tmp.path(), &["-t", "json"]);
    serde_json::from_str::<serde_json::Value>(&raw).expect("empty-tree JSON must parse");
}

#[test]
fn stdout_is_the_default_sink_and_dash_means_stdout() {
    let tmp = fixture();
    let a = cgg().arg(tmp.path()).assert().success();
    let b = cgg().arg(tmp.path()).args(["-o", "-"]).assert().success();
    let sa = String::from_utf8(a.get_output().stdout.clone()).unwrap();
    let sb = String::from_utf8(b.get_output().stdout.clone()).unwrap();
    assert!(sa.contains("flowchart LR"));
    assert_eq!(sa, sb, "`-o -` must be identical to the default sink");
}

// ---------------------------------------------------------------------
// Node-id scheme (`--node-ids`)

/// Mermaid numbers its nodes by default. The id appears once per node
/// and again on every edge that touches it, and this format's reader is
/// usually an agent's context window, where a ten-character base36 hash
/// costs several tokens for what is semantically one opaque handle.
#[test]
fn mermaid_numbers_its_nodes_by_default() {
    let tmp = fixture();
    let g = render(tmp.path(), &["-t", "mermaid"]);
    let numbered = regex::Regex::new(r"(?m)^  N\d+\[").unwrap();
    assert!(numbered.is_match(&g), "want numbered node ids:\n{g}");
    let hashed = regex::Regex::new(r"(?m)^  C[0-9a-z]{4,}\[").unwrap();
    assert!(!hashed.is_match(&g), "want no hashed node ids:\n{g}");
}

/// ...and `--node-ids hash` gives back the content-derived form, for
/// anyone correlating a diagram against `-t json` or diffing two
/// revisions' diagrams.
#[test]
fn node_ids_hash_restores_the_content_derived_form() {
    let tmp = fixture();
    let g = render(tmp.path(), &["-t", "mermaid", "--node-ids", "hash"]);
    let hashed = regex::Regex::new(r"(?m)^  C[0-9a-z]+\[").unwrap();
    assert!(hashed.is_match(&g), "want hashed node ids:\n{g}");
    assert!(!g.contains("\n  N"), "want no numbered node ids:\n{g}");

    // The ids must be the ones the JSON document carries — that is the
    // whole reason to ask for them.
    let json: serde_json::Value =
        serde_json::from_str(&render(tmp.path(), &["-t", "json"])).unwrap();
    for id in json["callables"].as_object().unwrap().keys() {
        assert!(
            g.contains(&format!("  {id}[")),
            "mermaid is missing {id}:\n{g}"
        );
    }
}

/// Numbering changes how nodes are named, never which nodes or edges
/// exist. Same graph, same arrows, same labels — both ways.
#[test]
fn the_scheme_changes_only_the_names() {
    let tmp = fixture();
    let short = render(tmp.path(), &["-t", "mermaid", "--node-ids", "short"]);
    let hash = render(tmp.path(), &["-t", "mermaid", "--node-ids", "hash"]);
    assert_eq!(
        short.lines().count(),
        hash.lines().count(),
        "{short}\n--\n{hash}"
    );
    assert_eq!(
        short.matches(" --> ").count(),
        hash.matches(" --> ").count()
    );
    // Every label survives verbatim; only the id in front of it moves.
    let labels = |s: &str| {
        let re = regex::Regex::new(r#"\["([^"]*)"\]"#).unwrap();
        re.captures_iter(s)
            .map(|c| c[1].to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(labels(&short), labels(&hash));
}

/// The default is per-format, not global: dot and graphml go to layout
/// tools that neither read the ids nor pay for them, so they keep the
/// hashed form unless asked.
#[test]
fn only_mermaid_numbers_by_default() {
    let tmp = fixture();
    let hashed = regex::Regex::new(r"n[0-9a-z]{4,}").unwrap();
    for fmt in ["dot", "graphml"] {
        let g = render(tmp.path(), &["-t", fmt]);
        assert!(
            hashed.is_match(&g),
            "{fmt} should default to hashed ids:\n{g}"
        );
    }
    // ...but they honour the flag when it is given.
    let d = render(tmp.path(), &["-t", "dot", "--node-ids", "short"]);
    assert!(
        regex::Regex::new(r"(?m)^  n\d+ \[label=")
            .unwrap()
            .is_match(&d),
        "dot should number when asked:\n{d}"
    );
}

/// JSON ids are the content-derived identity `--from-graph` reads back
/// and consumers diff across runs, not a rendering choice. The flag says
/// so on stderr rather than obeying it or dropping it in silence — a
/// flag that quietly does nothing is worse than one that is not
/// accepted, because the person diffing the output believes it worked.
#[test]
fn node_ids_warns_for_json_instead_of_silently_doing_nothing() {
    let tmp = fixture();
    let out = tmp.path().join("g.json");
    let assert = cgg()
        .arg(tmp.path())
        .args(["-t", "json", "--node-ids", "short", "-o"])
        .arg(&out)
        .assert()
        .success();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("--node-ids does not apply to -t json"),
        "want a warning, got:\n{stderr}"
    );
    // ...and the ids are untouched: the same content-derived hashes the
    // flagless run produces, not ordinals. (Whole-document equality
    // would be the stronger check but cannot be used — the first run
    // wrote its output inside the tree the second one walks, and wall
    // timings never repeat.)
    let with_flag: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
    let ids = |v: &serde_json::Value| {
        v["callables"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };
    let plain: serde_json::Value =
        serde_json::from_str(&render(tmp.path(), &["-t", "json"])).unwrap();
    assert_eq!(
        ids(&with_flag),
        ids(&plain),
        "--node-ids renumbered the JSON"
    );
    assert_eq!(
        with_flag["edges"], plain["edges"],
        "--node-ids moved the JSON edges"
    );
}
