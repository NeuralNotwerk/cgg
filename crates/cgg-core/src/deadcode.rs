//! Dead-code report schema.
//!
//! This module holds only the serializable shape of a dead-code
//! analysis. The analysis itself lives in `cgg-resolve::deadcode`; it is
//! split out here because `cgg-format` depends on `cgg-core` alone, and
//! must not be dragged through `cgg-resolve` (and therefore `cgg-lang`,
//! and therefore 44 linked tree-sitter grammars) just to render a
//! report.
//!
//! Two design commitments run through every type below.
//!
//! **The report is evidence, not a verdict.** A finding carries the
//! enumerated reasons cgg believes it is dead *and* the enumerated
//! reasons cgg might be wrong ([`Evidence`]). The confidence level is a
//! summary of that list, never a substitute for it. This is what makes
//! [`DEAD_CODE_DISCLAIMER`] an honest statement rather than boilerplate.
//!
//! **Absence of signal is reported, not hidden.** Extraction coverage is
//! very uneven across the 44 language plugins, so every report states
//! per language what cgg could and could not see
//! ([`LanguageCapabilityReport`]) and how many findings it withheld and
//! why ([`SuppressedCategory`]). A report that ranks a Rust finding and
//! a Fortran finding identically would be lying.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::graph::{CallableKind, Confidence};
use crate::ids::{CallableId, FileId};

/// The mandatory preamble on every dead-code report, on every surface.
///
/// Held as a constant in `cgg-core` — rather than composed by whichever
/// formatter happens to run — so that no output path can omit it. It is
/// serialized without `skip_serializing_if`, so every JSON consumer
/// receives it too.
pub const DEAD_CODE_DISCLAIMER: &str = "\
BEST EFFORT — EVERY FINDING IS A HYPOTHESIS, NOT A FACT. This is a \
static over-approximation computed from an incomplete call graph. \
Reflection, string-keyed dispatch, dynamic imports, calls inside macro \
arguments, build-time codegen, conditional compilation, framework entry \
points, FFI consumers outside this tree, and any language whose signals \
cgg cannot extract are all invisible to it. cgg reports what it could \
not find a caller for, which is not the same as proving no caller \
exists. Every finding MUST be manually reviewed against the source \
before it is acted on. See the per-language capability table for what \
cgg could and could not see, each finding's evidence for why its \
confidence is what it is, and --why-live to check the reasoning in the \
opposite direction.";

/// Below this share of a language's callables being reachable from a
/// discovered root, whole-program categories are withheld for that
/// language and [`FindingCategory::NeverReferenced`] is capped.
///
/// Root discovery is the weakest link in the whole analysis: on a Java
/// repo whose only root is `public static void main`, "unreachable from
/// roots" describes the entire codebase. Measured on cgg's own source,
/// only 132 of 1178 callables (11%) are reachable from `main`, which is
/// exactly the situation this gate exists to catch.
///
/// **This number is a starting value, not a derived one.** It must be
/// re-measured against a spread of real repositories before 1.0.
pub const DEFAULT_MIN_ROOT_COVERAGE_PCT: u8 = 25;

/// Why a callable is believed to be dead.
///
/// These are not severity levels — they are different *shapes* of
/// unreferenced code, and each warrants a different reading.
/// `NeverReferenced` stands alone. `ReachableOnlyFromDeadCode` is
/// contingent: it is only unreferenced because its callers are.
/// `DeadCycle` has no entry point at all — every member is referenced,
/// but only from inside the ring.
#[derive(
    Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum FindingCategory {
    /// Zero inbound edges of any kind, and not a root. A pure graph
    /// fact, independent of how good root discovery was.
    NeverReferenced,
    /// Has callers, but every caller is itself unreferenced. A
    /// name-based tool cannot produce this in a single pass.
    ReachableOnlyFromDeadCode,
    /// A member of a mutually-recursive group with no reachable entry
    /// point. A name-based tool cannot report this at all: each member
    /// counts as a use of the others.
    DeadCycle,
    /// Live, but every path proving it originates in test scope.
    OnlyUsedByTests,
    /// No path from any declared or discovered root. Only computed when
    /// root coverage clears [`DEFAULT_MIN_ROOT_COVERAGE_PCT`].
    UnreachableFromRoots,
}

