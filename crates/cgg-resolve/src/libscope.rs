//! Library scope for languages whose imports name files (WS14).
//!
//! Dart: a file sees its own library's top-level names (itself and its
//! `part` files), and the names each `import` brings in — the imported
//! library's top-level declarations and everything it `export`s, filtered
//! by `show` / `hide` — unless the import has a prefix (`as p`), when they
//! are reached only as `p.name`. `package:<name>/x.dart` is
//! `<root>/lib/x.dart` of the `pubspec.yaml` declaring `<name>`; a package
//! no pubspec in the tree declares, and `dart:` libraries, are
//! dependencies.
//!
//! A call this scope decides is bound only to what the scope holds, or
//! refused (recorded): never matched by name against the rest of the tree,
//! which bound `expect()` from `package:test` to whichever workspace file
//! declared an `expect`. Every lookup is keyed by file and name; nothing
//! here is built from a list of names.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use cgg_core::{
    FileFacts, RefRecord,
    graph::Graph,
    ids::{CallableId, FileId},
};

use crate::modules::{Scoped, normalize};

/// Export chains deeper than this are not followed.
const MAX_DEPTH: u8 = 16;

/// Where a Dart URI leads from a given file.
#[derive(Debug, Clone, PartialEq)]
enum Target {
    /// An analyzed file of the tree.
    File(FileId),
    /// A library outside the tree (`dart:async`, a package no pubspec of
    /// the tree declares).
    External(String),
    /// A path into the tree that names no analyzed file (generated code,
    /// a file in a language filter's shadow): nothing can be judged.
    Unknown,
}

/// One `import` / `export` directive.
#[derive(Debug, Clone, Default)]
struct Directive {
    uri: String,
    prefix: String,
    show: Option<HashSet<String>>,
    hide: HashSet<String>,
    /// Where `uri` leads, resolved once when the index is built.
    target: Option<Target>,
}

impl Directive {
    fn allows(&self, name: &str) -> bool {
        self.show.as_ref().is_none_or(|s| s.contains(name)) && !self.hide.contains(name)
    }
}

#[derive(Debug, Default)]
struct DartFile {
    imports: Vec<Directive>,
    exports: Vec<Directive>,
    parts: Vec<String>,
    part_of: Option<String>,
    part_of_library: Option<String>,
    /// The library the file belongs to, and that library's files (it and
    /// its parts), resolved once when the index is built.
    library: Option<FileId>,
    library_files: Vec<FileId>,
}

/// A Zig file-level name bound to another file or a path.
#[derive(Debug, Clone)]
enum ZigBind {
    /// `@import("x")`, as written.
    Import(String),
    /// `a.b.c`, a path through this file's other bindings.
    Alias(String),
    /// `@import("x").a.b`: members of an imported file.
    ImportMember(String, String),
}

/// A library's exported namespace: name → the callables it denotes.
type Namespace = HashMap<String, Vec<CallableId>>;

/// Whole-tree index, built once per run.
#[derive(Debug, Default)]
pub(crate) struct LibIndex<'a> {
    dart_by_path: HashMap<PathBuf, FileId>,
    dart_files: HashMap<FileId, DartFile>,
    /// Package name → its root directories (several pubspecs may share a
    /// name — templates, fixtures).
    dart_pkg_roots: HashMap<String, Vec<PathBuf>>,
    /// `library a.b;` name → files declaring it.
    dart_library_names: HashMap<String, Vec<FileId>>,
    /// File → the top-level callables it declares, by simple name.
    dart_top: HashMap<FileId, HashMap<&'a str, Vec<CallableId>>>,
    /// File → every simple name any callable of it carries.
    dart_any: HashMap<FileId, HashSet<&'a str>>,
    paths: HashMap<FileId, &'a Path>,
    /// Zig: normalized path → file.
    zig_by_path: HashMap<PathBuf, FileId>,
    /// Zig: each file's top-level `const NAME = @import(…)` /
    /// `const NAME = a.b` bindings.
    zig_binds: HashMap<FileId, HashMap<String, ZigBind>>,
    /// Zig: modules build scripts declare, `(build dir, name, root)`.
    zig_modules: Vec<(PathBuf, String, PathBuf)>,
    /// Zig: every file stem and directory name of a Zig file.
    zig_names: HashSet<String>,
    /// Zig: file → callables not nested in another, by simple name.
    zig_top: HashMap<FileId, HashMap<&'a str, Vec<CallableId>>>,
    /// Exported namespaces, computed on first use, and whether some
    /// export chain leaves the tree.
    ns_cache: Mutex<HashMap<FileId, (Arc<Namespace>, bool)>>,
}

