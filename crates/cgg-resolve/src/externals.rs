//! `--externals`: attribute every call that leaves the tree to the
//! library it belongs to.
//!
//! Runs once, after every resolver and after the audit reconciliation, so
//! a site is external only when nothing in the tree bound it. It reads
//! the per-file `stdlib_calls` / `external_calls` buckets, the import
//! records, the `RefRecord` behind each site and the
//! [`cgg_core::ExternalUse`] side channel. It writes nothing back: the
//! buckets, the graph and the facts leave exactly as they came in.
//!
//! Every bucket site is accounted for exactly once — under a library, as
//! `unattributed`, or as `withheld` when its kind was not selected.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use cgg_core::audit::{AuditFileRecord, AuditUnresolvedCall, UnresolvedReason};
use cgg_core::external::{module_is_stdlib, split_qualified_name};
use cgg_core::externals::{
    CallerUse, EXTERNALS_DISCLAIMER, EXTERNALS_SCHEMA, ExternalEvidence, ExternalKind,
    ExternalSite, ExternalTotals, ExternalUse, ExternalUseKind, ExternalsReport,
    ExternalsSpec, ImportSite, LanguageCoverage, Library, SymbolUse, UnattributedGroup,
};
use cgg_core::graph::{Confidence, Graph};
use cgg_core::ids::{CallableId, FileId};
use cgg_core::stdlib::{
    self, LIBRARY_LOAD_APIS, OS_BINDING_MODULES, PROCESS_SPAWN_APIS, SHELL_BUILTINS,
    SYSTEM_APIS, c_header_class, externals_table, stdlib_library_name, under_stdlib_root,
};
use cgg_core::{FileFacts, RefRecord};

use crate::heads::{FileHeads, WorkspaceVocab};

/// A shown site, for the graph: one edge from `caller` (or the file's
/// top level) to the `(kind, library, symbol)` leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeafSite {
    pub kind: ExternalKind,
    pub library: String,
    pub symbol: String,
    pub caller: Option<CallableId>,
    pub file: FileId,
    pub line: u32,
    pub byte: u32,
    pub confidence: Confidence,
}

/// The report, plus the leaf sites the graph is built from.
#[derive(Clone, Debug, Default)]
pub struct ExternalsInventory {
    pub report: ExternalsReport,
    pub leaves: Vec<LeafSite>,
    /// Every bucket site plus side-channel site counted, shown or not.
    /// Equal to the bucket totals plus the side-channel sites — the
    /// accounting invariant the tests check.
    pub accounted: u64,
}

/// Where an import's module lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Internal,
    Stdlib,
    System,
    External,
}

/// A local name an import binds.
#[derive(Clone, Debug)]
struct Bind {
    module: String,
    class: Class,
    /// The name is the module itself (`import requests`), not a symbol
    /// taken out of one (`from requests import post`).
    module_binding: bool,
}

/// One classified import, for the inventory.
#[derive(Clone, Debug)]
struct ClassifiedImport {
    module: String,
    alias: String,
    line: u32,
    class: Class,
}

/// Names the tree declares, per language: what makes an import internal
/// in the languages `heads.rs` does not judge.
#[derive(Default)]
struct TreeNames {
    /// (language, name): first segments of qualified names, declared
    /// packages and their heads, file stems and directory names.
    names: HashSet<(String, String)>,
    /// Every analyzed file's path, `/`-separated, without extension.
    paths: Vec<String>,
    /// PHP names, lowercased.
    names_lower: HashSet<String>,
}

impl TreeNames {
    fn build(facts: &[FileFacts]) -> Self {
        let mut t = TreeNames::default();
        for f in facts {
            let lang = f.language.as_str();
            let mut add = |n: &str| {
                let n = n.trim();
                if !n.is_empty() {
                    t.names.insert((lang.to_string(), n.to_string()));
                }
            };
            for d in &f.definitions {
                let head = d
                    .qualified_name
                    .split([':', '.', '/', '\\', '#'])
                    .find(|s| !s.is_empty())
                    .unwrap_or("");
                add(head);
            }
            for p in &f.packages {
                add(p);
                add(head_segment(p));
            }
            for u in &f.external_uses {
                if u.kind == ExternalUseKind::Declares {
                    add(&u.module);
                }
            }
            for c in &f.classes {
                add(c.class_qn.rsplit(['.', ':', '\\']).next().unwrap_or(""));
            }
            for d in &f.definitions {
                // Every type segment of a qualified name: `Node` in
                // `crate::ast::Node::kind`.
                for seg in d.qualified_name.split([':', '.', '/', '\\', '#']) {
                    if seg.starts_with(char::is_uppercase) {
                        add(seg.split('<').next().unwrap_or(seg));
                    }
                }
            }
            for imp in &f.imports {
                if imp.kind == "crate-root" {
                    add(&imp.path);
                    add(head_segment(&imp.path));
                }
            }
            if let Some(stem) = f.path.file_stem().and_then(|s| s.to_str()) {
                add(stem);
                // OCaml, F#, Haskell, Elixir: `util.ml` is module `Util`.
                let mut cap = stem.to_string();
                if let Some(c) = cap.get_mut(0..1) {
                    c.make_ascii_uppercase();
                }
                add(&cap);
            }
            for c in f.path.parent().into_iter().flat_map(|p| p.components()) {
                if let std::path::Component::Normal(s) = c
                    && let Some(s) = s.to_str()
                {
                    add(s);
                }
            }
            let p = f.path.with_extension("");
            t.paths.push(p.to_string_lossy().replace('\\', "/"));
        }
        t.paths.sort();
        t.names_lower = t
            .names
            .iter()
            .filter(|(l, _)| l == "php")
            .map(|(_, n)| n.to_ascii_lowercase())
            .collect();
        t
    }

    fn knows(&self, lang: &str, name: &str) -> bool {
        self.names.contains(&(lang.to_string(), name.to_string()))
            // PHP namespaces and class names are case-insensitive.
            || (lang == "php" && self.names_lower.contains(&name.to_ascii_lowercase()))
    }

    /// Whether an include-style path names an analyzed file.
    fn names_file(&self, module: &str) -> bool {
        let m = module
            .trim()
            .trim_matches(['"', '\'', '<', '>'])
            .trim_start_matches("./");
        if m.is_empty() {
            return false;
        }
        let m = m.replace('\\', "/");
        let stem = std::path::Path::new(&m).with_extension("");
        let stem = stem.to_string_lossy();
        self.paths.iter().any(|p| {
            p.ends_with(stem.as_ref())
                && (p.len() == stem.len() || p[..p.len() - stem.len()].ends_with('/'))
        })
    }
}

/// The first segment of a module path.
fn head_segment(m: &str) -> &str {
    let m = m.trim().trim_start_matches(['@', ':', '\\']);
    m.split([':', '.', '/', '\\', '#'])
        .find(|s| !s.is_empty())
        .unwrap_or("")
}

/// The leading identifier of a receiver or qualifier: `os` in
/// `os.path`, `Command` in `Command::new("git").arg(…)`, `$x` in `$x->f`.
fn leading_ident(rh: &str) -> &str {
    let rh = rh.trim().trim_start_matches("::").trim_start_matches('\\');
    let end = rh
        .char_indices()
        .find(|&(i, c)| {
            !(c.is_alphanumeric() || c == '_' || (i == 0 && matches!(c, '$' | '@' | ':')))
        })
        .map_or(rh.len(), |(i, _)| i);
    &rh[..end]
}

/// The separator a language writes between module segments.
fn separator(lang: &str) -> &'static str {
    match lang {
        "rust" | "cpp" | "perl" | "ruby" | "php" | "c" | "objc" => "::",
        "clojure" => "/",
        "erlang" => ":",
        _ => ".",
    }
}

