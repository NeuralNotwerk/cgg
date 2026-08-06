//! Resolver service trait.
//!
//! The trait is modeled on the subset of LSP operations that a call
//! graph generator actually needs. Concrete implementations (added in
//! later tasks) include a stack-graphs-backed resolver for
//! Python / JS / TS / Java, custom `.tsg` rules for Rust / Go / C#,
//! and a purpose-built preprocessor-aware resolver for C / C++.

#![deny(missing_debug_implementations)]
#![warn(unreachable_pub)]
// See cgg-lang/src/lib.rs: the `..Default::default()` spreads are
// deliberate source-compatibility for future optional fields.
#![allow(clippy::needless_update)]

pub mod cross_file;
pub mod deadcode;
pub mod dispatch;
pub mod ffi;
pub mod frameworks;
pub mod intra_file;
pub mod names;
pub mod stack_graphs_resolver;
pub mod type_hints;

use cgg_core::{CallableId, FileId};
use serde::{Deserialize, Serialize};

/// A cursor position in source: file plus byte offset.
///
/// Byte offsets (not line/column) are the canonical form inside the
/// resolver because tree-sitter ranges are byte-indexed; conversion
/// to line/column happens at the audit boundary.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Location {
    pub file: FileId,
    pub byte: u32,
}

/// A named symbol discovered inside a file (function, method,
/// constructor, …). Flows out of `document_symbols`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Symbol {
    pub id: CallableId,
    pub qualified_name: String,
    pub start_byte: u32,
    pub end_byte: u32,
}

/// A single call site. Flows out of `incoming_calls` / `outgoing_calls`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CallRef {
    pub from: CallableId,
    pub to: CallableId,
    pub site: Location,
}

/// The resolver's contract, designed to mirror the LSP operations we
/// would otherwise call out to.
pub trait ResolverService: Send + Sync {
    /// Stable id identifying which resolver produced results
    /// (`"stack-graphs:python"`, `"tsg:rust"`, `"custom:cpp"`).
    fn id(&self) -> &str;

    /// All definitions discovered inside the given file.
    fn document_symbols(&self, file: FileId) -> Vec<Symbol>;

    /// Resolve the symbol(s) at `cursor` to their defining
    /// location(s). Multiple locations indicate ambiguity.
    fn definition(&self, cursor: Location) -> Vec<Location>;

    /// All references to the given symbol across the scanned roots.
    fn references(&self, symbol: CallableId) -> Vec<Location>;

    /// The callable (if any) whose body contains `cursor`.
    fn enclosing_callable(&self, cursor: Location) -> Option<CallableId>;

    fn incoming_calls(&self, callable: CallableId) -> Vec<CallRef>;
    fn outgoing_calls(&self, callable: CallableId) -> Vec<CallRef>;
}

/// Stub resolver used before any concrete resolver is wired up. Every
/// operation returns the empty result. Useful for Task 1's integration
/// bring-up and for tests that only need the graph structure.
#[derive(Debug, Default)]
pub struct NoopResolver;

impl NoopResolver {
    pub fn new() -> Self {
        Self
    }
}

impl ResolverService for NoopResolver {
    fn id(&self) -> &str {
        "noop"
    }
    fn document_symbols(&self, _file: FileId) -> Vec<Symbol> {
        Vec::new()
    }
    fn definition(&self, _cursor: Location) -> Vec<Location> {
        Vec::new()
    }
    fn references(&self, _symbol: CallableId) -> Vec<Location> {
        Vec::new()
    }
    fn enclosing_callable(&self, _cursor: Location) -> Option<CallableId> {
        None
    }
    fn incoming_calls(&self, _callable: CallableId) -> Vec<CallRef> {
        Vec::new()
    }
    fn outgoing_calls(&self, _callable: CallableId) -> Vec<CallRef> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_is_noop() {
        let r = NoopResolver::new();
        assert_eq!(r.id(), "noop");
        assert!(r.document_symbols(FileId::new(0)).is_empty());
        assert!(
            r.enclosing_callable(Location {
                file: FileId::new(0),
                byte: 0,
            })
            .is_none()
        );
    }
}