impl<'a> LibIndex<'a> {
    pub(crate) fn build(graph: &'a Graph, facts: &'a [FileFacts]) -> Self {
        let mut ix = LibIndex::default();
        ix.build_zig(graph, facts);
        if !facts.iter().any(|f| f.language == "dart") {
            return ix;
        }
        // Top-level: a callable no other callable of its file contains,
        // with no owner in its qualified name.
        let mut spans: HashMap<FileId, Vec<(u32, u32)>> = HashMap::new();
        for c in graph.callables.values().filter(|c| c.language == "dart") {
            spans
                .entry(c.file)
                .or_default()
                .push((c.start_byte, c.end_byte));
        }
        let mut ids: Vec<&cgg_core::graph::CallableNode> = graph
            .callables
            .values()
            .filter(|c| c.language == "dart")
            .collect();
        ids.sort_by_key(|c| (c.file, c.start_byte, c.id));
        for c in ids {
            ix.dart_any
                .entry(c.file)
                .or_default()
                .insert(c.simple_name.as_str());
            // A method whose class the plugin missed (a `mixin`, a parse
            // error) also carries a bare qualified name; its kind says
            // what it is.
            if c.qualified_name != c.simple_name
                || matches!(
                    c.kind,
                    cgg_core::graph::CallableKind::Method
                        | cgg_core::graph::CallableKind::Property
                        | cgg_core::graph::CallableKind::Destructor
                )
            {
                continue;
            }
            let nested = spans.get(&c.file).is_some_and(|v| {
                v.iter().any(|&(s, e)| {
                    s <= c.start_byte
                        && c.end_byte <= e
                        && (s, e) != (c.start_byte, c.end_byte)
                })
            });
            if nested {
                continue;
            }
            ix.dart_top
                .entry(c.file)
                .or_default()
                .entry(c.simple_name.as_str())
                .or_default()
                .push(c.id);
        }
        for f in facts.iter().filter(|f| f.language == "dart") {
            ix.paths.insert(f.file, f.path.as_path());
            ix.dart_by_path.entry(normalize(&f.path)).or_insert(f.file);
            let mut df = DartFile::default();
            // Directives in source order; `show`/`hide` follow the
            // directive they belong to.
            let mut last: Option<(bool, usize)> = None;
            for imp in &f.imports {
                let uri = imp.path.trim();
                match imp.kind.as_str() {
                    "import" | "export" => {
                        let d = Directive {
                            uri: uri.to_string(),
                            prefix: imp.alias.trim().to_string(),
                            ..Directive::default()
                        };
                        if imp.kind == "import" {
                            df.imports.push(d);
                            last = Some((true, df.imports.len() - 1));
                        } else {
                            df.exports.push(d);
                            last = Some((false, df.exports.len() - 1));
                        }
                    }
                    "show" | "hide" => {
                        let Some((is_import, i)) = last else { continue };
                        let d = if is_import {
                            &mut df.imports[i]
                        } else {
                            &mut df.exports[i]
                        };
                        if d.uri != uri {
                            continue;
                        }
                        let names = imp
                            .alias
                            .split(',')
                            .map(|n| n.trim().to_string())
                            .filter(|n| !n.is_empty());
                        if imp.kind == "show" {
                            d.show.get_or_insert_with(HashSet::new).extend(names);
                        } else {
                            d.hide.extend(names);
                        }
                    }
                    "part" => df.parts.push(uri.to_string()),
                    "part-of" => df.part_of = Some(uri.to_string()),
                    "part-of-library" => df.part_of_library = Some(uri.to_string()),
                    "library" => ix
                        .dart_library_names
                        .entry(uri.to_string())
                        .or_default()
                        .push(f.file),
                    "package-root" => {
                        if let Some(name) = f.packages.first() {
                            let roots =
                                ix.dart_pkg_roots.entry(name.clone()).or_default();
                            let root = normalize(Path::new(uri));
                            if !roots.contains(&root) {
                                roots.push(root);
                            }
                        }
                    }
                    _ => {}
                }
            }
            ix.dart_files.insert(f.file, df);
        }
        for roots in ix.dart_pkg_roots.values_mut() {
            roots.sort();
        }
        // Resolve every directive's target once.
        let mut files: Vec<FileId> = ix.dart_files.keys().copied().collect();
        files.sort();
        for f in &files {
            let df = &ix.dart_files[f];
            let imports: Vec<Target> = df
                .imports
                .iter()
                .map(|d| ix.dart_target(*f, &d.uri))
                .collect();
            let exports: Vec<Target> = df
                .exports
                .iter()
                .map(|d| ix.dart_target(*f, &d.uri))
                .collect();
            let library = ix.dart_library_of(*f);
            if let Some(df) = ix.dart_files.get_mut(f) {
                for (d, t) in df.imports.iter_mut().zip(imports) {
                    d.target = Some(t);
                }
                for (d, t) in df.exports.iter_mut().zip(exports) {
                    d.target = Some(t);
                }
                df.library = Some(library);
            }
        }
        for f in &files {
            let lib_files = ix.dart_library_files_uncached(*f);
            if let Some(df) = ix.dart_files.get_mut(f) {
                df.library_files = lib_files;
            }
        }
        ix
    }

    fn target_of(&self, from: FileId, d: &Directive) -> Target {
        d.target
            .clone()
            .unwrap_or_else(|| self.dart_target(from, &d.uri))
    }