/// Map a module as written to its distribution unit.
fn library_of(lang: &str, module: &str, class: Class) -> String {
    let m = module.trim().trim_matches(['"', '\'']);
    let segs = |sep: &[char]| -> Vec<&str> {
        m.split(sep).filter(|s| !s.is_empty()).collect::<Vec<_>>()
    };
    match lang {
        "javascript" | "typescript" => {
            if let Some(rest) = m.strip_prefix("node:") {
                return format!("node:{}", rest.split('/').next().unwrap_or(rest));
            }
            if m.starts_with("npm:") || m.starts_with("jsr:") {
                return m.to_string();
            }
            let s = segs(&['/']);
            if m.starts_with('@') && s.len() >= 2 {
                format!("{}/{}", s[0], s[1])
            } else {
                s.first().map_or(m.to_string(), |x| x.to_string())
            }
        }
        "go" => {
            let s = segs(&['/']);
            if class == Class::Stdlib || !s.first().is_some_and(|h| h.contains('.')) {
                return m.to_string();
            }
            let n = match s[0] {
                "github.com" | "gitlab.com" | "bitbucket.org" | "golang.org"
                | "codeberg.org" | "gitee.com" | "sr.ht" | "git.sr.ht" => 3,
                "gopkg.in" => 2,
                _ => 2,
            };
            let mut end = n.min(s.len());
            // A major-version suffix is part of the module path:
            // `github.com/gdamore/tcell/v2`.
            if s.get(end).is_some_and(|v| {
                v.len() > 1
                    && v.starts_with('v')
                    && v[1..].chars().all(|c| c.is_ascii_digit())
            }) {
                end += 1;
            }
            s[..end].join("/")
        }
        "java" | "kotlin" | "scala" | "groovy" => {
            let mut s = segs(&['.']);
            while s.len() > 1
                && s.last()
                    .is_some_and(|l| l.starts_with(char::is_uppercase) || *l == "*")
            {
                s.pop();
            }
            let n = if class == Class::Stdlib { 2 } else { 3 };
            s[..n.min(s.len())].join(".")
        }
        "csharp" | "fsharp" | "powershell" => {
            let s = segs(&['.']);
            s[..2.min(s.len())].join(".")
        }
        "php" => {
            let s = segs(&['\\']);
            s[..2.min(s.len())].join("\\")
        }
        "c" | "cpp" | "objc" if class == Class::System => {
            cgg_core::stdlib::c_system_header_family(m).to_string()
        }
        "c" | "cpp" | "objc" if class == Class::Stdlib => {
            if lang == "c" {
                "libc".to_string()
            } else {
                "std".to_string()
            }
        }
        "c" | "cpp" | "objc" => {
            let h = m.trim_matches(['<', '>']);
            match h.split_once('/') {
                Some((dir, _)) => dir.to_string(),
                None => h
                    .strip_suffix(".h")
                    .or_else(|| h.strip_suffix(".hpp"))
                    .or_else(|| h.strip_suffix(".hh"))
                    .unwrap_or(h)
                    .to_string(),
            }
        }
        "dart" => {
            if let Some(rest) = m.strip_prefix("package:") {
                rest.split('/').next().unwrap_or(rest).to_string()
            } else {
                m.split('/').next().unwrap_or(m).to_string()
            }
        }
        "ruby" | "lean" => m.split(['/', '.']).next().unwrap_or(m).to_string(),
        "haskell" => {
            let s = segs(&['.']);
            s[..2.min(s.len())].join(".")
        }
        "smithy" => m.split('#').next().unwrap_or(m).to_string(),
        "vhdl" => m.split('.').next().unwrap_or(m).to_string(),
        "proto" | "openapi" | "asyncapi" | "hcl" | "starlark" | "nix" | "cmake"
        | "verilog" | "solidity" | "asm" => {
            if lang == "solidity" && m.starts_with('@') {
                let s = segs(&['/']);
                return s[..2.min(s.len())].join("/");
            }
            if lang == "starlark" && m.starts_with('@') {
                return m.split("//").next().unwrap_or(m).to_string();
            }
            m.to_string()
        }
        "rust" | "perl" => {
            let h = m.trim_start_matches("::");
            h.split("::").next().unwrap_or(h).to_string()
        }
        "clojure" => {
            let s = segs(&['.']);
            if class == Class::Stdlib {
                s[..2.min(s.len())].join(".")
            } else {
                s.first().map_or(m.to_string(), |x| x.to_string())
            }
        }
        _ => {
            let h = m.trim_start_matches('.');
            h.split(['.', ':', '/'])
                .find(|s| !s.is_empty())
                .unwrap_or(h)
                .to_string()
        }
    }
}

/// Import kinds that declare the file's own identity, not a dependency.
fn is_identity_kind(lang: &str, kind: &str) -> bool {
    matches!(
        kind,
        "package"
            | "package-root"
            | "package-path"
            | "package-name"
            | "package-export"
            | "local-export"
            | "crate-root"
            | "scala-package"
            | "perl-package"
            | "elixir-module"
            | "perl-isa"
            | "behaviour"
            | "re-export"
            | "reexport"
            | "pub-use"
            | "zig-module"
            | "part"
            | "part-of"
            | "part-of-library"
    ) || (kind == "library" && lang == "dart")
        || kind == "export"
}

/// Per-file context for attribution.
struct FileCtx<'a> {
    lang: &'a str,
    binds: HashMap<String, Bind>,
    /// External modules imported whole (`from m import *`, `using N`,
    /// `open M`, `use M`): the candidates for a bare unattributed call.
    wildcards: BTreeSet<String>,
    imports: Vec<ClassifiedImport>,
    /// Third-party C-family headers: (library, header).
    c_headers: Vec<String>,
    refs: HashMap<(u32, &'a str), &'a RefRecord>,
    uses_at: HashMap<u32, Vec<&'a ExternalUse>>,
    uses_by_line: HashMap<u32, Vec<&'a ExternalUse>>,
}

struct Pass {
    tree: TreeNames,
    vocab: WorkspaceVocab,
}

impl Pass {
    fn classify(&self, lang: &str, kind: &str, module: &str) -> Class {
        let m = module.trim().trim_matches(['"', '\'']);
        // Nothing nameable, or a path built at run time (`source "$CONF"`):
        // not a library cgg can name.
        let m = m.strip_prefix("global::").unwrap_or(m);
        if m.is_empty()
            || !m.chars().any(char::is_alphanumeric)
            || m.starts_with(['$', '(', '{'])
            || m.contains("${")
            || m.starts_with("../")
            || (lang == "vhdl" && (m == "work" || m.starts_with("work.")))
        {
            return Class::Internal;
        }
        if matches!(lang, "c" | "cpp" | "objc") {
            // A quoted include names the project's own header, found or
            // not (a generated `config.h` is not in the source tree).
            if kind == "include" {
                return Class::Internal;
            }
            return match c_header_class(m) {
                Some(false) => Class::Stdlib,
                Some(true) => Class::System,
                None if self.tree.names_file(m) => Class::Internal,
                None => Class::External,
            };
        }
        if m.starts_with('.')
            || m.starts_with('/')
            || m.starts_with("self::")
            || m.starts_with("crate::")
            || m.starts_with("super::")
            || m == "self"
            || m == "crate"
            || m == "super"
            || m.starts_with("$crate")
            || m.starts_with("//")
            || m.starts_with(':') && lang == "starlark"
        {
            return Class::Internal;
        }
        if self.is_stdlib(lang, m) {
            if externals_table(OS_BINDING_MODULES, lang)
                .iter()
                .any(|o| *o == m || m.starts_with(&format!("{o}.")))
            {
                return Class::System;
            }
            return Class::Stdlib;
        }
        // The resolver's own judgement where it has one.
        let judged = match lang {
            "python" | "kivy" => Some(self.vocab.py_workspace(m)),
            "javascript" | "typescript" => Some(self.vocab.js_workspace(m)),
            "go" => Some(self.vocab.go_workspace(m, true)),
            _ => None,
        };
        if let Some(ws) = judged {
            return if ws {
                Class::Internal
            } else {
                self.external_class(lang, m)
            };
        }
        if self.tree.names_file(m) {
            return Class::Internal;
        }
        let head = head_segment(m);
        if self.tree.knows(lang, m) || (!head.is_empty() && self.tree.knows(lang, head)) {
            return Class::Internal;
        }
        self.external_class(lang, m)
    }

