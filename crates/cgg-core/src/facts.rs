//! Intermediate facts produced by the per-file extraction pass.
//!
//! Each language plugin walks its tree-sitter AST once and emits a
//! [`FileFacts`] value describing the callables defined in the file
//! and the call sites referencing other callables. Every later stage —
//! the intra-file linker, the scope-aware cross-file resolvers, the FFI
//! linker — consumes this same shape.
//!
//! The shape follows codescope's "definition vs reference" split but
//! is strongly typed, carries fully-qualified names from day one, and
//! tracks byte ranges alongside line ranges (byte ranges are what
//! tree-sitter speaks natively and the intra-file linker needs for
//! smallest-enclosing-range containment).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::graph::CallableKind;
use crate::ids::FileId;

/// Sentinel `RefRecord::receiver_hint` value marking a function
/// referenced as a *value* (passed by name to a registrar/callback)
/// rather than called — Issue 4. The leading control char can never
/// appear in real source, so it cannot collide with a real receiver.
/// The resolver turns these into `Via::Reference` edges (gated behind
/// `--reference-edges`) and never lets them reach the unresolved /
/// external buckets.
pub const VALUE_REF_HINT: &str = "\u{1}value-ref";

/// Sentinel `RefRecord::receiver_hint` value marking a *string literal*
/// argument of a registration call — `Route::get('/x', 'C@method')`,
/// `get 'photos', to: 'photos#index'`. `name` holds the literal
/// verbatim; decoding it into a symbol is framework-specific and
/// therefore belongs to the framework rule engine, not to a plugin.
///
/// Like [`VALUE_REF_HINT`] this never reaches the unresolved / external
/// buckets, and per §8 of the design it never manufactures an edge on
/// its own: a string that happens to look like a method name is not
/// evidence of a call. It only feeds entry-node synthesis, where the
/// framework rule supplies the missing premise that the framework does
/// invoke whatever that string names.
pub const STRING_REF_HINT: &str = "\u{1}string-ref";

/// Variant tag on a definition, refining the callable kind with
/// language-specific hints. The pipeline collapses this onto the
/// [`CallableKind`] enum when building the final graph node.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefVariant {
    #[default]
    FreeFunction,
    InherentMethod,
    TraitMethod,
    TraitDefaultMethod,
    AsyncFunction,
    Constructor,
    Destructor,
    NamedClosure,
    NamedLambda,
    StaticMethod,
    ClassMethod,
    Property,
}

impl DefVariant {
    pub fn to_callable_kind(self) -> CallableKind {
        match self {
            DefVariant::FreeFunction | DefVariant::AsyncFunction => {
                CallableKind::Function
            }
            DefVariant::InherentMethod
            | DefVariant::TraitMethod
            | DefVariant::TraitDefaultMethod
            | DefVariant::StaticMethod
            | DefVariant::ClassMethod => CallableKind::Method,
            DefVariant::Constructor => CallableKind::Constructor,
            DefVariant::Destructor => CallableKind::Destructor,
            DefVariant::NamedClosure | DefVariant::NamedLambda => CallableKind::Closure,
            DefVariant::Property => CallableKind::Property,
        }
    }
}

/// Normalized cross-language visibility.
///
/// The language-native spelling stays in [`DefRecord::visibility`]
/// (`"pub(crate)"`, `"fileprivate"`, `"protected internal"`); this is
/// its canonical projection. Normalization happens in the *plugin*,
/// because only the plugin knows the language's default rule — Java's
/// absent modifier means package-private, Kotlin's means public, C#'s
/// means private, and Go encodes visibility in the identifier's first
/// letter. A downstream normalizer could not tell `""` meaning
/// "language default" from `""` meaning "cgg never looked".
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Vis {
    /// Not determined. Cross-check `PluginSignals::visibility` to learn
    /// whether that means "not extractable" or "not implemented".
    #[default]
    Unknown,
    /// Visible outside the compilation unit.
    Public,
    /// Visible within the crate / assembly / package, but not outside.
    Internal,
    /// Declaring type and its subtypes only.
    Protected,
    /// Declaring type / file / module only.
    Private,
}