    /// Where `uri`, written in `from`, leads.
    fn dart_target(&self, from: FileId, uri: &str) -> Target {
        let uri = uri.trim();
        if uri.starts_with("dart:") {
            return Target::External(uri.to_string());
        }
        if let Some(rest) = uri.strip_prefix("package:") {
            let (name, sub) = rest.split_once('/').unwrap_or((rest, ""));
            let Some(roots) = self.dart_pkg_roots.get(name) else {
                return Target::External(uri.to_string());
            };
            // Several packages of one name: the one the importer sits in,
            // else the one sharing the longest directory prefix with it.
            let here = self.paths.get(&from).map(|p| normalize(p));
            let root = roots
                .iter()
                .max_by_key(|r| {
                    here.as_ref().map_or(0, |h| {
                        h.components()
                            .zip(r.components())
                            .take_while(|(a, b)| a == b)
                            .count()
                    })
                })
                .cloned();
            let Some(root) = root else {
                return Target::Unknown;
            };
            return match self
                .dart_by_path
                .get(&normalize(&root.join("lib").join(sub)))
            {
                Some(f) => Target::File(*f),
                None => Target::Unknown,
            };
        }
        if uri.contains(':') {
            // `http:`, `file:`: not a library of the tree.
            return Target::External(uri.to_string());
        }
        let Some(dir) = self.paths.get(&from).and_then(|p| p.parent()) else {
            return Target::Unknown;
        };
        match self.dart_by_path.get(&normalize(&dir.join(uri))) {
            Some(f) => Target::File(*f),
            None => Target::Unknown,
        }
    }

    /// The library a file belongs to: its own file, or the file its
    /// `part of` names.
    fn dart_library_of(&self, f: FileId) -> FileId {
        let Some(df) = self.dart_files.get(&f) else {
            return f;
        };
        if let Some(lib) = df.library {
            return lib;
        }
        if let Some(uri) = &df.part_of
            && let Target::File(lib) = self.dart_target(f, uri)
        {
            return lib;
        }
        if let Some(name) = &df.part_of_library
            && let Some([lib]) = self.dart_library_names.get(name).map(Vec::as_slice)
        {
            return *lib;
        }
        f
    }

    /// The files of library `lib`: itself and its parts.
    fn dart_library_files(&self, lib: FileId) -> Vec<FileId> {
        match self.dart_files.get(&lib) {
            Some(df) if !df.library_files.is_empty() => df.library_files.clone(),
            _ => self.dart_library_files_uncached(lib),
        }
    }

    fn dart_library_files_uncached(&self, lib: FileId) -> Vec<FileId> {
        let mut out = vec![lib];
        if let Some(df) = self.dart_files.get(&lib) {
            for p in &df.parts {
                if let Target::File(f) = self.dart_target(lib, p)
                    && !out.contains(&f)
                {
                    out.push(f);
                }
            }
        }
        out
    }

    /// What library `lib` exports: its own top-level declarations and,
    /// through `export`, everything the libraries it re-exports do
    /// (filtered by `show`/`hide`). Whether some export left the tree is
    /// returned too: a name not found may come from there.
    fn dart_namespace(&self, lib: FileId) -> (Arc<Namespace>, bool) {
        if let Ok(cache) = self.ns_cache.lock()
            && let Some(ns) = cache.get(&lib)
        {
            return ns.clone();
        }
        let mut ns = Namespace::new();
        let mut seen = HashSet::new();
        self.dart_collect(lib, &mut ns, &mut seen, &|_| true, 0);
        let outside = self.dart_exports_outside(lib, &mut HashSet::new(), 0);
        let ns = (Arc::new(ns), outside);
        if let Ok(mut cache) = self.ns_cache.lock() {
            cache.entry(lib).or_insert_with(|| ns.clone());
        }
        ns
    }

    fn dart_collect(
        &self,
        lib: FileId,
        ns: &mut Namespace,
        seen: &mut HashSet<FileId>,
        allow: &dyn Fn(&str) -> bool,
        depth: u8,
    ) {
        if depth > MAX_DEPTH || !seen.insert(lib) {
            return;
        }
        for f in self.dart_library_files(lib) {
            if let Some(top) = self.dart_top.get(&f) {
                let mut names: Vec<(&&str, &Vec<CallableId>)> = top.iter().collect();
                names.sort_by_key(|(n, _)| **n);
                for (name, cids) in names {
                    if allow(name) {
                        let slot = ns.entry((*name).to_string()).or_default();
                        for c in cids {
                            if !slot.contains(c) {
                                slot.push(*c);
                            }
                        }
                    }
                }
            }
        }
        let Some(df) = self.dart_files.get(&lib) else {
            return;
        };
        for e in &df.exports {
            if let Target::File(t) = self.target_of(lib, e) {
                let both = |n: &str| allow(n) && e.allows(n);
                self.dart_collect(t, ns, seen, &both, depth + 1);
            }
        }
    }

    /// Whether some export chain from `lib` leaves the tree (or names a
    /// file cgg did not analyze).
    fn dart_exports_outside(
        &self,
        lib: FileId,
        seen: &mut HashSet<FileId>,
        depth: u8,
    ) -> bool {
        if depth > MAX_DEPTH || !seen.insert(lib) {
            return false;
        }
        let Some(df) = self.dart_files.get(&lib) else {
            return false;
        };
        df.exports.iter().any(|e| match self.target_of(lib, e) {
            Target::File(t) => self.dart_exports_outside(t, seen, depth + 1),
            _ => true,
        })
    }