    fn external_class(&self, lang: &str, m: &str) -> Class {
        let lib = library_of(lang, m, Class::External);
        if externals_table(OS_BINDING_MODULES, lang).iter().any(|o| {
            *o == m
                || *o == lib
                || m.starts_with(&format!("{o}/"))
                || m.starts_with(&format!("{o}::"))
        }) {
            Class::System
        } else {
            Class::External
        }
    }

    fn is_stdlib(&self, lang: &str, m: &str) -> bool {
        if under_stdlib_root(lang, m) {
            return true;
        }
        match lang {
            "go" => {
                // Go's rule: a standard-library path has no dot in its
                // first element.
                return !m.split('/').next().unwrap_or(m).contains('.') && m != "C";
            }
            "dart" => return m.starts_with("dart:"),
            _ => {}
        }
        let std = stdlib::stdlib_names(if lang == "kivy" { "python" } else { lang });
        let modules = stdlib::stdlib_modules(lang);
        match std {
            Some(s) => module_is_stdlib(m, s, modules, false),
            None => modules.contains(&m),
        }
    }

    fn file_ctx<'a>(&self, facts: &'a FileFacts) -> FileCtx<'a> {
        let lang = facts.language.as_str();
        let heads = matches!(
            lang,
            "python"
                | "javascript"
                | "typescript"
                | "go"
                | "java"
                | "kotlin"
                | "scala"
                | "csharp"
                | "cpp"
        )
        .then(|| FileHeads::build(facts, &self.vocab));
        let mut ctx = FileCtx {
            lang,
            binds: HashMap::new(),
            wildcards: BTreeSet::new(),
            imports: Vec::new(),
            c_headers: Vec::new(),
            refs: facts
                .references
                .iter()
                .map(|r| ((r.site_byte, r.name.as_str()), r))
                .collect(),
            uses_at: HashMap::new(),
            uses_by_line: HashMap::new(),
        };
        for u in &facts.external_uses {
            ctx.uses_at.entry(u.site_byte).or_default().push(u);
            ctx.uses_by_line.entry(u.site_line).or_default().push(u);
        }
        let bind = |ctx: &mut FileCtx,
                    local: &str,
                    module: &str,
                    class: Class,
                    module_binding: bool| {
            let local = local.trim();
            if local.is_empty() || local == "*" || local == "_" {
                return;
            }
            // A heads-judged binding is the resolver's verdict.
            let class = match heads.as_ref().and_then(|h| h.binding(local)) {
                Some((_, false, _))
                    if class != Class::Stdlib && class != Class::System =>
                {
                    Class::Internal
                }
                Some((_, true, _)) if class == Class::Internal => Class::External,
                _ => class,
            };
            ctx.binds
                .entry(local.to_string())
                .and_modify(|b| {
                    if class == Class::Internal {
                        b.class = Class::Internal;
                        b.module = module.to_string();
                    }
                })
                .or_insert_with(|| Bind {
                    module: module.to_string(),
                    class,
                    module_binding,
                });
        };
        let records = facts
            .imports
            .iter()
            .map(|i| {
                (
                    i.kind.as_str(),
                    i.path.as_str(),
                    i.alias.as_str(),
                    i.site_line,
                )
            })
            .chain(
                facts
                    .external_uses
                    .iter()
                    .filter(|u| u.kind == ExternalUseKind::Import)
                    .map(|u| {
                        (
                            "side-import",
                            u.module.as_str(),
                            u.alias.as_str(),
                            u.site_line,
                        )
                    }),
            );
        for (kind, path, alias, line) in records {
            if is_identity_kind(lang, kind) {
                continue;
            }
            match kind {
                "from-import" | "import-member" | "refer" if !path.trim().is_empty() => {
                    let module = path.trim();
                    let class = self.classify(lang, kind, module);
                    ctx.imports.push(ClassifiedImport {
                        module: module.to_string(),
                        alias: alias.to_string(),
                        line,
                        class,
                    });
                    for item in alias.split([',', ' ']).filter(|s| !s.is_empty()) {
                        if item == "*" {
                            if class != Class::Internal {
                                ctx.wildcards.insert(module.to_string());
                            }
                            continue;
                        }
                        if item == "as" {
                            continue;
                        }
                        bind(&mut ctx, item, module, class, false);
                    }
                    // `from m import a as b`: the bound name is the last word.
                    for item in alias.split(',') {
                        if let Some((_, b)) = item.split_once(" as ") {
                            bind(&mut ctx, b, module, class, false);
                        }
                    }
                }
                _ => {
                    // Python keeps `import a, b as c` as one record.
                    let whole = if lang == "python" && path.contains(',') {
                        path.split(',')
                            .map(str::trim)
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    } else {
                        vec![path.trim().to_string()]
                    };
                    for item in whole {
                        let (module, local_alias) = match item.split_once(" as ") {
                            Some((m, a)) => (m.trim().to_string(), a.trim().to_string()),
                            None => (item.clone(), alias.trim().to_string()),
                        };
                        let module = module.trim_matches(['"', '\'']).to_string();
                        if module.is_empty() {
                            continue;
                        }
                        let class = self.classify(lang, kind, &module);
                        ctx.imports.push(ClassifiedImport {
                            module: module.clone(),
                            alias: local_alias.clone(),
                            line,
                            class,
                        });
                        if matches!(lang, "c" | "cpp" | "objc") {
                            if class == Class::External {
                                ctx.c_headers.push(module.clone());
                            }
                            continue;
                        }
                        let wildcard = module.ends_with("::*")
                            || module.ends_with(".*")
                            || module.ends_with("._")
                            || matches!(
                                kind,
                                "open" | "using" | "refer-all" | "module-use" | "use"
                            ) && matches!(
                                lang,
                                "ocaml"
                                    | "fsharp"
                                    | "csharp"
                                    | "haskell"
                                    | "vhdl"
                                    | "lean"
                                    | "julia"
                                    | "elixir"
                            )
                            || (lang == "haskell"
                                && kind.starts_with("import")
                                && local_alias.is_empty());
                        if wildcard && class != Class::Internal {
                            ctx.wildcards.insert(
                                module
                                    .trim_end_matches("::*")
                                    .trim_end_matches(".*")
                                    .to_string(),
                            );
                        }
                        let last = module
                            .rsplit([':', '.', '/', '\\'])
                            .find(|s| !s.is_empty() && *s != "*")
                            .unwrap_or(&module)
                            .trim_end_matches(".h")
                            .to_string();
                        let first = head_segment(&module).to_string();
                        if !local_alias.is_empty() {
                            bind(&mut ctx, &local_alias, &module, class, true);
                        } else {
                            // `import a.b` binds `a` in Python; most other
                            // languages bind the last segment.
                            let module_binding = !last.starts_with(char::is_uppercase)
                                || matches!(
                                    lang,
                                    "go" | "ruby"
                                        | "elixir"
                                        | "haskell"
                                        | "ocaml"
                                        | "lua"
                                        | "perl"
                                        | "r"
                                        | "julia"
                                        | "erlang"
                                        | "lean"
                                );
                            if lang == "python" {
                                bind(&mut ctx, &first, &first, class, true);
                            } else if lang == "go" {
                                bind(
                                    &mut ctx,
                                    &last.replace('-', "_"),
                                    &module,
                                    class,
                                    true,
                                );
                            } else {
                                bind(&mut ctx, &last, &module, class, module_binding);
                            }
                            if matches!(
                                lang,
                                "elixir"
                                    | "haskell"
                                    | "ocaml"
                                    | "fsharp"
                                    | "lean"
                                    | "julia"
                                    | "perl"
                                    | "ruby"
                                    | "lua"
                            ) {
                                bind(&mut ctx, &module, &module, class, true);
                            }
                        }
                    }
                }
            }
        }
        ctx
    }
}