impl FindingCategory {
    /// Stable, greppable report code.
    pub fn code(self) -> &'static str {
        match self {
            FindingCategory::NeverReferenced => "D001",
            FindingCategory::ReachableOnlyFromDeadCode => "D002",
            FindingCategory::DeadCycle => "D003",
            FindingCategory::OnlyUsedByTests => "D004",
            FindingCategory::UnreachableFromRoots => "D005",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            FindingCategory::NeverReferenced => "never-referenced",
            FindingCategory::ReachableOnlyFromDeadCode => "reachable-only-from-dead-code",
            FindingCategory::DeadCycle => "dead-cycle",
            FindingCategory::OnlyUsedByTests => "only-used-by-tests",
            FindingCategory::UnreachableFromRoots => "unreachable-from-roots",
        }
    }

    /// Confidence before any [`Evidence`] cap is applied.
    ///
    /// Only `NeverReferenced` starts high: it is a statement about the
    /// graph rather than about root discovery, so it cannot be wrong in
    /// the way the others can.
    pub fn base_confidence(self) -> Confidence {
        match self {
            FindingCategory::NeverReferenced => Confidence::High,
            _ => Confidence::Medium,
        }
    }

    /// Whether this category depends on the quality of root discovery,
    /// and is therefore subject to the root-coverage gate.
    pub fn depends_on_roots(self) -> bool {
        !matches!(self, FindingCategory::NeverReferenced)
    }
}

/// Which direction a piece of evidence pushes.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Polarity {
    /// Corroborates the finding.
    Raises,
    /// Explains the category without arguing either way.
    Neutral,
    /// A reason cgg may be wrong.
    Lowers,
}

/// A reference to a specific call site, denormalized so a finding is
/// self-describing without needing the graph alongside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SiteRef {
    pub file: FileId,
    pub path: PathBuf,
    pub line: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub receiver_hint: String,
}