    /// Library-scope resolution of one Dart call. `None`: not a call this
    /// scope can judge; the resolver's other steps apply.
    pub(crate) fn resolve_dart(
        &self,
        file: FileId,
        r: &RefRecord,
        caller_qn: Option<&str>,
    ) -> Option<Scoped> {
        let df = self.dart_files.get(&file)?;
        let rh = r.receiver_hint.trim();
        if rh.is_empty() {
            if r.dot_call {
                return None;
            }
            // The file's own declarations are intra-file's to weigh.
            if self
                .dart_any
                .get(&file)
                .is_some_and(|names| names.contains(r.name.as_str()))
            {
                return None;
            }
            // Another part of the same library.
            let lib = self.dart_library_of(file);
            let mut own: Vec<CallableId> = Vec::new();
            for f in self.dart_library_files(lib) {
                if let Some(cids) =
                    self.dart_top.get(&f).and_then(|t| t.get(r.name.as_str()))
                {
                    own.extend(cids.iter().copied());
                }
            }
            if !own.is_empty() {
                return Some(Scoped::Bound {
                    cids: own,
                    certain: true,
                });
            }
            // A part's imports are its library's.
            let lib_df = self.dart_files.get(&lib).unwrap_or(df);
            let import_file = if lib_df.imports.is_empty() { file } else { lib };
            let imports = if lib_df.imports.is_empty() {
                &df.imports
            } else {
                &lib_df.imports
            };
            let mut found: Vec<Vec<CallableId>> = Vec::new();
            let mut external: Option<String> = None;
            let mut unknown = false;
            for d in imports.iter().filter(|d| d.prefix.is_empty()) {
                if !d.allows(&r.name) {
                    continue;
                }
                match self.target_of(import_file, d) {
                    Target::File(t) => {
                        let (ns, outside) = self.dart_namespace(t);
                        if let Some(cids) = ns.get(r.name.as_str()) {
                            if !found.contains(cids) {
                                found.push(cids.clone());
                            }
                        } else if outside {
                            external.get_or_insert_with(|| d.uri.clone());
                        }
                    }
                    Target::External(u) => {
                        external.get_or_insert(u);
                    }
                    Target::Unknown => unknown = true,
                }
            }
            return match found.len() {
                0 if unknown => None,
                // A member's bare call may name an inherited member; the
                // plugin records no bases to tell.
                0 if caller_qn.is_some_and(|q| q.contains("::")) => None,
                // `dart:core` is imported implicitly.
                0 => Some(Scoped::External(
                    external.unwrap_or_else(|| "dart:core".to_string()),
                )),
                1 => Some(Scoped::Bound {
                    cids: found.pop().unwrap_or_default(),
                    certain: true,
                }),
                _ => Some(Scoped::Bound {
                    cids: found.into_iter().flatten().collect(),
                    certain: false,
                }),
            };
        }
        // `p.f()` through an import prefix.
        let lib = self.dart_library_of(file);
        let lib_df = self.dart_files.get(&lib).unwrap_or(df);
        let (import_file, imports) = if lib_df.imports.is_empty() {
            (file, &df.imports)
        } else {
            (lib, &lib_df.imports)
        };
        let head = rh.split('.').next().unwrap_or(rh);
        let prefixed: Vec<&Directive> =
            imports.iter().filter(|d| d.prefix == head).collect();
        if prefixed.is_empty() {
            return None;
        }
        let mut found: Vec<Vec<CallableId>> = Vec::new();
        let mut external: Option<String> = None;
        let mut unknown = false;
        let mut first_ws: Option<String> = None;
        for d in &prefixed {
            match self.target_of(import_file, d) {
                Target::File(t) => {
                    first_ws.get_or_insert_with(|| d.uri.clone());
                    if rh != head {
                        // `p.Type.m()`: a member, not judged here.
                        unknown = true;
                        continue;
                    }
                    if !d.allows(&r.name) {
                        continue;
                    }
                    let (ns, outside) = self.dart_namespace(t);
                    if let Some(cids) = ns.get(r.name.as_str()) {
                        if !found.contains(cids) {
                            found.push(cids.clone());
                        }
                    } else if outside {
                        external.get_or_insert_with(|| d.uri.clone());
                    }
                }
                Target::External(u) => {
                    external.get_or_insert(u);
                }
                Target::Unknown => unknown = true,
            }
        }
        match found.len() {
            1 => Some(Scoped::Bound {
                cids: found.pop().unwrap_or_default(),
                certain: true,
            }),
            n if n > 1 => Some(Scoped::Bound {
                cids: found.into_iter().flatten().collect(),
                certain: false,
            }),
            _ if unknown => None,
            _ => match (external, first_ws) {
                (Some(u), _) => Some(Scoped::External(u)),
                (None, Some(u)) => Some(Scoped::Refused(u)),
                (None, None) => None,
            },
        }
    }
}

