//! Qualified-name helpers shared across resolver stages.
//!
//! These operate purely on the string form of a callable's
//! `qualified_name` — no type inference, no allocation. The dominant
//! cost of the precision fixes in `necessary_fixes.md` is exactly this:
//! splitting and comparing already-parsed qualified names.

/// Extract the owner type/namespace from a fully-qualified callable
/// name — the path segment immediately before the simple name.
///
/// Handles the forms cgg's plugins emit:
/// * inherent / free path: `crate::mod::Type::method` → `Type`
/// * Rust trait-impl wrapper: `<Type as Trait>::method` → `Type`
/// * generic owner: `Type<V>::method` → `Type`
/// * dot-joined languages: `module.Class.method` → `Class`
///
/// Returns `None` for a name with no owner segment (a free function
/// like `crate::mod::func` returns `mod`; a bare `func` returns
/// `None`). The result borrows from `qn`.
pub fn owner_from_qn(qn: &str) -> Option<&str> {
    // Strip the trailing simple-name segment, then take the last
    // segment of what remains as the owner.
    let (prefix, _simple) = split_last_segment(qn)?;
    let owner = match split_last_segment(prefix) {
        Some((_, last)) => last,
        None => prefix,
    };
    let owner = normalize_owner(owner);
    if owner.is_empty() {
        None
    } else {
        Some(owner)
    }
}

/// Split a qualified name into `(prefix, last_segment)` at the rightmost
/// path separator (`::` or `.`, whichever appears later).
fn split_last_segment(qn: &str) -> Option<(&str, &str)> {
    let colon = qn.rfind("::");
    let dot = qn.rfind('.');
    match (colon, dot) {
        (Some(c), Some(d)) if c >= d => Some((&qn[..c], &qn[c + 2..])),
        (Some(_), Some(d)) => Some((&qn[..d], &qn[d + 1..])),
        (Some(c), None) => Some((&qn[..c], &qn[c + 2..])),
        (None, Some(d)) => Some((&qn[..d], &qn[d + 1..])),
        (None, None) => None,
    }
}

/// Normalize an owner segment to its bare type name:
/// `<Type as Trait>` → `Type`, `Type<Generic>` → `Type`.
fn normalize_owner(owner: &str) -> &str {
    let owner = owner.strip_prefix('<').unwrap_or(owner);
    // Trait-impl wrapper: `Type as Trait` → `Type`.
    if let Some(idx) = owner.find(" as ") {
        return owner[..idx].trim();
    }
    // Strip generic parameters and any stray closing angle bracket.
    let end = owner.find('<').unwrap_or(owner.len());
    owner[..end].trim_end_matches('>').trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inherent_rust_path() {
        assert_eq!(owner_from_qn("crate::mod::Parser::new"), Some("Parser"));
        assert_eq!(owner_from_qn("m::Cursor::new"), Some("Cursor"));
    }

    #[test]
    fn trait_impl_wrapper() {
        assert_eq!(
            owner_from_qn("crate::io::<DiskStorage as Storage>::put"),
            Some("DiskStorage")
        );
    }

    #[test]
    fn generic_owner_stripped() {
        assert_eq!(owner_from_qn("m::Map<K, V>::insert"), Some("Map"));
    }

    #[test]
    fn dot_joined() {
        assert_eq!(owner_from_qn("module.Class.method"), Some("Class"));
        assert_eq!(owner_from_qn("pkg.svc.S.handle"), Some("S"));
    }

    #[test]
    fn free_function_has_no_owner_type() {
        // A single-segment name has no owner at all.
        assert_eq!(owner_from_qn("func"), None);
        // `mod::func` yields the module as the "owner" segment — callers
        // that only want type owners can compare and discard.
        assert_eq!(owner_from_qn("mod::func"), Some("mod"));
    }
}
