//! C# plugin. Task 6b adds the custom `.tsg` rules and extraction.

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn id(&self) -> &'static str {
        "csharp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".cs", ".csx"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c_sharp::LANGUAGE.into()
    }
}