impl<'a> LibIndex<'a> {
    fn build_zig(&mut self, graph: &'a Graph, facts: &'a [FileFacts]) {
        if !facts.iter().any(|f| f.language == "zig") {
            return;
        }
        let mut spans: HashMap<FileId, Vec<(u32, u32)>> = HashMap::new();
        for c in graph.callables.values().filter(|c| c.language == "zig") {
            spans
                .entry(c.file)
                .or_default()
                .push((c.start_byte, c.end_byte));
        }
        let mut ids: Vec<&cgg_core::graph::CallableNode> = graph
            .callables
            .values()
            .filter(|c| c.language == "zig")
            .collect();
        ids.sort_by_key(|c| (c.file, c.start_byte, c.id));
        for c in ids {
            let nested = spans.get(&c.file).is_some_and(|v| {
                v.iter().any(|&(s, e)| {
                    s <= c.start_byte
                        && c.end_byte <= e
                        && (s, e) != (c.start_byte, c.end_byte)
                })
            });
            if !nested {
                self.zig_top
                    .entry(c.file)
                    .or_default()
                    .entry(c.simple_name.as_str())
                    .or_default()
                    .push(c.id);
            }
        }
        for f in facts.iter().filter(|f| f.language == "zig") {
            self.paths.insert(f.file, f.path.as_path());
            self.zig_by_path.entry(normalize(&f.path)).or_insert(f.file);
            if let Some(stem) = f.path.file_stem() {
                self.zig_names.insert(stem.to_string_lossy().into_owned());
            }
            if let Some(dir) = f.path.parent() {
                for c in dir.components() {
                    if let std::path::Component::Normal(n) = c {
                        self.zig_names.insert(n.to_string_lossy().into_owned());
                    }
                }
            }
            let mut binds: HashMap<String, ZigBind> = HashMap::new();
            for imp in &f.imports {
                let name = imp.alias.trim();
                match imp.kind.as_str() {
                    "import" if !name.is_empty() => {
                        binds.insert(
                            name.to_string(),
                            ZigBind::Import(imp.path.trim().into()),
                        );
                    }
                    "alias" if !name.is_empty() => {
                        binds.insert(
                            name.to_string(),
                            ZigBind::Alias(imp.path.trim().into()),
                        );
                    }
                    "import-member" if !name.is_empty() => {
                        if let Some((spec, members)) = imp.path.trim().split_once('#') {
                            binds.insert(
                                name.to_string(),
                                ZigBind::ImportMember(
                                    spec.to_string(),
                                    members.to_string(),
                                ),
                            );
                        }
                    }
                    "zig-module" => {
                        if let Some(dir) = f.path.parent() {
                            self.zig_modules.push((
                                normalize(dir),
                                name.to_string(),
                                normalize(&dir.join(imp.path.trim())),
                            ));
                        }
                    }
                    _ => {}
                }
            }
            self.zig_binds.insert(f.file, binds);
        }
        self.zig_modules.sort();
        self.zig_modules.dedup();
    }

    /// The module names the tree's build scripts declare.
    pub(crate) fn zig_module_names(&self) -> impl Iterator<Item = &str> {
        self.zig_modules.iter().map(|(_, n, _)| n.as_str())
    }

    /// Where `@import(spec)`, written in `from`, leads.
    fn zig_target(&self, from: FileId, spec: &str) -> Target {
        let spec = spec.trim();
        let here = self.paths.get(&from).map(|p| normalize(p));
        if spec.ends_with(".zig") || spec.ends_with(".zon") {
            let Some(dir) = here
                .as_ref()
                .and_then(|p| p.parent().map(Path::to_path_buf))
            else {
                return Target::Unknown;
            };
            return match self.zig_by_path.get(&normalize(&dir.join(spec))) {
                Some(f) => Target::File(*f),
                None => Target::Unknown,
            };
        }
        // A module a build script of the tree declares: the nearest one
        // above the importer, else the one declaration of the name.
        let declared: Vec<&(PathBuf, String, PathBuf)> = self
            .zig_modules
            .iter()
            .filter(|(_, n, _)| n == spec)
            .collect();
        let nearest = declared
            .iter()
            .filter(|(dir, _, _)| here.as_ref().is_some_and(|h| h.starts_with(dir)))
            .max_by_key(|(dir, _, _)| dir.components().count())
            .or(match declared.as_slice() {
                [one] => Some(one),
                _ => None,
            });
        if let Some((_, _, root)) = nearest {
            return match self.zig_by_path.get(root) {
                Some(f) => Target::File(*f),
                None => Target::Unknown,
            };
        }
        if !declared.is_empty() {
            return Target::Unknown;
        }
        // A module whose root file is named after its directory
        // (`lib/std/std.zig` in the Zig tree itself).
        let rooted: Vec<FileId> = self
            .zig_by_path
            .iter()
            .filter(|(p, _)| {
                p.file_stem().is_some_and(|s| s == spec)
                    && p.parent()
                        .and_then(|d| d.file_name())
                        .is_some_and(|d| d == spec)
            })
            .map(|(_, f)| *f)
            .collect();
        match rooted.as_slice() {
            [one] => Target::File(*one),
            [] if !self.zig_names.contains(spec) => Target::External(spec.to_string()),
            _ => Target::Unknown,
        }
    }

