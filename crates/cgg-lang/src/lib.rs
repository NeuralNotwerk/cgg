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

/// Per-run extraction switches, threaded rather than process-global.
///
/// Both values used to be `static`s in this module, set by the driver just
/// before the parallel phase. That was fine for a binary running one
/// analysis and exiting, and wrong for a library: two analyses in one
/// process wrote each other's switches, so `cgg::analyze` had to take a
/// process-wide lock for its whole duration to stop them. Passing them
/// instead removes the globals, the lock, and the reason a Python caller
/// could not use a thread pool.
///
/// Borrowed, not owned: one of these is built per run in `cgg::analyze` and
/// shared by every worker, so `extract` pays a pointer rather than a clone.
#[derive(Clone, Copy, Debug)]
pub struct ExtractCtx<'a> {
    /// Collect the extra signals only `--dead-code` consumes.
    ///
    /// Unreachable-statement detection and reflection capture are an extra
    /// tree walk per file: ~6% on Go and ~11% on JavaScript on the
    /// benchmark corpus, so an ordinary run must not pay for them.
    pub deadcode_signals: bool,

    /// Registrar verbs contributed by user-authored framework rules,
    /// pre-lowercased.
    ///
    /// Argument capture is gated on the built-in verb list so an ordinary
    /// `foo(x)` costs nothing — measured on TypeORM, capturing
    /// unconditionally doubled the run and minted four thousand nodes for
    /// `describe('...', () => {})` blocks shaped exactly like a route
    /// registration and not one. A user rule naming a verb cgg does not
    /// ship would be inert under that gate, which is what this widens.
    extra_verbs: &'a std::collections::HashSet<String>,

    /// The language currently being extracted, or `""` for "any".
    ///
    /// A verb can only match a rule of its own language, so knowing it
    /// narrows the gate from the union of every rule in the table to the
    /// handful that could actually fire. Empty means the caller did not
    /// say, and the union is used — the conservative answer.
    language: &'a str,
}

/// The built-in registrar verbs, lowercased once per process.
///
/// Still a global, and legitimately so: it is a cache of a compile-time
/// constant, identical for every run, not per-run state.
fn builtin_verbs() -> &'static std::collections::HashSet<String> {
    static SET: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    SET.get_or_init(|| {
        cgg_core::frameworks::rules::registrar_verbs()
            .iter()
            .map(|v| v.to_ascii_lowercase())
            .collect()
    })
}

/// The built-in verbs of one language, lowercased once per process.
///
/// `None` when the language has no registrar-bearing rule, which is the
/// cheapest possible answer: every call in such a file skips the gate.
fn builtin_verbs_for(
    language: &str,
) -> Option<&'static std::collections::HashSet<String>> {
    #[allow(clippy::type_complexity)]
    static BY_LANG: std::sync::OnceLock<
        std::collections::HashMap<&'static str, std::collections::HashSet<String>>,
    > = std::sync::OnceLock::new();
    BY_LANG
        .get_or_init(|| {
            let mut m: std::collections::HashMap<
                &'static str,
                std::collections::HashSet<String>,
            > = std::collections::HashMap::new();
            for spec in cgg_core::frameworks::rules::SPECS {
                if spec.registrars.is_empty() {
                    continue;
                }
                m.entry(spec.language)
                    .or_default()
                    .extend(spec.registrars.iter().map(|v| v.to_ascii_lowercase()));
            }
            m
        })
        .get(language)
}

/// A shared empty set, so [`ExtractCtx::plain`] allocates nothing.
fn no_extra_verbs() -> &'static std::collections::HashSet<String> {
    static EMPTY: std::sync::OnceLock<std::collections::HashSet<String>> =
        std::sync::OnceLock::new();
    EMPTY.get_or_init(std::collections::HashSet::new)
}

impl<'a> ExtractCtx<'a> {
    /// Context for a run with user framework rules.
    pub fn new(
        deadcode_signals: bool,
        extra_verbs: &'a std::collections::HashSet<String>,
    ) -> Self {
        Self {
            deadcode_signals,
            extra_verbs,
            language: "",
        }
    }

    /// The same context, narrowed to one language's registrar verbs.
    ///
    /// Called once per file by the pipeline, which knows the language the
    /// generic constructor does not. Copies two pointers and a bool.
    #[must_use]
    pub fn for_language(self, language: &'a str) -> Self {
        Self { language, ..self }
    }

