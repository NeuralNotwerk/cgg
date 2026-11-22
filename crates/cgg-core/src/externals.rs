//! External-dependency inventory (`--externals`).
//!
//! The data model for the report that lists every library a scanned tree
//! reaches outside itself — third-party packages, the standard library,
//! the operating system, natively linked libraries and spawned programs —
//! and every call site that reaches one.
//!
//! The inventory is built by `cgg_resolve::externals` after every resolver
//! has run. It only *reads* resolver state: the per-file `stdlib_calls` /
//! `external_calls` audit buckets, the import records, and the
//! [`ExternalUse`] side channel that extraction fills when the run asks
//! for it. Nothing here ever becomes a resolver input, so turning the
//! feature on cannot change an in-tree edge.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::graph::Confidence;
use crate::ids::CallableId;

/// Schema identifier of the JSON report.
pub const EXTERNALS_SCHEMA: &str = "cgg.externals.v1";

/// Printed at the top of every report, text and JSON alike.
pub const EXTERNALS_DISCLAIMER: &str = "BEST EFFORT — a call is attributed to a library only \
from evidence cgg can see in this tree: the import or include the call is written through, a \
declared receiver type, a foreign-function declaration, or a literal program or library name. \
A call through a value whose type cgg cannot see is counted as `unattributed`, never guessed. \
Calls cgg never sees at all (reflection, string-built commands, generated code, macros it does \
not expand) are absent. Dependency manifests are not read: a library appears here because the \
code calls or imports it.";

/// What kind of thing a call leaves the tree for.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalKind {
    /// A package, crate, module or namespace from outside the tree.
    ThirdParty,
    /// The language's standard library, runtime or builtins.
    Stdlib,
    /// The operating system: syscalls, libc/POSIX/Win32 APIs.
    System,
    /// A foreign library reached by FFI or dynamic loading.
    Native,
    /// An external program the code runs.
    Process,
    /// Left the tree, but no evidence names the library.
    Unattributed,
}

impl ExternalKind {
    /// Every kind, in report order.
    pub const ALL: [ExternalKind; 6] = [
        ExternalKind::ThirdParty,
        ExternalKind::System,
        ExternalKind::Native,
        ExternalKind::Process,
        ExternalKind::Stdlib,
        ExternalKind::Unattributed,
    ];

    /// What a bare `--externals` selects.
    pub const DEFAULT: [ExternalKind; 4] = [
        ExternalKind::ThirdParty,
        ExternalKind::System,
        ExternalKind::Native,
        ExternalKind::Process,
    ];

    /// The stable slug used on the command line and in JSON.
    pub fn slug(self) -> &'static str {
        match self {
            ExternalKind::ThirdParty => "third-party",
            ExternalKind::Stdlib => "stdlib",
            ExternalKind::System => "system",
            ExternalKind::Native => "native",
            ExternalKind::Process => "process",
            ExternalKind::Unattributed => "unattributed",
        }
    }

    /// Parse a slug.
    pub fn from_slug(s: &str) -> Option<Self> {
        ExternalKind::ALL.into_iter().find(|k| k.slug() == s)
    }

    /// The sentinel path leaf nodes of this kind live in, and the first
    /// segment of their qualified names.
    pub fn sentinel(self) -> &'static str {
        match self {
            ExternalKind::ThirdParty => "<external>",
            ExternalKind::Stdlib => "<stdlib>",
            ExternalKind::System => "<system>",
            ExternalKind::Native => "<native>",
            ExternalKind::Process => "<process>",
            ExternalKind::Unattributed => "<unattributed>",
        }
    }
}

impl std::fmt::Display for ExternalKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.slug())
    }
}

/// Which kinds a run asked for. `None` on `RunOptions` means the feature
/// is off; an empty list is never constructed by the parsers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SpecRepr", into = "SpecRepr")]
pub struct ExternalsSpec {
    kinds: Vec<ExternalKind>,
}

impl Default for ExternalsSpec {
    fn default() -> Self {
        Self {
            kinds: ExternalKind::DEFAULT.to_vec(),
        }
    }
}

impl ExternalsSpec {
    /// Every kind.
    pub fn all() -> Self {
        Self {
            kinds: ExternalKind::ALL.to_vec(),
        }
    }

    /// Parse the command-line value: `all`, `default`, or a comma list
    /// of kind slugs.
    pub fn parse(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() || s == "default" {
            return Ok(Self::default());
        }
        let mut kinds = Vec::new();
        for part in s.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            if part == "all" {
                return Ok(Self::all());
            }
            let k = ExternalKind::from_slug(part).ok_or_else(|| {
                format!(
                    "unknown externals kind `{part}` (expected all, or a comma list of: {})",
                    ExternalKind::ALL.map(|k| k.slug()).join(", ")
                )
            })?;
            if !kinds.contains(&k) {
                kinds.push(k);
            }
        }
        if kinds.is_empty() {
            return Ok(Self::default());
        }
        kinds.sort();
        Ok(Self { kinds })
    }

    /// Whether a kind is shown (as opposed to withheld).
    pub fn shows(&self, k: ExternalKind) -> bool {
        self.kinds.contains(&k)
    }

    /// The selected kinds, in canonical order.
    pub fn kinds(&self) -> &[ExternalKind] {
        &self.kinds
    }
}