/// A single reason a finding is believed, or doubted.
///
/// The design commitment here is **caps rather than additive weights**.
/// Most of cgg's negative evidence is not "somewhat less likely" — it is
/// "cgg structurally cannot know". A language with no visibility
/// extraction does not make a finding 20% weaker; it makes `High`
/// unreachable. Addition models the former; capping models the latter,
/// and the latter is the truth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Evidence {
    // ---- structural facts (neutral; they explain the category) ----
    /// Nothing in the analyzed source references this callable.
    NoIncomingEdges,
    /// Every caller is itself dead.
    IncomingOnlyFromDeadCode { callers: u32, example: CallableId },
    /// Member of a strongly-connected component.
    InCycle { scc_size: u32 },
    /// Member of a connected group of unreferenced callables.
    InRegion {
        region: u32,
        members: u32,
        files: u32,
    },

    // ---- lowering: reasons cgg may be wrong ----
    /// An unresolved call site somewhere uses this simple name. The
    /// resolver saw a call it could not place; this callable may be its
    /// target.
    NameMatchesUnresolvedSite {
        name: String,
        sites: u32,
        reason: String,
        same_file_sites: u32,
        owner_match_sites: u32,
        example: SiteRef,
    },
    /// The resolver found several same-name candidates *in this very
    /// file* and declined to choose. The sharpest form of the above:
    /// the ambiguity is local, so this callable is very plausibly one of
    /// the candidates.
    AmbiguousSiteInSameFile {
        sites: u32,
        file_local_candidates: u32,
        example: SiteRef,
    },
    /// This language's plugin does not extract visibility, so cgg cannot
    /// tell an internal helper from exported API surface.
    LanguageLacksVisibility,
    /// This language's plugin does not extract attributes/decorators, so
    /// framework entry points are invisible.
    LanguageLacksAttributes,
    /// This language's plugin does not record functions passed as
    /// values, so `register(handler)` is invisible.
    LanguageLacksValueReferences,
    /// This language has no interface/trait implementation model, so
    /// dynamic dispatch is invisible.
    LanguageLacksDispatchModel,
    /// An interface/descriptor language. An unreferenced schema is a
    /// wire contract whose consumers are, by definition, in another
    /// repository.
    LanguageIsDescriptor,
    /// Invocation is normally from top-level script code that cgg does
    /// not model as a callable.
    LanguageIsScriptDriven,
    /// Too little of this language was reachable from a root for
    /// whole-program reasoning to mean anything.
    LowRootCoverage {
        roots: u32,
        callables: u32,
        reachable_pct: u8,
    },
    /// Constructors, destructors and properties are routinely invoked by
    /// syntax rather than by a visible call.
    ImplicitlyInvokableKind { callable_kind: CallableKind },
    /// A name-based screen decided some same-named call targets
    /// third-party code. Weak — it means the resolver asserted the call
    /// was *not* ours — but worth showing a reviewer.
    NameCollidesWithScreenedSite { screen: String, sites: u32 },
    /// Visible outside its compilation unit, so a caller may exist in
    /// code cgg never analyzed. For a library crate this is the normal
    /// case, and it is exactly where "nothing references it" is weakest:
    /// the analyzed tree is not the whole world, and the consumers that
    /// make an exported symbol worth keeping are the ones cgg cannot
    /// see. The mirror image of [`Evidence::PrivateVisibility`].
    PublicVisibility { token: String },

    // ---- raising: corroboration ----
    /// Not visible outside its compilation unit, so no out-of-tree
    /// caller can exist.
    PrivateVisibility { token: String },
    /// No unresolved call site anywhere uses this name.
    NoUnresolvedSiteWithName,
    /// Nothing in this file is exported over FFI.
    NoFfiExportInFile,
    /// cgg extracts every signal it has for this language.
    LanguageSignalsComplete,
    /// Not re-exported from its module/crate.
    NotReexported,
    /// Confined to a single file, so the blast radius of removing it is
    /// small.
    SingleFileBlastRadius,
}

impl Evidence {
    /// The highest confidence a finding carrying this evidence may
    /// reach. `None` means this evidence does not limit confidence.
    pub fn cap(&self) -> Option<Confidence> {
        match self {
            // A file-local ambiguity is the strongest reason to doubt:
            // the resolver had candidates in hand and refused to pick.
            Evidence::AmbiguousSiteInSameFile { .. } => Some(Confidence::Low),
            Evidence::NameMatchesUnresolvedSite {
                same_file_sites,
                owner_match_sites,
                ..
            } => {
                if *same_file_sites > 0 || *owner_match_sites > 0 {
                    Some(Confidence::Low)
                } else {
                    Some(Confidence::Medium)
                }
            }
            Evidence::LanguageIsDescriptor | Evidence::LanguageIsScriptDriven => {
                Some(Confidence::Low)
            }
            Evidence::LowRootCoverage { roots, .. } => {
                if *roots == 0 {
                    Some(Confidence::Low)
                } else {
                    Some(Confidence::Medium)
                }
            }
            Evidence::LanguageLacksVisibility
            | Evidence::LanguageLacksAttributes
            | Evidence::LanguageLacksValueReferences
            | Evidence::LanguageLacksDispatchModel
            | Evidence::ImplicitlyInvokableKind { .. } => Some(Confidence::Medium),
            // Not "somewhat less likely" — unknowable. cgg cannot see
            // past the edge of the analyzed tree at all, so an exported
            // symbol's callers are structurally invisible to it and such
            // a finding must not sit in the top band.
            Evidence::PublicVisibility { .. } => Some(Confidence::Medium),
            _ => None,
        }
    }

