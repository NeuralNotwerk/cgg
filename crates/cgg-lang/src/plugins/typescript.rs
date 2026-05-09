//! TypeScript plugin.
//!
//! Task 3: identity only. Task 7a adds callable extraction.
//! Uses the TSX grammar — a proper superset of TypeScript source.

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct TypeScriptPlugin;

impl LanguagePlugin for TypeScriptPlugin {
    fn id(&self) -> &'static str {
        "typescript"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".ts", ".tsx", ".mts", ".cts"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["ts-node", "tsx", "deno", "bun"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_typescript::language_tsx()
    }
}