/// Languages whose stdlib calls name a module (`os.getcwd`, `string.format`,
/// `lists:map`), so the module is a better library name than the
/// language's stdlib as a whole.
fn module_headed(lang: &str) -> bool {
    matches!(
        lang,
        "python"
            | "kivy"
            | "lua"
            | "ruby"
            | "perl"
            | "r"
            | "julia"
            | "elixir"
            | "erlang"
            | "ocaml"
            | "haskell"
            | "go"
            | "clojure"
            | "fsharp"
            | "lean"
            | "nix"
            | "zig"
    )
}

/// Unify a qualified API name for matching: every separator becomes `.`.
fn unify(s: &str) -> String {
    s.replace("::", ".")
        .replace(['/', ':', '\\'], ".")
        .replace("->", ".")
}

/// Whether an API table entry matches the call. A bare entry matches only
/// a bare call; a qualified one matches the call's written qualifier or
/// the module it is bound to.
fn api_matches(entry: &str, qual: &str, name: &str, module: Option<&str>) -> bool {
    let e = unify(entry);
    if !e.contains('.') && !entry.contains('(') {
        return qual.is_empty() && name == entry;
    }
    let candidates = [
        unify(&format!("{qual}.{name}")),
        module
            .map(|m| unify(&format!("{m}.{name}")))
            .unwrap_or_default(),
    ];
    candidates.iter().filter(|c| !c.is_empty()).any(|c| {
        c == &e || c.ends_with(&format!(".{e}")) || c.ends_with(&e) && e.contains('(')
    })
}

/// The program a shell command line runs: its first word, path stripped.
fn program_of(cmd: &str) -> String {
    let w = cmd
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(['"', '\'', '`']);
    if w.is_empty() || w.starts_with(['$', '{', '(']) || w.contains("${") {
        return "<dynamic>".to_string();
    }
    w.rsplit(['/', '\\']).next().unwrap_or(w).to_string()
}

/// One attributed site before aggregation.
struct Attributed {
    kind: ExternalKind,
    language: String,
    library: String,
    module: String,
    symbol: String,
    evidence: ExternalEvidence,
    confidence: Confidence,
}