    pub fn polarity(&self) -> Polarity {
        match self {
            Evidence::NoIncomingEdges
            | Evidence::IncomingOnlyFromDeadCode { .. }
            | Evidence::InCycle { .. }
            | Evidence::InRegion { .. } => Polarity::Neutral,

            Evidence::PrivateVisibility { .. }
            | Evidence::NoUnresolvedSiteWithName
            | Evidence::NoFfiExportInFile
            | Evidence::LanguageSignalsComplete
            | Evidence::NotReexported
            | Evidence::SingleFileBlastRadius => Polarity::Raises,

            _ => Polarity::Lowers,
        }
    }

    /// Stable machine-readable tag, mirroring `SkipReason::slug` and
    /// `UnresolvedReason::slug`.
    pub fn slug(&self) -> &'static str {
        match self {
            Evidence::NoIncomingEdges => "no-incoming-edges",
            Evidence::IncomingOnlyFromDeadCode { .. } => "incoming-only-from-dead-code",
            Evidence::InCycle { .. } => "in-cycle",
            Evidence::InRegion { .. } => "in-region",
            Evidence::NameMatchesUnresolvedSite { .. } => "name-matches-unresolved-site",
            Evidence::AmbiguousSiteInSameFile { .. } => "ambiguous-site-in-same-file",
            Evidence::LanguageLacksVisibility => "language-lacks-visibility",
            Evidence::LanguageLacksAttributes => "language-lacks-attributes",
            Evidence::LanguageLacksValueReferences => "language-lacks-value-references",
            Evidence::LanguageLacksDispatchModel => "language-lacks-dispatch-model",
            Evidence::LanguageIsDescriptor => "language-is-descriptor",
            Evidence::LanguageIsScriptDriven => "language-is-script-driven",
            Evidence::LowRootCoverage { .. } => "low-root-coverage",
            Evidence::ImplicitlyInvokableKind { .. } => "implicitly-invokable-kind",
            Evidence::NameCollidesWithScreenedSite { .. } => {
                "name-collides-with-screened-site"
            }
            Evidence::PublicVisibility { .. } => "public-visibility",
            Evidence::PrivateVisibility { .. } => "private-visibility",
            Evidence::NoUnresolvedSiteWithName => "no-unresolved-site-with-name",
            Evidence::NoFfiExportInFile => "no-ffi-export-in-file",
            Evidence::LanguageSignalsComplete => "language-signals-complete",
            Evidence::NotReexported => "not-reexported",
            Evidence::SingleFileBlastRadius => "single-file-blast-radius",
        }
    }

    /// Contribution to a finding's `rank`. Ordering only — see
    /// [`DeadCodeFinding::rank`].
    pub fn weight(&self) -> i32 {
        match self.polarity() {
            Polarity::Raises => 2,
            Polarity::Neutral => 0,
            Polarity::Lowers => -3,
        }
    }

    /// Deterministic sort key: lowering evidence first (a reader needs
    /// the caveats before the corroboration), then alphabetical by slug.
    pub fn sort_key(&self) -> (u8, &'static str) {
        let rank = match self.polarity() {
            Polarity::Lowers => 0,
            Polarity::Neutral => 1,
            Polarity::Raises => 2,
        };
        (rank, self.slug())
    }
}

/// Why a callable is treated as live regardless of its in-degree.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RootKind {
    /// A program entry point (`main`, `_start`, `init`).
    ProgramEntry,
    /// Called from module top level, where cgg has no enclosing callable
    /// to attribute the edge to.
    TopLevelInvocation,
    /// Exported over an FFI boundary, so callers are outside this tree.
    FfiExport,
    /// A test case or harness hook.
    TestEntry,
    /// Invoked by a framework rather than by project code.
    FrameworkCallback,
    /// Part of the public API surface.
    ExportedApi,
    /// A lifecycle method invoked by a runtime or language construct.
    LifecycleCallback,
    /// Declared in a build manifest.
    ManifestEntry,
    /// Declared by the user.
    UserDeclared,
}

