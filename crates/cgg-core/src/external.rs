//! External call classification.
//!
//! Splits unresolved call sites into three buckets so the headline
//! summary reads truthfully:
//!
//!   * **unresolved** — the project defines a function with this name
//!     somewhere, but cgg couldn't bind this particular call site to
//!     it. A real gap that may indicate a resolver limitation or an
//!     incomplete scan.
//!   * **stdlib** — call site targets the language's standard library
//!     (`Vec::push`, `clone()`, `format!`, etc.). Expected; not a gap.
//!   * **external** — neither in the project nor in stdlib — third-party
//!     crates, framework methods, or anything the scan didn't see.
//!     Useful for spotting dependency surface area.
//!
//! Stdlib detection uses the per-language manifest under `stdlib/*.txt`.
//! Three matching strategies, in order:
//!   1. Exact: receiver hint or bare name equals an entry in the manifest
//!      (`Vec`, `HashMap`, `format`).
//!   2. Dotted-first-segment: for languages that emit module-qualified
//!      receivers like Python `os.path` or Go `fmt.Sprintf`, the first
//!      segment of the receiver (`os`, `fmt`) is tested too.
//!   3. Import-alias resolution: per-file alias maps from `FileFacts.imports`
//!      let `t.TypeVar` (after `import typing as t`) be recognized as a
//!      stdlib call, and bare `TypeVar(...)` (after `from typing import
//!      TypeVar`) likewise.
//!
//! When the project also defines a callable of the same name, the
//! classifier prefers the project reading over a bare-name stdlib match:
//! we'd rather surface a real `Unresolved` resolver gap than silently
//! mask it as stdlib because of a manifest entry. The stdlib bucket
//! still wins when there is positive evidence the receiver is stdlib —
//! a stdlib receiver hint, a stdlib import alias, or a `from <stdlib>
//! import name` binding.

use std::collections::{HashMap, HashSet};

use crate::audit::AuditUnresolvedCall;
use crate::facts::FileFacts;
use crate::ids::FileId;
use crate::stdlib;

/// Result of classifying unresolved calls into three buckets.
#[derive(Debug, Default)]
pub struct ClassifyResult {
    /// Calls whose target name matches a definition in the scanned files
    /// but couldn't be linked. The genuine gaps.
    pub unresolved: Vec<AuditUnresolvedCall>,
    /// Calls that target the language standard library. Expected noise.
    pub stdlib: Vec<AuditUnresolvedCall>,
    /// Calls that target neither the project nor stdlib — third-party
    /// crates, framework methods, etc.
    pub external: Vec<AuditUnresolvedCall>,
}

/// Per-file import-alias tables. Built from `FileFacts.imports`.
///
/// * `import_aliases` — locally-bound receiver name → canonical module
///   path. Populated from `import M` (M→M), `import M as A` (A→M),
///   and `import M.N as A` (A→M.N).
/// * `from_imports` — locally-bound bare name → source module. Populated
///   from `from M import X` (X→M) and `from M import X as Y` (Y→M).
///
/// Empty for languages whose plugin doesn't surface imports this way,
/// in which case classification falls back to exact + dotted-segment
/// matching only.
#[derive(Debug, Default, Clone)]
pub struct FileAliases {
    pub import_aliases: HashMap<String, String>,
    pub from_imports: HashMap<String, String>,
}

