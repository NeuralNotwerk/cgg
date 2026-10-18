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

pub mod closure_types;
pub mod detect;
pub mod locals;
pub mod notebook;
pub mod parser;
pub mod plugins;
pub mod receiver;

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

    /// Record value references that carry a receiver (Java method
    /// references, C# method groups) and the value positions added with
    /// them (Python `return f`, `yield f`, `@deco`, defaults, …).
    ///
    /// Their only consumers are `--reference-edges` and `--dead-code`
    /// (which turns reference edges on): an ordinary run drops every
    /// reference edge, so recording these there would only add audit
    /// noise and extraction time.
    pub reference_sites: bool,

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

    /// The file being extracted is test code
    /// (`cgg_core::classify_test_file`), so a test harness's verbs count
    /// as registrar verbs in it.
    test_file: bool,
}

/// The verb a registrar gates on, lowercased. A rule may name the
/// receiver too (`ipcMain.handle`) so that the resolver can tell Electron
/// IPC from every other `.handle(…)`; the extraction gate sees only the
/// callee's last segment, so that is what it must be keyed on.
fn registrar_verb(v: &str) -> String {
    v.rsplit(['.', ':'])
        .next()
        .unwrap_or(v)
        .to_ascii_lowercase()
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
            .map(|v| registrar_verb(v))
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
                // A test harness's verbs gate only test files
                // ([`builtin_harness_verbs_for`]).
                if spec.registrars.is_empty() || is_harness(spec) {
                    continue;
                }
                m.entry(spec.language)
                    .or_default()
                    .extend(spec.registrars.iter().map(|v| registrar_verb(v)));
            }
            m
        })
        .get(language)
}

/// Whether a rule's registrars are a test harness's block verbs.
fn is_harness(spec: &cgg_core::frameworks::rules::RuleSpec) -> bool {
    spec.kind == cgg_core::frameworks::TrustKind::Test
}

/// The test-harness verbs of one language (`describe`, `it`, `test`, …),
/// lowercased once per process.
///
/// Kept apart from [`builtin_verbs_for`] because they are the commonest
/// call names in a test suite and mean nothing outside one: a gate that
/// admitted them everywhere would capture `re.test(s)` in production code
/// and mint a handler per `describe` block of a tree no harness rule
/// covers. A file classified as test code admits them
/// ([`ExtractCtx::for_test_file`]).
fn builtin_harness_verbs_for(
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
                if spec.registrars.is_empty() || !is_harness(spec) {
                    continue;
                }
                m.entry(spec.language)
                    .or_default()
                    .extend(spec.registrars.iter().map(|v| registrar_verb(v)));
            }
            m
        })
        .get(language)
}

