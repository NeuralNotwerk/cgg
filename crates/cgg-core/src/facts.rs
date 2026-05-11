//! Intermediate facts produced by the per-file extraction pass.
//!
//! Each language plugin walks its tree-sitter AST once and emits a
//! [`FileFacts`] value describing the callables defined in the file
//! and the call sites referencing other callables. Later stages
//! (Task 5's intra-file linker, Task 6+'s scope-aware resolvers, the
//! FFI linker in Task 8, and the cache in Task 11) all consume this
//! same shape.
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

/// Variant tag on a definition, refining the callable kind with
/// language-specific hints. The pipeline collapses this onto the
/// [`CallableKind`] enum when building the final graph node.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefVariant {
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
            DefVariant::NamedClosure | DefVariant::NamedLambda => {
                CallableKind::Closure
            }
            DefVariant::Property => CallableKind::Property,
        }
    }
}

/// A single callable definition extracted from a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
}

/// A single call-site reference extracted from a file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
}

/// The two-phase AST pass output for a single file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
        }
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty() && self.references.is_empty() && self.imports.is_empty()
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
        });
        let s = serde_json::to_string(&f).unwrap();
        let f2: FileFacts = serde_json::from_str(&s).unwrap();
        assert_eq!(f, f2);
    }
}
