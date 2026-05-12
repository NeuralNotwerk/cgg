//! Language plugin trait, registry, detection, and parser pool.
//!
//! Task 3 landed detection and parser pooling. Task 4 adds the
//! [`LanguagePlugin::extract`] entry point through which each plugin
//! produces `FileFacts` from a parsed tree-sitter tree.

#![deny(missing_debug_implementations)]
#![warn(unreachable_pub)]

pub mod detect;
pub mod parser;
pub mod plugins;

use std::fmt;
use std::path::Path;

use cgg_core::{FileFacts, ids::FileId};

pub use cgg_core as core;
pub use detect::{DetectResult, DetectVerdict, LanguageDetector};
pub use parser::{ParseOutcome, ParserPool};

/// The resolver family a plugin expects to be driven by.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum ResolverKind {
    IntraFile,
    StackGraphs,
    Custom,
}

impl fmt::Display for ResolverKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ResolverKind::IntraFile => "intra-file",
            ResolverKind::StackGraphs => "stack-graphs",
            ResolverKind::Custom => "custom",
        })
    }
}

/// Language plugin contract.
///
/// `id` / `extensions` / `shebangs` / `resolver_kind` / `ts_language`
/// are stable and small — the detector, parser pool, and driver all
/// rely on them. `extract` is the per-file analysis entry point.
pub trait LanguagePlugin: Send + Sync + fmt::Debug {
    fn id(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn resolver_kind(&self) -> ResolverKind;
    fn ts_language(&self) -> tree_sitter::Language;

    /// Walk `tree` once and produce the definitions + references +
    /// imports for the file. The default returns an empty `FileFacts`,
    /// which is appropriate for languages that don't have a plugin
    /// implementation yet (they still parse, they just contribute no
    /// callables).
    fn extract(
        &self,
        _file: FileId,
        _path: &Path,
        _tree: &tree_sitter::Tree,
        _source: &[u8],
    ) -> FileFacts {
        FileFacts::new(_file, _path.to_path_buf(), self.id())
    }
}

/// Registry of registered plugins.
#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: Vec<Box<dyn LanguagePlugin>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, plugin: Box<dyn LanguagePlugin>) {
        self.plugins.push(plugin);
    }

    pub fn all(&self) -> &[Box<dyn LanguagePlugin>] {
        &self.plugins
    }

    pub fn by_id(&self, id: &str) -> Option<&dyn LanguagePlugin> {
        self.plugins
            .iter()
            .find(|p| p.id() == id)
            .map(|p| &**p)
    }

    /// Registry preloaded with every v1 language plugin.
    pub fn with_v1_plugins() -> Self {
        let mut reg = Self::new();
        plugins::register_all(&mut reg);
        reg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_registry_has_21_languages() {
        let reg = PluginRegistry::with_v1_plugins();
        assert_eq!(reg.all().len(), 24);
    }
}