impl FileAliases {
    /// Build an alias table from a single file's import records.
    /// The interpretation of `ImportRecord.path` / `.alias` is the same
    /// across the language plugins that surface dotted module access
    /// (Python primarily; other plugins are tolerated — bogus entries
    /// just produce inert map keys).
    pub fn from_facts(facts: &FileFacts) -> Self {
        let mut import_aliases = HashMap::new();
        let mut from_imports = HashMap::new();
        for imp in &facts.imports {
            match imp.kind.as_str() {
                "import" => {
                    // `import a.b` → bind "a" (the head receiver) to "a.b".
                    // `import a.b as c` → bind "c" to "a.b".
                    let path = imp.path.trim();
                    if path.is_empty() {
                        continue;
                    }
                    if !imp.alias.is_empty() {
                        import_aliases.insert(imp.alias.trim().to_string(), path.to_string());
                    } else {
                        let head = path.split('.').next().unwrap_or(path);
                        import_aliases.insert(head.to_string(), path.to_string());
                    }
                }
                "from-import" => {
                    // `from m import X, Y as Z` → X→m, Z→m.
                    let module = imp.path.trim();
                    if module.is_empty() {
                        continue;
                    }
                    for item in imp.alias.split(',') {
                        let item = item.trim();
                        if item.is_empty() {
                            continue;
                        }
                        let bound = if let Some((_orig, alias)) = item.split_once(" as ") {
                            alias.trim()
                        } else {
                            item
                        };
                        if !bound.is_empty() && bound != "*" {
                            from_imports.insert(bound.to_string(), module.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        Self { import_aliases, from_imports }
    }
}

/// Verdict for a single call.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
enum Verdict {
    Unresolved,
    Stdlib,
    External,
}

/// Classify unresolved calls into unresolved / stdlib / external buckets.
///
/// `aliases` is a per-file table of import aliases used to recognize
/// stdlib calls written through `import x as y` or `from x import y`.
/// Pass `None` when alias resolution is not available (e.g. multi-file
/// batches with no language-specific stdlib).
pub fn classify_external(
    unresolved: Vec<AuditUnresolvedCall>,
    known_names: &HashSet<&str>,
    language: &str,
    aliases: Option<&HashMap<FileId, FileAliases>>,
) -> ClassifyResult {
    let stdlib = stdlib::stdlib_names(language);
    let mut result = ClassifyResult::default();
    for mut call in unresolved {
        let file_aliases = aliases.and_then(|m| m.get(&call.file));
        match classify_one(&call, known_names, stdlib, file_aliases) {
            Verdict::Unresolved => result.unresolved.push(call),
            Verdict::Stdlib => {
                // Record which name-screen fired (Issue 9 evidence).
                call.name_screen_applied = Some("stdlib".to_string());
                result.stdlib.push(call);
            }
            Verdict::External => {
                call.name_screen_applied = Some("external".to_string());
                result.external.push(call);
            }
        }
    }
    result
}

/// Test whether a module path (possibly dotted) belongs to stdlib.
/// Checks the full path first, then its first dotted segment. The
/// first-segment fallback handles cases like `os.path` where the bare
/// stdlib manifest only contains `os`.
fn module_is_stdlib(path: &str, std: &HashSet<&str>) -> bool {
    if std.contains(path) {
        return true;
    }
    let head = path.split('.').next().unwrap_or(path);
    if head != path && std.contains(head) {
        return true;
    }
    false
}

fn classify_one(
    call: &AuditUnresolvedCall,
    known_names: &HashSet<&str>,
    stdlib: Option<&HashSet<&str>>,
    aliases: Option<&FileAliases>,
) -> Verdict {
    // Some language plugins emit dotted callable names like Lua
    // `table.insert` or `string.format` as the bare `name` field with
    // an empty receiver. Split on the last separator so the rest of
    // this function can apply receiver-based rules uniformly.
    let (rh_owned, name_owned);
    if call.receiver_hint.is_empty() {
        let n = call.name.as_str();
        if let Some(idx) = n.rfind(|c: char| c == '.' || c == ':').filter(|&i| i > 0 && i < n.len() - 1) {
            rh_owned = n[..idx].to_string();
            name_owned = n[idx + 1..].to_string();
        } else {
            rh_owned = String::new();
            name_owned = call.name.clone();
        }
    } else {
        rh_owned = call.receiver_hint.clone();
        name_owned = call.name.clone();
    }
    let name = name_owned.as_str();
    let rh = rh_owned.as_str();
    let is_self_receiver =
        rh == "self" || rh == "Self" || rh == "cls" || rh == "this";

    // Owner-aware screen (Issue 6): when the project defines BOTH a
    // method of this name AND a type matching the receiver, prefer the
    // project reading over any name-based stdlib screen. A project
    // `EntityId::len` must not be siphoned into the stdlib bucket just
    // because `len` is stdlib vocabulary — and a project type that
    // happens to share a stdlib module's name (`io`, `os`) must not be
    // masked by rule (a) below either. The owner-based question is asked
    // *before* the name-based one, which is the whole point of Issue 6.
    let project_owns_pair =
        !is_self_receiver && !rh.is_empty() && known_names.contains(name) && known_names.contains(rh);

    // Stdlib detection runs first so a Vec::push or .clone() lands in
    // the stdlib bucket even when the project also defines a `push`
    // or `clone` method of its own.
    if let Some(std) = stdlib {
        if !rh.is_empty() && !is_self_receiver {
            // (a) Receiver hint matches stdlib directly, or its first
            //     dotted segment does (`os.path` → check `os`). Skipped
            //     when the project owns the (receiver type, method) pair.
            if !project_owns_pair && module_is_stdlib(rh, std) {
                return Verdict::Stdlib;
            }
            // (b) Receiver hint is a local alias for a stdlib module
            //     (`import typing as t` → check `typing`).
            if let Some(a) = aliases {
                if let Some(resolved) = a.import_aliases.get(rh) {
                    if module_is_stdlib(resolved, std) {
                        return Verdict::Stdlib;
                    }
                }
            }
        }
        if rh.is_empty() {
            // (c) Bare call whose name is in stdlib AND the project does
            //     NOT define a function of that name → Stdlib. When the
            //     project also defines the name we deliberately fall
            //     through to the project rules below: the call is a real
            //     resolver gap (`Unresolved`) and we'd rather surface it
            //     than silently mask it with a stdlib reading. This is
            //     the safety net for bad manifest entries — a stray
            //     keyword like `break` in the manifest can no longer
            //     swallow every project method of the same name.
            if std.contains(name) && !known_names.contains(name) {
                return Verdict::Stdlib;
            }
            // (d) Bare call to a name brought in by `from <module> import
            //     name` where <module> is stdlib. Import alias is strong
            //     positive evidence the call really is stdlib, so this
            //     rule applies even when the project also defines `name`.
            if let Some(a) = aliases {
                if let Some(source) = a.from_imports.get(name) {
                    if module_is_stdlib(source, std) {
                        return Verdict::Stdlib;
                    }
                }
            }
        } else if !is_self_receiver && std.contains(name) {
            // (e) Method whose name is in the stdlib manifest, called on
            //     some receiver `rh`. We classify as stdlib only when
            //     there is positive evidence `rh` is a stdlib module or
            //     a known stdlib type — receiver matches stdlib directly
            //     (already handled by rule (a) above), or `rh` is an
            //     import alias for a stdlib module. If we have no project
            //     corpus to compare against (`known_names` empty — e.g.
            //     stack-graphs reclassification path) we keep the old
            //     permissive behaviour. Otherwise we fall through so the
            //     project / external rules below decide. The previous
            //     "unknown receiver → stdlib" trade-off was too eager:
            //     it let one bad manifest entry mask hundreds of project
            //     edges through receivers whose type we simply couldn't
            //     infer.
            let receiver_aliases_stdlib = aliases
                .and_then(|a| a.import_aliases.get(rh))
                .map(|resolved| module_is_stdlib(resolved, std))
                .unwrap_or(false);
            if receiver_aliases_stdlib || known_names.is_empty() {
                return Verdict::Stdlib;
            }
        }
    }

    // Rule 1 (was external rule 1): name not defined anywhere in the
    // project AND not in stdlib → third-party / unknown.
    if !known_names.contains(name) {
        return Verdict::External;
    }

    // Rule 2: receiver is a type not defined in the project (and not
    // stdlib — that was caught above) → third-party.
    if !rh.is_empty() && !is_self_receiver && !known_names.contains(rh) {
        return Verdict::External;
    }

    // Project defines this name and either we have a project receiver
    // or no receiver at all → a real gap that cgg couldn't bind.
    Verdict::Unresolved
}

/// Build the set of known simple names from all definitions.
/// This includes function names, method names, struct/class names,
/// trait names — anything that could be a call target or receiver type.
pub fn build_known_names(facts: &[crate::FileFacts]) -> HashSet<String> {
    let mut names = HashSet::new();
    for f in facts {
        for d in &f.definitions {
            names.insert(d.simple_name.clone());
            // Also extract the type/struct name from qualified names
            // so receiver_hint matching works.
            for seg in d.qualified_name.split("::") {
                if seg.starts_with(char::is_uppercase) {
                    names.insert(seg.to_string());
                }
            }
            for seg in d.qualified_name.split('.') {
                if seg.starts_with(char::is_uppercase) {
                    names.insert(seg.to_string());
                }
            }
        }
    }
    names
}

/// Build per-file alias tables for an entire fact set. Convenience for
/// the multi-file classification path; per-file callers can use
/// `FileAliases::from_facts` directly.
pub fn build_alias_map(facts: &[FileFacts]) -> HashMap<FileId, FileAliases> {
    facts.iter().map(|f| (f.file, FileAliases::from_facts(f))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::FileId;

    fn mk_call(name: &str, rh: &str) -> AuditUnresolvedCall {
        AuditUnresolvedCall::new(
            None,
            FileId::new(0),
            1,
            0,
            name.to_string(),
            rh.to_string(),
            crate::audit::UnresolvedReason::NoCandidateInFile,
        )
    }

    fn names<'a>(items: &'a [&'a str]) -> HashSet<&'a str> {
        items.iter().copied().collect()
    }

    #[test]
    fn bare_stdlib_name_no_project_collision_is_stdlib() {
        let std = names(&["len"]);
        let known: HashSet<&str> = HashSet::new();
        assert_eq!(
            classify_one(&mk_call("len", ""), &known, Some(&std), None),
            Verdict::Stdlib
        );
    }

    #[test]
    fn bare_stdlib_name_with_project_collision_falls_through() {
        let std = names(&["clone"]);
        let known = names(&["clone"]);
        assert_eq!(
            classify_one(&mk_call("clone", ""), &known, Some(&std), None),
            Verdict::Unresolved
        );
    }

    #[test]
    fn dotted_receiver_stdlib_module_is_stdlib() {
        let std = names(&["os"]);
        let known: HashSet<&str> = HashSet::new();
        assert_eq!(
            classify_one(&mk_call("getcwd", "os.path"), &known, Some(&std), None),
            Verdict::Stdlib
        );
    }

    #[test]
    fn project_owned_pair_beats_stdlib_module_name() {
        // Issue 6: a project type named like a stdlib module (`io`) with
        // its own method (`read`) must be asked about by owner *before*
        // the name-based stdlib screen — owner lookup wins.
        let std = names(&["io", "os", "read"]);
        let known = names(&["io", "read"]);
        assert_eq!(
            classify_one(&mk_call("read", "io"), &known, Some(&std), None),
            Verdict::Unresolved
        );
        // But when the project does NOT own the type, the stdlib module
        // reading still applies.
        let known_empty: HashSet<&str> = HashSet::new();
        assert_eq!(
            classify_one(&mk_call("read", "io"), &known_empty, Some(&std), None),
            Verdict::Stdlib
        );
    }

    #[test]
    fn import_alias_receiver_resolves_to_stdlib() {
        let std = names(&["typing"]);
        let known: HashSet<&str> = HashSet::new();
        let aliases = FileAliases {
            import_aliases: [("t".to_string(), "typing".to_string())]
                .into_iter()
                .collect(),
            from_imports: Default::default(),
        };
        assert_eq!(
            classify_one(&mk_call("TypeVar", "t"), &known, Some(&std), Some(&aliases)),
            Verdict::Stdlib
        );
    }

    #[test]
    fn from_import_brings_name_into_stdlib_even_with_collision() {
        let std = names(&["typing"]);
        let known = names(&["TypeVar"]);
        let aliases = FileAliases {
            import_aliases: Default::default(),
            from_imports: [("TypeVar".to_string(), "typing".to_string())]
                .into_iter()
                .collect(),
        };
        assert_eq!(
            classify_one(&mk_call("TypeVar", ""), &known, Some(&std), Some(&aliases)),
            Verdict::Stdlib
        );
    }

    #[test]
    fn unknown_receiver_no_stdlib_evidence_is_external() {
        // The tightened rule (e): name `push` is in stdlib, receiver
        // `myvec` is not a known project type AND not stdlib-shaped, so
        // we no longer eagerly call it stdlib. The project does not
        // define `push` either, so it falls through to External.
        let std = names(&["push"]);
        let known: HashSet<&str> = HashSet::new();
        let known_with_project = names(&["MyType"]);
        assert_eq!(
            classify_one(
                &mk_call("push", "myvec"),
                &known_with_project,
                Some(&std),
                None
            ),
            Verdict::External
        );
        // But when there's no project corpus at all, we keep the
        // permissive fallback so single-file / stack-graph paths still
        // attribute reasonably.
        assert_eq!(
            classify_one(&mk_call("push", "myvec"), &known, Some(&std), None),
            Verdict::Stdlib
        );
    }

    #[test]
    fn method_on_known_stdlib_receiver_is_stdlib() {
        // Rule (a): receiver itself matches stdlib directly.
        let std = names(&["Vec", "push"]);
        let known = names(&["push"]);
        assert_eq!(
            classify_one(&mk_call("push", "Vec"), &known, Some(&std), None),
            Verdict::Stdlib
        );
    }

    #[test]
    fn self_receiver_skips_stdlib_match() {
        // self/Self/this/cls receivers should not be classified as
        // stdlib even when the name appears in the manifest — they're
        // calls into the enclosing project type.
        let std = names(&["clone"]);
        let known = names(&["clone", "MyType"]);
        for rh in ["self", "Self", "this", "cls"] {
            assert_eq!(
                classify_one(&mk_call("clone", rh), &known, Some(&std), None),
                Verdict::Unresolved,
                "receiver={rh}"
            );
        }
    }

    #[test]
    fn dotted_first_segment_match_in_stdlib() {
        // `module_is_stdlib` should match the head segment when the
        // full dotted path isn't in the manifest.
        let std = names(&["os"]);
        assert!(module_is_stdlib("os.path", &std));
        assert!(module_is_stdlib("os", &std));
        assert!(!module_is_stdlib("requests.get", &std));
    }
}
