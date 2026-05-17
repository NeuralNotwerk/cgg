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
//! A receiver hint matching a stdlib type (e.g. `Vec`, `HashMap`,
//! `Arc`) or a bare name matching a stdlib function/method/macro
//! routes the call into the `stdlib` bucket instead of `external`.

use std::collections::HashSet;

use crate::audit::AuditUnresolvedCall;
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

/// Verdict for a single call.
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
enum Verdict {
    Unresolved,
    Stdlib,
    External,
}

/// Classify unresolved calls into unresolved / stdlib / external buckets.
pub fn classify_external(
    unresolved: Vec<AuditUnresolvedCall>,
    known_names: &HashSet<&str>,
    language: &str,
) -> ClassifyResult {
    let stdlib = stdlib::stdlib_names(language);
    let mut result = ClassifyResult::default();
    for call in unresolved {
        match classify_one(&call, known_names, stdlib) {
            Verdict::Unresolved => result.unresolved.push(call),
            Verdict::Stdlib => result.stdlib.push(call),
            Verdict::External => result.external.push(call),
        }
    }
    result
}

fn classify_one(
    call: &AuditUnresolvedCall,
    known_names: &HashSet<&str>,
    stdlib: Option<&HashSet<&str>>,
) -> Verdict {
    let name = call.name.as_str();
    let rh = call.receiver_hint.as_str();
    let is_self_receiver =
        rh == "self" || rh == "Self" || rh == "cls" || rh == "this";

    // Stdlib detection runs first so a Vec::push or .clone() lands in
    // the stdlib bucket even when the project also defines a `push`
    // or `clone` method of its own.
    if let Some(std) = stdlib {
        // Receiver type is a known stdlib type → method call into stdlib.
        if !rh.is_empty() && !is_self_receiver && std.contains(rh) {
            return Verdict::Stdlib;
        }
        // Bare name (no receiver) is a stdlib function or macro.
        if rh.is_empty() && std.contains(name) {
            return Verdict::Stdlib;
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