/// The serde form of [`ExternalsSpec`]: `true`/`"all"`/`"a,b"`/`["a","b"]`.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum SpecRepr {
    Flag(bool),
    Text(String),
    List(Vec<String>),
}

impl TryFrom<SpecRepr> for ExternalsSpec {
    type Error = String;
    fn try_from(r: SpecRepr) -> Result<Self, String> {
        match r {
            SpecRepr::Flag(true) => Ok(Self::default()),
            SpecRepr::Flag(false) => Err("`externals: false` is spelled by omitting the key".into()),
            SpecRepr::Text(s) => Self::parse(&s),
            SpecRepr::List(v) => Self::parse(&v.join(",")),
        }
    }
}

impl From<ExternalsSpec> for SpecRepr {
    fn from(s: ExternalsSpec) -> Self {
        SpecRepr::List(s.kinds.iter().map(|k| k.slug().to_string()).collect())
    }
}

/// What a site's attribution rests on, strongest first.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalEvidence {
    /// The cross-file resolver refused the call as written through an
    /// out-of-tree import (`UnresolvedReason::ExternalModule`).
    ResolverRefusal,
    /// The receiver head or written qualifier is bound by an import.
    QualifiedImport,
    /// A bare name the file imports selectively.
    SelectiveImport,
    /// A foreign-function declaration that names its library.
    ForeignDeclaration,
    /// A process or library-loading API with a literal argument.
    LiteralArgument,
    /// A syscall instruction, an OS API table entry, or a system header.
    SystemInterface,
    /// The receiver's type, bound by an import.
    TypedReceiver,
    /// The first segment of a receiver expression, bound by an import.
    ChainHead,
    /// A C-family name carrying the prefix of the one included
    /// third-party header it could come from (`curl_easy_init`,
    /// `<curl/curl.h>`).
    NamePrefix,
    /// The language's stdlib vocabulary, with no import evidence.
    StdlibScreen,
    /// The only external wildcard import that could supply the name.
    SoleWildcard,
    /// Nothing: the call left the tree, the library is unknown.
    None,
}

impl ExternalEvidence {
    /// How sure the attribution is, on cgg's edge-confidence scale.
    pub fn confidence(self) -> Confidence {
        match self {
            ExternalEvidence::ResolverRefusal
            | ExternalEvidence::QualifiedImport
            | ExternalEvidence::SelectiveImport
            | ExternalEvidence::ForeignDeclaration
            | ExternalEvidence::LiteralArgument
            | ExternalEvidence::SystemInterface => Confidence::High,
            ExternalEvidence::TypedReceiver
            | ExternalEvidence::ChainHead
            | ExternalEvidence::NamePrefix
            | ExternalEvidence::StdlibScreen => Confidence::Medium,
            ExternalEvidence::SoleWildcard | ExternalEvidence::None => Confidence::Low,
        }
    }

    /// The stable slug.
    pub fn slug(self) -> &'static str {
        match self {
            ExternalEvidence::ResolverRefusal => "resolver-refusal",
            ExternalEvidence::QualifiedImport => "qualified-import",
            ExternalEvidence::SelectiveImport => "selective-import",
            ExternalEvidence::ForeignDeclaration => "foreign-declaration",
            ExternalEvidence::LiteralArgument => "literal-argument",
            ExternalEvidence::SystemInterface => "system-interface",
            ExternalEvidence::TypedReceiver => "typed-receiver",
            ExternalEvidence::ChainHead => "chain-head",
            ExternalEvidence::NamePrefix => "name-prefix",
            ExternalEvidence::StdlibScreen => "stdlib-screen",
            ExternalEvidence::SoleWildcard => "sole-wildcard",
            ExternalEvidence::None => "none",
        }
    }
}

/// What an [`ExternalUse`] records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExternalUseKind {
    /// An import form the plugin's `imports` does not carry (dynamic
    /// `import()`, Kivy `#:import`, HCL `module` source, Proto `import`).
    Import,
    /// A foreign-function declaration (`DllImport`, `#[link]` block,
    /// `foreign import`, OCaml `external`, `bind(c)`).
    ForeignDecl,
    /// A call that spawns a program (`subprocess.run`, `exec.Command`).
    ProcessSpawn,
    /// A call that loads a native library (`dlopen`, `ctypes.CDLL`).
    LibraryLoad,
    /// A syscall instruction (`syscall`, `int 0x80`, `svc`).
    Syscall,
    /// A use of an external provider or package that is not a call
    /// (Terraform provider, Bazel repository, Nix fetcher, HDL library).
    Provider,
    /// A call whose qualifier the plugin does not record on its call
    /// site (Haskell `T.pack`): side-channel only.
    QualifiedCall,
    /// A type the file declares (`module` is its name). Not a use: it
    /// makes a qualifier naming that type internal, for the languages
    /// whose plugin records no declaration for it (a Rust `enum` with no
    /// `impl`).
    Declares,
}