impl Pass {
    fn attribute(
        &self,
        ctx: &FileCtx,
        c: &AuditUnresolvedCall,
        bucket_stdlib: bool,
    ) -> Attributed {
        let lang = ctx.lang;
        let r = ctx.refs.get(&(c.site_byte, c.name.as_str())).copied();
        let (qual, name): (String, String) = if !c.receiver_hint.is_empty() {
            (c.receiver_hint.clone(), c.name.clone())
        } else if matches!(lang, "r" | "julia") && !c.name.contains("::") {
            (String::new(), c.name.clone())
        } else {
            let (q, n, _) = split_qualified_name(&c.name);
            (q.trim_end_matches(':').to_string(), n.to_string())
        };
        let sep = separator(lang);
        let mk = |kind,
                  library: String,
                  module: String,
                  symbol: String,
                  evidence: ExternalEvidence| {
            let mut confidence = evidence.confidence();
            if evidence == ExternalEvidence::TypedReceiver
                && r.is_some_and(|r| r.weak_receiver_type)
            {
                confidence = Confidence::Low;
            }
            Attributed {
                kind,
                language: lang.to_string(),
                library,
                module,
                symbol,
                evidence,
                confidence,
            }
        };
        let kind_of = |class: Class| match class {
            Class::Stdlib => ExternalKind::Stdlib,
            Class::System => ExternalKind::System,
            _ => ExternalKind::ThirdParty,
        };
        // Shell commands: the program is the command.
        if matches!(lang, "bash" | "powershell") && qual.is_empty() {
            let n = name.as_str();
            let builtin = lang == "bash" && SHELL_BUILTINS.contains(&n);
            let cmdlet = lang == "powershell"
                && n.contains('-')
                && n.chars().next().is_some_and(char::is_uppercase);
            if builtin || cmdlet {
                return mk(
                    ExternalKind::Stdlib,
                    stdlib_library_name(lang).into(),
                    String::new(),
                    n.into(),
                    ExternalEvidence::StdlibScreen,
                );
            }
            // A capitalized PowerShell command that is not Verb-Noun is a
            // module's function (Pester `Should`, `It`), not a program.
            if lang == "powershell"
                && n.starts_with(char::is_uppercase)
                && !n.contains('.')
            {
                return mk(
                    ExternalKind::Unattributed,
                    String::new(),
                    String::new(),
                    n.into(),
                    ExternalEvidence::None,
                );
            }
            let prog = program_of(n);
            let ev = if prog == "<dynamic>" {
                ExternalEvidence::None
            } else {
                ExternalEvidence::LiteralArgument
            };
            return mk(ExternalKind::Process, prog.clone(), prog, String::new(), ev);
        }
        // Resolve the library the call goes through.
        let head = leading_ident(&qual).trim_start_matches('$').to_string();
        let chain = qual.contains('(') || qual.contains('[');
        let mut found: Option<(ExternalKind, String, String, ExternalEvidence)> = None;
        if let UnresolvedReason::ExternalModule { module } = &c.reason {
            let class = self.classify(lang, "import", module);
            let class = if class == Class::Internal {
                Class::External
            } else {
                class
            };
            found = Some((
                kind_of(class),
                library_of(lang, module, class),
                module.clone(),
                ExternalEvidence::ResolverRefusal,
            ));
        }
        if found.is_none() && !qual.is_empty() && !head.is_empty() {
            if let Some(b) = ctx.binds.get(&head)
                && b.class != Class::Internal
            {
                // A chained call is the dependency's only when the head is
                // the module itself or a type (`requests.get(u).json()`,
                // `Command::new(…).arg(…)`), never a symbol taken out of
                // one: `verify(mock).run()` returns a workspace value.
                let type_head = head.starts_with(char::is_uppercase);
                if chain && !b.module_binding && !type_head {
                    // fall through to the rungs below
                } else {
                    let ev = if chain {
                        ExternalEvidence::ChainHead
                    } else if r.is_some_and(|r| {
                        r.typed_receiver || r.weak_receiver_type || r.retyped_receiver
                    }) {
                        ExternalEvidence::TypedReceiver
                    } else {
                        ExternalEvidence::QualifiedImport
                    };
                    found = Some((
                        kind_of(b.class),
                        library_of(lang, &b.module, b.class),
                        b.module.clone(),
                        ev,
                    ));
                }
            } else if !chain && !ctx.binds.contains_key(&head) {
                // A path written whole, through no import: `std::fs::read`,
                // `serde_json::from_str`, `os.path.join` (Python binds it).
                let plain_path = qual
                    .chars()
                    .all(|ch| ch.is_alphanumeric() || matches!(ch, '_' | ':' | '.'));
                let own = matches!(
                    head.as_str(),
                    "Self" | "self" | "super" | "crate" | "this" | "base"
                ) || self.tree.knows(lang, &head);
                // Rust: a crate path starts lowercase; an unimported
                // capitalized head is a type of this module or the
                // prelude, never a dependency.
                let crate_shaped =
                    lang != "rust" || head.starts_with(|ch: char| ch.is_lowercase());
                // Languages whose qualified call names its module outright,
                // with no import needed: Erlang `m:f`, Elixir `Mod.f`, OCaml
                // `M.f`, Perl `Pkg::f`, R `pkg::f`.
                // Elixir/OCaml modules are capitalized (a lowercase head is
                // a variable: `conn.assigns`); a Perl package is written
                // with `::`, and `main`/`SUPER`/`shift` are not packages.
                let capital = head.starts_with(char::is_uppercase);
                let names_module = match lang {
                    "erlang" | "r" => true,
                    "elixir" | "ocaml" => capital,
                    "perl" => {
                        qual.contains("::")
                            && !matches!(
                                head.as_str(),
                                "main" | "SUPER" | "CORE" | "__PACKAGE__"
                            )
                    }
                    _ => false,
                };
                let path_written = plain_path
                    && !own
                    && crate_shaped
                    && (names_module
                        || r.is_some_and(|r| r.path_receiver)
                        || qual.contains("::"));
                if path_written {
                    let class = self.classify(lang, "import", &qual);
                    if class != Class::Internal {
                        found = Some((
                            kind_of(class),
                            library_of(lang, &qual, class),
                            qual.clone(),
                            ExternalEvidence::QualifiedImport,
                        ));
                    }
                } else if self.is_stdlib(lang, &head)
                    && bucket_stdlib
                    && module_headed(lang)
                {
                    found = Some((
                        ExternalKind::Stdlib,
                        library_of(lang, &head, Class::Stdlib),
                        head.clone(),
                        ExternalEvidence::StdlibScreen,
                    ));
                } else if !own
                    && head.starts_with(char::is_uppercase)
                    && !bucket_stdlib
                    && ctx.wildcards.len() == 1
                    // Lean's prelude namespaces (`Option`, `List`) are
                    // open everywhere; an import does not supply them.
                    && lang != "lean"
                {
                    // `JsonConvert.Serialize()` after `using Newtonsoft.Json;`
                    // as the file's only external wildcard import.
                    let m = ctx.wildcards.iter().next().cloned().unwrap_or_default();
                    let class = self.classify(lang, "import", &m);
                    found = Some((
                        kind_of(class),
                        library_of(lang, &m, class),
                        m,
                        ExternalEvidence::SoleWildcard,
                    ));
                } else if !own
                    && head.starts_with(char::is_uppercase)
                    && !bucket_stdlib
                    && matches!(lang, "objc" | "swift")
                    && ctx.c_headers.len() + ctx.wildcards.len() == 1
                {
                    let m = ctx
                        .c_headers
                        .first()
                        .or(ctx.wildcards.iter().next())
                        .cloned()
                        .unwrap_or_default();
                    let class = self.classify(lang, "import", &m);
                    found = Some((
                        kind_of(class),
                        library_of(lang, &m, class),
                        m,
                        ExternalEvidence::SoleWildcard,
                    ));
                }
            }
        }
        // Assembly calls a C library function by its symbol.
        if found.is_none() && qual.is_empty() && lang == "asm" {
            let n = name.trim_start_matches('_');
            if stdlib::stdlib_names("c").is_some_and(|s| s.contains(n)) {
                found = Some((
                    ExternalKind::Stdlib,
                    "libc".into(),
                    "libc".into(),
                    ExternalEvidence::StdlibScreen,
                ));
            } else if ctx.binds.contains_key(name.as_str()) {
                found = Some((
                    ExternalKind::ThirdParty,
                    "<extern>".into(),
                    "<extern>".into(),
                    ExternalEvidence::SelectiveImport,
                ));
            }
        }
        if found.is_none()
            && qual.is_empty()
            && let Some(b) = ctx.binds.get(name.as_str())
            && b.class != Class::Internal
        {
            found = Some((
                kind_of(b.class),
                library_of(lang, &b.module, b.class),
                b.module.clone(),
                ExternalEvidence::SelectiveImport,
            ));
        }
        // C family: the OS interface, then a prefix of an included header.
        if found.is_none()
            && qual.is_empty()
            && matches!(lang, "c" | "cpp" | "objc" | "asm")
        {
            if SYSTEM_APIS.contains(&name.as_str()) {
                let lib = if name.starts_with(char::is_uppercase) {
                    "win32"
                } else {
                    "posix"
                };
                found = Some((
                    ExternalKind::System,
                    lib.into(),
                    lib.into(),
                    ExternalEvidence::SystemInterface,
                ));
            } else if !bucket_stdlib {
                let lname = name.to_ascii_lowercase();
                let hits: Vec<&String> = ctx
                    .c_headers
                    .iter()
                    .filter(|h| {
                        let lib =
                            library_of(lang, h, Class::External).to_ascii_lowercase();
                        let lib = lib.trim_start_matches("lib");
                        lib.len() >= 2 && lname.starts_with(lib)
                    })
                    .collect();
                if hits.len() == 1 {
                    let h = hits[0];
                    found = Some((
                        ExternalKind::ThirdParty,
                        library_of(lang, h, Class::External),
                        h.clone(),
                        ExternalEvidence::NamePrefix,
                    ));
                } else if ctx.c_headers.len() == 1 {
                    let h = &ctx.c_headers[0];
                    found = Some((
                        ExternalKind::ThirdParty,
                        library_of(lang, h, Class::External),
                        h.clone(),
                        ExternalEvidence::SoleWildcard,
                    ));
                }
            }
        }
        // Verilog `$display`, `$finish`: system tasks are the language's own.
        if found.is_none()
            && qual.is_empty()
            && lang == "verilog"
            && name.starts_with('$')
        {
            found = Some((
                ExternalKind::Stdlib,
                "system-tasks".into(),
                "system-tasks".into(),
                ExternalEvidence::StdlibScreen,
            ));
        }
        // Verilog/VHDL: an instance of a module the tree never defines is a
        // vendor primitive or IP core.
        if found.is_none()
            && qual.is_empty()
            && !bucket_stdlib
            && matches!(lang, "verilog" | "vhdl")
        {
            found = Some((
                ExternalKind::ThirdParty,
                name.clone(),
                name.clone(),
                ExternalEvidence::UndefinedInstance,
            ));
        }
        if found.is_none() && bucket_stdlib {
            let lib = stdlib_library_name(lang).to_string();
            found = Some((
                ExternalKind::Stdlib,
                lib.clone(),
                lib,
                ExternalEvidence::StdlibScreen,
            ));
        }
        if found.is_none() && qual.is_empty() && ctx.wildcards.len() == 1 {
            let m = ctx.wildcards.iter().next().cloned().unwrap_or_default();
            let class = self.classify(lang, "import", &m);
            found = Some((
                kind_of(class),
                library_of(lang, &m, class),
                m,
                ExternalEvidence::SoleWildcard,
            ));
        }
        // cgo: `C.f()` through `import "C"` is a native call.
        if lang == "go" && head == "C" && ctx.binds.contains_key("C") {
            found = Some((
                ExternalKind::Native,
                "cgo".into(),
                "C".into(),
                ExternalEvidence::ForeignDeclaration,
            ));
        }
        // Process and native boundaries override the library's kind.
        let module = found.as_ref().map(|f| f.2.as_str());
        let uses = ctx.uses_at.get(&c.site_byte);
        // A plugin may anchor the site at the callee rather than the call
        // node; then the record on the same line, naming this callee, is
        // the one.
        let literal = |k: ExternalUseKind| {
            uses.and_then(|v| v.iter().find(|u| u.kind == k))
                .or_else(|| {
                    ctx.uses_by_line.get(&c.site_line).and_then(|v| {
                        v.iter().find(|u| {
                            u.kind == k && unify(&u.symbol).ends_with(&unify(&name))
                        })
                    })
                })
                .map(|u| u.module.clone())
        };
        if externals_table(PROCESS_SPAWN_APIS, lang)
            .iter()
            .any(|e| api_matches(e, &qual, &name, module))
        {
            let prog = literal(ExternalUseKind::ProcessSpawn)
                .map(|p| program_of(&p))
                .unwrap_or_else(|| "<dynamic>".into());
            let ev = if prog == "<dynamic>" {
                ExternalEvidence::None
            } else {
                ExternalEvidence::LiteralArgument
            };
            let via = if qual.is_empty() {
                name.clone()
            } else {
                format!("{qual}{sep}{name}")
            };
            return mk(ExternalKind::Process, prog.clone(), prog, via, ev);
        }
        if externals_table(LIBRARY_LOAD_APIS, lang)
            .iter()
            .any(|e| api_matches(e, &qual, &name, module))
        {
            let lib = literal(ExternalUseKind::LibraryLoad)
                .unwrap_or_else(|| "<dynamic>".into());
            let ev = if lib == "<dynamic>" {
                ExternalEvidence::None
            } else {
                ExternalEvidence::LiteralArgument
            };
            let via = if qual.is_empty() {
                name.clone()
            } else {
                format!("{qual}{sep}{name}")
            };
            return mk(ExternalKind::Native, lib.clone(), lib, via, ev);
        }
        match found {
            Some((kind, library, module, evidence)) => {
                let symbol =
                    symbol_for(&library, &module, &qual, &head, &name, sep, evidence);
                mk(kind, library, module, symbol, evidence)
            }
            None => mk(
                ExternalKind::Unattributed,
                String::new(),
                String::new(),
                name.clone(),
                ExternalEvidence::None,
            ),
        }
    }
}

