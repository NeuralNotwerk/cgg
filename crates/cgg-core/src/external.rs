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
                        import_aliases
                            .insert(imp.alias.trim().to_string(), path.to_string());
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
                        let bound = if let Some((_orig, alias)) = item.split_once(" as ")
                        {
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
        Self {
            import_aliases,
            from_imports,
        }
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
    let modules = stdlib::stdlib_modules(language);
    let mut result = ClassifyResult::default();
    for mut call in unresolved {
        let file_aliases = aliases.and_then(|m| m.get(&call.file));
        match classify_one(&call, known_names, stdlib, modules, file_aliases) {
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

/// Separators between a qualifier's segments: `std::fs`, `os.path`,
/// `clojure.string/join`, PHP `App\Models`.
const QUALIFIER_SEPARATORS: [&str; 4] = ["::", ".", "/", "\\"];

/// Test whether a module path (possibly qualified) belongs to stdlib.
///
/// Checks the full path against the manifest and against the language's
/// [`stdlib::stdlib_modules`] (`POSIX`, `File::Spec`, `clojure.string`;
/// exact names only, since `IO::Socket` being core says nothing of
/// `IO::Socket::SSL`), then its head segment — split on `::`, `.`, `/`
/// or `\` — against the manifest (`std::fs` → `std`, `os.path` → `os`).
/// A `::`, `/` or `\` head is only read off a plain path: a receiver
/// that is a value expression (`std::forward<T>(j)`,
/// `Command::new("git")`) names whatever that expression returns, not a
/// module. (The first `.` segment is read off any receiver, as before.)
/// A leading `\` or `::` (the global namespace) and a trailing `:` (left
/// by a split at `::`) are not part of the name.
///
/// `qualifier` is true for a call's own receiver or qualifier (rule (a)).
/// For an import's resolved module path it is false: only the exact path
/// and its first `.` segment count, never a `::`/`/`/`\` head.
fn module_is_stdlib(
    path: &str,
    std: &HashSet<&str>,
    modules: &[&str],
    qualifier: bool,
) -> bool {
    let path = path.trim_start_matches('\\');
    let path = path.strip_prefix("::").unwrap_or(path);
    let path = path.trim_end_matches(':');
    if path.is_empty() {
        return false;
    }
    if std.contains(path) || modules.contains(&path) {
        return true;
    }
    // The first dotted segment, read off any receiver (`os.path`), as
    // it always was.
    let dotted_head = path.split('.').next().unwrap_or(path);
    if dotted_head != path && std.contains(dotted_head) {
        return true;
    }
    // An import's module path (`test/utils/x`, `testing/fixtures.zig`)
    // names a file, whose first directory says nothing about stdlib; and a
    // class literal (`String::class`) is a value, not a module path.
    if !qualifier
        || path.ends_with("::class")
        || !path
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '/' | '\\'))
    {
        return false;
    }
    let head_end = QUALIFIER_SEPARATORS
        .iter()
        .filter_map(|sep| path.find(sep))
        .min();
    head_end.is_some_and(|i| i > 0 && std.contains(&path[..i]))
}

/// Split a receiver-less qualified name into (qualifier, name) at its
/// last separator: Lua `table.insert`, Perl `POSIX::floor` (qualifier
/// `POSIX:`, as it always was), Clojure `clojure.string/join`, PHP
/// `App\helper`. A `/` or `\` splits only between identifier
/// characters, so an operator such as Haskell's `</>` stays whole. A
/// leading `\` (PHP's global namespace) is dropped first. The third
/// value is the separator split at; a `.` in R and Julia is part of an
/// ordinary name (`grid.draw`) rather than a module qualifier.
fn split_qualified_name(name: &str) -> (&str, &str, Option<char>) {
    let n = name.trim_start_matches('\\');
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let slash = n.rfind(['/', '\\']).filter(|&i| {
        i > 0
            && i < n.len() - 1
            && n[..i].chars().next_back().is_some_and(ident)
            && n[i + 1..].chars().next().is_some_and(ident)
    });
    let dot = n.rfind(['.', ':']).filter(|&i| i > 0 && i < n.len() - 1);
    match slash.max(dot) {
        Some(i) => (&n[..i], &n[i + 1..], n[i..].chars().next()),
        None => ("", n, None),
    }
}

fn classify_one(
    call: &AuditUnresolvedCall,
    known_names: &HashSet<&str>,
    stdlib: Option<&HashSet<&str>>,
    modules: &[&str],
    aliases: Option<&FileAliases>,
) -> Verdict {
    if !call.receiver_hint.is_empty() {
        return classify_parts(
            &call.receiver_hint,
            &call.name,
            false,
            known_names,
            stdlib,
            modules,
            aliases,
        );
    }
    // Some language plugins emit qualified callable names like Lua
    // `table.insert`, Perl `POSIX::floor` or Clojure `clojure.string/join`
    // as the bare `name` field with an empty receiver. Split on the last
    // separator so the rules below apply to (qualifier, name) uniformly.
    //
    // The split at `.`/`:` is the one this function always made. The
    // split at `/` or `\` (and the PHP global-namespace `\strlen`) is
    // read only as stdlib evidence: when it does not show a stdlib call,
    // the call is classified exactly as before, so the new separators
    // move sites from `external` to `stdlib` and nowhere else.
    let n = call.name.as_str();
    let (qual, name, sep) = split_qualified_name(n);
    let whole = n.trim_start_matches('\\');
    let slash = matches!(sep, Some('/' | '\\'));
    // A manifest entry written qualified (Clojure `io/copy`, `str/join`)
    // is read whole, as it always was.
    let manifest_entry = slash && stdlib.is_some_and(|s| s.contains(whole));
    if (slash || n.starts_with('\\')) && !manifest_entry {
        let global_function = n.starts_with('\\') && qual.is_empty();
        let v = classify_parts(
            qual,
            name,
            global_function,
            known_names,
            stdlib,
            modules,
            aliases,
        );
        if v == Verdict::Stdlib {
            return v;
        }
    }
    let (rh, name) = match n.rfind(['.', ':']).filter(|&i| i > 0 && i < n.len() - 1) {
        Some(i) => (&n[..i], &n[i + 1..]),
        None => ("", n),
    };
    // In R and Julia a `.` is part of an ordinary name (`grid.draw`), so
    // a dotted qualifier is never looked up as a module.
    let modules = if n[rh.len()..].starts_with('.') {
        &[][..]
    } else {
        modules
    };
    classify_parts(rh, name, false, known_names, stdlib, modules, aliases)
}

/// [`classify_one`] for a call already split into receiver `rh` and
/// `name`. `global_function`: the name was written qualified with the
/// global namespace alone (PHP `\strlen()`), which only a global function
/// can be — so a workspace method of the same name is no reason to doubt
/// a stdlib reading.
fn classify_parts(
    rh: &str,
    name: &str,
    global_function: bool,
    known_names: &HashSet<&str>,
    stdlib: Option<&HashSet<&str>>,
    modules: &[&str],
    aliases: Option<&FileAliases>,
) -> Verdict {
    let is_self_receiver = rh == "self" || rh == "Self" || rh == "cls" || rh == "this";

    // Owner-aware screen (Issue 6): when the project defines BOTH a
    // method of this name AND a type matching the receiver, prefer the
    // project reading over any name-based stdlib screen. A project
    // `EntityId::len` must not be siphoned into the stdlib bucket just
    // because `len` is stdlib vocabulary — and a project type that
    // happens to share a stdlib module's name (`io`, `os`) must not be
    // masked by rule (a) below either. The owner-based question is asked
    // *before* the name-based one, which is the whole point of Issue 6.
    let project_owns_pair = !is_self_receiver
        && !rh.is_empty()
        && known_names.contains(name)
        && known_names.contains(rh);

    // Stdlib detection runs first so a Vec::push or .clone() lands in
    // the stdlib bucket even when the project also defines a `push`
    // or `clone` method of its own.
    if let Some(std) = stdlib {
        if !rh.is_empty() && !is_self_receiver {
            // (a) Receiver hint matches stdlib directly, or its first
            //     dotted segment does (`os.path` → check `os`). Skipped
            //     when the project owns the (receiver type, method) pair.
            if !project_owns_pair && module_is_stdlib(rh, std, modules, true) {
                return Verdict::Stdlib;
            }
            // (b) Receiver hint is a local alias for a stdlib module
            //     (`import typing as t` → check `typing`).
            if let Some(a) = aliases
                && let Some(resolved) = a.import_aliases.get(rh)
                && module_is_stdlib(resolved, std, modules, false)
            {
                return Verdict::Stdlib;
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
            if std.contains(name) && (!known_names.contains(name) || global_function) {
                return Verdict::Stdlib;
            }
            // (d) Bare call to a name brought in by `from <module> import
            //     name` where <module> is stdlib. Import alias is strong
            //     positive evidence the call really is stdlib, so this
            //     rule applies even when the project also defines `name`.
            if let Some(a) = aliases
                && let Some(source) = a.from_imports.get(name)
                && module_is_stdlib(source, std, modules, false)
            {
                return Verdict::Stdlib;
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
                .map(|resolved| module_is_stdlib(resolved, std, modules, false))
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
    facts
        .iter()
        .map(|f| (f.file, FileAliases::from_facts(f)))
        .collect()
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
            classify_one(&mk_call("len", ""), &known, Some(&std), &[], None),
            Verdict::Stdlib
        );
    }

    #[test]
    fn bare_stdlib_name_with_project_collision_falls_through() {
        let std = names(&["clone"]);
        let known = names(&["clone"]);
        assert_eq!(
            classify_one(&mk_call("clone", ""), &known, Some(&std), &[], None),
            Verdict::Unresolved
        );
    }

    #[test]
    fn dotted_receiver_stdlib_module_is_stdlib() {
        let std = names(&["os"]);
        let known: HashSet<&str> = HashSet::new();
        assert_eq!(
            classify_one(&mk_call("getcwd", "os.path"), &known, Some(&std), &[], None),
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
            classify_one(&mk_call("read", "io"), &known, Some(&std), &[], None),
            Verdict::Unresolved
        );
        // But when the project does NOT own the type, the stdlib module
        // reading still applies.
        let known_empty: HashSet<&str> = HashSet::new();
        assert_eq!(
            classify_one(&mk_call("read", "io"), &known_empty, Some(&std), &[], None),
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
            classify_one(
                &mk_call("TypeVar", "t"),
                &known,
                Some(&std),
                &[],
                Some(&aliases)
            ),
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
            classify_one(
                &mk_call("TypeVar", ""),
                &known,
                Some(&std),
                &[],
                Some(&aliases)
            ),
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
                &[],
                None
            ),
            Verdict::External
        );
        // But when there's no project corpus at all, we keep the
        // permissive fallback so single-file / stack-graph paths still
        // attribute reasonably.
        assert_eq!(
            classify_one(&mk_call("push", "myvec"), &known, Some(&std), &[], None),
            Verdict::Stdlib
        );
    }

    #[test]
    fn method_on_known_stdlib_receiver_is_stdlib() {
        // Rule (a): receiver itself matches stdlib directly.
        let std = names(&["Vec", "push"]);
        let known = names(&["push"]);
        assert_eq!(
            classify_one(&mk_call("push", "Vec"), &known, Some(&std), &[], None),
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
                classify_one(&mk_call("clone", rh), &known, Some(&std), &[], None),
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
        assert!(module_is_stdlib("os.path", &std, &[], true));
        assert!(module_is_stdlib("os", &std, &[], true));
        assert!(!module_is_stdlib("requests.get", &std, &[], true));
    }

    #[test]
    fn qualified_heads_split_on_every_separator() {
        // Rust/C++ `std::fs`, Perl `File::Spec`, R `stats`, Clojure
        // `clojure.string`: the head (or a prefix the module table names)
        // decides.
        let rust = names(&["std"]);
        assert!(module_is_stdlib("std::fs", &rust, &[], true));
        assert!(module_is_stdlib("std::this_thread", &rust, &[], true));
        assert!(module_is_stdlib("::std::chrono", &rust, &[], true));
        assert!(!module_is_stdlib("tokio::fs", &rust, &[], true));
        assert!(!module_is_stdlib("stdx::fs", &rust, &[], true));
        // A value expression is not a module path.
        assert!(!module_is_stdlib("std::forward<T>(j)", &rust, &[], true));
        assert!(module_is_stdlib(
            "POSIX:",
            &names(&[]),
            stdlib::stdlib_modules("perl"),
            true
        ));
        let none = names(&[]);
        let perl = stdlib::stdlib_modules("perl");
        assert!(module_is_stdlib("POSIX", &none, perl, true));
        assert!(module_is_stdlib("File::Spec", &none, perl, true));
        assert!(!module_is_stdlib("IO::Socket::SSL", &none, perl, true));
        assert!(!module_is_stdlib("File", &none, perl, true));
        assert!(!module_is_stdlib("Mojo::File", &none, perl, true));
        let clj = stdlib::stdlib_modules("clojure");
        assert!(module_is_stdlib("clojure.string", &none, clj, true));
        assert!(!module_is_stdlib("clojure.tools.logging", &none, clj, true));
    }

    #[test]
    fn receiverless_qualified_names_split_for_classification() {
        let known: HashSet<&str> = HashSet::new();
        let none = names(&[]);
        let perl = stdlib::stdlib_modules("perl");
        assert_eq!(
            classify_one(
                &mk_call("POSIX::floor", ""),
                &known,
                Some(&none),
                perl,
                None
            ),
            Verdict::Stdlib
        );
        let clj = stdlib::stdlib_modules("clojure");
        assert_eq!(
            classify_one(
                &mk_call("clojure.string/join", ""),
                &known,
                Some(&none),
                clj,
                None
            ),
            Verdict::Stdlib
        );
        assert_eq!(
            classify_one(
                &mk_call("acme.util/join", ""),
                &known,
                Some(&none),
                clj,
                None
            ),
            Verdict::External
        );
        // An operator containing `/` is not a qualified name.
        assert_eq!(split_qualified_name("</>"), ("", "</>", None));
        assert_eq!(split_qualified_name("a.b/c"), ("a.b", "c", Some('/')));
        assert_eq!(split_qualified_name("A::B::c"), ("A::B:", "c", Some(':')));
        assert_eq!(
            split_qualified_name("grid.draw"),
            ("grid", "draw", Some('.'))
        );
        assert_eq!(
            split_qualified_name("\\App\\helper"),
            ("App", "helper", Some('\\'))
        );
        assert_eq!(split_qualified_name("\\strlen"), ("", "strlen", None));
        // R: a dotted name is a name, never a module qualifier.
        let r = stdlib::stdlib_modules("r");
        assert_eq!(
            classify_one(&mk_call("stats.helper", ""), &known, Some(&none), r, None),
            Verdict::External
        );
        assert_eq!(
            classify_one(&mk_call("median", "stats"), &known, Some(&none), r, None),
            Verdict::Stdlib
        );
        // A manifest entry written qualified is read whole.
        let clj_std = names(&["io/copy"]);
        assert_eq!(
            classify_one(&mk_call("io/copy", ""), &known, Some(&clj_std), clj, None),
            Verdict::Stdlib
        );
    }

    #[test]
    fn php_global_function_is_stdlib_even_with_a_workspace_method() {
        // `\strlen($s)` names the global function; a workspace class's
        // `strlen` method cannot be called that way.
        let std = names(&["strlen"]);
        let known = names(&["strlen", "Text"]);
        assert_eq!(
            classify_one(&mk_call("\\strlen", ""), &known, Some(&std), &[], None),
            Verdict::Stdlib
        );
        // Near miss: the unqualified call and the method call keep the
        // project reading.
        assert_eq!(
            classify_one(&mk_call("strlen", ""), &known, Some(&std), &[], None),
            Verdict::Unresolved
        );
        assert_eq!(
            classify_one(&mk_call("strlen", "$t"), &known, Some(&std), &[], None),
            Verdict::External
        );
        assert_eq!(
            classify_one(&mk_call("strlen", "Text"), &known, Some(&std), &[], None),
            Verdict::Unresolved
        );
    }

    #[test]
    fn import_paths_and_class_literals_are_not_split() {
        // `import { f } from "test/utils/f"` / `@import("testing/x.zig")`:
        // a module path's first directory is not a stdlib module.
        let std = names(&["test", "testing", "String", "os"]);
        assert!(!module_is_stdlib(
            "test/utils/randomString",
            &std,
            &[],
            false
        ));
        assert!(!module_is_stdlib("testing/fixtures.zig", &std, &[], false));
        assert!(module_is_stdlib("os.path", &std, &[], false));
        let known = names(&["randomString"]);
        let aliases = FileAliases {
            from_imports: [(
                "randomString".to_string(),
                "test/utils/randomString".to_string(),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        assert_eq!(
            classify_one(
                &mk_call("randomString", ""),
                &known,
                Some(&std),
                &[],
                Some(&aliases)
            ),
            Verdict::Unresolved
        );
        // Kotlin `String::class.asClassName()`: a class literal is a value.
        assert!(!module_is_stdlib("String::class", &std, &[], true));
        assert!(module_is_stdlib("String", &std, &[], true));
    }
}
