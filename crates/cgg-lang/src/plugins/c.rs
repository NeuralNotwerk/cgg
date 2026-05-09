//! C plugin. Task 7 adds the preprocessor-aware resolver.

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct CPlugin;

impl LanguagePlugin for CPlugin {
    fn id(&self) -> &'static str {
        "c"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".c", ".h"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::Custom
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c::language()
    }
}