    /// Module-path resolution of a Zig call written through the file's
    /// imports (`std.debug.print()`, `link.addRunArtifact()`): each
    /// segment a file-level `@import` or alias of the file reached so
    /// far, the call a function declared in the file the path ends in.
    /// `None` when the path enters a type or value, or nothing decides.
    pub(crate) fn resolve_zig(&self, file: FileId, r: &RefRecord) -> Option<Scoped> {
        let rh = r.receiver_hint.trim();
        if rh.is_empty() {
            return None;
        }
        let segs: Vec<&str> = rh.split('.').collect();
        if !segs
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'))
        {
            return None;
        }
        let segs: Vec<String> = segs.into_iter().map(str::to_string).collect();
        let (g, _) = self.zig_walk(file, &segs, 0)?;
        let mut g = match g {
            Target::File(g) => g,
            Target::External(m) => return Some(Scoped::External(m)),
            Target::Unknown => return None,
        };
        let mut name = r.name.clone();
        // The name may itself be a re-export of the file reached:
        // `pub const checksum = @import("vsr/checksum.zig").checksum;`.
        for _ in 0..MAX_DEPTH {
            if let Some(cands) = self.zig_top.get(&g).and_then(|t| t.get(name.as_str())) {
                return match cands.as_slice() {
                    [_] => Some(Scoped::Bound {
                        cids: cands.clone(),
                        certain: true,
                    }),
                    _ => None,
                };
            }
            let path: Vec<String> = match self.zig_binds.get(&g)?.get(&name)? {
                ZigBind::Alias(p) => p.split('.').map(str::to_string).collect(),
                ZigBind::ImportMember(spec, members) => {
                    // Resolved from `g`: the import spec, then the members.
                    let mut p = vec![format!("@{spec}")];
                    p.extend(members.split('.').map(str::to_string));
                    p
                }
                ZigBind::Import(_) => return None,
            };
            let (last, prefix) = path.split_last()?;
            let target =
                if let Some(spec) = prefix.first().and_then(|h| h.strip_prefix('@')) {
                    match self.zig_target(g, spec) {
                        Target::File(f) if prefix.len() == 1 => Target::File(f),
                        Target::File(f) => self.zig_walk(f, &prefix[1..], 0)?.0,
                        other => other,
                    }
                } else {
                    self.zig_walk(g, prefix, 0)?.0
                };
            match target {
                Target::File(f) => {
                    g = f;
                    name = last.clone();
                }
                Target::External(m) => return Some(Scoped::External(m)),
                Target::Unknown => return None,
            }
        }
        None
    }

    /// Follow `segs` from `file` through file-level bindings to the file
    /// (or dependency) they name, with the import that started the path.
    fn zig_walk(
        &self,
        file: FileId,
        segs: &[String],
        depth: u8,
    ) -> Option<(Target, String)> {
        if depth > MAX_DEPTH {
            return None;
        }
        let (head, rest) = segs.split_first()?;
        match self.zig_binds.get(&file)?.get(head)? {
            ZigBind::Alias(path) => {
                let mut expanded: Vec<String> =
                    path.split('.').map(str::to_string).collect();
                if expanded.first() == Some(head) {
                    return None;
                }
                expanded.extend(rest.iter().cloned());
                self.zig_walk(file, &expanded, depth + 1)
            }
            ZigBind::Import(spec) => match self.zig_target(file, spec) {
                Target::File(g) if rest.is_empty() => {
                    Some((Target::File(g), spec.clone()))
                }
                Target::File(g) => self.zig_walk(g, rest, depth + 1),
                other => Some((other, spec.clone())),
            },
            ZigBind::ImportMember(spec, members) => match self.zig_target(file, spec) {
                Target::File(g) => {
                    let mut path: Vec<String> =
                        members.split('.').map(str::to_string).collect();
                    path.extend(rest.iter().cloned());
                    self.zig_walk(g, &path, depth + 1)
                }
                other => Some((other, spec.clone())),
            },
        }
    }
}

/// PHP: what the tree declares, by fully qualified name (lowercase: PHP
/// class and function names are case-insensitive).
#[derive(Debug, Default)]
pub(crate) struct PhpIndex {
    /// File → the file names its `require`/`include` statements name.
    requires: HashMap<FileId, Vec<String>>,
    /// File name → files of that name.
    by_basename: HashMap<String, Vec<FileId>>,
    /// The file each PHP callable is declared in.
    file_of: HashMap<CallableId, FileId>,
    /// Class / interface / trait / enum FQN → declaring files.
    classes: HashMap<String, Vec<FileId>>,
    /// Every declared namespace and each of its ancestors.
    namespaces: HashSet<String>,
    /// `(file, lowercase class short name, lowercase method)` → callables.
    methods: HashMap<(FileId, String, String), Vec<CallableId>>,
    /// Function FQN → callables.
    functions: HashMap<String, Vec<CallableId>>,
    /// Lowercase simple names of every PHP function the tree declares.
    function_names: HashSet<String>,
    /// File → `(byte, namespace)`, in source order.
    file_ns: HashMap<FileId, Vec<(u32, String)>>,
    /// File → `use` table: lowercase alias → FQN.
    uses: HashMap<FileId, HashMap<String, String>>,
    /// `(file, lowercase class short name)` → its declared bases as
    /// written.
    bases: HashMap<(FileId, String), Vec<String>>,
}

fn php_ns_at(ns: &[(u32, String)], byte: u32) -> &str {
    ns.iter()
        .rev()
        .find(|(b, _)| *b <= byte)
        .or(ns.first())
        .map_or("", |(_, n)| n.as_str())
}

fn php_join(ns: &str, name: &str) -> String {
    let name = name.trim_start_matches('\\');
    if ns.is_empty() {
        name.to_ascii_lowercase()
    } else {
        format!("{ns}\\{name}").to_ascii_lowercase()
    }
}

