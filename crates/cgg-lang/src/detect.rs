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

/// Disambiguate `.h`: prefer C++ when the file content contains
/// C++-only syntax or a sibling source file with the same stem is C++.
fn header_verdict(path: &Path) -> DetectResult {
    // Rule 3a: content sniffing — C++-only constructs in the first 8 KiB.
    if has_cpp_content(path) {
        return DetectResult {
            verdict: DetectVerdict::Language("cpp"),
            detected_via: "header-content:cpp".into(),
        };
    }

    // Rule 3b: sibling heuristic — `foo.h` next to `foo.cpp`.
    const CPP_EXTS: &[&str] = &[".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx", ".C"];
    if let (Some(stem), Some(dir)) = (path.file_stem(), path.parent())
        && let Ok(entries) = fs::read_dir(dir)
    {
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

/// Sniff the head of a `.h` file for C++-only syntax.
///
/// Returns `true` when the file almost certainly needs the C++ grammar.
/// Comments and string literals are stripped first, so a keyword inside
/// either does not count. `extern "C"` is deliberately not a trigger —
/// it is a C-linkage declaration that is valid (and common) in headers
/// consumed by both languages.
fn has_cpp_content(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 8192];
    let n = f.read(&mut buf).unwrap_or(0);
    if n == 0 {
        return false;
    }
    let head = String::from_utf8_lossy(&buf[..n]);
    let (stripped, saw_raw_string) = strip_c_lexemes(&head);
    // `R"(…)"` is C++. The stripper stops at it so the contents cannot
    // also be matched as code.
    if saw_raw_string {
        return true;
    }

    for line in stripped.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // Includes are checked before the preprocessor skip: the
        // directive itself starts with `#`. A trailing comment has
        // already been removed, so `#include <vector> // note` still
        // matches, and a commented-out include does not.
        if has_cpp_include(t) {
            return true;
        }
        if t.starts_with('#') {
            continue;
        }
        if t.starts_with("namespace ") || t == "namespace" {
            return true;
        }
        if let Some(rest) = t.strip_prefix("template")
            && !rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            && rest.trim_start().starts_with('<')
        {
            return true;
        }
        if has_enum_class(t) || has_cxx_base_clause(t) || has_access_specifier(t) {
            return true;
        }
        if t.starts_with("using ") {
            return true;
        }
        if contains_word(t, "constexpr")
            || contains_word(t, "noexcept")
            || t.contains("::")
        {
            return true;
        }
    }
    false
}

/// C++ standard-library headers that have no `.h` suffix. Matched on
/// the leaf of an angle-bracket include, so `<vector>` and
/// `<experimental/filesystem>` count and `<stdio.h>` / `<MyHeader>` do
/// not. Sorted for binary search.
const CPP_STDLIB_HEADERS: &[&str] = &[
    "algorithm",
    "any",
    "array",
    "atomic",
    "barrier",
    "bit",
    "bitset",
    "cassert",
    "ccomplex",
    "cctype",
    "cerrno",
    "cfenv",
    "cfloat",
    "charconv",
    "chrono",
    "cinttypes",
    "ciso646",
    "climits",
    "clocale",
    "cmath",
    "codecvt",
    "compare",
    "complex",
    "concepts",
    "condition_variable",
    "coroutine",
    "csetjmp",
    "csignal",
    "cstdalign",
    "cstdarg",
    "cstdbool",
    "cstddef",
    "cstdint",
    "cstdio",
    "cstdlib",
    "cstring",
    "ctgmath",
    "ctime",
    "cuchar",
    "cwchar",
    "cwctype",
    "debugging",
    "deque",
    "exception",
    "execution",
    "expected",
    "filesystem",
    "flat_map",
    "flat_set",
    "format",
    "forward_list",
    "fstream",
    "functional",
    "future",
    "generator",
    "hazard_pointer",
    "initializer_list",
    "inplace_vector",
    "iomanip",
    "ios",
    "iosfwd",
    "iostream",
    "istream",
    "iterator",
    "latch",
    "limits",
    "linalg",
    "list",
    "locale",
    "map",
    "mdspan",
    "memory",
    "memory_resource",
    "mutex",
    "new",
    "numbers",
    "numeric",
    "optional",
    "ostream",
    "print",
    "queue",
    "random",
    "ranges",
    "ratio",
    "rcu",
    "regex",
    "scoped_allocator",
    "semaphore",
    "set",
    "shared_mutex",
    "source_location",
    "span",
    "spanstream",
    "sstream",
    "stack",
    "stacktrace",
    "stdexcept",
    "stdfloat",
    "stop_token",
    "streambuf",
    "string",
    "string_view",
    "strstream",
    "syncstream",
    "system_error",
    "text_encoding",
    "thread",
    "tuple",
    "type_traits",
    "typeindex",
    "typeinfo",
    "unordered_map",
    "unordered_set",
    "utility",
    "valarray",
    "variant",
    "vector",
    "version",
];