/// The symbol a site calls, relative to its library: `post` for
/// `requests.post`, `linalg.norm` for `np.linalg.norm`, `fs::read` for
/// `std::fs::read`, `Node::kind` for a `tree_sitter::Node` receiver.
fn symbol_for(
    library: &str,
    module: &str,
    qual: &str,
    head: &str,
    name: &str,
    sep: &str,
    evidence: ExternalEvidence,
) -> String {
    let rel = |path: &str| -> String {
        let p = path.trim_start_matches("::");
        let rest = p.strip_prefix(library).unwrap_or(p);
        rest.trim_start_matches(['.', ':', '/', '\\']).to_string()
    };
    let mut parts: Vec<String> = Vec::new();
    match evidence {
        ExternalEvidence::QualifiedImport
        | ExternalEvidence::TypedReceiver
        | ExternalEvidence::ResolverRefusal
            if !qual.is_empty() =>
        {
            let m = rel(module);
            if !m.is_empty() && module != qual {
                parts.push(m);
            }
            // The qualifier past its head: `linalg` in `np.linalg`.
            let tail = qual
                .strip_prefix(head)
                .unwrap_or("")
                .trim_start_matches(['.', ':', '/', '\\']);
            if module == qual {
                let q = rel(qual);
                if !q.is_empty() {
                    parts.push(q);
                }
            } else if !tail.is_empty() {
                parts.push(tail.to_string());
            }
        }
        ExternalEvidence::ChainHead => {
            let m = rel(module);
            if !m.is_empty() {
                parts.push(format!("{m}(…)"));
            } else {
                parts.push(format!("{head}(…)"));
            }
        }
        ExternalEvidence::SelectiveImport => {
            let m = rel(module);
            if !m.is_empty() {
                parts.push(m);
            }
        }
        _ => {
            if !qual.is_empty() {
                parts.push(qual.to_string());
            }
        }
    }
    parts.push(name.to_string());
    parts.retain(|p| !p.is_empty());
    parts.join(sep)
}

/// The innermost callable of `file` containing `byte`.
fn enclosing(spans: &[(u32, u32, CallableId)], byte: u32) -> Option<CallableId> {
    spans
        .iter()
        .filter(|(s, e, _)| *s <= byte && byte < *e)
        .min_by_key(|(s, e, _)| e - s)
        .map(|(_, _, id)| *id)
}

#[derive(Default)]
struct LibAcc {
    modules: BTreeSet<String>,
    imports: Vec<ImportSite>,
    symbols: BTreeMap<String, u64>,
    callers: BTreeMap<(String, String, u32), (Option<CallableId>, u64)>,
    sites: Vec<ExternalSite>,
}

