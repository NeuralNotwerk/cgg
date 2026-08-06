//! Language detection.
//!
//! Rules, applied top-to-bottom; the first rule to name a language
//! wins:
//!
//! 1. **Shebang** — if the file starts with `#!` and the first line
//!    contains a substring matching any plugin's registered shebang
//!    keyword, that language is chosen. `detected_via = "shebang:<word>"`.
//! 2. **Structured API descriptors** — for `.yaml`/`.yml`/`.json`, sniff
//!    the head for a root `openapi:`/`swagger:`/`asyncapi:` key and route
//!    to the OpenAPI or AsyncAPI plugin. No match → `Unknown` (these
//!    extensions never fall through to the extension rule, so ordinary
//!    config/data YAML/JSON is left alone). `detected_via = "content:<id>"`.
//! 3. **Extension** — case-insensitive match against each plugin's
//!    extension list. `detected_via = "extension:<ext>"`.
//! 3. **`.h` ambiguity** — if the extension is `.h`, look for a
//!    sibling file with the same stem and a C++ extension
//!    (`.cpp`, `.cc`, `.cxx`, `.hpp`, `.hh`, `.hxx`) inside the same
//!    directory. If present, pick C++; else C.
//!    `detected_via = "header-heuristic:cpp"` or `"header-heuristic:c"`.
//! 4. **Unknown** — returns [`DetectVerdict::Unknown`], which callers
//!    translate to `SkipReason::UnknownExtension` in the audit log.

use std::fs;
use std::path::Path;

use crate::PluginRegistry;

/// Result of running the detector on a single path.
#[derive(Debug, Clone)]
pub struct DetectResult {
    pub verdict: DetectVerdict,
    /// Human-readable label for the audit log, e.g. `"extension:.py"`,
    /// `"shebang:python3"`, `"header-heuristic:cpp"`.
    pub detected_via: String,
}

/// Verdict portion of [`DetectResult`].
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DetectVerdict {
    /// Plugin id (`"rust"`, `"python"`, `"cpp"`, …) that should own
    /// the file.
    Language(&'static str),
    /// No plugin claimed the file.
    Unknown,
}

#[derive(Debug)]
pub struct LanguageDetector<'r> {
    registry: &'r PluginRegistry,
}

impl<'r> LanguageDetector<'r> {
    pub fn new(registry: &'r PluginRegistry) -> Self {
        Self { registry }
    }

    /// Detect the language of a file at `path`. The function may read
    /// the first line of the file for shebang checks and may `readdir`
    /// the parent directory for `.h` disambiguation; neither opens the
    /// full file.
    pub fn detect(&self, path: &Path) -> DetectResult {
        // --- Rule 1: shebang --------------------------------------------------
        if let Some(word) = read_shebang(path) {
            for plugin in self.registry.all() {
                for &needle in plugin.shebangs() {
                    if word.contains(needle) {
                        return DetectResult {
                            verdict: DetectVerdict::Language(plugin.id()),
                            detected_via: format!("shebang:{needle}"),
                        };
                    }
                }
            }
        }

        // --- Rule 1b: structured API descriptors -----------------------------
        // `.yaml` / `.yml` / `.json` carry no language by extension alone, so
        // sniff their content for a root `openapi:` / `swagger:` / `asyncapi:`
        // key (required, near the top of every such document). A match routes
        // to the OpenAPI or AsyncAPI plugin; anything else stays Unknown, so
        // ordinary config/data YAML/JSON is unaffected. This deliberately does
        // not fall through to the extension rule — no other plugin owns these.
        if let Some(ext) = extension(path) {
            let lower = ext.to_ascii_lowercase();
            if matches!(lower.as_str(), ".yaml" | ".yml" | ".json") {
                if let Some(id) = sniff_structured_descriptor(path) {
                    return DetectResult {
                        verdict: DetectVerdict::Language(id),
                        detected_via: format!("content:{id}"),
                    };
                }
                return DetectResult {
                    verdict: DetectVerdict::Unknown,
                    detected_via: "none".to_string(),
                };
            }
        }

        // --- Rule 2: extension ------------------------------------------------
        if let Some(ext) = extension(path) {
            // Case-sensitive match first (so `.C` stays C++).
            if let Some(lang) = self.match_ext(&ext) {
                // Rule 3: `.h` needs special handling, regardless of
                // whether extension matched C or C++ first.
                if ext.eq_ignore_ascii_case(".h") {
                    return header_verdict(path);
                }
                return DetectResult {
                    verdict: DetectVerdict::Language(lang),
                    detected_via: format!("extension:{ext}"),
                };
            }

            // Case-insensitive fall-back.
            let lower = ext.to_ascii_lowercase();
            if let Some(lang) = self.match_ext(&lower) {
                if lower == ".h" {
                    return header_verdict(path);
                }
                return DetectResult {
                    verdict: DetectVerdict::Language(lang),
                    detected_via: format!("extension:{lower}"),
                };
            }
        }

        DetectResult {
            verdict: DetectVerdict::Unknown,
            detected_via: "none".to_string(),
        }
    }

