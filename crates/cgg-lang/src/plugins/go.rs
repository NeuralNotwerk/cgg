//! Go plugin. Task 6b fills in resolution; extraction lands alongside.

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct GoPlugin;

impl LanguagePlugin for GoPlugin {
    fn id(&self) -> &'static str {
        "go"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".go"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_go::language()
    }
}