impl RootKind {
    /// Whether liveness proved only through this root kind should be
    /// reported as [`FindingCategory::OnlyUsedByTests`] rather than as
    /// plain liveness.
    pub fn is_test(self) -> bool {
        matches!(self, RootKind::TestEntry)
    }
}

/// A discovered or declared root, with the provenance that justifies it.
///
/// Recorded for every root and listed in the report: whole-program
/// categories are unfalsifiable unless a reader can see, and disagree
/// with, the root set they were computed from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RootRecord {
    pub id: CallableId,
    pub qualified_name: String,
    pub language: String,
    pub kind: RootKind,
    /// Which rule fired: `"builtin:main"`, `"ffi:export"`,
    /// `"user:--entry-point[2]"`.
    pub rule: String,
    /// Human-readable specifics: `"#[pyfunction]"`, `"called from module
    /// top level at src/app.py:14"`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// A member's role within its [`DeadRegion`].
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegionRole {
    /// Nothing in the group references it — the entry point of the
    /// group, and the place a review of it naturally starts.
    Anchor,
    /// Referenced only from elsewhere in the same group, so this
    /// finding is contingent on the group's other members.
    Downstream,
    /// Part of a mutually-recursive cycle, so no member is the entry
    /// point.
    CycleMember,
}

/// A connected group of unreferenced callables.
///
/// Regions exist so that a chain `a -> b -> c`, where only `a` is truly
/// unreferenced, reads as one finding about one cluster rather than
/// three independent accusations — and so that a pure cycle, which has
/// no entry point at all, is visibly different from a lone unused
/// function.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeadRegion {
    /// Minimum member node index, so the id is stable across runs.
    pub id: u32,
    pub members: Vec<CallableId>,
    /// Members with no in-edges from inside the group: the entry
    /// points, and the members whose finding does not depend on the
    /// rest of the group being correct.
    pub anchors: Vec<CallableId>,
    /// Non-trivial strongly-connected components inside the region.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cycles: Vec<Vec<CallableId>>,
    pub files: Vec<FileId>,
    pub total_lines: u32,
    pub languages: Vec<String>,
    /// A group is only as trustworthy as its weakest member.
    pub confidence: Confidence,
}

/// One reported callable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeadCodeFinding {
    pub id: CallableId,
    pub qualified_name: String,
    pub simple_name: String,
    pub language: String,
    pub kind: CallableKind,
    /// The finer `DefVariant` string, denormalized the way
    /// `AuditCallableRef::kind` is: "trait_default_method" reads very
    /// differently from "method" in a dead-code report.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub def_variant: String,
    pub file: FileId,
    pub path: PathBuf,
    pub start_line: u32,
    pub end_line: u32,
    pub size_lines: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_hint: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub visibility: String,

    pub category: FindingCategory,
    pub confidence: Confidence,
    /// Deterministic sort key within a confidence band.
    ///
    /// **Not a probability, not a percentage, and never a threshold.**
    /// It exists only to give findings a stable total order.
    pub rank: i32,
    pub region: u32,
    pub role: RegionRole,
    /// Sorted by [`Evidence::sort_key`]: caveats first.
    pub evidence: Vec<Evidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dead_callers: Vec<CallableId>,
    pub out_degree: u32,
}

impl DeadCodeFinding {
    /// `D001:some::qualified::name` — stable across runs *and* across
    /// line churn, so a baseline file can key on it. Deliberately not
    /// line-based.
    pub fn stable_id(&self) -> String {
        format!("{}:{}", self.category.code(), self.qualified_name)
    }
}

/// What a language's plugin is *capable* of extracting.
///
/// Distinct from what a given run happened to observe. A codebase with
/// no callbacks yields no value-reference edges, but that says nothing
/// about whether the plugin can find them — and treating the two as the
/// same caps every finding in small or simple codebases, which silently
/// empties the default report.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LanguageSignals {
    pub visibility: bool,
    pub attributes: bool,
    pub exports: bool,
    pub test_defs: bool,
    /// Records functions passed by name as values (`register(handler)`).
    pub value_refs: bool,
    pub dyn_uses: bool,
    pub unreachable: bool,
    pub impls: bool,
}

