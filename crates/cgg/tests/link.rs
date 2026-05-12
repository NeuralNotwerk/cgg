//! Task 5 integration tests: intra-file linker + mermaid output.

use assert_cmd::Command;
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::TempDir;

fn cgg() -> Command {
    Command::cargo_bin("cgg").expect("cgg binary built")
}

fn write(dir: &Path, name: &str, body: &[u8]) {
    let p = dir.join(name);
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::File::create(&p).unwrap().write_all(body).unwrap();
}

#[test]
fn rust_intra_file_edges_emit_mermaid() {
    let tmp = TempDir::new().unwrap();
    let src = r#"
fn start() { middle(); }
fn middle() { end(); }
fn end() {}
"#;
    write(tmp.path(), "a.rs", src.as_bytes());
    let mmd = tmp.path().join("g.mmd");
    let audit = tmp.path().join("g.jsonl");

    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .args(["--audit-format", "jsonl", "--metrics"])
        .arg(&audit)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.starts_with("flowchart LR\n"));
    // Three callables -> three C<n> nodes.
    assert!(g.contains("C0"));
    assert!(g.contains("C1"));
    assert!(g.contains("C2"));
    // Two edges: start -> middle, middle -> end.
    let arrow_count = g.matches(" --> ").count();
    assert_eq!(arrow_count, 2, "mermaid:\n{g}");

    // Audit should record three callables and two edges total.
    let audit_text = fs::read_to_string(&audit).unwrap();
    assert!(audit_text.contains("\"callables\":"));
    assert!(audit_text.contains("\"edges\":2"));
}

#[test]
fn recursion_preserves_self_edge() {
    let tmp = TempDir::new().unwrap();
    let src = r#"
fn fib(n: u32) -> u32 { if n < 2 { n } else { fib(n - 1) + fib(n - 2) } }
"#;
    write(tmp.path(), "r.rs", src.as_bytes());
    let mmd = tmp.path().join("r.mmd");

    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    // One node, two self-edges preserved (cycle not removed).
    assert!(g.contains("fib"));
    assert!(g.contains("C0 --> C0"), "want self-edge:\n{g}");
}

#[test]
fn python_intra_file_method_to_method() {
    let tmp = TempDir::new().unwrap();
    let src = r#"
class S:
    def handle(self):
        self._process()
    def _process(self):
        pass
"#;
    write(tmp.path(), "svc.py", src.as_bytes());
    let mmd = tmp.path().join("svc.mmd");
    let audit = tmp.path().join("svc.jsonl");

    cgg()
        .args(["-t", "mermaid", "-o"])
        .arg(&mmd)
        .args(["--audit-format", "jsonl", "--metrics"])
        .arg(&audit)
        .arg(tmp.path())
        .assert()
        .success();

    let g = fs::read_to_string(&mmd).unwrap();
    assert!(g.contains("svc.S.handle"));
    assert!(g.contains("svc.S._process"));
    assert!(g.contains(" --> "));
}

#[test]
fn unresolved_reference_shows_up_in_audit() {
    let tmp = TempDir::new().unwrap();
    // `mystery` is never defined; the linker records it as unresolved.
    let src = "fn main() { mystery(); }\n";
    write(tmp.path(), "u.rs", src.as_bytes());
    let audit = tmp.path().join("u.jsonl");

    cgg()
        .args(["--audit-format", "jsonl", "--metrics"])
        .arg(&audit)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&audit).unwrap();
    // `mystery` is not defined in the scanned files, so it's external.
    assert!(text.contains("\"name\":\"mystery\""));
    assert!(text.contains("external_calls"));
}

#[test]
fn metrics_count_edges_and_confidence() {
    let tmp = TempDir::new().unwrap();
    let src = r#"
fn a() { b(); }
fn b() {}
"#;
    write(tmp.path(), "m.rs", src.as_bytes());
    let audit = tmp.path().join("m.jsonl");

    cgg()
        .args(["--audit-format", "jsonl", "--metrics"])
        .arg(&audit)
        .arg(tmp.path())
        .assert()
        .success();

    let text = fs::read_to_string(&audit).unwrap();
    // Exactly one high-confidence intra-file edge.
    assert!(text.contains("\"confidence_histogram\":{\"high\":1,\"medium\":0,\"low\":0}"));
    assert!(text.contains("\"edges\":1"));
    assert!(text.contains("\"callables\":2"));
}
