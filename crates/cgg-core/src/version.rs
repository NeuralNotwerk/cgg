//! Version constants.
//!
//! These values are mixed into cache keys so a bump of any version
//! invalidates affected cache entries without manual cleanup.

/// Human-readable crate version, derived from the workspace.
pub const CGG_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Bumped whenever the on-disk `FileFacts` layout or any resolver's
/// output shape changes in a way that invalidates cached results.
///
/// Keep this as a plain integer so ordering checks are trivial.
pub const RESOLVER_FORMAT_VERSION: u32 = 1;
