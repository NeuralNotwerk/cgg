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
    for call in unresolved {
        let file_aliases = aliases.and_then(|m| m.get(&call.file));
        match classify_one(&call, known_names, stdlib, file_aliases) {
            Verdict::Unresolved => result.unresolved.push(call),
            Verdict::Stdlib => result.stdlib.push(call),
            Verdict::External => result.external.push(call),
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
    let name = call.name.as_str();
    let rh = call.receiver_hint.as_str();
    let is_self_receiver =
        rh == "self" || rh == "Self" || rh == "cls" || rh == "this";

    // Stdlib detection runs first so a Vec::push or .clone() lands in
    // the stdlib bucket even when the project also defines a `push`
    // or `clone` method of its own.
    if let Some(std) = stdlib {
        if !rh.is_empty() && !is_self_receiver {
            // (a) Receiver hint matches stdlib directly, or its first
            //     dotted segment does (`os.path` → check `os`).
            if module_is_stdlib(rh, std) {
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
            // (c) Bare call into stdlib (e.g. `len(...)`, `format(...)`).
            if std.contains(name) {
                return Verdict::Stdlib;
            }
            // (d) Bare call to a name brought in by `from <module> import
            //     name` where <module> is stdlib.
            if let Some(a) = aliases {
                if let Some(source) = a.from_imports.get(name) {
                    if module_is_stdlib(source, std) {
                        return Verdict::Stdlib;
                    }
                }
            }
        } else if !is_self_receiver && std.contains(name) {
            // (e) Method whose name is in the stdlib manifest, called on
            //     a receiver that is neither stdlib nor a known project
            //     type — typically a variable like `lst.append(...)` or
            //     `s.lower()`. Without type inference the receiver type
            //     is unknown, so we treat the name match as stdlib.
            //     This is the same trade-off the plan explicitly endorses
            //     for ambiguous method names: bare/unknown receivers go
            //     to stdlib so the bucket reflects "calls into stdlib"
            //     even when the project happens to define a method with
            //     the same name.
            let receiver_is_unknown = !known_names.contains(rh)
                && rh.split('.').next().map(|h| !known_names.contains(h)).unwrap_or(true);
            if receiver_is_unknown {
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
