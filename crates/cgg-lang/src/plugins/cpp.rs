//! C++ plugin. Task 7 adds the preprocessor-aware resolver.

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct CppPlugin;

impl LanguagePlugin for CppPlugin {
    fn id(&self) -> &'static str {
        "cpp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".cc", ".cpp", ".cxx", ".C", ".hpp", ".hh", ".hxx"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::Custom
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_cpp::LANGUAGE.into()
    }
}
