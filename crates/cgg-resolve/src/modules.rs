//! Module-scoped resolution (FINDINGS A4, A5).
//!
//! A call written through an import of a *workspace* module names that
//! module: `svc.spawnJob()` after `import * as svc from './svc'`,
//! `legacy.run()` after `const legacy = require('./legacy')`,
//! `helpers.f()` after `from . import helpers`, `pkg.sub.jobs.Runner().m()`
//! after `import pkg`, `jobs.NewRunner().M()` through a Go package. The
//! cross-file resolver used to look such a call up by its bare name —
//! JavaScript qualified names carry no module, so the lookup returned
//! whichever same-named function was indexed last — or let it fall
//! through to duck-typed fan-out, which added every same-named method in
//! the tree beside the right one.
//!
//! Here the import is followed to the file it names and the call is
//! looked up *in that file*, through the module's own re-exports (`export
//! * from`, `export { a as b } from`, `module.exports = { … }`, a package
//! `__init__.py`'s `from .x import y`). When the call goes through a
//! module and the module's file cannot be determined, or provides nothing
//! the call can name, the call is refused and recorded
//! ([`Scoped::Refused`]) — never bound by name somewhere else.
//!
//! Every lookup is keyed by file, so no answer depends on the order
//! callables were indexed in, and nothing here is built from a list of
//! names: the inputs are import records, file paths, package manifests
//! and qualified names.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use cgg_core::{
    FileFacts, RefRecord,
    graph::Graph,
    ids::{CallableId, FileId},
};

use crate::heads::WorkspaceVocab;

/// Re-export chains longer than this are not followed (a cycle is cut by
/// the visited set first).
const MAX_DEPTH: u8 = 8;

/// The local name the JS/TS plugin gives an export it cannot name.
const UNKNOWN_EXPORT: &str = "<value>";

/// File extensions a JavaScript / TypeScript specifier may omit, in the
/// order a specifier is probed with them.
const JS_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

/// What module-scoped resolution decided for one call.
#[derive(Debug, PartialEq)]
pub(crate) enum Scoped {
    /// The call names these callables inside the module it is written
    /// through. `certain` when the lookup was exact (a qualified name in
    /// the module's file), rather than a member found by name inside it.
    Bound {
        cids: Vec<CallableId>,
        certain: bool,
    },
    /// The call is through a workspace module that provides nothing it
    /// can name (or whose file cannot be determined). Carries the import
    /// as written.
    Refused(String),
    /// `new X()` / `X()` for a class the module declares without an
    /// explicit constructor.
    NoCtor,
    /// Several copies of one definition (one name declared in several
    /// files) and nothing the caller can see singles one out: nothing
    /// binds, and the site keeps its ordinary unresolved record.
    Ambiguous,
    /// The call is on a value whose type comes from a dependency (a Go
    /// package function declared to return `ext.T`): the external bucket,
    /// with the import path.
    External(String),
}

/// The files a JS/TS specifier names, best first, and whether that is
/// a single file the specifier names exactly (a relative path, a
/// workspace package's own directory) rather than every file a path
/// alias's suffix matches.
#[derive(Debug, Clone, Default, PartialEq)]
struct JsTargets {
    files: Vec<(FileId, (u8, usize))>,
    exact: bool,
}

/// What one lookup across a [`JsTargets`] found.
enum Pick {
    /// One provider (or several that agree), and whether exact.
    One(Vec<CallableId>, bool),
    /// Several matching modules provide different callables and nothing
    /// ranks one first.
    Ambiguous,
    /// No target provides the name.
    Miss,
}

/// A JS/TS module's exports that are not plain top-level definitions.
#[derive(Debug, Default)]
struct JsExports {
    /// `export * from 'spec'`, `module.exports = require('spec')`: the
    /// files `spec` names, best first (resolved once, when the index is
    /// built).
    star: Vec<Vec<FileId>>,
    /// `export * as ns from 'spec'`: `(ns, files)`.
    namespace: Vec<(String, Vec<FileId>)>,
    /// `export { orig as exported } from 'spec'`: `(exported, orig, files)`.
    named: Vec<(String, String, Vec<FileId>)>,
    /// `export { local as exported }`, `export default local`,
    /// `module.exports = { local }`: `(exported, local)`.
    local: Vec<(String, String)>,
}

/// One segment of a receiver path: an identifier, and whether the
/// segment is called (`Runner()` in `pkg.Runner().m()`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Seg<'r> {
    ident: &'r str,
    called: bool,
}