impl Vis {
    pub fn is_unknown(&self) -> bool {
        matches!(self, Vis::Unknown)
    }
    /// Whether a caller could exist outside the analyzed unit.
    pub fn escapes_unit(&self) -> bool {
        matches!(self, Vis::Public | Vis::Unknown)
    }
}

/// What role a definition plays in a test suite.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TestRole {
    /// A test case, invoked by a harness and never by project code.
    Case,
    /// A harness lifecycle hook (`setUp`, `@BeforeEach`, a pytest
    /// fixture, `TestMain`).
    Fixture,
    /// Not a test itself, but lexically inside a test-only container
    /// such as `#[cfg(test)] mod tests` or a `describe` block.
    Support,
}

/// A callable referenced by an identifier-shaped literal rather than by
/// a call — `getattr(o, "method")`, `Class.forName("...")`, `send(:sym)`.
///
/// This is a **suppression-only** signal. It never creates an edge:
/// doing so would put a guess into the call graph, which is the one
/// thing cgg refuses to do. It exists so the dead-code report can say
/// "something names this string, so cgg may simply be unable to see the
/// call".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DynUse {
    /// The identifier-shaped literal.
    pub name: String,
    /// The construct that named it (`getattr`, `send`, `forName`).
    pub via: String,
    pub site_line: u32,
    pub site_byte: u32,
}

/// A region of code that cannot be reached because control cannot get
/// there — statements after an unconditional terminator.
///
/// Unlike everything else in a dead-code report, this is a proof rather
/// than a hypothesis: it is derived from the shape of the syntax tree,
/// not from whether cgg could find a caller.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UnreachableRegion {
    pub start_line: u32,
    pub end_line: u32,
    pub start_byte: u32,
    pub end_byte: u32,
    /// `"after-return"`, `"after-throw"`, `"after-break"`,
    /// `"after-continue"`, `"after-panic"`.
    pub cause: String,
}

/// A name this file makes visible to other modules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportRecord {
    /// The exported name as other modules see it.
    pub name: String,
    /// `"__all__"`, `"export"`, `"pub-use"`, `"capitalized"`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target: String,
}

/// A single callable definition extracted from a file.
///
/// `Default` is derived so plugins can construct a record with
/// `..Default::default()` and stay source-compatible as optional
/// extraction signals are added.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DefRecord {
    /// Last segment of `qualified_name`, i.e. the identifier as
    /// written in source.
    pub simple_name: String,
    /// Fully-qualified name. Joiner is `::` for Rust, `.` for Python
    /// and other `.`-ish languages, `/` for Go (package path + func).
    pub qualified_name: String,
    pub variant: DefVariant,

    /// Inclusive 1-based line numbers for human display.
    pub start_line: u32,
    pub end_line: u32,

    /// Half-open byte range in the source.
    pub start_byte: u32,
    pub end_byte: u32,

    /// Optional single-line signature preview.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_hint: String,

    /// Language-native visibility (`"pub"`, `"public"`, `""`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub visibility: String,

    /// Attributes / decorators attached at the definition site
    /// (`"#[get]"`, `"@app.route('/x')"`, `"@Override"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attributes: Vec<String>,

    /// Normalized visibility. See [`Vis`].
    #[serde(default, skip_serializing_if = "Vis::is_unknown")]
    pub vis: Vis,

    /// Test role, when this definition is part of a test suite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_role: Option<TestRole>,

    /// Base classes / interfaces of the type owning this definition —
    /// `class Encoder(nn.Module)`, `implements Runnable`, `: IJob`.
    ///
    /// Recorded on the *method*, not the type, because cgg's model has
    /// no node for a type: the only thing a framework rule can mark is a
    /// callable, so the contract has to travel with one. Names are
    /// stored as written, including any qualifier (`nn.Module`), since
    /// the matcher compares both the full path and the last segment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub base_types: Vec<String>,
}

