//! Language plugin trait, registry, detection, and parser pool.
//!
//! Task 3 landed detection and parser pooling. Task 4 adds the
//! [`LanguagePlugin::extract`] entry point through which each plugin
//! produces `FileFacts` from a parsed tree-sitter tree.

#![deny(missing_debug_implementations)]
#![warn(unreachable_pub)]
// `..Default::default()` on a fully-specified struct literal is
// intentional across the plugins: it is what keeps ~90 construction
// sites compiling unchanged when a new optional extraction signal is
// added to `DefRecord`. The lint is right that it has no effect
// *today* — the point is the day a field lands.
#![allow(clippy::needless_update)]

pub mod detect;
pub mod notebook;
pub mod parser;
pub mod plugins;

use std::fmt;
use std::path::Path;

use cgg_core::{FileFacts, ids::FileId};
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the run needs dead-code-only extraction signals.
///
/// Unreachable-statement detection and reflection capture are an extra
/// tree walk per file and feed nothing but `--dead-code`. Measured on
/// the benchmark corpus they cost ~6% on Go and ~11% on JavaScript, so
/// an ordinary `cgg <path>` must not pay for them.
///
/// A process-global rather than a parameter because the alternative is
/// widening `LanguagePlugin::extract` across all 44 plugins for a value
/// none of them vary. The driver sets it once before the parallel
/// extraction phase and nothing writes it again, so the reads below are
/// uncontended and `Relaxed` is sufficient.
static DEADCODE_SIGNALS: AtomicBool = AtomicBool::new(false);

/// Enable the dead-code-only extraction signals for this process.
pub fn set_deadcode_signals(on: bool) {
    DEADCODE_SIGNALS.store(on, Ordering::Relaxed);
}

/// Whether to collect dead-code-only extraction signals.
#[inline]
pub fn deadcode_signals() -> bool {
    DEADCODE_SIGNALS.load(Ordering::Relaxed)
}

/// Registrar verbs contributed by user-authored framework rules.
///
/// Argument capture is gated on the built-in verb list so that an
/// ordinary `foo(x)` costs nothing — measured on TypeORM, capturing
/// unconditionally doubled the run and minted four thousand nodes for
/// `describe('...', () => {})` blocks that are shaped exactly like a
/// route registration and are not one.
///
/// A user rule naming a verb cgg does not ship would be silently inert
/// under that gate, which is the failure mode this whole feature exists
/// to avoid. The driver widens the gate from the config file before
/// extraction starts.
///
/// A process-global for the same reason as [`DEADCODE_SIGNALS`]: the
/// alternative is widening `LanguagePlugin::extract` across 44 plugins
/// for a value none of them vary. Written once before the parallel
/// phase and never again.
static EXTRA_REGISTRAR_VERBS: std::sync::OnceLock<Vec<String>> =
    std::sync::OnceLock::new();

/// Register verbs from user-authored framework rules. Idempotent; only
/// the first call takes effect.
pub fn set_extra_registrar_verbs(verbs: Vec<String>) {
    let _ = EXTRA_REGISTRAR_VERBS.set(verbs);
}

/// Whether a call's verb could ever be matched by a framework rule.
#[inline]
pub fn is_registrar_verb(verb: &str) -> bool {
    if verb.is_empty() {
        return false;
    }
    if cgg_core::frameworks::rules::registrar_verbs()
        .iter()
        .any(|v| v.eq_ignore_ascii_case(verb))
    {
        return true;
    }
    EXTRA_REGISTRAR_VERBS
        .get()
        .is_some_and(|v| v.iter().any(|x| x.eq_ignore_ascii_case(verb)))
}

pub use cgg_core as core;
pub use detect::{DetectResult, DetectVerdict, LanguageDetector};
pub use parser::{ParseOutcome, ParserPool};

/// Which optional extraction signals a plugin actually produces.
///
/// This is a *manifest*, not a behaviour switch. It lets a consumer
/// distinguish "this definition genuinely has no attributes" from "cgg
/// never looked for attributes in this language" — the difference
/// between a finding and a guess. Dead-code analysis reports it as a
/// per-language capability table so uneven coverage is disclosed rather
/// than silently papered over.
///
/// Defaults to all-false, so a plugin that extracts nothing optional
/// needs no implementation and no edit.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct PluginSignals {
    /// Populates `DefRecord::visibility` / `vis`.
    pub visibility: bool,
    /// Populates `DefRecord::attributes` with real source attributes or
    /// decorators (not hardcoded node-kind tags).
    pub attributes: bool,
    /// Records what the file exports (`__all__`, `export`, `pub use`).
    pub exports: bool,
    /// Tags definitions with a `TestRole`.
    pub test_defs: bool,
    /// Captures identifier-shaped literals used for dynamic dispatch
    /// (`getattr(o, "m")`), as a suppression-only signal.
    /// Records functions passed by name as values (`register(handler)`).
    pub value_refs: bool,
    pub dyn_uses: bool,
    /// Detects statements following an unconditional terminator.
    pub unreachable: bool,
    /// Records which interface/trait a definition implements.
    pub impls: bool,
}

/// Language plugin contract.
///
/// `id` / `extensions` / `shebangs` / `ts_language` are stable and
/// small — the detector, parser pool, and driver all rely on them.
/// `extract` is the per-file analysis entry point.
pub trait LanguagePlugin: Send + Sync + fmt::Debug {
    fn id(&self) -> &'static str;
    fn extensions(&self) -> &'static [&'static str];
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }

    /// Which optional extraction signals this plugin actually produces.
    /// Cheap and constant; never called per file.
    fn signals(&self) -> PluginSignals {
        PluginSignals::default()
    }

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
        self.plugins.iter().find(|p| p.id() == id).map(|p| &**p)
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
    fn v1_registry_has_all_languages() {
        let reg = PluginRegistry::with_v1_plugins();
        assert_eq!(reg.all().len(), 44);
    }
}

