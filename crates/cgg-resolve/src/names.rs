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
    let owner = normalize_owner(raw_owner(qn)?);
    if owner.is_empty() { None } else { Some(owner) }
}

/// Like [`owner_from_qn`], but for a trait-impl wrapper keeps the
/// implementing type's FULL qualified path (`kvm_bindings::CpuId`)
/// rather than reducing it to its bare final segment (`CpuId`).
///
/// Used only to match a receiver's full path VERBATIM against a
/// candidate's owner when the receiver's head names a crate outside the
/// workspace. A bare-name match there is a coincidence, not a call:
/// `kvm_ioctls::Kvm::new()` (an external crate's constructor) must not
/// bind to a LOCAL `vmm::vstate::kvm::Kvm::new` just because both
/// owners reduce to the same bare `Kvm` — `kvm_ioctls` is external and
/// the two `Kvm`s are unrelated types that happen to share a name. This
/// function lets the caller compare the full text instead.
pub fn full_owner_from_qn(qn: &str) -> Option<&str> {
    let owner = normalize_owner_keep_qualified(raw_owner(qn)?);
    if owner.is_empty() { None } else { Some(owner) }
}

/// Strip the trailing simple-name segment, then take the last segment
/// of what remains as the (not yet normalized) owner. Shared by
/// [`owner_from_qn`] and [`full_owner_from_qn`], which differ only in
/// how they normalize.
fn raw_owner(qn: &str) -> Option<&str> {
    let (prefix, _simple) = split_last_segment(qn)?;
    // A trait-impl wrapper (`<A as Trait<...>>`) can have a `::` INSIDE
    // it — `A` or the trait's own generic argument can be a qualified
    // path (`<kvm_bindings::CpuId as TryFrom<Cpuid>>`,
    // `<Cpuid as TryFrom<kvm_bindings::CpuId>>`). `split_last_segment`'s
    // rightmost-`::` search cannot tell that `::` apart from the one
    // separating an outer module path from the wrapper, so it can land
    // INSIDE the wrapper and return a fragment of the trait's generic
    // argument as the "owner" instead of `A` — measured: both directions
    // of one `TryFrom` pair ended up registered under the SAME (wrong)
    // owner key, so a query for either owner returned both impls.
    // Detected by the literal " as " a wrapper always contains (never
    // present in a plain qualified path); handled by finding the
    // wrapper's OWN leading `<` — which, in a module-path-prefixed
    // qualified name, is unambiguous, since a plain module segment is
    // never itself generic — rather than splitting on `::`.
    if prefix.contains(" as ") {
        let open = prefix.find('<')?;
        Some(&prefix[open..])
    } else {
        match split_last_segment(prefix) {
            Some((_, last)) => Some(last),
            None => Some(prefix),
        }
    }
}

/// Split a qualified name into `(prefix, last_segment)` at the rightmost
/// path separator (`::` or `.`, whichever appears later).
///
/// Public because the rollup pass groups by the *path to* the owner
/// rather than by the owner's bare name — `crate::io::DiskStorage`, not
/// `DiskStorage` — and two `Parser` types in different modules must not
/// collapse into one group.
pub fn split_last_segment(qn: &str) -> Option<(&str, &str)> {
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
///
/// Public for the same reason as [`split_last_segment`]: a rollup key
/// built from a raw path prefix would give a Rust trait impl the group
/// name `crate::io::<DiskStorage as Storage>`, which is both ugly and a
/// *different* group from the type's inherent methods.
pub fn normalize_owner(owner: &str) -> &str {
    let owner = owner.strip_prefix('<').unwrap_or(owner);
    // Trait-impl wrapper: `Type as Trait` → `Type`. `Type` here may
    // itself be a qualified path (`kvm_bindings::CpuId`) when the
    // wrapper was isolated by `owner_from_qn`'s bracket-aware split
    // rather than by a plain rightmost-`::` search, so it is reduced to
    // its bare final segment — matching how a receiver written the same
    // way (an imported bare `CpuId`) is compared against this key. This
    // one addition aside, this branch is deliberately UNCHANGED from
    // before: it does NOT also strip a generic (`Option<CpuTemplateType>`
    // stays whole) the way the non-wrapper branch below does. Measured:
    // adding that generic-strip here broke 25+25+23 real edges on
    // llmitm-v5 whose receiver is typed with the generic still attached
    // (`type_hints` rewrites a variable's receiver to its full type,
    // generic included) — `Option<CpuTemplateType>` must stay
    // `Option<CpuTemplateType>`, not become bare `Option`.
    if let Some(idx) = owner.find(" as ") {
        let ty = owner[..idx].trim();
        return ty.rsplit("::").next().unwrap_or(ty);
    }
    // Strip generic parameters and any stray closing angle bracket.
    let end = owner.find('<').unwrap_or(owner.len());
    owner[..end].trim_end_matches('>').trim()
}

/// Same as [`normalize_owner`]'s wrapper branch, but keeps the
/// implementing type's qualified path whole (`kvm_bindings::CpuId`
/// stays `kvm_bindings::CpuId`, not reduced to `CpuId`) — see
/// [`full_owner_from_qn`].
fn normalize_owner_keep_qualified(owner: &str) -> &str {
    let owner = owner.strip_prefix('<').unwrap_or(owner);
    if let Some(idx) = owner.find(" as ") {
        return owner[..idx].trim();
    }
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

    // c1 amendment 3: a trait-impl wrapper whose implementing type OR
    // whose trait's generic argument is itself a qualified path (has its
    // own `::`) must not let that inner `::` be mistaken for the
    // boundary between an outer module path and the wrapper. Both
    // directions of a `TryFrom` pair must resolve to their OWN owner,
    // never the other's.
    #[test]
    fn trait_impl_wrapper_with_qualified_generic_argument() {
        assert_eq!(
            owner_from_qn(
                "vmm::cpuid::<kvm_bindings::CpuId as TryFrom<Cpuid>>::try_from"
            ),
            Some("CpuId")
        );
        assert_eq!(
            owner_from_qn(
                "vmm::cpuid::<Cpuid as TryFrom<kvm_bindings::CpuId>>::try_from"
            ),
            Some("Cpuid")
        );
    }

    // c1 fourth amendment: `full_owner_from_qn` keeps the implementing
    // type's qualified path whole, for matching a receiver's FULL path
    // verbatim rather than by bare last segment.
    #[test]
    fn full_owner_keeps_qualified_path() {
        assert_eq!(
            full_owner_from_qn(
                "vmm::cpuid::<kvm_bindings::CpuId as TryFrom<Cpuid>>::try_from"
            ),
            Some("kvm_bindings::CpuId")
        );
        // Non-wrapper paths are already bare — no difference from
        // `owner_from_qn`.
        assert_eq!(
            full_owner_from_qn("crate::mod::Parser::new"),
            Some("Parser")
        );
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