/// Whether a signal is genuinely extracted, approximated by convention,
/// or absent.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignalSupport {
    /// Read from the syntax tree.
    Full,
    /// Inferred from a naming convention (Python's `_` prefix, Go's
    /// capitalization). Exact for some languages, a heuristic in others.
    Convention,
    /// Not extracted.
    None,
}

impl SignalSupport {
    pub fn is_present(self) -> bool {
        !matches!(self, SignalSupport::None)
    }
}

/// How far a language's extraction supports the analysis.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LanguageClass {
    /// Enough signal for the full model.
    Analyzable,
    /// Missing visibility and/or attributes; confidence is capped.
    Degraded,
    /// Invocation is primarily from top-level script code that cgg does
    /// not model as a callable.
    ScriptDriven,
    /// An interface/descriptor language, where "unreferenced" means
    /// something else entirely.
    Descriptor,
}

/// Why a category was withheld for a language.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SuppressionReason {
    DescriptorLanguage,
    ScriptDriven,
    NoRootsFound,
    LowRootCoverage,
    MissingSignal,
}

/// A category cgg declined to report, and how much it withheld.
///
/// `would_have_reported` is the point of this type: "cgg withheld 312
/// Java findings because it found 0 roots" is a true and useful
/// statement, whereas silently emitting nothing is a lie of omission.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SuppressedCategory {
    pub language: String,
    pub category: FindingCategory,
    pub reason: SuppressionReason,
    pub would_have_reported: u32,
}

/// What cgg could and could not see for one language, in this run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LanguageCapabilityReport {
    pub language: String,
    pub class: LanguageClass,
    pub visibility: SignalSupport,
    pub attributes: SignalSupport,
    pub value_references: SignalSupport,
    pub dispatch: SignalSupport,
    pub exports: SignalSupport,
    pub test_tagging: SignalSupport,
    /// Ceiling this language's findings may reach.
    pub max_confidence: Confidence,
    pub root_rules_active: Vec<String>,
    // Observed in this run:
    pub files: u32,
    pub callables: u32,
    pub roots: u32,
    pub reachable: u32,
    pub reachable_pct: u8,
    pub findings: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blind_spots: Vec<String>,
}

/// Roll-up counts for the run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeadCodeSummary {
    /// Always true. Present so that even a truncated summary carries the
    /// caveat.
    pub review_required: bool,
    pub callables: u32,
    pub edges: u32,
    pub unresolved_call_sites: u32,
    pub roots: u32,
    pub candidates: u32,
    pub reported: u32,
    pub regions: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub withheld: Vec<SuppressedCategory>,
    /// Patterns from the roots file that matched nothing. Suppression
    /// files rot silently; this is how a reader finds out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stale_suppressions: Vec<String>,
}

/// Effective configuration, echoed so a report is reproducible.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeadCodeConfig {
    pub confidence_threshold: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roots_file: Option<PathBuf>,
    pub include_tests: bool,
    pub reference_edges: bool,
    pub dynamic_dispatch: bool,
    /// Why whole-program reachability did or did not run.
    pub root_reachability: String,
}

/// The complete analysis result.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeadCodeReport {
    /// Schema tag for downstream tooling.
    pub schema: String,
    /// [`DEAD_CODE_DISCLAIMER`]. Declared first so serde emits it as the
    /// first key, and deliberately not skippable.
    pub disclaimer: String,
    pub best_effort: bool,
    pub cgg_version: String,
    pub config: DeadCodeConfig,
    pub capabilities: Vec<LanguageCapabilityReport>,
    pub summary: DeadCodeSummary,
    pub roots: Vec<RootRecord>,
    pub regions: Vec<DeadRegion>,
    pub findings: Vec<DeadCodeFinding>,
}

