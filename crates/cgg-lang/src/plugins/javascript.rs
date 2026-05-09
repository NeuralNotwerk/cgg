//! JavaScript plugin.
//!
//! Task 3: identity only. Task 7a adds callable extraction.

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct JavaScriptPlugin;

impl LanguagePlugin for JavaScriptPlugin {
    fn id(&self) -> &'static str {
        "javascript"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".js", ".mjs", ".cjs", ".jsx"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["node"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_javascript::LANGUAGE.into()
    }
}