/// The receiver-qualified registrars of one language (`lambda.run`),
/// lowercased, with every unqualified verb of the language beside them.
fn builtin_registrar_paths_for(
    language: &str,
) -> Option<&'static (Vec<String>, std::collections::HashSet<String>)> {
    #[allow(clippy::type_complexity)]
    static BY_LANG: std::sync::OnceLock<
        std::collections::HashMap<
            &'static str,
            (Vec<String>, std::collections::HashSet<String>),
        >,
    > = std::sync::OnceLock::new();
    BY_LANG
        .get_or_init(|| {
            let mut m: std::collections::HashMap<
                &'static str,
                (Vec<String>, std::collections::HashSet<String>),
            > = std::collections::HashMap::new();
            for spec in cgg_core::frameworks::rules::SPECS {
                if spec.registrars.is_empty() || is_harness(spec) {
                    continue;
                }
                let e = m.entry(spec.language).or_default();
                for v in spec.registrars {
                    if v.contains(['.', ':']) {
                        e.0.push(v.to_ascii_lowercase());
                    } else {
                        e.1.insert(v.to_ascii_lowercase());
                    }
                }
            }
            for e in m.values_mut() {
                e.0.sort();
                e.0.dedup();
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
            reference_sites: false,
            extra_verbs,
            language: "",
            test_file: false,
        }
    }

    /// The same context, recording receiver-carrying value references
    /// (see [`ExtractCtx::reference_sites`]).
    #[must_use]
    pub fn with_reference_sites(self, on: bool) -> Self {
        Self {
            reference_sites: on,
            ..self
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

    /// The same context for a file that is (or is not) test code: in one,
    /// the language's test-harness verbs gate like registrar verbs, so
    /// `it('adds', () => …)` hands its callback to the harness rule.
    #[must_use]
    pub fn for_test_file(self, test_file: bool) -> Self {
        Self { test_file, ..self }
    }

    /// Whether a call written as `context` (`Lambda.run`, `app.get`) is a
    /// registration some rule of this language names *with its receiver*
    /// when it names one.
    ///
    /// [`Self::is_registrar_verb`] gates on the callee's last segment only,
    /// which is right for argument capture: the rule engine still checks
    /// the receiver. A plugin that would *mint a callable* for a closure
    /// handed to the call needs the stricter answer, or every
    /// `MainActor.run { … }` becomes a handler because some rule
    /// registers `Lambda.run`.
    pub fn is_registrar_context(&self, context: &str) -> bool {
        let verb = context.rsplit(['.', ':']).next().unwrap_or(context);
        if !self.is_registrar_verb(verb) {
            return false;
        }
        let lower = context.to_ascii_lowercase();
        let verb = verb.to_ascii_lowercase();
        if self.extra_verbs.contains(&verb)
            || (self.test_file
                && builtin_harness_verbs_for(self.language)
                    .is_some_and(|s| s.contains(&verb)))
        {
            return true;
        }
        let Some((paths, bare)) = builtin_registrar_paths_for(self.language) else {
            // Unscoped context: the per-language tables do not apply.
            return self.language.is_empty();
        };
        bare.contains(&verb)
            || paths.iter().any(|p| {
                lower == *p
                    || lower
                        .strip_suffix(p.as_str())
                        .is_some_and(|head| head.ends_with(['.', ':']))
            })
    }

    /// An ordinary run: no dead-code signals, no user rules.
    ///
    /// What the plugin tests use, and the default for anyone calling
    /// `extract` directly.
    pub fn plain() -> ExtractCtx<'static> {
        ExtractCtx {
            deadcode_signals: false,
            reference_sites: false,
            extra_verbs: no_extra_verbs(),
            language: "",
            test_file: false,
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
                    return (self.test_file && self.is_harness_verb(verb))
                        || (!self.extra_verbs.is_empty()
                            && (self.extra_verbs.contains(verb)
                                || self
                                    .extra_verbs
                                    .contains(&verb.to_ascii_lowercase())));
                }
            }
        };
        if builtin.contains(verb) {
            return true;
        }
        if self.test_file && self.is_harness_verb(verb) {
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

    /// Whether `verb` gates only as a test-harness verb here: this is a
    /// test file and no route rule or user rule of the language names it.
    pub fn is_harness_only_verb(&self, verb: &str) -> bool {
        if !self.test_file || !self.is_harness_verb(verb) {
            return false;
        }
        let lower = verb.to_ascii_lowercase();
        let routed = builtin_verbs_for(self.language).is_some_and(|s| s.contains(&lower));
        !routed && !self.extra_verbs.contains(&lower) && !self.extra_verbs.contains(verb)
    }

    /// Whether `verb` is one of this language's test-harness verbs.
    pub fn is_harness_verb(&self, verb: &str) -> bool {
        let Some(set) = builtin_harness_verbs_for(self.language) else {
            return false;
        };
        set.contains(verb)
            || (verb.bytes().any(|b| b.is_ascii_uppercase())
                && set.contains(&verb.to_ascii_lowercase()))
    }
}

pub use cgg_core as core;
pub use detect::{DetectResult, DetectVerdict, LanguageDetector};
pub use parser::{ParseOutcome, ParserPool, exceeds_depth};

/// Byte length of the longest line in `source`.
///
/// A "line" is the span between consecutive `\n` bytes (or start/end of
/// the slice).  The `\n` delimiter itself is not counted.  Used by the
/// pipeline to skip files whose longest line exceeds a grammar's
/// [`LanguagePlugin::max_line_bytes`] threshold.
pub fn longest_line_bytes(source: &[u8]) -> usize {
    source
        .split(|&b| b == b'\n')
        .map(|l| l.len())
        .max()
        .unwrap_or(0)
}

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

    /// Per-language cap on the longest source line (in bytes) that the
    /// tree-sitter grammar can parse in reasonable time.  `None` means
    /// no limit (the default for all grammars whose parse time is
    /// linear in line length).  A file whose longest line exceeds this
    /// is skipped with [`cgg_core::audit::SkipReason::LongLine`].
    fn max_line_bytes(&self) -> Option<usize> {
        None
    }

    fn ts_language(&self) -> tree_sitter::Language;

    /// The grammar to parse one file with. Defaults to [`Self::ts_language`];
    /// a plugin whose language has dialects told apart by file extension
    /// (TypeScript's `.ts` and `.tsx`) chooses per file.
    fn ts_language_for(&self, _path: &Path) -> tree_sitter::Language {
        self.ts_language()
    }

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
        assert_eq!(reg.all().len(), 46);
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

    /// WS24: a test harness's block verbs gate only a test file, and only
    /// in a language with a harness rule that registers them.
    #[test]
    fn harness_verbs_gate_only_test_files() {
        let js = ExtractCtx::plain().for_language("javascript");
        assert!(!js.is_registrar_verb("describe"));
        assert!(js.for_test_file(true).is_registrar_verb("describe"));
        assert!(js.for_test_file(true).is_registrar_verb("beforeEach"));
        // Route verbs are unaffected by the test switch.
        assert!(js.is_registrar_verb("get"));
        assert!(js.for_test_file(true).is_registrar_verb("get"));
        let py = ExtractCtx::plain()
            .for_language("python")
            .for_test_file(true);
        assert!(!py.is_registrar_verb("describe"));
    }

    /// WS24: a receiver-qualified registrar mints a handler only through
    /// its receiver, an unqualified one through any.
    #[test]
    fn a_receiver_qualified_registrar_needs_its_receiver() {
        let swift = ExtractCtx::plain().for_language("swift");
        assert!(swift.is_registrar_context("Lambda.run"));
        assert!(swift.is_registrar_context("AWSLambdaRuntime.Lambda.run"));
        assert!(!swift.is_registrar_context("MainActor.run"));
        assert!(!swift.is_registrar_context("run"));
        let js = ExtractCtx::plain().for_language("javascript");
        assert!(js.is_registrar_context("app.get"));
        assert!(!js.is_registrar_context("it"));
        assert!(js.for_test_file(true).is_registrar_context("it"));
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