/// Build the inventory.
pub fn inventory(
    graph: &Graph,
    facts: &[FileFacts],
    records: &[AuditFileRecord],
    spec: &ExternalsSpec,
) -> ExternalsInventory {
    let pass = Pass {
        tree: TreeNames::build(facts),
        vocab: WorkspaceVocab::build(graph, facts),
    };
    let by_file: HashMap<FileId, &FileFacts> =
        facts.iter().map(|f| (f.file, f)).collect();
    let mut spans: HashMap<FileId, Vec<(u32, u32, CallableId)>> = HashMap::new();
    for c in graph.callables.values().filter(|c| !c.synthetic) {
        spans
            .entry(c.file)
            .or_default()
            .push((c.start_byte, c.end_byte, c.id));
    }
    for v in spans.values_mut() {
        v.sort();
    }

    let mut libs: BTreeMap<(ExternalKind, String, String), LibAcc> = BTreeMap::new();
    let mut unattributed: BTreeMap<(String, String), UnattributedGroup> = BTreeMap::new();
    let mut coverage: BTreeMap<String, (LanguageCoverage, BTreeSet<String>)> =
        BTreeMap::new();
    let mut totals = ExternalTotals::default();
    let mut leaves: Vec<LeafSite> = Vec::new();
    let mut accounted = 0u64;

    for rec in records {
        let Some(f) = by_file.get(&rec.file) else {
            continue;
        };
        let ctx = pass.file_ctx(f);
        let lang = f.language.clone();
        let path = rec.path.display().to_string();
        let cov = coverage.entry(lang.clone()).or_insert_with(|| {
            (
                LanguageCoverage {
                    language: lang.clone(),
                    ..Default::default()
                },
                BTreeSet::new(),
            )
        });
        cov.0.files += 1;
        cov.0.imports += ctx.imports.len() as u64;
        for i in &f.imports {
            if !is_identity_kind(&lang, &i.kind) {
                cov.1.insert(i.kind.clone());
            }
        }
        for u in &f.external_uses {
            cov.1.insert(u.form.clone());
        }
        let caller_qn = |id: Option<CallableId>| -> String {
            id.and_then(|id| graph.callables.get(&id))
                .map(|c| c.qualified_name.clone())
                .unwrap_or_else(|| format!("<toplevel>::{path}"))
        };
        // Imports into the inventory.
        for imp in &ctx.imports {
            let kind = match imp.class {
                Class::Internal => continue,
                Class::Stdlib => ExternalKind::Stdlib,
                Class::System => ExternalKind::System,
                Class::External => ExternalKind::ThirdParty,
            };
            if !spec.shows(kind) {
                continue;
            }
            let class = imp.class;
            let lib = library_of(&lang, &imp.module, class);
            let acc = libs.entry((kind, lang.clone(), lib)).or_default();
            acc.modules.insert(imp.module.clone());
            acc.imports.push(ImportSite {
                path: path.clone(),
                line: imp.line,
                module: imp.module.clone(),
                alias: imp.alias.clone(),
            });
        }
        let file_spans = spans.get(&rec.file).map(Vec::as_slice).unwrap_or(&[]);
        let mut record =
            |a: Attributed,
             caller: Option<CallableId>,
             line: u32,
             byte: u32,
             libs: &mut BTreeMap<(ExternalKind, String, String), LibAcc>,
             unattributed: &mut BTreeMap<(String, String), UnattributedGroup>,
             cov: &mut LanguageCoverage| {
                accounted += 1;
                totals.sites += 1;
                *totals.by_kind.entry(a.kind.slug().into()).or_default() += 1;
                cov.sites += 1;
                if a.kind == ExternalKind::Unattributed {
                    cov.unattributed += 1;
                } else {
                    cov.attributed += 1;
                }
                if !spec.shows(a.kind) {
                    *totals.withheld.entry(a.kind.slug().into()).or_default() += 1;
                    return;
                }
                *totals
                    .by_confidence
                    .entry(conf_slug(a.confidence).into())
                    .or_default() += 1;
                let site = ExternalSite {
                    caller,
                    caller_qualified_name: caller_qn(caller),
                    path: path.clone(),
                    line,
                    symbol: a.symbol.clone(),
                    evidence: a.evidence,
                    confidence: a.confidence,
                };
                if a.kind == ExternalKind::Unattributed {
                    let g = unattributed
                        .entry((a.language.clone(), a.symbol.clone()))
                        .or_insert_with(|| UnattributedGroup {
                            language: a.language.clone(),
                            name: a.symbol.clone(),
                            ..Default::default()
                        });
                    g.sites += 1;
                    if g.sample.len() < 5 {
                        g.sample.push(site);
                    }
                    leaves.push(LeafSite {
                        kind: a.kind,
                        library: a.language.clone(),
                        symbol: a.symbol,
                        caller,
                        file: rec.file,
                        line,
                        byte,
                        confidence: a.confidence,
                    });
                    return;
                }
                let acc = libs
                    .entry((a.kind, a.language.clone(), a.library.clone()))
                    .or_default();
                if !a.module.is_empty() {
                    acc.modules.insert(a.module.clone());
                }
                *acc.symbols.entry(a.symbol.clone()).or_default() += 1;
                let ck = (
                    site.caller_qualified_name.clone(),
                    path.clone(),
                    caller
                        .and_then(|id| graph.callables.get(&id))
                        .map_or(0, |c| c.start_line),
                );
                acc.callers.entry(ck).or_insert((caller, 0)).1 += 1;
                acc.sites.push(site);
                leaves.push(LeafSite {
                    kind: a.kind,
                    library: a.library,
                    symbol: a.symbol,
                    caller,
                    file: rec.file,
                    line,
                    byte,
                    confidence: a.confidence,
                });
            };
        let mut bucket_bytes: HashSet<(u32, &str)> = HashSet::new();
        for (calls, is_std) in [(&rec.stdlib_calls, true), (&rec.external_calls, false)] {
            for c in calls {
                bucket_bytes.insert((c.site_byte, c.name.as_str()));
                let a = pass.attribute(&ctx, c, is_std);
                let caller = c.src.or_else(|| enclosing(file_spans, c.site_byte));
                record(
                    a,
                    caller,
                    c.site_line,
                    c.site_byte,
                    &mut libs,
                    &mut unattributed,
                    &mut cov.0,
                );
            }
        }
        // Java `native` / Kotlin `external` methods: implemented outside
        // the tree, in whatever library `System.loadLibrary` loads.
        if matches!(lang.as_str(), "java" | "kotlin" | "scala") {
            for d in &f.definitions {
                if d.attributes
                    .iter()
                    .any(|a| a == "native" || a == "external")
                {
                    let caller = enclosing(file_spans, d.start_byte);
                    let a = Attributed {
                        kind: ExternalKind::Native,
                        language: lang.clone(),
                        library: "<jni>".into(),
                        module: "<jni>".into(),
                        symbol: d.simple_name.clone(),
                        evidence: ExternalEvidence::ForeignDeclaration,
                        confidence: Confidence::High,
                    };
                    record(
                        a,
                        caller,
                        d.start_line,
                        d.start_byte,
                        &mut libs,
                        &mut unattributed,
                        &mut cov.0,
                    );
                }
            }
        }
        // Side-channel sites the call graph does not carry.
        for u in &f.external_uses {
            let (kind, evidence) = match u.kind {
                ExternalUseKind::ForeignDecl => {
                    (ExternalKind::Native, ExternalEvidence::ForeignDeclaration)
                }
                ExternalUseKind::Syscall => {
                    (ExternalKind::System, ExternalEvidence::SystemInterface)
                }
                ExternalUseKind::Provider => {
                    let class = pass.classify(&lang, "provider", &u.module);
                    if class == Class::Internal {
                        continue;
                    }
                    (
                        if class == Class::Stdlib {
                            ExternalKind::Stdlib
                        } else {
                            ExternalKind::ThirdParty
                        },
                        ExternalEvidence::QualifiedImport,
                    )
                }
                ExternalUseKind::QualifiedCall => {
                    if bucket_bytes.contains(&(u.site_byte, u.symbol.as_str())) {
                        continue;
                    }
                    let c = AuditUnresolvedCall::new(
                        None,
                        rec.file,
                        u.site_line,
                        u.site_byte,
                        u.symbol.clone(),
                        u.module.clone(),
                        UnresolvedReason::NoCandidateInFile,
                    );
                    // A qualified call through an in-tree module bound
                    // nothing only if the resolver never saw it; skip
                    // internal ones.
                    let head = leading_ident(&u.module).to_string();
                    if ctx
                        .binds
                        .get(&head)
                        .is_some_and(|b| b.class == Class::Internal)
                        || pass.tree.knows(&lang, &head) && !pass.is_stdlib(&lang, &head)
                    {
                        continue;
                    }
                    let a = pass.attribute(&ctx, &c, pass.is_stdlib(&lang, &u.module));
                    let caller = enclosing(file_spans, u.site_byte);
                    record(
                        a,
                        caller,
                        u.site_line,
                        u.site_byte,
                        &mut libs,
                        &mut unattributed,
                        &mut cov.0,
                    );
                    continue;
                }
                ExternalUseKind::Import
                | ExternalUseKind::ProcessSpawn
                | ExternalUseKind::LibraryLoad
                | ExternalUseKind::Declares => continue,
            };
            let library = if u.module.is_empty() {
                "<dynamic>".to_string()
            } else {
                u.module.clone()
            };
            let a = Attributed {
                kind,
                language: lang.clone(),
                library: if kind == ExternalKind::ThirdParty
                    || kind == ExternalKind::Stdlib
                {
                    library_of(&lang, &library, Class::External)
                } else {
                    library.clone()
                },
                module: library,
                symbol: if u.symbol.is_empty() {
                    u.form.clone()
                } else {
                    u.symbol.clone()
                },
                evidence,
                confidence: evidence.confidence(),
            };
            let caller = enclosing(file_spans, u.site_byte);
            record(
                a,
                caller,
                u.site_line,
                u.site_byte,
                &mut libs,
                &mut unattributed,
                &mut cov.0,
            );
        }
    }

    // Assemble, most-used first within each kind.
    let mut libraries: Vec<Library> = libs
        .into_iter()
        .map(|((kind, language, name), acc)| {
            let mut symbols: Vec<SymbolUse> = acc
                .symbols
                .into_iter()
                .map(|(symbol, sites)| SymbolUse { symbol, sites })
                .collect();
            symbols.sort_by(|a, b| {
                b.sites.cmp(&a.sites).then_with(|| a.symbol.cmp(&b.symbol))
            });
            let mut callers: Vec<CallerUse> = acc
                .callers
                .into_iter()
                .map(|((qn, path, line), (callable, sites))| CallerUse {
                    callable,
                    qualified_name: qn,
                    path,
                    line,
                    sites,
                })
                .collect();
            callers.sort_by(|a, b| {
                b.sites
                    .cmp(&a.sites)
                    .then_with(|| a.qualified_name.cmp(&b.qualified_name))
            });
            Library {
                id: String::new(),
                kind,
                language,
                name,
                modules: acc.modules.into_iter().collect(),
                imports: acc.imports,
                symbols,
                callers,
                sites: acc.sites,
                branches: Vec::new(),
            }
        })
        .collect();
    libraries.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then_with(|| b.sites.len().cmp(&a.sites.len()))
            .then_with(|| b.imports.len().cmp(&a.imports.len()))
            .then_with(|| a.language.cmp(&b.language))
            .then_with(|| a.name.cmp(&b.name))
    });
    for (i, l) in libraries.iter_mut().enumerate() {
        l.id = format!("L{i}");
        *totals
            .libraries_by_kind
            .entry(l.kind.slug().into())
            .or_default() += 1;
    }
    let mut unattributed: Vec<UnattributedGroup> = unattributed.into_values().collect();
    unattributed.sort_by(|a, b| {
        b.sites
            .cmp(&a.sites)
            .then_with(|| (&a.language, &a.name).cmp(&(&b.language, &b.name)))
    });

    let coverage = coverage
        .into_values()
        .map(|(mut c, forms)| {
            c.forms = forms.into_iter().collect();
            c
        })
        .collect();

    ExternalsInventory {
        report: ExternalsReport {
            schema: EXTERNALS_SCHEMA.to_string(),
            cgg_version: cgg_core::CGG_VERSION.to_string(),
            disclaimer: EXTERNALS_DISCLAIMER.to_string(),
            kinds: spec.kinds().to_vec(),
            totals,
            coverage,
            libraries,
            unattributed,
        },
        leaves,
        accounted,
    }
}