fn is_cpp_stdlib_header(name: &str) -> bool {
    CPP_STDLIB_HEADERS.binary_search(&name).is_ok()
}

/// `#include <vector>`, `#include <experimental/filesystem>`,
/// `#include <bits/stdc++.h>`. `#include_next` is a different directive.
/// Any other extensionless name (`<MyHeader>`) is not a signal: C
/// projects include those too.
fn has_cpp_include(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("#include") else {
        return false;
    };
    if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    let rest = rest.trim_start();
    let Some(rest) = rest.strip_prefix('<') else {
        return false;
    };
    let Some(end) = rest.find('>') else {
        return false;
    };
    let inner = &rest[..end];
    if inner.contains("++") {
        return true;
    }
    let leaf = inner.rsplit('/').next().unwrap_or(inner);
    is_cpp_stdlib_header(leaf)
}

/// `enum class` / `enum struct`, but not `enum classification`.
fn has_enum_class(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("enum") else {
        return false;
    };
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return false;
    }
    let rest = rest.trim_start();
    for kw in ["class", "struct"] {
        if let Some(after) = rest.strip_prefix(kw)
            && (after.is_empty()
                || after.starts_with(|c: char| !c.is_ascii_alphanumeric() && c != '_'))
        {
            return true;
        }
    }
    false
}

/// `class Foo: public Bar`, `class Foo :public Bar`, `struct S final`.
/// A colon inside the brace (`struct Foo { int x : 3; }`) is a bitfield.
/// A single colon is required, so `::` alone does not count here.
fn has_cxx_base_clause(line: &str) -> bool {
    let rest = if let Some(r) = line.strip_prefix("class ") {
        r
    } else if let Some(r) = line.strip_prefix("struct ") {
        r
    } else {
        return false;
    };
    let head = match rest.find('{') {
        Some(i) => &rest[..i],
        None => rest,
    };
    if contains_word(head, "final") {
        return true;
    }
    let b = head.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b':' {
            let prev = i > 0 && b[i - 1] == b':';
            let next = i + 1 < b.len() && b[i + 1] == b':';
            if !prev && !next {
                return true;
            }
            if next {
                i += 1;
            }
        }
        i += 1;
    }
    false
}

/// `public:`, `public :`, `public: void draw();`.
fn has_access_specifier(line: &str) -> bool {
    for kw in ["public", "private", "protected"] {
        let Some(rest) = line.strip_prefix(kw) else {
            continue;
        };
        if rest.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        if rest.trim_start().starts_with(':') {
            return true;
        }
    }
    false
}