    /// An ordinary run: no dead-code signals, no user rules.
    ///
    /// What the plugin tests use, and the default for anyone calling
    /// `extract` directly.
    pub fn plain() -> ExtractCtx<'static> {
        ExtractCtx {
            deadcode_signals: false,
            extra_verbs: no_extra_verbs(),
            language: "",
        }
    }

    /// Whether a call's verb could ever be matched by a framework rule.
    ///
    /// Runs once per call site in every file, and misses for nearly all of
    /// them, so the miss path must not allocate: probe the borrowed string
    /// and only build a lowercased copy when the verb actually contains an
    /// uppercase byte (Go's `GET`, NestJS's `Get`). An unconditional
    /// `to_ascii_lowercase()` here allocated per call site and gave back
    /// everything the set lookup won.
    ///
    /// A set, not a linear scan: the built-in list grew from ~30 to 157
    /// verbs, which turned this into O(call sites x 157) string compares
    /// and showed up as a ~25% extraction regression.
    #[inline]
    pub fn is_registrar_verb(&self, verb: &str) -> bool {
        if verb.is_empty() {
            return false;
        }
        // Narrowed to this file's language when the caller named one.
        // A language with no registrar-bearing rule can only match a
        // user verb, and most calls in every other language stop here
        // instead of probing the union of the whole table.
        let builtin = if self.language.is_empty() {
            builtin_verbs()
        } else {
            match builtin_verbs_for(self.language) {
                Some(set) => set,
                None => {
                    return !self.extra_verbs.is_empty()
                        && (self.extra_verbs.contains(verb)
                            || self.extra_verbs.contains(&verb.to_ascii_lowercase()));
                }
            }
        };
        if builtin.contains(verb) {
            return true;
        }
        // `is_empty` first: the overwhelmingly common case is no user rules,
        // and a length check beats hashing the verb a second time.
        if !self.extra_verbs.is_empty() && self.extra_verbs.contains(verb) {
            return true;
        }
        if verb.bytes().any(|b| b.is_ascii_uppercase()) {
            let lower = verb.to_ascii_lowercase();
            return builtin.contains(&lower)
                || (!self.extra_verbs.is_empty() && self.extra_verbs.contains(&lower));
        }
        false
    }
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
        _ctx: &ExtractCtx<'_>,
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

    /// A verb belongs to its own language's rules and no other's.
    ///
    /// `Start` is a registrar only because Go's aws-lambda rule passes
    /// the handler to `lambda.Start`. Before the gate was narrowed,
    /// every Ruby, PHP and Python file paid an argument scan for its own
    /// `start` calls — measured as ~5% on a Go corpus once the verb was
    /// added, and the same tax already existed for every other rule's
    /// vocabulary.
    #[test]
    fn the_registrar_gate_is_scoped_to_one_language() {
        let ctx = ExtractCtx::plain();
        // Unscoped: the union, which is the conservative default.
        assert!(ctx.is_registrar_verb("Start"));
        assert!(ctx.is_registrar_verb("middy"));

        // Scoped: only the language that registers it.
        assert!(ctx.for_language("go").is_registrar_verb("Start"));
        assert!(!ctx.for_language("python").is_registrar_verb("Start"));
        assert!(!ctx.for_language("ruby").is_registrar_verb("Start"));

        assert!(ctx.for_language("javascript").is_registrar_verb("middy"));
        assert!(!ctx.for_language("go").is_registrar_verb("middy"));

        // A language with no registrar-bearing rule matches nothing.
        assert!(!ctx.for_language("cobol").is_registrar_verb("Start"));
    }

    /// Two contexts do not see each other's verbs.
    ///
    /// The property that matters, stated directly. It used to be violated by
    /// a `OnceLock`: the second project analyzed in a process kept applying
    /// the first one's rules. Now the verbs travel with the context, so
    /// there is no shared cell for a second run to inherit — this test can
    /// only fail if someone reintroduces one.
    #[test]
    fn contexts_do_not_share_registrar_verbs() {
        use std::collections::HashSet;

        // Verbs no fixture in this crate contains.
        let a: HashSet<String> = ["zzq_alpha".to_string()].into_iter().collect();
        let b: HashSet<String> = ["zzq_beta".to_string()].into_iter().collect();
        let ctx_a = ExtractCtx::new(false, &a);
        let ctx_b = ExtractCtx::new(false, &b);

        assert!(ctx_a.is_registrar_verb("zzq_alpha"));
        assert!(!ctx_a.is_registrar_verb("zzq_beta"));
        assert!(ctx_b.is_registrar_verb("zzq_beta"));
        assert!(!ctx_b.is_registrar_verb("zzq_alpha"));

        // Interleaving them changes nothing — no order dependence to have.
        assert!(ctx_a.is_registrar_verb("zzq_alpha"));

        // A plain context sees neither, and the built-in table is intact
        // for all three.
        let plain = ExtractCtx::plain();
        assert!(!plain.is_registrar_verb("zzq_alpha"));
        assert!(!plain.is_registrar_verb("zzq_beta"));
        for c in [&ctx_a, &ctx_b, &plain] {
            assert!(c.is_registrar_verb("route"), "built-in table disturbed");
        }
        assert!(!plain.is_registrar_verb(""));
    }

    /// `deadcode_signals` is per-context, not per-process.
    #[test]
    fn deadcode_signals_travel_with_the_context() {
        let empty = std::collections::HashSet::new();
        assert!(ExtractCtx::new(true, &empty).deadcode_signals);
        assert!(!ExtractCtx::new(false, &empty).deadcode_signals);
        assert!(!ExtractCtx::plain().deadcode_signals);
    }
}