fn kind_rank(k: ExternalKind) -> usize {
    ExternalKind::ALL
        .iter()
        .position(|x| *x == k)
        .unwrap_or(usize::MAX)
}

fn conf_slug(c: Confidence) -> &'static str {
    match c {
        Confidence::High => "high",
        Confidence::Medium => "medium",
        Confidence::Low => "low",
    }
}

/// Fill each library's example branches: up to three chains from a root
/// (a callable nothing calls, or a framework entry) down to its busiest
/// boundary callers. One multi-source BFS from every root, parent
/// pointers kept, so the whole fill is O(V + E).
pub fn fill_branches(report: &mut ExternalsReport, graph: &Graph) {
    use std::collections::VecDeque;
    let mut callers_of: HashMap<CallableId, Vec<CallableId>> = HashMap::new();
    let mut out: HashMap<CallableId, Vec<CallableId>> = HashMap::new();
    let mut has_in: HashSet<CallableId> = HashSet::new();
    for e in &graph.edges {
        if e.src == e.dst {
            continue;
        }
        if graph
            .callables
            .get(&e.dst)
            .is_some_and(|c| c.synthetic && c.framework_entry.is_none())
        {
            continue;
        }
        out.entry(e.src).or_default().push(e.dst);
        callers_of.entry(e.dst).or_default().push(e.src);
        has_in.insert(e.dst);
    }
    // Roots in graph order, so the result is stable.
    let mut parent: HashMap<CallableId, Option<CallableId>> = HashMap::new();
    let mut queue: VecDeque<CallableId> = VecDeque::new();
    for (id, c) in &graph.callables {
        if (!c.synthetic || c.framework_entry.is_some()) && !has_in.contains(id) {
            parent.insert(*id, None);
            queue.push_back(*id);
        }
    }
    while let Some(n) = queue.pop_front() {
        if let Some(next) = out.get(&n) {
            for m in next {
                if !parent.contains_key(m) {
                    parent.insert(*m, Some(n));
                    queue.push_back(*m);
                }
            }
        }
    }
    let qn = |id: CallableId| {
        graph
            .callables
            .get(&id)
            .map_or_else(String::new, |c| c.qualified_name.clone())
    };
    for lib in &mut report.libraries {
        let mut branches = Vec::new();
        for caller in lib.callers.iter().take(3) {
            let Some(id) = caller.callable else {
                continue;
            };
            let mut chain = vec![];
            let mut cur = Some(id);
            let mut guard = 0;
            while let Some(c) = cur {
                chain.push(qn(c));
                cur = parent.get(&c).copied().flatten();
                guard += 1;
                if guard > 64 {
                    break;
                }
            }
            chain.reverse();
            let sym = lib
                .sites
                .iter()
                .find(|s| s.caller == Some(id))
                .map_or_else(String::new, |s| s.symbol.clone());
            chain.push(format!("{}::{}", lib.name, sym));
            branches.push(chain);
        }
        lib.branches = branches;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn libraries_normalize_per_language() {
        assert_eq!(
            library_of("javascript", "@scope/pkg/sub", Class::External),
            "@scope/pkg"
        );
        assert_eq!(
            library_of("javascript", "lodash/fp", Class::External),
            "lodash"
        );
        assert_eq!(
            library_of("javascript", "node:fs/promises", Class::Stdlib),
            "node:fs"
        );
        assert_eq!(
            library_of("go", "github.com/a/b/c/d", Class::External),
            "github.com/a/b"
        );
        assert_eq!(library_of("go", "net/http", Class::Stdlib), "net/http");
        assert_eq!(
            library_of("go", "github.com/gdamore/tcell/v2", Class::External),
            "github.com/gdamore/tcell/v2"
        );
        assert_eq!(
            library_of(
                "java",
                "com.google.common.collect.ImmutableMap",
                Class::External
            ),
            "com.google.common"
        );
        assert_eq!(
            library_of("rust", "tree_sitter::Node", Class::External),
            "tree_sitter"
        );
        assert_eq!(library_of("python", "os.path", Class::Stdlib), "os");
        assert_eq!(library_of("c", "curl/curl.h", Class::External), "curl");
        assert_eq!(library_of("c", "zlib.h", Class::External), "zlib");
        assert_eq!(
            library_of("dart", "package:http/http.dart", Class::External),
            "http"
        );
        assert_eq!(
            library_of("php", "GuzzleHttp\\Client", Class::External),
            "GuzzleHttp\\Client"
        );
        assert_eq!(
            library_of("starlark", "@rules_cc//cc:defs.bzl", Class::External),
            "@rules_cc"
        );
        assert_eq!(
            library_of(
                "solidity",
                "@openzeppelin/contracts/token/ERC20.sol",
                Class::External
            ),
            "@openzeppelin/contracts"
        );
    }

    #[test]
    fn api_tables_match_written_and_bound_forms() {
        assert!(api_matches("subprocess.run", "subprocess", "run", None));
        assert!(api_matches("subprocess.run", "", "run", Some("subprocess")));
        assert!(!api_matches("system", "obj", "system", None));
        assert!(api_matches("system", "", "system", None));
        assert!(api_matches(
            "Command::new",
            "Command",
            "new",
            Some("std::process::Command")
        ));
        assert!(api_matches(
            "getRuntime().exec",
            "Runtime.getRuntime()",
            "exec",
            None
        ));
        assert_eq!(program_of("/usr/bin/git status"), "git");
        assert_eq!(program_of("$CURL -s"), "<dynamic>");
    }

    #[test]
    fn leading_ident_reads_heads() {
        assert_eq!(leading_ident("os.path"), "os");
        assert_eq!(leading_ident("Command::new(\"git\").arg(x)"), "Command");
        assert_eq!(leading_ident("::std::fs"), "std");
    }
}
