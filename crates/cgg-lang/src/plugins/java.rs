//! Java plugin. Task 6 adds extraction + stack-graphs wiring.

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct JavaPlugin;

impl LanguagePlugin for JavaPlugin {
    fn id(&self) -> &'static str {
        "java"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".java"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_java::language()
    }
}