fn contains_word(line: &str, word: &str) -> bool {
    let b = line.as_bytes();
    let w = word.as_bytes();
    if w.is_empty() || b.len() < w.len() {
        return false;
    }
    let mut i = 0;
    while i + w.len() <= b.len() {
        if &b[i..i + w.len()] == w {
            let before_ok = i == 0 || !is_ident_byte(b[i - 1]);
            let after = i + w.len();
            let after_ok = after == b.len() || !is_ident_byte(b[after]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Strip comments and string/character literals so keyword detection
/// does not match inside them. Newlines are preserved. A C++ raw
/// string (`R"delim(…)delim"`) is reported and the scan stops: the
/// file is C++, and the contents must not be read as code.
fn strip_c_lexemes(src: &str) -> (String, bool) {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    let mut prev_ident = false;
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            prev_ident = false;
            loop {
                match chars.next() {
                    Some('*') if chars.peek() == Some(&'/') => {
                        chars.next();
                        break;
                    }
                    Some('\n') => out.push('\n'),
                    None => break,
                    _ => {}
                }
            }
        } else if c == '/' && chars.peek() == Some(&'/') {
            prev_ident = false;
            for d in chars.by_ref() {
                if d == '\n' {
                    out.push('\n');
                    break;
                }
            }
        } else if c == 'R' && !prev_ident && chars.peek() == Some(&'"') {
            return (out, true);
        } else if c == '"' || c == '\'' {
            let quote = c;
            prev_ident = false;
            out.push(' ');
            while let Some(d) = chars.next() {
                if d == '\\' {
                    chars.next();
                    continue;
                }
                if d == '\n' {
                    out.push('\n');
                    break;
                }
                if d == quote {
                    break;
                }
            }
        } else {
            prev_ident = c == '_' || c.is_ascii_alphanumeric();
            out.push(c);
        }
    }
    (out, false)
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

    // --- .h content detection tests ------------------------------------------

    #[test]
    fn h_with_namespace_detected_as_cpp() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "util.h",
            b"#pragma once\nnamespace util {\nvoid foo();\n}\n",
        );
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let r = det.detect(&dir.path().join("util.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
        assert_eq!(r.detected_via, "header-content:cpp");
    }

    #[test]
    fn h_with_template_detected_as_cpp() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "vec.h",
            b"template<typename T>\nclass Vec {};\n",
        );
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let r = det.detect(&dir.path().join("vec.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_with_class_inheritance_detected_as_cpp() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "shape.h",
            b"class Circle : public Shape {\npublic:\n  void draw();\n};\n",
        );
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let r = det.detect(&dir.path().join("shape.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_with_using_detected_as_cpp() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "types.h",
            b"#pragma once\nusing Callback = void(*)();\n",
        );
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let r = det.detect(&dir.path().join("types.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_with_cpp_stdlib_include_detected_as_cpp() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "str.h",
            b"#include <string>\n#include <vector>\nvoid foo();\n",
        );
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let r = det.detect(&dir.path().join("str.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn pure_c_header_stays_c() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "quicklz.h",
            b"#ifndef QLZ_H\n#define QLZ_H\n#include <string.h>\ntypedef unsigned int ui32;\nsize_t qlz_decompress(const char *src, void *dst);\n#endif\n");
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let r = det.detect(&dir.path().join("quicklz.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
        assert_eq!(r.detected_via, "header-heuristic:c");
    }

    #[test]
    fn extern_c_header_stays_c() {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "api.h",
            b"#ifdef __cplusplus\nextern \"C\" {\n#endif\nvoid init(void);\n#ifdef __cplusplus\n}\n#endif\n");
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let r = det.detect(&dir.path().join("api.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
    }

    #[test]
    fn cpp_keyword_inside_comment_does_not_trigger() {
        let dir = TempDir::new().unwrap();
        write(
            dir.path(),
            "plain.h",
            b"/* namespace foo { } */\n// template<int> class X;\nvoid bar(void);\n",
        );
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        let r = det.detect(&dir.path().join("plain.h"));
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
    }

    fn detect_h(src: &[u8]) -> DetectResult {
        let dir = TempDir::new().unwrap();
        write(dir.path(), "t.h", src);
        let reg = reg();
        let det = LanguageDetector::new(&reg);
        det.detect(&dir.path().join("t.h"))
    }

    #[test]
    fn cpp_stdlib_header_list_is_sorted() {
        assert!(
            CPP_STDLIB_HEADERS.windows(2).all(|w| w[0] < w[1]),
            "CPP_STDLIB_HEADERS must stay sorted for binary_search"
        );
        assert!(is_cpp_stdlib_header("vector"));
        assert!(is_cpp_stdlib_header("string"));
        assert!(!is_cpp_stdlib_header("stdio.h"));
        assert!(!is_cpp_stdlib_header("MyHeader"));
    }

    #[test]
    fn h_class_base_without_spaces_around_colon() {
        let r = detect_h(b"class Circle: public Shape {\n  void draw();\n};\n");
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_access_specifier_with_declaration_on_same_line() {
        let r = detect_h(b"class Circle {\npublic: void draw();\n};\n");
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_enum_class_detected_as_cpp() {
        let r = detect_h(b"#pragma once\nenum class Color { Red, Green };\n");
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_constexpr_and_noexcept_detected_as_cpp() {
        let r = detect_h(b"inline int add(int a, int b) noexcept { return a + b; }\n");
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
        let r = detect_h(b"constexpr int N = 3;\n");
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_scope_resolution_detected_as_cpp() {
        let r = detect_h(b"void foo(std::string s);\n");
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_stdcpp_header_with_dot_detected_as_cpp() {
        let r = detect_h(b"#include <bits/stdc++.h>\nvoid foo();\n");
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn h_include_with_trailing_comment_detected_as_cpp() {
        let r = detect_h(b"#include <vector> // std::vector\nvoid foo();\n");
        assert_eq!(r.verdict, DetectVerdict::Language("cpp"));
    }

    #[test]
    fn commented_include_and_string_do_not_trigger() {
        let r = detect_h(
            b"/* #include <vector> */\n// #include <string>\nconst char *s = \"namespace foo\";\nvoid bar(void);\n",
        );
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
    }

    #[test]
    fn extensionless_project_header_stays_c() {
        let r = detect_h(b"#include <MyHeader>\nvoid foo(void);\n");
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
    }

    #[test]
    fn include_next_of_c_header_stays_c() {
        let r = detect_h(b"#include_next <stdio.h>\nvoid foo(void);\n");
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
    }

    #[test]
    fn bitfield_struct_stays_c() {
        let r = detect_h(b"struct Flags { unsigned x : 3; unsigned y : 1; };\n");
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
    }

    #[test]
    fn string_containing_comment_opener_does_not_swallow_code() {
        let r = detect_h(b"const char *s = \"/*\";\nvoid bar(void);\n");
        assert_eq!(r.verdict, DetectVerdict::Language("c"));
    }
}