    fn match_ext(&self, ext: &str) -> Option<&'static str> {
        for plugin in self.registry.all() {
            for &e in plugin.extensions() {
                if e == ext {
                    return Some(plugin.id());
                }
            }
        }
        None
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
}

/// Sniff a YAML/JSON file for an OpenAPI/Swagger or AsyncAPI root key.
///
/// Returns the owning plugin id (`"openapi"` or `"asyncapi"`) or `None`.
/// `openapi:` and `swagger:` (Swagger 2.0) both map to the OpenAPI plugin.
/// These are required root keys, so they appear near the top of the
/// document; we read only a small head. Two matchers cover both layouts:
/// a line-leading `key:` (block YAML and pretty-printed JSON) and a quoted
/// `"key"` near the start of brace-led (minified) JSON.
fn sniff_structured_descriptor(path: &Path) -> Option<&'static str> {
    use std::io::Read;
    let mut f = fs::File::open(path).ok()?;
    let mut buf = [0u8; 8192];
    let n = f.read(&mut buf).ok()?;
    let head = String::from_utf8_lossy(&buf[..n]);

    const KEYS: &[(&str, &str)] = &[
        ("openapi", "openapi"),
        ("swagger", "openapi"),
        ("asyncapi", "asyncapi"),
    ];

    // Line-leading `key:` — YAML, and JSON with one key per line.
    for line in head.lines().take(64) {
        let t = line.trim_start().trim_start_matches(['"', '\'']);
        for (key, id) in KEYS {
            if let Some(rest) = t.strip_prefix(key) {
                let rest = rest.trim_start_matches(['"', '\'']).trim_start();
                if rest.starts_with(':') {
                    return Some(id);
                }
            }
        }
    }

    // Minified JSON: a quoted root key within the opening object.
    if head.trim_start().starts_with('{') {
        // Byte-slicing a `str` panics unless the index lands on a char
        // boundary, and 2048 lands mid-codepoint in any file whose first
        // 2 KiB contain non-ASCII text — a translation catalogue, say.
        // Walking back to the nearest boundary costs at most three bytes
        // of window and cannot fail.
        let mut end = head.len().min(2048);
        while end > 0 && !head.is_char_boundary(end) {
            end -= 1;
        }
        let window = &head[..end];
        for (key, id) in KEYS {
            if window.contains(&format!("\"{key}\"")) {
                return Some(id);
            }
        }
    }
    None
}

/// Read up to the first 256 bytes; if it starts with `#!`, return the
/// first line.
fn read_shebang(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut f = fs::File::open(path).ok()?;
    let mut buf = [0u8; 256];
    let n = f.read(&mut buf).ok()?;
    let head = &buf[..n];
    if !head.starts_with(b"#!") {
        return None;
    }
    let end = head.iter().position(|&b| b == b'\n').unwrap_or(head.len());
    std::str::from_utf8(&head[..end])
        .ok()
        .map(|s| s.to_string())
}

