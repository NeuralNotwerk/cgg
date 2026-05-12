//! External call classification.
//!
//! Distinguishes call sites that target symbols defined within the
//! scanned project from those targeting external code (stdlib,
//! third-party deps, framework methods).

use std::collections::HashSet;

use crate::audit::AuditUnresolvedCall;
use crate::stdlib;

/// Result of classifying unresolved calls.
#[derive(Debug, Default)]
pub struct ClassifyResult {
    /// Calls whose target name matches a definition in the scanned files
    /// but couldn't be linked (genuinely unresolved).
    pub unresolved: Vec<AuditUnresolvedCall>,
    /// Calls whose target name does not match any definition in the
    /// scanned files (external/out-of-scope).
    pub external: Vec<AuditUnresolvedCall>,
}

/// Classify unresolved calls as internal (unresolved) or external.
///
/// A call is external if:
/// 1. Its `name` does not appear in `known_names`, OR
/// 2. Its `receiver_hint` is a type not defined in the project, OR
/// 3. Its `name` is in the stdlib manifest for the file's language AND
///    the name is NOT also defined in the project (stdlib shadow check).
pub fn classify_external(
    unresolved: Vec<AuditUnresolvedCall>,
    known_names: &HashSet<&str>,
    language: &str,
) -> ClassifyResult {
    let stdlib = stdlib::stdlib_names(language);
    let mut result = ClassifyResult::default();
    for call in unresolved {
        if is_external(&call, known_names, stdlib) {
            result.external.push(call);
        } else {
            result.unresolved.push(call);
        }
    }
    result
}

fn is_external(
    call: &AuditUnresolvedCall,
    known_names: &HashSet<&str>,
    stdlib: Option<&HashSet<&str>>,
) -> bool {
    let name = call.name.as_str();
    let rh = call.receiver_hint.as_str();

    // Rule 1: name not defined anywhere in the project
    if !known_names.contains(name) {
        return true;
    }

    // Rule 2: receiver is a type not defined in the project
    if !rh.is_empty() && rh != "self" && rh != "Self" && rh != "cls" {
        if !known_names.contains(rh) {
            return true;
        }
        // Rule 2b: receiver IS in stdlib (even if a same-named type exists locally)
        if let Some(std) = stdlib {
            if std.contains(rh) && !known_names.contains(rh) {
                return true;
            }
        }
    }

    // Rule 3: name is in stdlib AND has a non-empty receiver that's in stdlib
    // This catches cases like Vec::new where "new" exists in the project
    // but the receiver "Vec" is stdlib.
    if let Some(std) = stdlib {
        if !rh.is_empty() && rh != "self" && rh != "Self" && rh != "cls" {
            if std.contains(rh) {
                return true;
            }
        }
    }

    false
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
