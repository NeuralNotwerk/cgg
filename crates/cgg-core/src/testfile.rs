//! Test-file classification.
//!
//! A pure function of `(path, language)` — no I/O, so it is
//! deterministic, testable without a filesystem, and cacheable.
//!
//! It lives here rather than in `cgg-walk` because the walker runs
//! *before* language detection and so could only apply a
//! language-agnostic rule. That rule would be wrong: `*_test.go` is the
//! Go compiler's own convention and means nothing in Python, while a
//! `test/` directory is exact for Rust integration tests and routinely
//! holds fixture data in a C project.
//!
//! The patterns are deliberately narrower than the equivalents in
//! name-matching tools. vulture's `*test*.py` glob, for instance,
//! swallows `latest.py`, `contest.py` and `protest.py`; a tool that
//! claims an audit-grade trail cannot afford that.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Why a file was classified as test code, recorded so the audit can
/// justify the classification instead of merely asserting it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TestFileReason {
    /// A path component is a conventional test directory.
    Directory,
    /// The file name matches a language convention.
    FileName,
    /// A language-specific structural rule (Maven/Gradle `src/test/`,
    /// a Rust `tests/` or `benches/` crate container).
    LanguageRule,
}

/// Directory components that mean "test" in essentially every ecosystem.
const TEST_DIRS: &[&str] = &[
    "test", "tests", "__tests__", "spec", "specs", "testdata", "e2e", "Tests", "Test",
];

/// Classify a file. `language` is the detected plugin id.
///
/// Performs no I/O.
pub fn classify_test_file(path: &Path, language: &str) -> Option<TestFileReason> {
    let comps: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let name = path.file_name()?.to_string_lossy().to_string();
    let lower = name.to_lowercase();

    // Maven / Gradle layout is exact and worth checking before the
    // generic directory rule so the reason is the more specific one.
    if matches!(language, "java" | "kotlin" | "scala" | "groovy") {
        for w in comps.windows(2) {
            if w[0] == "src" && w[1] == "test" {
                return Some(TestFileReason::LanguageRule);
            }
        }
    }

    // Rust: only a crate-level `tests/` or `benches/` container counts.
    // A `tests` module inside a source file is handled by the in-file
    // `#[cfg(test)]` signal instead.
    if language == "rust" && comps.iter().any(|c| c == "tests" || c == "benches") {
        return Some(TestFileReason::LanguageRule);
    }

    let by_name = match language {
        // The Go compiler's own rule.
        "go" => name.ends_with("_test.go"),
        "python" => {
            lower.starts_with("test_") && lower.ends_with(".py")
                || lower.ends_with("_test.py")
                || name == "conftest.py"
        }
        "javascript" | "typescript" => [
            ".test.js", ".test.jsx", ".test.ts", ".test.tsx", ".test.mjs", ".test.cjs",
            ".spec.js", ".spec.jsx", ".spec.ts", ".spec.tsx",
        ]
        .iter()
        .any(|s| lower.ends_with(s)),
        "java" => name.ends_with("Test.java") || name.ends_with("Tests.java") || name.ends_with("IT.java"),
        "kotlin" => name.ends_with("Test.kt") || name.ends_with("Tests.kt"),
        "csharp" => name.ends_with("Test.cs") || name.ends_with("Tests.cs"),
        "ruby" => lower.ends_with("_spec.rb") || lower.ends_with("_test.rb"),
        "php" => name.ends_with("Test.php"),
        "swift" => name.ends_with("Tests.swift") || name.ends_with("Test.swift"),
        "dart" => lower.ends_with("_test.dart"),
        "elixir" => lower.ends_with("_test.exs"),
        "erlang" => name.ends_with("_SUITE.erl") || lower.ends_with("_tests.erl"),
        "c" | "cpp" => {
            lower.ends_with("_test.c")
                || lower.ends_with("_test.cc")
                || lower.ends_with("_test.cpp")
                || lower.starts_with("test_")
                || lower.contains("_unittest.")
        }
        "scala" => name.ends_with("Spec.scala") || name.ends_with("Test.scala"),
        _ => false,
    };
    if by_name {
        return Some(TestFileReason::FileName);
    }

    // Generic directory rule last: it is the weakest signal.
    if comps
        .iter()
        .take(comps.len().saturating_sub(1))
        .any(|c| TEST_DIRS.contains(&c.as_str()))
    {
        return Some(TestFileReason::Directory);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn c(p: &str, l: &str) -> Option<TestFileReason> {
        classify_test_file(&PathBuf::from(p), l)
    }

    #[test]
    fn go_uses_the_compilers_own_rule() {
        assert_eq!(c("pkg/thing_test.go", "go"), Some(TestFileReason::FileName));
        assert_eq!(c("pkg/thing.go", "go"), None);
    }

    #[test]
    fn python_conventions_do_not_swallow_ordinary_words() {
        // vulture's `*test*.py` glob eats all three of these.
        assert_eq!(c("src/latest.py", "python"), None);
        assert_eq!(c("src/contest.py", "python"), None);
        assert_eq!(c("src/protest.py", "python"), None);
        assert_eq!(c("src/test_thing.py", "python"), Some(TestFileReason::FileName));
        assert_eq!(c("src/thing_test.py", "python"), Some(TestFileReason::FileName));
        assert_eq!(c("src/conftest.py", "python"), Some(TestFileReason::FileName));
    }

    #[test]
    fn rust_counts_only_crate_level_containers() {
        assert_eq!(c("crates/x/tests/cli.rs", "rust"), Some(TestFileReason::LanguageRule));
        assert_eq!(c("crates/x/benches/b.rs", "rust"), Some(TestFileReason::LanguageRule));
        // An inline `mod tests` lives in an ordinary source file.
        assert_eq!(c("crates/x/src/lib.rs", "rust"), None);
    }

    #[test]
    fn maven_layout_is_exact() {
        assert_eq!(
            c("app/src/test/java/com/x/FooTest.java", "java"),
            Some(TestFileReason::LanguageRule)
        );
        assert_eq!(c("app/src/main/java/com/x/Foo.java", "java"), None);
    }

    #[test]
    fn js_spec_and_test_suffixes() {
        assert_eq!(c("src/a.test.ts", "typescript"), Some(TestFileReason::FileName));
        assert_eq!(c("src/a.spec.tsx", "typescript"), Some(TestFileReason::FileName));
        assert_eq!(c("src/attest.ts", "typescript"), None);
    }

    #[test]
    fn generic_test_directories_are_the_weakest_signal() {
        assert_eq!(c("proj/tests/helper.rb", "ruby"), Some(TestFileReason::Directory));
        assert_eq!(c("proj/__tests__/x.js", "javascript"), Some(TestFileReason::Directory));
        // The directory rule must not fire on the file itself.
        assert_eq!(c("test", "rust"), None);
    }

    #[test]
    fn classification_is_pure_and_stable() {
        let p = PathBuf::from("pkg/a_test.go");
        assert_eq!(classify_test_file(&p, "go"), classify_test_file(&p, "go"));
    }
}