/// Disambiguate `.h`: prefer C++ when a sibling source file with the
/// same stem is C++.
fn header_verdict(path: &Path) -> DetectResult {
    const CPP_EXTS: &[&str] = &[".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx", ".C"];
    if let (Some(stem), Some(dir)) = (path.file_stem(), path.parent())
        && let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.file_stem() != Some(stem) {
                    continue;
                }
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    let dotext = format!(".{ext}");
                    if CPP_EXTS.iter().any(|c| *c == dotext) {
                        return DetectResult {
                            verdict: DetectVerdict::Language("cpp"),
                            detected_via: "header-heuristic:cpp".into(),
                        };
                    }
                }
            }
        }
    DetectResult {
        verdict: DetectVerdict::Language("c"),
        detected_via: "header-heuristic:c".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    use crate::PluginRegistry;

    fn reg() -> PluginRegistry {
        PluginRegistry::with_v1_plugins()
    }

    fn write(dir: &Path, name: &str, body: &[u8]) {
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::File::create(&p).unwrap().write_all(body).unwrap();
    }

    #[test]
    fn each_v1_extension_detects() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        let cases: &[(&str, &str)] = &[
            ("a.rs", "rust"),
            ("b.py", "python"),
            ("c.js", "javascript"),
            ("d.mjs", "javascript"),
            ("e.ts", "typescript"),
            ("f.tsx", "typescript"),
            ("g.go", "go"),
            ("h.java", "java"),
            ("i.c", "c"),
            ("j.cpp", "cpp"),
            ("k.cc", "cpp"),
            ("l.cs", "csharp"),
        ];
        for (name, expected) in cases {
            write(tmp.path(), name, b"// noop\n");
            let r = det.detect(&tmp.path().join(name));
            assert_eq!(
                r.verdict,
                DetectVerdict::Language(expected),
                "expected {} -> {}, got {:?}",
                name,
                expected,
                r.verdict
            );
            assert!(r.detected_via.starts_with("extension:"));
        }
    }

    #[test]
    fn python_shebang_beats_extension() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        // Extension is `.txt`, unknown — but shebang says python.
        write(tmp.path(), "tool", b"#!/usr/bin/env python3\nprint(1)\n");
        let r = det.detect(&tmp.path().join("tool"));
        assert_eq!(r.verdict, DetectVerdict::Language("python"));
        assert_eq!(r.detected_via, "shebang:python3");
    }

    #[test]
    fn node_shebang_maps_to_javascript() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "script",
            b"#!/usr/bin/env node\nconsole.log(1)\n",
        );
        let r = det.detect(&tmp.path().join("script"));
        assert_eq!(r.verdict, DetectVerdict::Language("javascript"));
        assert_eq!(r.detected_via, "shebang:node");
    }

    #[test]
    fn header_sibling_picks_cpp() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "x.h", b"#pragma once\n");
        write(tmp.path(), "x.cpp", b"#include \"x.h\"\n");
        let r = det.detect(&tmp.path().join("x.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
        assert_eq!(r.detected_via, "header-heuristic:cpp");
    }

    #[test]
    fn header_without_cpp_sibling_is_c() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "y.h", b"#pragma once\n");
        write(tmp.path(), "y.c", b"#include \"y.h\"\n");
        let r = det.detect(&tmp.path().join("y.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
        assert_eq!(r.detected_via, "header-heuristic:c");
    }

    #[test]
    fn unknown_extension_is_unknown() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "notes.txt", b"plain text\n");
        let r = det.detect(&tmp.path().join("notes.txt"));
        assert_eq!(r.verdict, DetectVerdict::Unknown);
    }

    #[test]
    fn openapi_and_swagger_yaml_json_detect_by_content() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "api.yaml",
            b"openapi: 3.0.0\ninfo:\n  title: x\n",
        );
        write(tmp.path(), "v2.yaml", b"swagger: \"2.0\"\npaths: {}\n");
        write(
            tmp.path(),
            "api.json",
            b"{\"openapi\":\"3.0.0\",\"paths\":{}}",
        );
        for (name, via) in [
            ("api.yaml", "content:openapi"),
            ("v2.yaml", "content:openapi"),
            ("api.json", "content:openapi"),
        ] {
            let r = det.detect(&tmp.path().join(name));
            assert_eq!(r.verdict, DetectVerdict::Language("openapi"), "{name}");
            assert_eq!(r.detected_via, via, "{name}");
        }
    }

    #[test]
    fn asyncapi_detects_by_content() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        write(
            tmp.path(),
            "events.yaml",
            b"asyncapi: 2.6.0\ninfo:\n  title: x\n",
        );
        let r = det.detect(&tmp.path().join("events.yaml"));
        assert_eq!(r.verdict, DetectVerdict::Language("asyncapi"));
        assert_eq!(r.detected_via, "content:asyncapi");
    }

    #[test]
    fn ordinary_yaml_json_stays_unknown() {
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let tmp = TempDir::new().unwrap();
        write(tmp.path(), "config.yaml", b"name: app\nversion: 1.0\n");
        write(
            tmp.path(),
            "package.json",
            b"{\"name\":\"pkg\",\"scripts\":{}}",
        );
        // a description mentioning openapi must not trip the sniff
        write(
            tmp.path(),
            "doc.yaml",
            b"title: about openapi\nbody: text\n",
        );
        for name in ["config.yaml", "package.json", "doc.yaml"] {
            let r = det.detect(&tmp.path().join(name));
            assert_eq!(
                r.verdict,
                DetectVerdict::Unknown,
                "{name} should be Unknown"
            );
        }
    }

    #[test]
    fn a_non_ascii_json_head_does_not_panic() {
        // Slicing a `str` at byte 2048 panics unless that index lands on
        // a char boundary, and it will not in any file whose first 2 KiB
        // hold non-Latin text — a translation catalogue, say. This
        // aborted the entire run on Mastodon, whose `config/locales`
        // are exactly that.
        let dir = std::env::temp_dir().join("__cgg_detect_utf8__");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("el.json");
        // Pad with ASCII to exactly 2047 bytes, then start a two-byte
        // codepoint — so byte 2048 falls inside it.
        let mut body = String::from("{\n  \"about.blocks\": \"");
        while body.len() < 2047 {
            body.push('a');
        }
        body.push('έ');
        body.push_str("\",\n  \"x\": 1\n}\n");
        assert!(
            !body.is_char_boundary(2048),
            "fixture must straddle the cut"
        );
        std::fs::write(&f, &body).unwrap();
        // Must return a verdict rather than panicking. A locale
        // catalogue is not an API descriptor, so `None` is correct.
        assert_eq!(sniff_structured_descriptor(&f), None);
        std::fs::remove_file(&f).ok();
    }
}
