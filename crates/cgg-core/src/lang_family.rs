//! Language-family definitions for cross-language symbol resolution.
//!
//! A language family groups languages that share a runtime or linker
//! namespace and can call each other's definitions directly — no
//! marshalling, no adapter. Qualified-name lookup consults this when a
//! name misses in the caller's language, and tries the siblings. The
//! simple-name and owner-method indexes are not widened: those drive
//! duck-typed fan-out, and mixing families there pulls unrelated
//! functions into the candidate set.
//!
//! The mapping lives here (a pure function on language-id strings)
//! rather than on the `LanguagePlugin` trait because:
//! * the resolver needs the answer from a bare `&str`, without
//!   access to the plugin registry;
//! * family membership is a property of the *pair* of languages
//!   (their runtime interop), not a single plugin.

/// All languages in the same family as `lang`, **including `lang`
/// itself**.  Returns an empty slice when `lang` has no cross-language
/// family — it is its own singleton family and no sibling lookup is
/// needed.
///
/// A new row enables the qualified-name fallback for that family.
/// Prototype unification opts in on its own (`same_family(lang, "c")`),
/// and `.h` content detection does not read this table.
pub fn language_family(lang: &str) -> &'static [&'static str] {
    match lang {
        "c" | "cpp" | "objc" => &["c", "cpp", "objc"],
        // Uncomment when the qualified-name fallback is tested for these.
        // Do not implement that fallback by copying callables into the
        // simple-name index: fan-out would then see every sibling.
        // "java" | "kotlin" | "scala" | "groovy" => &["java", "kotlin", "scala", "groovy"],
        // "javascript" | "typescript" => &["javascript", "typescript"],
        // "csharp" | "fsharp" => &["csharp", "fsharp"],
        _ => &[],
    }
}

/// Whether `a` and `b` are in the same language family (or are the
/// same language).
pub fn same_family(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let fam = language_family(a);
    !fam.is_empty() && fam.contains(&b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_family_is_symmetric() {
        assert!(same_family("c", "cpp"));
        assert!(same_family("cpp", "c"));
        assert!(same_family("c", "objc"));
        assert!(same_family("objc", "cpp"));
    }

    #[test]
    fn same_language_is_always_same_family() {
        assert!(same_family("c", "c"));
        assert!(same_family("python", "python"));
        assert!(same_family("rust", "rust"));
    }

    #[test]
    fn unrelated_languages_are_not_family() {
        assert!(!same_family("c", "python"));
        assert!(!same_family("rust", "cpp"));
        assert!(!same_family("java", "python"));
    }

    #[test]
    fn no_family_returns_empty() {
        assert!(language_family("python").is_empty());
        assert!(language_family("rust").is_empty());
        assert!(language_family("java").is_empty());
    }

    #[test]
    fn family_includes_self() {
        let fam = language_family("c");
        assert!(fam.contains(&"c"));
        assert!(fam.contains(&"cpp"));
        assert!(fam.contains(&"objc"));
    }
}