impl Default for DeadCodeReport {
    fn default() -> Self {
        Self {
            schema: "cgg.deadcode.v1".to_string(),
            disclaimer: DEAD_CODE_DISCLAIMER.to_string(),
            best_effort: true,
            cgg_version: crate::version::CGG_VERSION.to_string(),
            config: DeadCodeConfig::default(),
            capabilities: Vec::new(),
            summary: DeadCodeSummary {
                review_required: true,
                ..Default::default()
            },
            roots: Vec::new(),
            regions: Vec::new(),
            findings: Vec::new(),
        }
    }
}

/// One hop of a liveness proof.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProofHop {
    pub from: CallableId,
    pub to: CallableId,
    pub to_qualified_name: String,
    pub path: PathBuf,
    pub line: u32,
    pub site_line: u32,
    pub via: String,
    pub confidence: Confidence,
    pub resolver: String,
}

/// The answer to "why do you think this is live?" — the dual of a
/// finding, and the thing that makes the analysis arguable in both
/// directions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LivenessProof {
    pub target: CallableId,
    pub target_qualified_name: String,
    /// `live`, `test-live`, or `dead`.
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<RootRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hops: Vec<ProofHop>,
    /// The weakest edge on the proving path — a chain of `Low`-
    /// confidence hops is a weak proof.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weakest_link: Option<Confidence>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disclaimer_is_non_empty_and_says_the_key_thing() {
        assert!(DEAD_CODE_DISCLAIMER.contains("BEST EFFORT"));
        assert!(DEAD_CODE_DISCLAIMER.contains("MUST be manually reviewed"));
        assert!(DEAD_CODE_DISCLAIMER.contains("HYPOTHESIS"));
    }

    #[test]
    fn default_report_carries_the_disclaimer() {
        let r = DeadCodeReport::default();
        assert_eq!(r.disclaimer, DEAD_CODE_DISCLAIMER);
        assert!(r.best_effort);
        assert!(r.summary.review_required);
        assert_eq!(r.schema, "cgg.deadcode.v1");
    }

    #[test]
    fn disclaimer_survives_serialization_and_is_first() {
        let json = serde_json::to_string(&DeadCodeReport::default()).unwrap();
        assert!(json.contains("BEST EFFORT"));
        // Declared first in the struct, so serde emits it first after
        // the schema tag — consumers that truncate still see it.
        let schema_at = json.find("\"schema\"").unwrap();
        let disc_at = json.find("\"disclaimer\"").unwrap();
        let findings_at = json.find("\"findings\"").unwrap();
        assert!(schema_at < disc_at && disc_at < findings_at);
    }

    #[test]
    fn caps_only_ever_lower_confidence() {
        // A cap must never promote: every declared cap is Medium or Low.
        let samples = [
            Evidence::LanguageLacksVisibility,
            Evidence::LanguageIsDescriptor,
            Evidence::ImplicitlyInvokableKind {
                callable_kind: CallableKind::Constructor,
            },
            Evidence::LowRootCoverage {
                roots: 0,
                callables: 10,
                reachable_pct: 0,
            },
            Evidence::PublicVisibility {
                token: "pub".into(),
            },
        ];
        for e in samples {
            let cap = e.cap().expect("sample should cap");
            assert!(
                matches!(cap, Confidence::Medium | Confidence::Low),
                "{:?} capped to {:?}",
                e.slug(),
                cap
            );
        }
    }

    #[test]
    fn same_file_ambiguity_caps_lower_than_a_bare_name_match() {
        let bare = Evidence::NameMatchesUnresolvedSite {
            name: "run".into(),
            sites: 1,
            reason: "no-candidate-in-file".into(),
            same_file_sites: 0,
            owner_match_sites: 0,
            example: SiteRef {
                file: FileId::new(0),
                path: PathBuf::from("a.rs"),
                line: 1,
                name: "run".into(),
                receiver_hint: String::new(),
            },
        };
        assert_eq!(bare.cap(), Some(Confidence::Medium));

        let local = Evidence::AmbiguousSiteInSameFile {
            sites: 1,
            file_local_candidates: 2,
            example: SiteRef {
                file: FileId::new(0),
                path: PathBuf::from("a.rs"),
                line: 1,
                name: "run".into(),
                receiver_hint: String::new(),
            },
        };
        assert_eq!(local.cap(), Some(Confidence::Low));
    }

    #[test]
    fn public_visibility_lowers_and_caps_below_high() {
        let public = Evidence::PublicVisibility {
            token: "pub".into(),
        };
        // cgg cannot see outside the analyzed tree, so an exported
        // symbol can never be a top-band finding.
        assert_eq!(public.cap(), Some(Confidence::Medium));
        assert_eq!(public.polarity(), Polarity::Lowers);
        assert_eq!(public.slug(), "public-visibility");

        // Its mirror image corroborates and imposes no ceiling.
        let private = Evidence::PrivateVisibility {
            token: "private".into(),
        };
        assert_eq!(private.cap(), None);
        assert_eq!(private.polarity(), Polarity::Raises);
        assert_ne!(public.slug(), private.slug());
    }

    #[test]
    fn evidence_sorts_caveats_before_corroboration() {
        let mut ev = [
            Evidence::PrivateVisibility {
                token: "pub".into(),
            },
            Evidence::NoIncomingEdges,
            Evidence::LanguageLacksVisibility,
        ];
        ev.sort_by(|a, b| a.sort_key().cmp(&b.sort_key()));
        assert_eq!(ev[0].slug(), "language-lacks-visibility");
        assert_eq!(ev[1].slug(), "no-incoming-edges");
        assert_eq!(ev[2].slug(), "private-visibility");
    }

    #[test]
    fn category_codes_and_slugs_are_unique() {
        let all = [
            FindingCategory::NeverReferenced,
            FindingCategory::ReachableOnlyFromDeadCode,
            FindingCategory::DeadCycle,
            FindingCategory::OnlyUsedByTests,
            FindingCategory::UnreachableFromRoots,
        ];
        let codes: std::collections::HashSet<_> = all.iter().map(|c| c.code()).collect();
        let slugs: std::collections::HashSet<_> = all.iter().map(|c| c.slug()).collect();
        assert_eq!(codes.len(), all.len());
        assert_eq!(slugs.len(), all.len());
    }

    #[test]
    fn only_never_referenced_is_root_independent() {
        assert!(!FindingCategory::NeverReferenced.depends_on_roots());
        assert_eq!(
            FindingCategory::NeverReferenced.base_confidence(),
            Confidence::High
        );
        for c in [
            FindingCategory::ReachableOnlyFromDeadCode,
            FindingCategory::DeadCycle,
            FindingCategory::UnreachableFromRoots,
        ] {
            assert!(c.depends_on_roots());
            assert_eq!(c.base_confidence(), Confidence::Medium);
        }
    }

    #[test]
    fn stable_id_is_not_line_based() {
        let f = DeadCodeFinding {
            id: CallableId::new(1),
            qualified_name: "a::b::c".into(),
            simple_name: "c".into(),
            language: "rust".into(),
            kind: CallableKind::Function,
            def_variant: String::new(),
            file: FileId::new(0),
            path: PathBuf::from("a.rs"),
            start_line: 10,
            end_line: 20,
            size_lines: 11,
            signature_hint: String::new(),
            visibility: String::new(),
            category: FindingCategory::NeverReferenced,
            confidence: Confidence::High,
            rank: 0,
            region: 0,
            role: RegionRole::Anchor,
            evidence: vec![],
            dead_callers: vec![],
            out_degree: 0,
        };
        assert_eq!(f.stable_id(), "D001:a::b::c");
        let moved = DeadCodeFinding {
            start_line: 999,
            end_line: 1009,
            ..f.clone()
        };
        assert_eq!(f.stable_id(), moved.stable_id());
    }
}