/// A use of something outside the tree that the call graph does not
/// carry. Recorded only when the run asks for externals; never read by a
/// resolver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalUse {
    pub kind: ExternalUseKind,
    /// The module, library, program or provider as written.
    pub module: String,
    /// The symbol used (function, API), or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub symbol: String,
    /// A local name the use binds, or empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub alias: String,
    /// The construct that produced it (`subprocess.run`, `DllImport`).
    pub form: String,
    pub site_line: u32,
    pub site_byte: u32,
}

/// The whole report.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ExternalsReport {
    pub schema: String,
    pub cgg_version: String,
    pub disclaimer: String,
    /// The kinds shown; every other kind is counted in `totals.withheld`.
    pub kinds: Vec<ExternalKind>,
    pub totals: ExternalTotals,
    /// What cgg could and could not attribute, per language.
    pub coverage: Vec<LanguageCoverage>,
    pub libraries: Vec<Library>,
    /// Sites with no evidence naming a library, grouped by language and
    /// called name. Present only when `unattributed` is shown.
    pub unattributed: Vec<UnattributedGroup>,
}

/// Run-level counts. Every external or stdlib call site is counted
/// exactly once, in `by_kind`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalTotals {
    pub sites: u64,
    pub by_kind: BTreeMap<String, u64>,
    pub libraries_by_kind: BTreeMap<String, u64>,
    /// Shown sites by attribution confidence.
    pub by_confidence: BTreeMap<String, u64>,
    /// Sites of kinds that were not selected.
    pub withheld: BTreeMap<String, u64>,
}

/// Per-language disclosure.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageCoverage {
    pub language: String,
    pub files: u64,
    /// Import records the plugin extracted, plus side-channel imports.
    pub imports: u64,
    pub sites: u64,
    /// Sites attributed to a library (any kind but unattributed).
    pub attributed: u64,
    pub unattributed: u64,
    /// Import and boundary forms the language reads, by name.
    pub forms: Vec<String>,
}

/// One library and everything that reaches it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Library {
    /// `L0`, `L1`, … in report order; stable for one document.
    pub id: String,
    pub kind: ExternalKind,
    pub language: String,
    /// The distribution unit: crate, package, module root, program.
    pub name: String,
    /// The modules as written that normalized to `name`.
    pub modules: Vec<String>,
    pub imports: Vec<ImportSite>,
    pub symbols: Vec<SymbolUse>,
    /// In-tree callables that call it directly, most sites first.
    pub callers: Vec<CallerUse>,
    pub sites: Vec<ExternalSite>,
    /// Example branches, root first, ending at the boundary caller and
    /// then `library::symbol`.
    pub branches: Vec<Vec<String>>,
}

impl Default for ExternalKind {
    fn default() -> Self {
        ExternalKind::Unattributed
    }
}

/// Where a library is imported.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportSite {
    pub path: String,
    pub line: u32,
    pub module: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub alias: String,
}

/// A symbol of a library and how many sites call it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolUse {
    pub symbol: String,
    pub sites: u64,
}

/// A boundary caller.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallerUse {
    /// `None` for code outside any callable (a script's top level).
    pub callable: Option<CallableId>,
    pub qualified_name: String,
    pub path: String,
    pub line: u32,
    pub sites: u64,
}

/// One call site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSite {
    pub caller: Option<CallableId>,
    pub caller_qualified_name: String,
    pub path: String,
    pub line: u32,
    pub symbol: String,
    pub evidence: ExternalEvidence,
    pub confidence: Confidence,
}

/// Unattributed sites of one name in one language.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnattributedGroup {
    pub language: String,
    pub name: String,
    pub sites: u64,
    /// The first sites, in file order.
    pub sample: Vec<ExternalSite>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_parses_every_spelling() {
        assert_eq!(ExternalsSpec::parse("").unwrap(), ExternalsSpec::default());
        assert_eq!(ExternalsSpec::parse("all").unwrap(), ExternalsSpec::all());
        let s = ExternalsSpec::parse("process,third-party,process").unwrap();
        assert_eq!(s.kinds(), &[ExternalKind::ThirdParty, ExternalKind::Process]);
        assert!(ExternalsSpec::parse("thirdparty").is_err());
        let j: ExternalsSpec = serde_json::from_str("true").unwrap();
        assert_eq!(j, ExternalsSpec::default());
        let j: ExternalsSpec = serde_json::from_str(r#"["stdlib"]"#).unwrap();
        assert!(j.shows(ExternalKind::Stdlib) && !j.shows(ExternalKind::Process));
        let back = serde_json::to_string(&j).unwrap();
        assert_eq!(back, r#"["stdlib"]"#);
    }

    #[test]
    fn evidence_confidence_follows_the_ladder() {
        assert_eq!(ExternalEvidence::QualifiedImport.confidence(), Confidence::High);
        assert_eq!(ExternalEvidence::ChainHead.confidence(), Confidence::Medium);
        assert_eq!(ExternalEvidence::SoleWildcard.confidence(), Confidence::Low);
    }
}