impl PhpIndex {
    pub(crate) fn build(graph: &Graph, facts: &[FileFacts]) -> Self {
        let mut ix = PhpIndex::default();
        for f in facts.iter().filter(|f| f.language == "php") {
            let mut ns: Vec<(u32, String)> = Vec::new();
            let mut uses: HashMap<String, String> = HashMap::new();
            for imp in &f.imports {
                let path = imp.path.trim().trim_start_matches('\\');
                match imp.kind.as_str() {
                    "package" => {
                        ns.push((imp.site_byte, path.to_string()));
                        let mut cut = path.to_ascii_lowercase();
                        loop {
                            ix.namespaces.insert(cut.clone());
                            match cut.rsplit_once('\\') {
                                Some((h, _)) => cut = h.to_string(),
                                None => break,
                            }
                        }
                    }
                    "use" if !path.is_empty() => {
                        let alias = if imp.alias.trim().is_empty() {
                            path.rsplit('\\').next().unwrap_or(path)
                        } else {
                            imp.alias.trim()
                        };
                        uses.insert(
                            alias.to_ascii_lowercase(),
                            path.to_ascii_lowercase(),
                        );
                    }
                    _ => {}
                }
            }
            for c in &f.classes {
                let files = ix
                    .classes
                    .entry(c.class_qn.to_ascii_lowercase())
                    .or_default();
                if !files.contains(&f.file) {
                    files.push(f.file);
                }
            }
            for d in &f.definitions {
                let Some((owner, _)) = d.qualified_name.split_once("::") else {
                    continue;
                };
                // A class with methods is declared whether or not the
                // plugin wrote its record.
                let fqn = php_join(php_ns_at(&ns, d.start_byte), owner);
                let files = ix.classes.entry(fqn).or_default();
                if !files.contains(&f.file) {
                    files.push(f.file);
                }
                if !d.base_types.is_empty() {
                    ix.bases
                        .entry((f.file, owner.to_ascii_lowercase()))
                        .or_insert_with(|| d.base_types.clone());
                }
            }
            ix.file_ns.insert(f.file, ns);
            ix.uses.insert(f.file, uses);
            if let Some(n) = f.path.file_name() {
                ix.by_basename
                    .entry(n.to_string_lossy().into_owned())
                    .or_default()
                    .push(f.file);
            }
            let reqs: Vec<String> = f
                .imports
                .iter()
                .filter(|i| {
                    i.kind == "php-require"
                        || i.kind.starts_with("require")
                        || i.kind.starts_with("include")
                })
                .filter_map(|i| {
                    let t = i.path.trim().trim_end_matches(['\'', '"', ')', ';', ' ']);
                    let base = t.rsplit(['/', '\'', '"']).next()?.trim();
                    (!base.is_empty()).then(|| base.to_string())
                })
                .collect();
            if !reqs.is_empty() {
                ix.requires.insert(f.file, reqs);
            }
        }
        let mut ids: Vec<&cgg_core::graph::CallableNode> = graph
            .callables
            .values()
            .filter(|c| c.language == "php")
            .collect();
        ids.sort_by_key(|c| (c.file, c.start_byte, c.id));
        for c in ids {
            ix.file_of.insert(c.id, c.file);
            match c.qualified_name.split_once("::") {
                Some((owner, m)) => ix
                    .methods
                    .entry((c.file, owner.to_ascii_lowercase(), m.to_ascii_lowercase()))
                    .or_default()
                    .push(c.id),
                None => {
                    let ns = ix
                        .file_ns
                        .get(&c.file)
                        .map_or("", |v| php_ns_at(v, c.start_byte));
                    ix.functions
                        .entry(php_join(ns, &c.simple_name))
                        .or_default()
                        .push(c.id);
                    ix.function_names.insert(c.simple_name.to_ascii_lowercase());
                }
            }
        }
        ix
    }