/// Whole-tree index, built once per run.
#[derive(Debug, Default)]
pub(crate) struct ModuleIndex<'a> {
    /// `(file, qualified name)` → callables, in graph order.
    by_file_qn: HashMap<(FileId, &'a str), Vec<CallableId>>,
    /// `(file, simple name)` → callables, in graph order.
    by_file_simple: HashMap<(FileId, &'a str), Vec<CallableId>>,
    /// `(file, owner)` for every owner a JS/TS member names (`Runner` of
    /// `Runner.start`): the classes a module declares.
    js_owners: HashSet<(FileId, &'a str)>,
    /// Callables strictly inside another callable of the same file — not
    /// something a module can export.
    nested: HashSet<CallableId>,
    qn_of: HashMap<CallableId, &'a str>,
    /// Go callables' signatures, for a constructor's result type.
    go_sig: HashMap<CallableId, &'a str>,
    paths: HashMap<FileId, &'a Path>,
    facts: HashMap<FileId, &'a FileFacts>,
    /// JS/TS: lexically normalized path → file.
    js_by_path: HashMap<PathBuf, FileId>,
    /// JS/TS: module key (path without extension; a directory for its
    /// `index` file) by its last segment.
    js_by_last: HashMap<String, Vec<(Vec<String>, FileId)>>,
    js_exports: HashMap<FileId, JsExports>,
    /// JS/TS: a workspace package's name (`package.json` / `deno.json`)
    /// → the directory its files share, where its manifest sits.
    js_pkg_roots: HashMap<String, PathBuf>,
    /// JS/TS: the package each file belongs to (its `package-name`).
    js_pkg_of: HashMap<FileId, &'a str>,
    /// JS/TS: the specifiers packages' `exports` maps give their files
    /// (`@std/assert/instance-of` → `assert/instance_of.ts`).
    js_exported: HashMap<&'a str, Vec<FileId>>,
    /// JS/TS: path-alias prefixes (tsconfig `paths`, import maps).
    js_alias_prefixes: Vec<String>,
    /// JS/TS: package names and exact aliases the tree declares.
    js_known_packages: HashSet<String>,
    /// Go: every `go.mod` module path the tree declares.
    go_modules: Vec<&'a str>,
    /// Python: file → (dotted module name, is a package `__init__.py`).
    py_module_of: HashMap<FileId, (&'a str, bool)>,
    /// Python: dotted module name → files, in walk order.
    py_by_module: HashMap<&'a str, Vec<FileId>>,
    /// Python: each file's `from M import x [as y]` with `M` made
    /// absolute: `(local, module, orig)`, `orig` `*` for a star import.
    py_from: HashMap<FileId, Vec<(String, String, String)>>,
    /// Python: qualified names of every class the tree declares, and
    /// whether it declares bases (other than `object`) — a method it lacks
    /// may then be inherited from outside the tree.
    py_classes: HashMap<&'a str, bool>,
    /// Go: package import path → files (from `go.mod`).
    go_by_path: HashMap<&'a str, Vec<FileId>>,
    /// Go: directory path by its last segment, for a tree without
    /// `go.mod`.
    go_by_last_dir: HashMap<String, Vec<(Vec<String>, FileId)>>,
    /// Go: file → package clause name (the qualified-name prefix).
    go_pkg: HashMap<FileId, &'a str>,
    /// Go: directory → its files, for the calling package.
    go_dir_files: HashMap<PathBuf, Vec<FileId>>,
    /// Go: each file's import bindings, `(name, import path, external)`.
    go_imports: HashMap<FileId, Vec<(String, String, bool)>>,
    /// The file each indexed callable is declared in.
    file_of: HashMap<CallableId, FileId>,
}

fn indexed(lang: &str) -> bool {
    matches!(lang, "javascript" | "typescript" | "python" | "go")
}

impl<'a> ModuleIndex<'a> {
    pub(crate) fn build(
        graph: &'a Graph,
        facts: &'a [FileFacts],
        vocab: &WorkspaceVocab,
    ) -> Self {
        let mut ix = ModuleIndex {
            js_alias_prefixes: vocab.js_alias_prefixes.clone(),
            js_known_packages: vocab.npm_packages.clone(),
            ..ModuleIndex::default()
        };
        let mut per_file: HashMap<FileId, Vec<(u32, u32, CallableId)>> = HashMap::new();
        for c in graph.callables.values() {
            if !indexed(&c.language) {
                continue;
            }
            let qn = c.qualified_name.as_str();
            ix.by_file_qn.entry((c.file, qn)).or_default().push(c.id);
            ix.by_file_simple
                .entry((c.file, c.simple_name.as_str()))
                .or_default()
                .push(c.id);
            ix.qn_of.insert(c.id, qn);
            ix.file_of.insert(c.id, c.file);
            if c.language == "go" {
                ix.go_sig.insert(c.id, c.signature_hint.as_str());
            }
            if matches!(c.language.as_str(), "javascript" | "typescript")
                && let Some((owner, _)) = qn.rsplit_once('.')
            {
                ix.js_owners.insert((c.file, owner));
            }
            per_file
                .entry(c.file)
                .or_default()
                .push((c.start_byte, c.end_byte, c.id));
        }
        for spans in per_file.values_mut() {
            // Outermost first at equal starts. A span strictly inside the
            // innermost open one is nested; an identical span (an
            // overload, a re-declaration) is not.
            spans.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
            let mut open: Vec<(u32, u32)> = Vec::new();
            for &(s, e, id) in spans.iter() {
                while open.last().is_some_and(|&(_, end)| end <= s) {
                    open.pop();
                }
                if open
                    .last()
                    .is_some_and(|&(os, oe)| os <= s && e <= oe && (os, oe) != (s, e))
                {
                    ix.nested.insert(id);
                }
                open.push((s, e));
            }
        }
        for f in facts {
            ix.paths.insert(f.file, f.path.as_path());
            ix.facts.insert(f.file, f);
            match f.language.as_str() {
                "javascript" | "typescript" => ix.add_js_file(f),
                "python" => {
                    if let Some(m) = f.packages.first() {
                        let is_init =
                            f.path.file_name().is_some_and(|n| n == "__init__.py");
                        ix.py_module_of.insert(f.file, (m.as_str(), is_init));
                        ix.py_by_module.entry(m.as_str()).or_default().push(f.file);
                    }
                    for c in &f.classes {
                        let based = c.base_types.iter().any(|b| b.trim() != "object");
                        ix.py_classes.insert(c.class_qn.as_str(), based);
                    }
                }
                "go" => {
                    for imp in &f.imports {
                        match imp.kind.as_str() {
                            "package-path" => ix
                                .go_by_path
                                .entry(imp.alias.as_str())
                                .or_default()
                                .push(f.file),
                            "package-root" => {
                                ix.go_pkg.insert(f.file, imp.path.as_str());
                            }
                            _ => {}
                        }
                    }
                    if let Some(dir) = f.path.parent() {
                        let segs = normal_components(&normalize(dir));
                        if let Some(last) = segs.last() {
                            ix.go_by_last_dir
                                .entry(last.clone())
                                .or_default()
                                .push((segs.clone(), f.file));
                        }
                        ix.go_dir_files
                            .entry(normalize(dir))
                            .or_default()
                            .push(f.file);
                    }
                    for m in &f.packages {
                        if !ix.go_modules.contains(&m.as_str()) {
                            ix.go_modules.push(m.as_str());
                        }
                    }
                    let has_module = !f.packages.is_empty();
                    let mut binds = Vec::new();
                    for imp in f.imports.iter().filter(|i| i.kind == "import") {
                        let path = imp.path.trim();
                        if path.is_empty() {
                            continue;
                        }
                        let external = !vocab.go_workspace(path, has_module);
                        for name in go_import_names(path, imp.alias.trim()) {
                            binds.push((name, path.to_string(), external));
                        }
                    }
                    ix.go_imports.insert(f.file, binds);
                }
                _ => {}
            }
        }
        // A package's root: the deepest directory every file carrying its
        // name shares (each file lists the manifests above it).
        // Only the package a file belongs to counts — not every name it
        // may import through (a tsconfig `paths` key `"@calcom/lib"` in
        // another package would stretch the root over both).
        let mut roots: HashMap<String, Vec<Component<'a>>> = HashMap::new();
        for f in facts
            .iter()
            .filter(|f| matches!(f.language.as_str(), "javascript" | "typescript"))
        {
            let Some(dir) = f.path.parent() else { continue };
            let comps: Vec<Component<'a>> = dir.components().collect();
            for imp in f.imports.iter().filter(|i| i.kind == "package-export") {
                ix.js_exported
                    .entry(imp.alias.as_str())
                    .or_default()
                    .push(f.file);
            }
            for imp in f.imports.iter().filter(|i| i.kind == "package-name") {
                let name = imp.alias.as_str();
                ix.js_pkg_of.insert(f.file, name);
                roots
                    .entry(name.to_string())
                    .and_modify(|r| {
                        let n = r.iter().zip(&comps).take_while(|(a, b)| a == b).count();
                        r.truncate(n);
                    })
                    .or_insert_with(|| comps.clone());
            }
        }
        ix.js_pkg_roots = roots
            .into_iter()
            .map(|(name, comps)| (name, normalize(&comps.iter().collect::<PathBuf>())))
            .collect();
        // Export tables last: their specifiers resolve against every file.
        for f in facts
            .iter()
            .filter(|f| matches!(f.language.as_str(), "javascript" | "typescript"))
        {
            let ex = ix.js_export_table(f);
            if !(ex.star.is_empty()
                && ex.namespace.is_empty()
                && ex.named.is_empty()
                && ex.local.is_empty())
            {
                ix.js_exports.insert(f.file, ex);
            }
        }
        // Absolute `from` modules need every file's own module name.
        for f in facts.iter().filter(|f| f.language == "python") {
            let mut out = Vec::new();
            for imp in f.imports.iter().filter(|i| i.kind == "from-import") {
                let Some(module) = ix.py_abs(f.file, imp.path.trim()) else {
                    continue;
                };
                for item in imp.alias.split(',') {
                    let item = item.trim().trim_matches(|c| c == '(' || c == ')');
                    let (orig, local) = match item.split_once(" as ") {
                        Some((o, l)) => (o.trim(), l.trim()),
                        None => (item, item),
                    };
                    if !orig.is_empty() {
                        out.push((local.to_string(), module.clone(), orig.to_string()));
                    }
                }
            }
            if !out.is_empty() {
                ix.py_from.insert(f.file, out);
            }
        }
        ix
    }

    fn add_js_file(&mut self, f: &'a FileFacts) {
        let p = normalize(&f.path);
        self.js_by_path.entry(p.clone()).or_insert(f.file);
        let mut segs = normal_components(&p);
        if let Some(last) = segs.pop() {
            let stem = strip_js_ext(&last).to_string();
            if stem == "index" && !segs.is_empty() {
                self.push_js_key(segs.clone(), f.file);
            }
            segs.push(stem);
            self.push_js_key(segs, f.file);
        }
    }

    fn js_export_table(&self, f: &FileFacts) -> JsExports {
        let mut ex = JsExports::default();
        for imp in &f.imports {
            match imp.kind.as_str() {
                "re-export" => {
                    let files: Vec<FileId> = self
                        .js_resolve(f.file, imp.path.trim())
                        .files
                        .into_iter()
                        .map(|(f, _)| f)
                        .collect();
                    for item in imp.alias.split(',') {
                        let item = item.trim();
                        if item == "*" {
                            ex.star.push(files.clone());
                        } else if let Some(ns) = item.strip_prefix("* as ") {
                            ex.namespace.push((ns.trim().to_string(), files.clone()));
                        } else if !item.is_empty() {
                            let (orig, exported) = split_as(item);
                            ex.named.push((
                                exported.to_string(),
                                orig.to_string(),
                                files.clone(),
                            ));
                        }
                    }
                }
                "local-export" => {
                    for item in imp.alias.split(',') {
                        let item = item.trim();
                        if !item.is_empty() {
                            let (local, exported) = split_as(item);
                            ex.local.push((exported.to_string(), local.to_string()));
                        }
                    }
                }
                _ => {}
            }
        }
        ex
    }

    fn push_js_key(&mut self, key: Vec<String>, file: FileId) {
        if let Some(last) = key.last() {
            self.js_by_last
                .entry(last.clone())
                .or_default()
                .push((key.clone(), file));
        }
    }

    // ---------------------------------------------------------------
    // JavaScript / TypeScript
    // ---------------------------------------------------------------

    /// The files a specifier written in `from` names. Relative: the one
    /// file Node/TypeScript resolution finds (extension and `index`
    /// probing, `.js` written for a `.ts` source); a workspace package by
    /// name: its own directory. Both exact. Otherwise (a path alias, a
    /// Deno root-relative `/x.ts`): every file whose module path ends with
    /// the longest suffix of the specifier any file has, ranked — the
    /// importer's own package first, then the files sharing the longest
    /// directory prefix with it. The caller looks the name up in all of
    /// them.
    fn js_resolve(&self, from: FileId, spec: &str) -> JsTargets {
        let exact = |f: Option<FileId>| JsTargets {
            files: f.into_iter().map(|f| (f, (0, 0))).collect(),
            exact: true,
        };
        let spec = spec.trim();
        if spec.contains(['!', '?']) {
            return JsTargets::default();
        }
        if spec == "."
            || spec == ".."
            || spec.starts_with("./")
            || spec.starts_with("../")
        {
            let Some(from_path) = self.paths.get(&from) else {
                return JsTargets::default();
            };
            let dir = from_path.parent().unwrap_or(Path::new(""));
            return exact(self.js_probe(&normalize(&dir.join(spec))));
        }
        // A specifier a package's `exports` map names (`@std/assert/
        // instance-of` → `instance_of.ts`).
        if let Some([f]) = self.js_exported.get(spec).map(Vec::as_slice) {
            return exact(Some(*f));
        }
        // A workspace package by name: its own directory, entry file
        // (`index`, Deno's `mod`, under `src/` too) or the sub-path.
        if let Some((name, root)) = self
            .js_pkg_roots
            .iter()
            .filter(|(n, _)| {
                spec == n.as_str()
                    || spec
                        .strip_prefix(n.as_str())
                        .is_some_and(|r| r.starts_with('/'))
            })
            .max_by_key(|(n, _)| n.len())
        {
            let rest = spec[name.len()..].trim_start_matches('/');
            let found = if rest.is_empty() {
                ["index", "mod", "src/index", "src/mod"]
                    .iter()
                    .find_map(|entry| self.js_probe(&root.join(entry)))
            } else {
                self.js_probe(&normalize(&root.join(rest)))
                    .or_else(|| self.js_probe(&normalize(&root.join("src").join(rest))))
            };
            if found.is_some() {
                return exact(found);
            }
        }
        let trimmed = spec
            .trim_start_matches(['@', '~', '#', '$'])
            .trim_start_matches('/');
        let mut segs: Vec<String> = trimmed
            .split('/')
            .filter(|s| !s.is_empty() && *s != "." && *s != "..")
            .map(str::to_string)
            .collect();
        if let Some(last) = segs.last_mut() {
            *last = strip_js_ext(last).to_string();
        }
        let own_root = self
            .js_pkg_of
            .get(&from)
            .and_then(|n| self.js_pkg_roots.get(*n));
        let here: Vec<Component<'_>> = self
            .paths
            .get(&from)
            .map(|p| p.components().collect())
            .unwrap_or_default();
        // Only a path alias (`@/x`, `~/x`, `#x`, `$lib/x`, a tsconfig
        // `paths` prefix, a Deno root-relative `/x`) stands for a directory
        // its first segment does not name. A bare specifier (`next/
        // navigation`) matches only as a whole and only inside the
        // importer's own package (`baseUrl`, `NODE_PATH`): that a
        // `navigation.ts` exists somewhere says nothing.
        let aliased = self.js_prefix_alias(spec);
        let tries = if aliased {
            segs.len()
        } else {
            segs.len().min(1)
        };
        for k in 0..tries {
            let suffix = &segs[k..];
            let Some(cands) = suffix.last().and_then(|l| self.js_by_last.get(l)) else {
                continue;
            };
            let mut hits: Vec<(FileId, (u8, usize))> = Vec::new();
            for (key, file) in cands {
                // A module never imports itself (`import dayjs from
                // "dayjs"` inside the workspace's own `dayjs` package).
                if !key.ends_with(suffix)
                    || *file == from
                    || hits.iter().any(|(f, _)| f == file)
                {
                    continue;
                }
                let Some(p) = self.paths.get(file) else {
                    continue;
                };
                let own = own_root.is_some_and(|r| normalize(p).starts_with(r));
                // `baseUrl` / `NODE_PATH` resolve within the importer's own
                // project: `react` is not another package's `react/` dir.
                if !aliased && own_root.is_some() && !own {
                    continue;
                }
                let shared = p
                    .components()
                    .zip(&here)
                    .take_while(|(a, b)| a == *b)
                    .count();
                hits.push((*file, (u8::from(own), shared)));
            }
            if !hits.is_empty() {
                // Best first; equal ranks keep walk order.
                hits.sort_by(|a, b| b.1.cmp(&a.1));
                return JsTargets {
                    files: hits,
                    exact: false,
                };
            }
        }
        JsTargets::default()
    }

    /// Whether `from`'s tsconfig maps every bare specifier into the tree
    /// (a catch-all `"*"` `paths` key): its bare specifiers name files.
    fn js_catch_all(&self, from: FileId) -> bool {
        self.facts
            .get(&from)
            .is_some_and(|f| f.packages.iter().any(|p| p == "*"))
    }

    /// Whether `spec` is a path alias whose head stands for a directory
    /// (`@/x`, `~/x`, `#x`, `$lib/x`, a tsconfig `paths` prefix, a Deno
    /// root-relative `/x`).
    fn js_prefix_alias(&self, spec: &str) -> bool {
        // A package or exact alias the tree names (`@acme/net` in a
        // `package.json` of the tree): its head is no directory either.
        self.js_known_packages.contains(spec)
            || self
                .js_known_packages
                .contains(crate::heads::npm_package_of(spec))
            || spec.starts_with(['/', '~', '#', '$'])
            || spec.starts_with("@/")
            || self
                .js_alias_prefixes
                .iter()
                .any(|p| spec.starts_with(p.as_str()))
    }

    /// The label for a call through `b` that its module does not provide:
    /// refused — unless the module was only a guess at what a bare
    /// specifier names (a same-named file of the importer's package, as
    /// `baseUrl` would find it, such as a test stub of the package), and
    /// then the package is the likelier reading.
    fn js_miss(&self, from: FileId, b: &JsBinding) -> Scoped {
        let bare = !b.spec.starts_with('.')
            && !self.js_prefix_alias(&b.spec)
            && !self.js_catch_all(from);
        if bare && !b.targets.exact {
            Scoped::External(b.spec.clone())
        } else {
            Scoped::Refused(b.spec.clone())
        }
    }

    /// Look a name up in every target and choose: the one provider, the
    /// best-ranked of several, or none (`Ambiguous`) when the best rank
    /// is shared by providers that disagree.
    fn js_pick(
        &self,
        targets: &JsTargets,
        mut lookup: impl FnMut(FileId) -> (Vec<CallableId>, bool),
    ) -> Pick {
        let mut provs: Vec<((u8, usize), Vec<CallableId>, bool)> = Vec::new();
        for (t, rank) in &targets.files {
            let (ids, certain) = lookup(*t);
            if !ids.is_empty() && !provs.iter().any(|(_, p, _)| *p == ids) {
                provs.push((*rank, ids, certain));
            }
        }
        let Some(best) = provs.iter().map(|(r, _, _)| *r).max() else {
            return Pick::Miss;
        };
        let mut top: Vec<_> = provs.into_iter().filter(|(r, _, _)| *r == best).collect();
        if top.len() == 1 {
            let (_, ids, certain) = top.pop().unwrap_or_default();
            Pick::One(ids, certain)
        } else {
            Pick::Ambiguous
        }
    }

    /// Whether `m` defines anything named `name` at all — nested in a
    /// function, a member of a returned object, `this.name = …` in a
    /// constructor. A call through the module may reach it by a value cgg
    /// does not type, so a miss is not proof of absence.
    fn js_defines_somewhere(&self, m: FileId, name: &str) -> bool {
        self.by_file_simple
            .get(&(m, name))
            .is_some_and(|v| !v.is_empty())
    }

    /// Whether `m` exports a class (one with members) as `export` —
    /// directly, or through `export default X` / `export { X as e }`.
    fn js_declares_class(&self, m: FileId, export: &str) -> bool {
        self.js_owners.contains(&(m, export))
            || self.js_exports.get(&m).is_some_and(|ex| {
                ex.local.iter().any(|(e, l)| {
                    e == export && self.js_owners.contains(&(m, l.as_str()))
                })
            })
    }

    /// The dependency `m` exports `export` from: `import x from "dep";
    /// export default x` (or `export { x }`), `export { x } from "dep"`,
    /// `export * from "dep"`, where `dep` is a package the tree does not
    /// contain. Asked only once the lookup missed.
    fn js_export_from_dependency(&self, m: FileId, export: &str) -> Option<String> {
        let f = self.facts.get(&m)?;
        let ex = self.js_exports.get(&m)?;
        let unfound = |spec: &str| -> bool {
            let rel = spec.starts_with('.') || spec.starts_with('/');
            !rel && self.js_resolve(m, spec).files.is_empty()
        };
        for (_, local) in ex.local.iter().filter(|(e, _)| e == export) {
            for imp in &f.imports {
                let binds = match imp.kind.as_str() {
                    "from-import" => imp
                        .alias
                        .split(',')
                        .any(|item| split_as(item.trim()).1 == local),
                    "import" => imp.alias.trim() == local,
                    _ => false,
                };
                let spec = imp.path.trim();
                if binds && unfound(spec) {
                    return Some(spec.to_string());
                }
            }
        }
        for imp in f.imports.iter().filter(|i| i.kind == "re-export") {
            let spec = imp.path.trim();
            // `export { x } from "dep"`, or `export * from "dep"` for a
            // name the module itself does not provide.
            let names = imp.alias.split(',').any(|item| {
                let item = item.trim();
                if item == "*" {
                    export != "default"
                } else {
                    !item.starts_with("* as ") && split_as(item).1 == export
                }
            });
            if names && unfound(spec) {
                return Some(spec.to_string());
            }
        }
        None
    }

    /// Whether `m`'s default export is a value cgg could not name
    /// (`export default createX()`, `module.exports = makeThing()`).
    /// So is a default export naming a variable (`const router =
    /// express.Router(); module.exports = router`), or an import whose
    /// value is one of those.
    fn js_unknown_default(&self, m: FileId) -> bool {
        self.js_unknown_default_at(m, 0)
    }

    fn js_unknown_default_at(&self, m: FileId, depth: u8) -> bool {
        if depth > MAX_DEPTH {
            return false;
        }
        let Some(ex) = self.js_exports.get(&m) else {
            return false;
        };
        ex.local
            .iter()
            .filter(|(e, _)| e == "default")
            .any(|(_, l)| self.js_unknown_value(m, l, depth))
            // `module.exports = require('./m')`: `./m`'s value.
            || ex
                .named
                .iter()
                .filter(|(e, o, _)| e == "default" && o == "default")
                .any(|(_, _, files)| {
                    files
                        .iter()
                        .any(|t| self.js_unknown_default_at(*t, depth + 1))
                })
    }

    /// Whether the module-level name `l` of `m` is a value cgg cannot
    /// read: not a function or class `m` declares, and not an import of
    /// one.
    fn js_unknown_value(&self, m: FileId, l: &str, depth: u8) -> bool {
        if l == UNKNOWN_EXPORT {
            return true;
        }
        if !self.js_module_level(m, l).is_empty() || self.js_owners.contains(&(m, l)) {
            return false;
        }
        let Some(f) = self.facts.get(&m) else {
            return true;
        };
        for imp in &f.imports {
            let orig = match imp.kind.as_str() {
                "from-import" => imp.alias.split(',').find_map(|item| {
                    let (o, local) = split_as(item.trim());
                    (local == l).then_some(Some(o))
                }),
                "import" if imp.alias.trim() == l => Some(None),
                _ => None,
            };
            let Some(orig) = orig else { continue };
            let targets = self.js_resolve(m, imp.path.trim()).files;
            return match orig {
                // A namespace / `require()` / default binding: that
                // module's value.
                None | Some("default") => targets
                    .iter()
                    .any(|(t, _)| self.js_unknown_default_at(*t, depth + 1)),
                Some(o) => targets.iter().any(|(t, _)| {
                    self.js_module_level(*t, o).is_empty()
                        && !self.js_owners.contains(&(*t, o))
                }),
            };
        }
        // A local variable.
        true
    }

    fn js_probe(&self, base: &Path) -> Option<FileId> {
        let raw = base.to_string_lossy();
        let has_ext = base
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| JS_EXTS.contains(&e));
        if has_ext && let Some(f) = self.js_by_path.get(base) {
            return Some(*f);
        }
        for ext in JS_EXTS {
            if let Some(f) = self.js_by_path.get(&PathBuf::from(format!("{raw}.{ext}"))) {
                return Some(*f);
            }
        }
        // `./svc.js` written in a TypeScript source names `svc.ts`.
        if let Some(stem) = ["js", "jsx", "mjs", "cjs"]
            .iter()
            .find_map(|e| raw.strip_suffix(&format!(".{e}")))
        {
            for ext in ["ts", "tsx", "mts", "cts"] {
                if let Some(f) =
                    self.js_by_path.get(&PathBuf::from(format!("{stem}.{ext}")))
                {
                    return Some(*f);
                }
            }
        }
        for ext in JS_EXTS {
            if let Some(f) = self.js_by_path.get(&base.join(format!("index.{ext}"))) {
                return Some(*f);
            }
        }
        None
    }

    /// Top-level callables of `m` named `name`.
    fn js_module_level(&self, m: FileId, name: &str) -> Vec<CallableId> {
        self.by_file_qn
            .get(&(m, name))
            .map(|v| {
                v.iter()
                    .copied()
                    .filter(|c| !self.nested.contains(c))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What `m` exports as `path.name` (`path` empty: the export `name`
    /// itself; `["Runner"]`: member `name` of the export `Runner`;
    /// `["default"]`: of the default export).
    fn js_lookup(
        &self,
        m: FileId,
        path: &[String],
        name: &str,
        depth: u8,
        seen: &mut HashSet<(FileId, String)>,
    ) -> Vec<CallableId> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }
        let key = if path.is_empty() {
            name.to_string()
        } else {
            format!("{}.{name}", path.join("."))
        };
        if !seen.insert((m, key.clone())) {
            return Vec::new();
        }
        let ex = self.js_exports.get(&m);
        if path.is_empty() {
            let ids = self.js_module_level(m, name);
            if !ids.is_empty() {
                return ids;
            }
        } else {
            let ids: Vec<CallableId> = self
                .by_file_qn
                .get(&(m, key.as_str()))
                .map(|v| {
                    v.iter()
                        .copied()
                        .filter(|c| !self.nested.contains(c))
                        .collect()
                })
                .unwrap_or_default();
            if !ids.is_empty() {
                return ids;
            }
        }
        let Some(ex) = ex else {
            // No export table: an imported binding re-exported implicitly
            // is not a thing; only a local `export { x }` re-exports.
            return Vec::new();
        };
        // The name the export path starts with: the export itself, or the
        // object it is a member of.
        let (head, rest): (&str, &[String]) = match path.split_first() {
            Some((h, r)) => (h.as_str(), r),
            None => (name, &[]),
        };
        let with_head = |h: &str| -> Vec<String> {
            let mut p = vec![h.to_string()];
            p.extend(rest.iter().cloned());
            p
        };
        // `export { local as head }` / `export default local`.
        for (exported, local) in ex.local.iter().filter(|(e, _)| e == head) {
            let ids = if local != head {
                if path.is_empty() {
                    self.js_lookup(m, &[], local, depth + 1, seen)
                } else {
                    self.js_lookup(m, &with_head(local), name, depth + 1, seen)
                }
            } else {
                Vec::new()
            };
            if !ids.is_empty() {
                return ids;
            }
            // The local name is itself imported into `m`.
            let ids =
                self.js_follow_import(m, local, rest, path.is_empty(), name, depth, seen);
            if !ids.is_empty() {
                return ids;
            }
            let _ = exported;
        }
        // `export { orig as head } from 'spec'`.
        for (_, orig, files) in ex.named.iter().filter(|(e, _, _)| e == head) {
            for &t in files {
                let ids = if path.is_empty() {
                    self.js_lookup(t, &[], orig, depth + 1, seen)
                } else {
                    self.js_lookup(t, &with_head(orig), name, depth + 1, seen)
                };
                if !ids.is_empty() {
                    return ids;
                }
            }
        }
        // `export * as head from 'spec'`; called itself (`{ doExport:
        // require('./exporter') }` then `m.doExport()`), it is that
        // module's default.
        for (_, files) in ex.namespace.iter().filter(|(ns, _)| ns == head) {
            for &t in files {
                let ids = if path.is_empty() {
                    self.js_lookup(t, &[], "default", depth + 1, seen)
                } else {
                    self.js_lookup(t, rest, name, depth + 1, seen)
                };
                if !ids.is_empty() {
                    return ids;
                }
            }
        }
        // `export * from 'spec'`: a name two star sources both provide is
        // ambiguous, and a module does not export it at all.
        if head != "default" {
            let mut found: Vec<Vec<CallableId>> = Vec::new();
            for files in &ex.star {
                for &t in files {
                    let ids = self.js_lookup(t, path, name, depth + 1, seen);
                    if !ids.is_empty() && !found.contains(&ids) {
                        found.push(ids);
                    }
                }
            }
            if found.len() == 1 {
                return found.pop().unwrap_or_default();
            }
        }
        Vec::new()
    }

    /// `import { x as local } from 'spec'` (or a default / namespace
    /// import) in `m`, followed to what `spec` exports.
    #[allow(clippy::too_many_arguments)]
    fn js_follow_import(
        &self,
        m: FileId,
        local: &str,
        rest: &[String],
        is_value: bool,
        name: &str,
        depth: u8,
        seen: &mut HashSet<(FileId, String)>,
    ) -> Vec<CallableId> {
        let Some(f) = self.facts.get(&m) else {
            return Vec::new();
        };
        for imp in &f.imports {
            let bound = match imp.kind.as_str() {
                "from-import" => imp.alias.split(',').find_map(|item| {
                    let (orig, l) = split_as(item.trim());
                    (l == local).then(|| Some(orig.to_string()))
                }),
                "import" if imp.alias.trim() == local => Some(None),
                _ => None,
            };
            let Some(orig) = bound else { continue };
            for (t, _) in self.js_resolve(m, imp.path.trim()).files {
                let ids = match (&orig, is_value) {
                    // `import { a as local }; export { local }`.
                    (Some(o), true) => self.js_lookup(t, &[], o, depth + 1, seen),
                    (Some(o), false) => {
                        let mut p = vec![o.clone()];
                        p.extend(rest.iter().cloned());
                        self.js_lookup(t, &p, name, depth + 1, seen)
                    }
                    // `import * as local; export { local }`.
                    (None, true) => Vec::new(),
                    // A namespace or `require()` binding: the module, or
                    // (CommonJS) the value it exports as `module.exports`.
                    (None, false) => {
                        let ids = self.js_lookup(t, rest, name, depth + 1, seen);
                        if ids.is_empty() {
                            self.js_lookup(
                                t,
                                &with_head("default", rest),
                                name,
                                depth + 1,
                                seen,
                            )
                        } else {
                            ids
                        }
                    }
                };
                if !ids.is_empty() {
                    return ids;
                }
            }
        }
        Vec::new()
    }

    /// Members of `m` named `name` that all belong to one owner — the
    /// methods of a default-exported instance or object.
    fn js_members_of_one_owner(&self, m: FileId, name: &str) -> Vec<CallableId> {
        let Some(ids) = self.by_file_simple.get(&(m, name)) else {
            return Vec::new();
        };
        let ids: Vec<CallableId> = ids
            .iter()
            .copied()
            .filter(|c| !self.nested.contains(c))
            .collect();
        let owners: HashSet<&str> = ids
            .iter()
            .filter_map(|c| self.qn_of.get(c))
            .map(|qn| qn.rsplit_once('.').map_or("", |(o, _)| o))
            .collect();
        if owners.len() == 1 { ids } else { Vec::new() }
    }

    // ---------------------------------------------------------------
    // Python
    // ---------------------------------------------------------------

    /// `from <spec> import …` written in `file`, made absolute.
    pub(crate) fn py_abs(&self, file: FileId, spec: &str) -> Option<String> {
        let spec = spec.trim();
        if !spec.starts_with('.') {
            return (!spec.is_empty()).then(|| spec.to_string());
        }
        let level = spec.chars().take_while(|c| *c == '.').count();
        let rest = &spec[level..];
        let (module, is_init) = self.py_module_of.get(&file)?;
        let mut segs: Vec<&str> = module.split('.').collect();
        if !is_init {
            segs.pop();
        }
        for _ in 1..level {
            segs.pop()?;
        }
        segs.extend(rest.split('.').filter(|s| !s.is_empty()));
        (!segs.is_empty()).then(|| segs.join("."))
    }

    /// `pkg.name` where `pkg`'s `__init__.py` itself binds `name` (`from
    /// .mailer import email_service`): the attribute, not the submodule of
    /// the same name.
    fn py_package_binds(&self, abs: &[String]) -> bool {
        let Some((name, pkg)) = abs.split_last() else {
            return false;
        };
        let pkg = pkg.join(".");
        self.py_by_module.get(pkg.as_str()).is_some_and(|files| {
            files.iter().any(|f| {
                self.py_module_of.get(f).is_some_and(|(_, init)| *init)
                    && self
                        .py_from
                        .get(f)
                        .is_some_and(|v| v.iter().any(|(l, _, _)| l == name))
            })
        })
    }

    /// The qualified-name prefix of a Python module file.
    fn py_prefix(&self, file: FileId) -> Option<String> {
        let (m, is_init) = self.py_module_of.get(&file)?;
        Some(if *is_init {
            format!("{m}.__init__")
        } else {
            (*m).to_string()
        })
    }

    /// The longest prefix of `segs` that is a module the tree has, with
    /// what remains of the path. Several files under one name (two
    /// script directories each with a `utils.py`) narrow to the caller's
    /// own directory, as `sys.path[0]` would.
    fn py_split(&self, caller: FileId, segs: &[String]) -> Option<(Vec<FileId>, usize)> {
        for k in (1..=segs.len()).rev() {
            let name = segs[..k].join(".");
            if let Some(files) = self.py_by_module.get(name.as_str()) {
                if files.len() > 1 {
                    let dir = self.paths.get(&caller).and_then(|p| p.parent());
                    let near: Vec<FileId> = files
                        .iter()
                        .copied()
                        .filter(|f| self.paths.get(f).and_then(|p| p.parent()) == dir)
                        .collect();
                    if !near.is_empty() {
                        return Some((near, k));
                    }
                }
                return Some((files.clone(), k));
            }
        }
        None
    }

    /// `rest.name` inside module file `t`, through the module's own
    /// `from x import y` re-exports.
    fn py_lookup(
        &self,
        t: FileId,
        rest: &[String],
        name: &str,
        depth: u8,
        seen: &mut HashSet<(FileId, String)>,
    ) -> Vec<CallableId> {
        if depth > MAX_DEPTH {
            return Vec::new();
        }
        let Some(prefix) = self.py_prefix(t) else {
            return Vec::new();
        };
        let mut qn = prefix;
        for r in rest {
            qn.push('.');
            qn.push_str(r);
        }
        qn.push('.');
        qn.push_str(name);
        if !seen.insert((t, qn.clone())) {
            return Vec::new();
        }
        if let Some(ids) = self.by_file_qn.get(&(t, qn.as_str()))
            && !ids.is_empty()
        {
            return ids.clone();
        }
        let head = rest.first().map_or(name, String::as_str);
        let Some(froms) = self.py_from.get(&t) else {
            return Vec::new();
        };
        // `from .core import head` in this module.
        for (_, module, orig) in froms.iter().filter(|(l, _, o)| l == head && o != "*") {
            let mut target: Vec<String> = module.split('.').map(str::to_string).collect();
            // `pkg.helper` → `helper` in `pkg.core`; `pkg.Runner.m` →
            // `m` under `Runner` in `pkg.jobs`.
            let found = if rest.is_empty() {
                self.py_path_lookup(t, &target, orig, depth + 1, seen)
            } else {
                target.push(orig.clone());
                target.extend(rest.iter().skip(1).cloned());
                self.py_path_lookup(t, &target, name, depth + 1, seen)
            };
            if let Some(ids) = found {
                return ids;
            }
        }
        // `from .core import *`: a name only one star source provides.
        let mut found: Vec<Vec<CallableId>> = Vec::new();
        for (_, module, _) in froms.iter().filter(|(_, _, o)| o == "*") {
            let mut target: Vec<String> = module.split('.').map(str::to_string).collect();
            target.extend(rest.iter().cloned());
            if let Some(ids) = self.py_path_lookup(t, &target, name, depth + 1, seen)
                && !found.contains(&ids)
            {
                found.push(ids);
            }
        }
        if found.len() == 1 {
            return found.pop().unwrap_or_default();
        }
        Vec::new()
    }

    /// `path.name`, with `path` absolute and possibly naming a module,
    /// then members inside it.
    fn py_path_lookup(
        &self,
        caller: FileId,
        path: &[String],
        name: &str,
        depth: u8,
        seen: &mut HashSet<(FileId, String)>,
    ) -> Option<Vec<CallableId>> {
        let (files, k) = self.py_split(caller, path)?;
        let mut found: Vec<Vec<CallableId>> = Vec::new();
        for t in files {
            let ids = self.py_lookup(t, &path[k..], name, depth, seen);
            if !ids.is_empty() && !found.contains(&ids) {
                found.push(ids);
            }
        }
        (found.len() == 1).then(|| found.pop().unwrap_or_default())
    }

    // ---------------------------------------------------------------
    // Go
    // ---------------------------------------------------------------

    /// The files of the package an import path names: by the package's
    /// own import path where a `go.mod` gave one, otherwise by the
    /// longest directory suffix of the path any directory has (a
    /// GOPATH-era or vendored tree). An external test package
    /// (`package x_test`) is never what an import reaches.
    fn go_files(&self, path: &str) -> Vec<FileId> {
        let not_test =
            |f: &FileId| !self.go_pkg.get(f).is_some_and(|p| p.ends_with("_test"));
        if let Some(files) = self.go_by_path.get(path) {
            return files.iter().copied().filter(not_test).collect();
        }
        let segs: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        for k in 0..segs.len() {
            let suffix = &segs[k..];
            let Some(cands) = suffix.last().and_then(|l| self.go_by_last_dir.get(l))
            else {
                continue;
            };
            let mut dirs: Vec<&Vec<String>> = Vec::new();
            let mut files: Vec<FileId> = Vec::new();
            for (dir, f) in cands {
                if dir.ends_with(suffix) {
                    if !dirs.contains(&dir) {
                        dirs.push(dir);
                    }
                    files.push(*f);
                }
            }
            match dirs.len() {
                0 => continue,
                1 => return files.into_iter().filter(not_test).collect(),
                _ => return Vec::new(),
            }
        }
        Vec::new()
    }

    /// The calling Go file's own package: callables named `name` at
    /// package level in the files of its directory that share its package
    /// clause.
    fn go_same_package(&self, file: FileId, name: &str) -> Vec<CallableId> {
        let (Some(path), Some(pkg)) = (self.paths.get(&file), self.go_pkg.get(&file))
        else {
            return Vec::new();
        };
        let Some(dir) = path.parent().map(normalize) else {
            return Vec::new();
        };
        let files: Vec<FileId> = self
            .go_dir_files
            .get(&dir)
            .map(|v| {
                v.iter()
                    .copied()
                    .filter(|f| self.go_pkg.get(f) == Some(pkg))
                    .collect()
            })
            .unwrap_or_default();
        self.go_lookup(&files, name)
    }

    /// Callables of package `files` whose qualified name is the package
    /// name joined with `tail`.
    fn go_lookup(&self, files: &[FileId], tail: &str) -> Vec<CallableId> {
        let mut out = Vec::new();
        for f in files {
            let Some(pkg) = self.go_pkg.get(f) else {
                continue;
            };
            let qn = format!("{pkg}.{tail}");
            if let Some(ids) = self.by_file_qn.get(&(*f, qn.as_str())) {
                for id in ids {
                    if !out.contains(id) {
                        out.push(*id);
                    }
                }
            }
        }
        out
    }
}

/// The calling file's module bindings, judged against the workspace.
#[derive(Debug, Default)]
pub(crate) struct FileModules {
    js: HashMap<String, JsBinding>,
    /// Python: local name → absolute dotted path it binds.
    py: HashMap<String, String>,
    /// Go: local name → package import path.
    go: HashMap<String, String>,
    /// Go: whether the file sits under a `go.mod` (module mode).
    go_has_module: bool,
    /// Go: byte spans where a parameter or local shadows an import name.
    go_shadow: HashMap<String, Vec<(u32, u32)>>,
    /// Simple names the file defines: a bare call to one may be the
    /// local definition shadowing an import.
    own_defs: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum JsKind {
    /// `import * as m`, `const m = require(…)`, `import m = require(…)`.
    Namespace,
    /// `import m from …`.
    Default,
    /// `import { orig as m } from …`.
    Named(String),
}

#[derive(Debug, Clone, PartialEq)]
struct JsBinding {
    spec: String,
    kind: JsKind,
    /// The files `spec` names, resolved once per calling file.
    targets: JsTargets,
}

impl FileModules {
    pub(crate) fn build(
        ix: &ModuleIndex<'_>,
        facts: &FileFacts,
        vocab: &WorkspaceVocab,
    ) -> Self {
        let mut fm = FileModules {
            own_defs: facts
                .definitions
                .iter()
                .map(|d| d.simple_name.clone())
                .collect(),
            ..FileModules::default()
        };
        match facts.language.as_str() {
            "javascript" | "typescript" => {
                // A name two imports bind is left to the old reading.
                let mut conflicted: HashSet<String> = HashSet::new();
                let mut bind = |name: &str, b: JsBinding| {
                    let name = name.trim();
                    if name.is_empty() || conflicted.contains(name) {
                        return;
                    }
                    match fm.js.get(name) {
                        Some(prev) if *prev != b => {
                            fm.js.remove(name);
                            conflicted.insert(name.to_string());
                        }
                        Some(_) => {}
                        None => {
                            fm.js.insert(name.to_string(), b);
                        }
                    }
                };
                for imp in &facts.imports {
                    let spec = imp.path.trim();
                    if spec.is_empty() || !vocab.js_workspace(spec) {
                        continue;
                    }
                    match imp.kind.as_str() {
                        "import" if !imp.alias.trim().is_empty() => bind(
                            &imp.alias,
                            JsBinding {
                                spec: spec.to_string(),
                                kind: JsKind::Namespace,
                                targets: ix.js_resolve(facts.file, spec),
                            },
                        ),
                        "from-import" => {
                            let targets = ix.js_resolve(facts.file, spec);
                            for item in imp.alias.split(',') {
                                let (orig, local) = split_as(item.trim());
                                if orig.is_empty() {
                                    continue;
                                }
                                let kind = if orig == "default" {
                                    JsKind::Default
                                } else {
                                    JsKind::Named(orig.to_string())
                                };
                                bind(
                                    local,
                                    JsBinding {
                                        spec: spec.to_string(),
                                        kind,
                                        targets: targets.clone(),
                                    },
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            "python" => {
                for imp in &facts.imports {
                    match imp.kind.as_str() {
                        "import" => {
                            let whole = if imp.alias.trim().is_empty() {
                                imp.path.clone()
                            } else {
                                format!("{} as {}", imp.path.trim(), imp.alias.trim())
                            };
                            for item in whole.split(',') {
                                let item = item.trim();
                                let (module, local) = match item.split_once(" as ") {
                                    Some((m, a)) => (m.trim(), a.trim()),
                                    None => {
                                        let head = item.split('.').next().unwrap_or(item);
                                        (head, head)
                                    }
                                };
                                if module.is_empty() || !vocab.py_workspace(module) {
                                    continue;
                                }
                                fm.py.insert(local.to_string(), module.to_string());
                            }
                        }
                        "from-import" => {
                            let raw = imp.path.trim();
                            if raw.is_empty() || !vocab.py_workspace(raw) {
                                continue;
                            }
                            let Some(module) = ix.py_abs(facts.file, raw) else {
                                continue;
                            };
                            for item in imp.alias.split(',') {
                                let item =
                                    item.trim().trim_matches(|c| c == '(' || c == ')');
                                let (orig, local) = split_as(item);
                                if orig.is_empty() || orig == "*" {
                                    continue;
                                }
                                fm.py.insert(
                                    local.to_string(),
                                    format!("{module}.{orig}"),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            "go" => {
                let has_module = !facts.packages.is_empty();
                fm.go_has_module = has_module;
                // `<local>` opens a shadow of the name, the next untyped
                // entry for it closes it (see the plugin).
                let mut by_name: HashMap<&str, Vec<(u32, &str)>> = HashMap::new();
                for lt in &facts.local_types {
                    by_name
                        .entry(lt.var_name.as_str())
                        .or_default()
                        .push((lt.scope_byte, lt.type_name.as_str()));
                }
                for (name, mut entries) in by_name {
                    entries.sort_by_key(|(b, _)| *b);
                    let mut open: Option<u32> = None;
                    for (b, ty) in entries {
                        if ty.starts_with('<') {
                            open.get_or_insert(b);
                        } else if ty.is_empty()
                            && let Some(start) = open.take()
                        {
                            fm.go_shadow
                                .entry(name.to_string())
                                .or_default()
                                .push((start, b));
                        }
                    }
                }
                for imp in facts.imports.iter().filter(|i| i.kind == "import") {
                    let path = imp.path.trim();
                    if path.is_empty() || !vocab.go_workspace(path, has_module) {
                        continue;
                    }
                    for name in go_import_names(path, imp.alias.trim()) {
                        fm.go.insert(name, path.to_string());
                    }
                }
            }
            _ => {}
        }
        fm
    }

    /// Module-scoped resolution of one call. `None`: the call is not
    /// written through a workspace module binding this pass can judge, and
    /// the resolver's other steps apply. A refusal is returned only when
    /// the lookup is decisive — the receiver is the module itself
    /// (`mod.f()`, `import { f }; f()`), or a path ending in a class the
    /// module provides whose method set is known. A path through a
    /// module-level value, a function's result of unknown type, or a
    /// chain returns `None`. `bases(owner, name)` is the resolver's
    /// inheritance walk.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve(
        &self,
        ix: &ModuleIndex<'_>,
        lang: &str,
        file: FileId,
        r: &RefRecord,
        caller_sig: Option<&str>,
        ctor_names: &[&str],
        bases: &dyn Fn(&str, &str) -> Option<Vec<CallableId>>,
    ) -> Option<Scoped> {
        let rh = r.receiver_hint.trim();
        if rh == cgg_core::VALUE_REF_HINT || rh == cgg_core::STRING_REF_HINT {
            return None;
        }
        // A parameter of the calling function named like an import shadows
        // it (`def f(helpers): helpers.x()`): not this pass's to judge.
        let head = if rh.is_empty() {
            r.name.as_str()
        } else {
            leading_ident(rh.strip_prefix("new ").unwrap_or(rh))
        };
        if lang != "go" && caller_sig.is_some_and(|sig| param_names(sig).contains(&head))
        {
            return None;
        }
        match lang {
            "javascript" | "typescript" => {
                self.resolve_js(ix, file, r, rh, ctor_names, bases)
            }
            "python" => self.resolve_py(ix, file, r, rh, bases),
            "go" => self.resolve_go(ix, file, r, rh, bases),
            _ => None,
        }
    }

    fn resolve_js(
        &self,
        ix: &ModuleIndex<'_>,
        file: FileId,
        r: &RefRecord,
        rh: &str,
        ctor_names: &[&str],
        bases: &dyn Fn(&str, &str) -> Option<Vec<CallableId>>,
    ) -> Option<Scoped> {
        let mut seen = HashSet::new();
        if rh.is_empty() {
            // A bare call to an imported name. The file's own definition
            // of the name, if any, is what intra-file already weighed.
            if r.dot_call || self.own_defs.contains(r.name.as_str()) {
                return None;
            }
            let b = self.js.get(r.name.as_str())?;
            if b.targets.files.is_empty() {
                return Some(js_unfound(&b.spec));
            }
            let export = match &b.kind {
                JsKind::Named(orig) => orig.clone(),
                JsKind::Default | JsKind::Namespace => "default".to_string(),
            };
            let pick = ix.js_pick(&b.targets, |t| {
                let mut ids = ix.js_lookup(t, &[], &export, 0, &mut seen);
                if ids.is_empty() {
                    // `new Runner()`: the class's constructor.
                    for ctor in ctor_names {
                        ids = ix.js_lookup(
                            t,
                            std::slice::from_ref(&export),
                            ctor,
                            0,
                            &mut seen,
                        );
                        if !ids.is_empty() {
                            break;
                        }
                    }
                }
                (ids, true)
            });
            return match pick {
                Pick::One(cids, certain) => Some(Scoped::Bound { cids, certain }),
                Pick::Ambiguous => None,
                Pick::Miss => {
                    let targets = || b.targets.files.iter().map(|(t, _)| *t);
                    if targets().any(|t| ix.js_declares_class(t, &export)) {
                        Some(Scoped::NoCtor)
                    } else if targets().any(|t| {
                        // An unreadable `module.exports` value: its
                        // properties are unknown too.
                        ix.js_unknown_default(t) || ix.js_defines_somewhere(t, &export)
                    }) {
                        None
                    } else if let Some(dep) =
                        targets().find_map(|t| ix.js_export_from_dependency(t, &export))
                    {
                        // `@scope/dayjs` re-exporting the `dayjs` package.
                        Some(Scoped::External(dep))
                    } else {
                        Some(ix.js_miss(file, b))
                    }
                }
            };
        }
        let constructed = rh.starts_with("new ");
        let expr = rh.strip_prefix("new ").unwrap_or(rh).trim();
        let head = leading_ident(expr);
        let b = self.js.get(head)?;
        // A receiver that is not a plain path (an index, a literal) goes
        // through a value: not this pass's to judge.
        let (segs, _) = parse_receiver(expr)?;
        // `useThing().get()`, `svc.make().run()`, a builder chain: a call
        // on what a call returned, whose type is unknown. Only the class
        // a `new` constructs may carry the call.
        let last = segs.len() - 1;
        if segs
            .iter()
            .enumerate()
            .any(|(i, s)| s.called && !(constructed && i == last))
        {
            return None;
        }
        let rest: Vec<String> = segs[1..].iter().map(|s| s.ident.to_string()).collect();
        if b.targets.files.is_empty() {
            // A package the tree lacks: external. A relative module cgg did
            // not analyze (`types.d.ts`, `data.json`) is decisive only for
            // `mod.f()`; a member of a type or value it declares may be
            // implemented anywhere.
            let decisive = rest.is_empty() && !constructed && b.kind == JsKind::Namespace;
            return match js_unfound(&b.spec) {
                Scoped::Refused(_) if !decisive => None,
                other => Some(other),
            };
        }
        let pick = ix.js_pick(&b.targets, |t| {
            let mut certain = true;
            let mut ids = match &b.kind {
                JsKind::Namespace => {
                    let ids = ix.js_lookup(t, &rest, &r.name, 0, &mut seen);
                    if ids.is_empty() {
                        // CommonJS: `require()` returns `module.exports`,
                        // which may be a class or an object.
                        ix.js_lookup(
                            t,
                            &with_head("default", &rest),
                            &r.name,
                            0,
                            &mut seen,
                        )
                    } else {
                        ids
                    }
                }
                JsKind::Default => {
                    let ids = ix.js_lookup(
                        t,
                        &with_head("default", &rest),
                        &r.name,
                        0,
                        &mut seen,
                    );
                    if ids.is_empty() {
                        ix.js_lookup(t, &rest, &r.name, 0, &mut seen)
                    } else {
                        ids
                    }
                }
                JsKind::Named(orig) => {
                    ix.js_lookup(t, &with_head(orig, &rest), &r.name, 0, &mut seen)
                }
            };
            if ids.is_empty() && rest.is_empty() {
                let class = match &b.kind {
                    JsKind::Named(orig) => Some(orig.as_str()),
                    _ => None,
                };
                if let Some(orig) = class.filter(|o| ix.js_owners.contains(&(t, *o))) {
                    // A class: a method it inherits.
                    if let Some(inherited) = bases(orig, &r.name) {
                        ids = inherited;
                        certain = false;
                    }
                } else {
                    // A default-exported instance or object literal, or an
                    // exported object or namespace (`export const api = {
                    // get() {} }`): its members, when they belong to one
                    // owner.
                    ids = ix.js_members_of_one_owner(t, &r.name);
                    certain = false;
                }
            }
            (ids, certain)
        });
        match pick {
            Pick::One(cids, certain) => Some(Scoped::Bound { cids, certain }),
            Pick::Ambiguous => None,
            // Decisive only for `mod.f()` through a namespace binding of a
            // module whose exports cgg can read: anything else — a
            // property path, an instance, a class whose bases may hold the
            // method — keeps the resolver's other readings.
            Pick::Miss => {
                let decisive = rest.is_empty()
                    && !constructed
                    && b.kind == JsKind::Namespace
                    && !b.targets.files.iter().any(|(t, _)| {
                        ix.js_unknown_default(*t) || ix.js_defines_somewhere(*t, &r.name)
                    });
                if !decisive {
                    return None;
                }
                Some(
                    match b
                        .targets
                        .files
                        .iter()
                        .find_map(|(t, _)| ix.js_export_from_dependency(*t, &r.name))
                    {
                        Some(dep) => Scoped::External(dep),
                        None => ix.js_miss(file, b),
                    },
                )
            }
        }
    }

    fn resolve_py(
        &self,
        ix: &ModuleIndex<'_>,
        file: FileId,
        r: &RefRecord,
        rh: &str,
        bases: &dyn Fn(&str, &str) -> Option<Vec<CallableId>>,
    ) -> Option<Scoped> {
        let mut seen = HashSet::new();
        if rh.is_empty() {
            // `from .x import f; f()`: exact, through re-exports; a miss
            // leaves the call to the resolver's other steps.
            if r.dot_call || self.own_defs.contains(r.name.as_str()) {
                return None;
            }
            let path = self.py.get(r.name.as_str())?;
            let mut segs: Vec<String> = path.split('.').map(str::to_string).collect();
            let orig = segs.pop()?;
            return ix
                .py_path_lookup(file, &segs, &orig, 0, &mut seen)
                .filter(|ids| !ids.is_empty())
                .map(|cids| Scoped::Bound {
                    cids,
                    certain: true,
                });
        }
        let head = leading_ident(rh);
        let bound = self.py.get(head)?;
        let mut abs: Vec<String> = bound.split('.').map(str::to_string).collect();
        // Is the binding itself a module (`import pkg`, `from . import
        // helpers`), or a name taken out of one (`from .jobs import
        // Runner`, or `from app import email_service` where the package
        // re-binds `email_service` to an instance)? Only through a module
        // can a miss be a refusal.
        let module_head = ix.py_split(file, &abs).is_some_and(|(_, k)| k == abs.len())
            && !ix.py_package_binds(&abs);
        let (segs, _) = parse_receiver(rh)?;
        // `pkg.Runner().m()`: a call is allowed only as the receiver's
        // last segment — the instance a class builds; any other call is a
        // value of unknown type.
        if segs[..segs.len() - 1].iter().any(|s| s.called) {
            return None;
        }
        abs.extend(segs[1..].iter().map(|s| s.ident.to_string()));
        let (files, k) = ix.py_split(file, &abs)?;
        let rest = &abs[k..];
        let mut found: Vec<Vec<CallableId>> = Vec::new();
        for t in &files {
            let ids = ix.py_lookup(*t, rest, &r.name, 0, &mut seen);
            if !ids.is_empty() && !found.contains(&ids) {
                found.push(ids);
            }
        }
        if found.len() == 1 {
            return Some(Scoped::Bound {
                cids: found.pop().unwrap_or_default(),
                certain: true,
            });
        }
        if !found.is_empty() {
            return None;
        }
        // The path ends in a class the module declares: its bases first.
        let class_based = files.iter().find_map(|t| {
            let p = ix.py_prefix(*t)?;
            ix.py_classes
                .get(format!("{p}.{}", rest.join(".")).as_str())
                .copied()
        });
        if let Some(based) = class_based
            && let Some(class) = rest.last()
        {
            if let Some(inherited) = bases(class, &r.name) {
                return Some(Scoped::Bound {
                    cids: inherited,
                    certain: false,
                });
            }
            // A class with no bases: its method set is what it declares.
            return (module_head && !based).then(|| Scoped::Refused(bound.clone()));
        }
        // `mod.f()` with the module itself as the receiver is decisive; a
        // path through a module-level value (`mod.instance.m()`) is not.
        (module_head && rest.is_empty()).then(|| Scoped::Refused(bound.clone()))
    }

    fn resolve_go(
        &self,
        ix: &ModuleIndex<'_>,
        file: FileId,
        r: &RefRecord,
        rh: &str,
        bases: &dyn Fn(&str, &str) -> Option<Vec<CallableId>>,
    ) -> Option<Scoped> {
        if rh.is_empty() {
            // A bare name in Go is the calling package's own: the files of
            // its directory under the same package clause.
            if r.dot_call {
                return None;
            }
            let ids = ix.go_same_package(file, &r.name);
            return (!ids.is_empty()).then_some(Scoped::Bound {
                cids: ids,
                certain: true,
            });
        }
        let head = leading_ident(rh);
        let path = self.go.get(head)?;
        // A parameter or local named like the import (`c *app.Ctx` in a
        // file importing `c ".../client"`) is the receiver here.
        if self.go_shadow.get(head).is_some_and(|spans| {
            spans
                .iter()
                .any(|(s, e)| *s <= r.site_byte && r.site_byte < *e)
        }) {
            return None;
        }
        let (segs, _) = parse_receiver(rh)?;
        // In module mode an import path no `go.mod` of the tree covers is
        // a dependency (`reflect`), even when a directory shares its name.
        if !ix.go_by_path.contains_key(path.as_str())
            && self.go_has_module
            && !ix.go_modules.iter().any(|m| {
                path == m || path.strip_prefix(m).is_some_and(|r| r.starts_with('/'))
            })
        {
            return Some(Scoped::External(path.clone()));
        }
        let files = ix.go_files(path);
        if files.is_empty() {
            return Some(Scoped::Refused(path.clone()));
        }
        let found = |ids: Vec<CallableId>, certain: bool| {
            (!ids.is_empty()).then_some(Scoped::Bound { cids: ids, certain })
        };
        match &segs[1..] {
            // `jobs.SpawnJob()`: the package itself is the receiver — the
            // one decisive miss.
            [] if !segs[0].called => {
                let ids = ix.go_lookup(&files, &r.name);
                Some(found(ids, true).unwrap_or_else(|| Scoped::Refused(path.clone())))
            }
            // `jobs.Runner` (a typed receiver): its method, else one its
            // bases declare. A package-level variable (`ttl.CacheDefault`)
            // or a method an embedded field promotes is not provably
            // absent, so a miss keeps the other readings.
            [t] if !t.called => {
                let ids = ix.go_lookup(&files, &format!("{}.{}", t.ident, r.name));
                found(ids, true)
                    .or_else(|| bases(t.ident, &r.name).and_then(|b| found(b, false)))
            }
            // `jobs.NewRunner()`: the method of the type the function
            // declares it returns.
            [f] => {
                let callee = ix.go_lookup(&files, f.ident);
                let (cid, ty) = callee.iter().find_map(|c| {
                    ix.go_sig
                        .get(c)
                        .and_then(|s| go_result_type(s))
                        .map(|t| (*c, t))
                })?;
                match ty.split_once('.') {
                    // The function's own package.
                    None => {
                        let ids = ix.go_lookup(&files, &format!("{ty}.{}", r.name));
                        found(ids, true)
                            .or_else(|| bases(&ty, &r.name).and_then(|b| found(b, false)))
                    }
                    // Another package, read through the callee's imports.
                    Some((q, t)) => {
                        let callee_file = ix.file_of.get(&cid)?;
                        let (_, qpath, external) = ix
                            .go_imports
                            .get(callee_file)?
                            .iter()
                            .find(|(n, _, _)| n == q)?;
                        if *external {
                            return Some(Scoped::External(qpath.clone()));
                        }
                        let qfiles = ix.go_files(qpath);
                        found(ix.go_lookup(&qfiles, &format!("{t}.{}", r.name)), true)
                    }
                }
            }
            _ => None,
        }
    }
}

/// A JS/TS module no analyzed file is: a relative specifier names a file
/// of the tree that was not analyzed (refused); a bare one names a package
/// the tree does not contain — a dependency (`next/headers`, `lexical`),
/// whatever directory shares its first segment.
fn js_unfound(spec: &str) -> Scoped {
    let relative =
        spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == "..";
    if relative {
        Scoped::Refused(spec.to_string())
    } else {
        Scoped::External(spec.to_string())
    }
}

/// The parameter names of a signature's parameter list (`def f(self, a,
/// b=1)`, `function f(a, {b})`, `(c: Ctx, ...rest) =>`): each piece's
/// leading identifier.
fn param_names(sig: &str) -> Vec<&str> {
    let Some(open) = sig.find('(') else {
        return Vec::new();
    };
    let Some(len) = group_end(&sig[open..], b'(', b')') else {
        return Vec::new();
    };
    let inner = &sig[open + 1..open + len];
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let bytes = inner.as_bytes();
    for i in 0..=bytes.len() {
        let at_end = i == bytes.len();
        if !at_end {
            match bytes[i] {
                b'(' | b'[' | b'{' | b'<' => depth += 1,
                b')' | b']' | b'}' | b'>' => depth -= 1,
                _ => {}
            }
        }
        if at_end || (bytes[i] == b',' && depth == 0) {
            let piece = inner[start..i].trim().trim_start_matches(['*', '.', '&']);
            let ident = leading_ident(piece);
            if !ident.is_empty() {
                out.push(ident);
            }
            start = i + 1;
        }
    }
    out
}

/// The names a Go import binds: its alias, or (unaliased) the path's last
/// element and the package name it implies. `_` and `.` bind none.
fn go_import_names(path: &str, alias: &str) -> Vec<String> {
    if !alias.is_empty() {
        return if alias == "_" || alias == "." {
            Vec::new()
        } else {
            vec![alias.to_string()]
        };
    }
    let raw_last = path.rsplit('/').next().unwrap_or(path).to_string();
    let pkg = crate::heads::go_package_name(path);
    if pkg == raw_last {
        vec![raw_last]
    } else {
        vec![raw_last, pkg]
    }
}

/// `[head, rest…]`.
fn with_head(head: &str, rest: &[String]) -> Vec<String> {
    let mut p = vec![head.to_string()];
    p.extend(rest.iter().cloned());
    p
}

/// `orig as alias` → `(orig, alias)`; `name` → `(name, name)`.
fn split_as(item: &str) -> (&str, &str) {
    match item.split_once(" as ") {
        Some((o, a)) => (o.trim(), a.trim()),
        None => (item.trim(), item.trim()),
    }
}

/// The leading identifier of a receiver expression.
fn leading_ident(rh: &str) -> &str {
    let end = rh
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
        .map_or(rh.len(), |(i, _)| i);
    &rh[..end]
}

/// A receiver written as a dotted path of identifiers, each optionally
/// followed by one argument list (`pkg.sub.Runner()`, `jobs.NewRunner()`).
/// `None` for anything else — an index, a literal, a nested expression.
/// The flag says whether any segment is called.
fn parse_receiver(rh: &str) -> Option<(Vec<Seg<'_>>, bool)> {
    let bytes = rh.as_bytes();
    let mut segs = Vec::new();
    let mut i = 0usize;
    let mut any_call = false;
    loop {
        let start = i;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'$'))
        {
            i += 1;
        }
        // Non-ASCII identifiers: take the run of non-separator chars.
        while i < bytes.len() && bytes[i] >= 0x80 {
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || matches!(bytes[i], b'_' | b'$')
                    || bytes[i] >= 0x80)
            {
                i += 1;
            }
        }
        if i == start {
            return None;
        }
        let ident = rh.get(start..i)?;
        let mut called = false;
        if i < bytes.len() && bytes[i] == b'(' {
            let mut depth = 0i32;
            let mut j = i;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            if depth != 0 {
                return None;
            }
            i = j + 1;
            called = true;
            any_call = true;
        }
        segs.push(Seg { ident, called });
        if i == bytes.len() {
            break;
        }
        if bytes[i] != b'.' {
            return None;
        }
        i += 1;
    }
    Some((segs, any_call))
}

/// The type a Go function declares it returns, unwrapped to a nominal
/// name: `func NewRunner() *Runner` → `Runner`, `func Open(p string)
/// (*DB, error)` → `DB`, `func New() *pkg.T[K]` → `pkg.T`.
fn go_result_type(sig: &str) -> Option<String> {
    let s = sig.trim();
    let s = s.strip_prefix("func")?.trim_start();
    // Skip a method receiver.
    let s = if s.starts_with('(') {
        &s[group_end(s, b'(', b')')? + 1..]
    } else {
        s
    };
    let s = s.trim_start();
    // Name and type parameters.
    let name_end = s.find(['(', '['])?;
    let mut rest = &s[name_end..];
    if rest.starts_with('[') {
        rest = &rest[group_end(rest, b'[', b']')? + 1..];
    }
    let rest = rest.trim_start();
    if !rest.starts_with('(') {
        return None;
    }
    let after = rest[group_end(rest, b'(', b')')? + 1..].trim();
    let after = after.split('{').next().unwrap_or("").trim();
    let first = if let Some(inner) = after.strip_prefix('(') {
        let end = inner.rfind(')')?;
        let inner = &inner[..end];
        let first = inner.split(',').next().unwrap_or("").trim();
        // `(r *Runner, err error)`: a named result.
        first.rsplit(' ').next().unwrap_or(first).trim()
    } else {
        after
    };
    let ty = first.trim_start_matches('*').trim();
    let ty = ty.split('[').next().unwrap_or(ty).trim();
    let ok = !ty.is_empty()
        && ty
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    ok.then(|| ty.to_string())
}

/// Index of the delimiter closing the group `s` opens with `open`.
fn group_end(s: &str, open: u8, close: u8) -> Option<usize> {
    let mut depth = 0i32;
    for (i, b) in s.bytes().enumerate() {
        if b == open {
            depth += 1;
        } else if b == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

fn strip_js_ext(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, ext)) if JS_EXTS.contains(&ext) && !stem.is_empty() => stem,
        _ => name,
    }
}

/// Resolve `.` and `..` without touching the filesystem; a `..` above the
/// first component is kept.
pub(crate) fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn normal_components(p: &Path) -> Vec<String> {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str().map(str::to_string),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(rh: &str) -> Option<Vec<(String, bool)>> {
        parse_receiver(rh).map(|(s, _)| {
            s.into_iter()
                .map(|x| (x.ident.to_string(), x.called))
                .collect()
        })
    }

    #[test]
    fn receiver_paths_and_the_shapes_that_are_not_paths() {
        assert_eq!(segs("svc"), Some(vec![("svc".into(), false)]));
        assert_eq!(
            segs("pkg.sub.Runner(cfg.x, 1)"),
            Some(vec![
                ("pkg".into(), false),
                ("sub".into(), false),
                ("Runner".into(), true)
            ])
        );
        assert_eq!(
            segs("jobs.NewRunner()"),
            Some(vec![("jobs".into(), false), ("NewRunner".into(), true)])
        );
        for not_a_path in ["a[0]", "a.b[1].c", "(x)", "a.", "a..b", "f(", "\"s\"", ""] {
            assert_eq!(segs(not_a_path), None, "{not_a_path}");
        }
    }

    #[test]
    fn go_result_types() {
        let t = |s: &str| go_result_type(s);
        assert_eq!(t("func NewRunner() *Runner").as_deref(), Some("Runner"));
        assert_eq!(t("func Open(p string) (*DB, error)").as_deref(), Some("DB"));
        assert_eq!(
            t("func Open(p string) (db *DB, err error)").as_deref(),
            Some("DB")
        );
        assert_eq!(
            t("func New[K comparable](k K) *Box[K] {").as_deref(),
            Some("Box")
        );
        assert_eq!(
            t("func (s *S) Client() *http.Client").as_deref(),
            Some("http.Client")
        );
        assert_eq!(t("func Do()"), None);
        assert_eq!(t("func Each(f func(int)) []T"), None);
    }

    #[test]
    fn python_relative_imports_are_made_absolute() {
        let mut ix = ModuleIndex::default();
        let (init, module) = (FileId::new(1), FileId::new(2));
        ix.py_module_of.insert(init, ("pkg.sub", true));
        ix.py_module_of.insert(module, ("pkg.sub.mod", false));
        assert_eq!(ix.py_abs(module, ".").as_deref(), Some("pkg.sub"));
        assert_eq!(ix.py_abs(module, ".jobs").as_deref(), Some("pkg.sub.jobs"));
        assert_eq!(ix.py_abs(module, "..core").as_deref(), Some("pkg.core"));
        assert_eq!(ix.py_abs(init, ".jobs").as_deref(), Some("pkg.sub.jobs"));
        assert_eq!(ix.py_abs(module, "....x"), None);
        assert_eq!(ix.py_abs(module, "abs.mod").as_deref(), Some("abs.mod"));
    }
}