/// A single call-site reference extracted from a file.
///
/// `Default` is derived so plugins can construct a record with
/// `..Default::default()` and stay source-compatible as optional
/// fields are added — the same contract [`DefRecord`] already has.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RefRecord {
    /// Identifier at the call site as written (`foo`, `do_thing`,
    /// `obj.method`'s `method`).
    pub name: String,
    /// Fully-qualified receiver path when determinable from the
    /// source alone (e.g. `a::b::c` in a Rust path; `self.method`
    /// yields `self.method`). Empty if no path information is
    /// available; resolution is the job of Task 6+.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub receiver_hint: String,

    pub site_line: u32,
    pub site_byte: u32,

    /// The registrar call this reference sits inside, when it is an
    /// argument to one: `app.get`, `Route::get`, `router.HandleFunc`.
    /// Empty for an ordinary call site.
    ///
    /// This is the landing zone for framework route metadata. Nothing
    /// else could hold it: `attribute_key` discards arguments by design
    /// (`@app.route('/x')` normalizes to `app.route`), and `DynUse` is
    /// suppression-only by explicit contract. Without it an entry node
    /// could say *that* a route exists but never *which* — and a list of
    /// anonymous routes is not an attack surface map.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub context: String,

    /// First string literal argument of `context`'s call — the route
    /// path, queue name, or command string. Empty when there was none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub route: String,
}

/// The two-phase AST pass output for a single file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct FileFacts {
    pub file: FileId,
    pub path: PathBuf,
    pub language: String,
    pub definitions: Vec<DefRecord>,
    pub references: Vec<RefRecord>,
    /// Import / use / package declarations, serialized as the raw
    /// source slice with a structural tag. Task 6 consumes these for
    /// scope-aware resolution.
    pub imports: Vec<ImportRecord>,
    /// Local variable type annotations: (var_name, type_name, scope_byte).
    /// Used by the type propagator to rewrite receiver_hints.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_types: Vec<LocalType>,
    /// Names this file makes visible to other modules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<ExportRecord>,
    /// Identifier-shaped literals used for dynamic dispatch.
    /// Suppression-only; never becomes an edge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dyn_uses: Vec<DynUse>,
    /// Statements that control flow cannot reach.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreachable: Vec<UnreachableRegion>,
}

/// A top-level import / use / include descriptor — not yet interpreted.
///
/// Task 4 emits the raw path; Task 6's resolvers parse it according to
/// language-specific rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImportRecord {
    /// `"use"`, `"import"`, `"from-import"`, `"include"`, `"package"`.
    pub kind: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub alias: String,
    pub site_line: u32,
    pub site_byte: u32,
}

/// A local variable with a known type (from explicit declaration).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalType {
    pub var_name: String,
    pub type_name: String,
    /// Byte offset of the enclosing scope (function body start).
    pub scope_byte: u32,
}

impl FileFacts {
    pub fn new(file: FileId, path: PathBuf, language: impl Into<String>) -> Self {
        Self {
            file,
            path,
            language: language.into(),
            definitions: Vec::new(),
            references: Vec::new(),
            imports: Vec::new(),
            local_types: Vec::new(),
            exports: Vec::new(),
            dyn_uses: Vec::new(),
            unreachable: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
            && self.references.is_empty()
            && self.imports.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::FileId;
    use std::path::PathBuf;

    #[test]
    fn variant_to_callable_kind_mapping() {
        use CallableKind as K;
        assert_eq!(DefVariant::FreeFunction.to_callable_kind(), K::Function);
        assert_eq!(DefVariant::InherentMethod.to_callable_kind(), K::Method);
        assert_eq!(DefVariant::TraitDefaultMethod.to_callable_kind(), K::Method);
        assert_eq!(DefVariant::Constructor.to_callable_kind(), K::Constructor);
        assert_eq!(DefVariant::NamedClosure.to_callable_kind(), K::Closure);
        assert_eq!(DefVariant::Property.to_callable_kind(), K::Property);
    }

    #[test]
    fn facts_round_trip() {
        let mut f = FileFacts::new(FileId::new(0), PathBuf::from("a.py"), "python");
        f.definitions.push(DefRecord {
            simple_name: "foo".into(),
            qualified_name: "mod.foo".into(),
            variant: DefVariant::FreeFunction,
            start_line: 1,
            end_line: 3,
            start_byte: 0,
            end_byte: 20,
            signature_hint: String::new(),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        });
        let s = serde_json::to_string(&f).unwrap();
        let f2: FileFacts = serde_json::from_str(&s).unwrap();
        assert_eq!(f, f2);
    }
}