    /// Definitions found under one name, narrowed when they are copies in
    /// several files (a stub file re-declaring a library's functions for
    /// a boot path): the copy the caller's own file, or the files its
    /// `require`/`include` chain reaches, declares. Copies the chain
    /// cannot tell apart bind nothing ([`Scoped::Ambiguous`]).
    fn one_copy(&self, from: FileId, ids: Vec<CallableId>) -> Scoped {
        let mut files: Vec<FileId> = Vec::new();
        for c in &ids {
            if let Some(f) = self.file_of.get(c)
                && !files.contains(f)
            {
                files.push(*f);
            }
        }
        if files.len() <= 1 {
            let certain = ids.len() == 1;
            return Scoped::Bound { cids: ids, certain };
        }
        // The caller's include closure (its own file first).
        let mut seen: Vec<FileId> = vec![from];
        let mut frontier = vec![from];
        for _ in 0..8 {
            let mut next = Vec::new();
            for f in frontier {
                for base in self.requires.get(&f).into_iter().flatten() {
                    for g in self.by_basename.get(base).into_iter().flatten() {
                        if !seen.contains(g) {
                            seen.push(*g);
                            next.push(*g);
                        }
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        let reached: Vec<FileId> =
            files.iter().copied().filter(|f| seen.contains(f)).collect();
        match reached.as_slice() {
            [one] => {
                let cids: Vec<CallableId> = ids
                    .into_iter()
                    .filter(|c| self.file_of.get(c) == Some(one))
                    .collect();
                let certain = cids.len() == 1;
                Scoped::Bound { cids, certain }
            }
            _ => Scoped::Ambiguous,
        }
    }

    /// A class name as written in `file` at `byte`, made fully qualified
    /// through the file's namespace and `use` table.
    fn class_fqn(&self, file: FileId, byte: u32, name: &str) -> String {
        if let Some(abs) = name.strip_prefix('\\') {
            return abs.to_ascii_lowercase();
        }
        let (head, rest) = match name.split_once('\\') {
            Some((h, r)) => (h, Some(r)),
            None => (name, None),
        };
        if let Some(target) = self
            .uses
            .get(&file)
            .and_then(|u| u.get(&head.to_ascii_lowercase()))
        {
            return match rest {
                Some(r) => format!("{target}\\{}", r.to_ascii_lowercase()),
                None => target.clone(),
            };
        }
        let ns = self.file_ns.get(&file).map_or("", |v| php_ns_at(v, byte));
        php_join(ns, name)
    }

    /// Whether a fully qualified name lies in a namespace the tree
    /// declares (or is a global name, which any file may declare).
    fn in_tree_namespace(&self, fqn: &str) -> bool {
        match fqn.rsplit_once('\\') {
            Some((ns, _)) => self.namespaces.contains(ns),
            None => true,
        }
    }

    /// `method` of the class `fqn`, walking its bases (`depth` levels).
    fn class_method(&self, fqn: &str, method: &str, depth: u8) -> ClassLookup {
        if depth > 8 {
            return ClassLookup::Unknown;
        }
        let Some(files) = self.classes.get(fqn) else {
            return if self.in_tree_namespace(fqn) {
                ClassLookup::Unknown
            } else {
                ClassLookup::External
            };
        };
        let short = fqn.rsplit('\\').next().unwrap_or(fqn).to_string();
        let mut found: Vec<CallableId> = Vec::new();
        for f in files {
            if let Some(ids) =
                self.methods
                    .get(&(*f, short.clone(), method.to_ascii_lowercase()))
            {
                found.extend(ids.iter().copied());
            }
        }
        if !found.is_empty() {
            return ClassLookup::Found(found, files.len() == 1);
        }
        // Inherited: the first base that provides it.
        for f in files {
            for b in self.bases.get(&(*f, short.clone())).into_iter().flatten() {
                let base = self.class_fqn(*f, 0, b);
                match self.class_method(&base, method, depth + 1) {
                    ClassLookup::Found(ids, _) => return ClassLookup::Found(ids, false),
                    ClassLookup::External => return ClassLookup::External,
                    ClassLookup::Unknown => {}
                }
            }
        }
        ClassLookup::Unknown
    }

    /// Namespace-scoped resolution of one PHP call. `None`: not a call
    /// this pass judges.
    pub(crate) fn resolve(
        &self,
        file: FileId,
        r: &RefRecord,
        caller_qn: Option<&str>,
    ) -> Option<Scoped> {
        let rh = r.receiver_hint.trim();
        if rh.is_empty() {
            if r.dot_call {
                return None;
            }
            // `f()`: a `use function` import, else the namespace's own
            // `f`, else the global one.
            let ns = self
                .file_ns
                .get(&file)
                .map_or("", |v| php_ns_at(v, r.site_byte));
            let lower = r.name.to_ascii_lowercase();
            let mut tries: Vec<String> = Vec::new();
            if let Some(t) = self.uses.get(&file).and_then(|u| u.get(&lower)) {
                tries.push(t.clone());
            }
            if r.name.starts_with('\\') || r.name.contains('\\') {
                tries.push(self.class_fqn(file, r.site_byte, &r.name));
            } else {
                tries.push(php_join(ns, &r.name));
                tries.push(lower.clone());
            }
            for t in &tries {
                if let Some(ids) = self.functions.get(t) {
                    return Some(self.one_copy(file, ids.clone()));
                }
            }
            // A function the tree declares only in other namespaces is
            // not this call's: a builtin or a dependency's.
            return self
                .function_names
                .contains(lower.trim_start_matches('\\'))
                .then(|| Scoped::External(tries.last().cloned().unwrap_or(lower)));
        }
        let lower = rh.to_ascii_lowercase();
        let fqn = match lower.as_str() {
            // `parent::m()`: the calling class's base.
            "parent" => {
                let (owner, _) = caller_qn?.split_once("::")?;
                let base = self
                    .bases
                    .get(&(file, owner.to_ascii_lowercase()))?
                    .first()?;
                self.class_fqn(file, r.site_byte, base)
            }
            "self" | "static" => return None,
            _ if rh.starts_with('$')
                || !rh
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '\\') =>
            {
                return None;
            }
            _ => self.class_fqn(file, r.site_byte, rh),
        };
        match self.class_method(&fqn, &r.name, 0) {
            ClassLookup::Found(cids, certain) => Some(match self.one_copy(file, cids) {
                Scoped::Bound { cids, certain: c } => Scoped::Bound {
                    cids,
                    certain: c && certain,
                },
                other => other,
            }),
            ClassLookup::External => Some(Scoped::External(fqn)),
            ClassLookup::Unknown => None,
        }
    }
}

/// What a class-method lookup found.
enum ClassLookup {
    Found(Vec<CallableId>, bool),
    /// The class (or the base the method must come from) is a
    /// dependency's.
    External,
    /// Not decided here.
    Unknown,
}
