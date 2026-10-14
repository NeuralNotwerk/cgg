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

/// A Scala type declaration (WS14b).
#[derive(Debug)]
struct ScalaType<'a> {
    file: FileId,
    /// Path within its package (`Outer.Inner`); empty for `new T { … }`.
    path: &'a str,
    package: &'a str,
    kind: &'a str,
    bases: &'a [String],
    span: (u32, u32),
}

/// One Scala file's imports.
#[derive(Debug, Default)]
struct ScalaImports {
    /// Name in scope → the path it imports (`Z` → `a.b.Y` for
    /// `import a.b.{Y => Z}`).
    exact: HashMap<String, String>,
    /// `import a.b._` / `a.b.*` prefixes.
    wild: Vec<String>,
}

/// A Scala base name resolved to types, and whether only a by-name
/// fallback found them.
type ScalaTypes = (Vec<usize>, bool);

/// Where a Zig receiver's type leads (WS14b).
enum ZigTy {
    /// A container type: file and body span.
    Container(FileId, u32, u32),
    /// A file (a file is a struct).
    File(FileId),
    /// A dependency.
    External(String),
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
    /// Zig: file → its named container types (`Outer.Inner`, a
    /// `@This()` alias) and their body spans (WS14b).
    zig_types: HashMap<FileId, Vec<(&'a str, u32, u32)>>,
    /// Zig: `(file, container body span)` → the callables declared
    /// directly in that container, by simple name.
    zig_members: HashMap<(FileId, u32, u32), HashMap<&'a str, Vec<CallableId>>>,
    /// Zig: callable → its byte span.
    zig_spans: HashMap<CallableId, (u32, u32)>,
    /// Zig: functions returning `type` (generic type constructors).
    zig_type_fns: HashSet<CallableId>,
    /// Zig: function → its declared return type, optional / pointer /
    /// error-union markers peeled (`!?*IO` → `IO`, `@This()` → `@This`).
    zig_returns: HashMap<CallableId, String>,
    /// Zig: callable → its file.
    zig_file_of: HashMap<CallableId, FileId>,
    /// Zig: `(file, container body span)` → its value declarations by
    /// name, as written (`frame` → `Functions(@This()).frame`).
    zig_decls: HashMap<(FileId, u32, u32), HashMap<&'a str, String>>,
    /// Perl: `(package, sub name)` → its subs (`main` for none).
    perl_subs: HashMap<(String, String), Vec<CallableId>>,
    /// Perl: every package a `package` statement of the tree declares.
    perl_packages: HashSet<String>,
    /// Perl: package → its bases and composed roles, in declaration
    /// order (`use parent`, `@ISA`, `extends`, `with`, …).
    perl_isa: HashMap<String, Vec<String>>,
    /// Scala: every class / trait / object / anonymous class (WS14b).
    scala_types: Vec<ScalaType<'a>>,
    /// Scala: fully qualified name → types carrying it.
    scala_by_fqn: HashMap<String, Vec<usize>>,
    /// Scala: simple name → named types.
    scala_by_name: HashMap<&'a str, Vec<usize>>,
    /// Scala: file → its types, in source order.
    scala_file_types: HashMap<FileId, Vec<usize>>,
    /// Scala: file → import table.
    scala_imports: HashMap<FileId, ScalaImports>,
    /// Scala: type → the callables declared directly in it, by name.
    scala_members: HashMap<usize, HashMap<&'a str, Vec<CallableId>>>,
    /// Scala: every package the tree declares (JVM packages of its
    /// Java / Kotlin files too), with each of their ancestors.
    scala_packages: HashSet<String>,
    /// Exported namespaces, computed on first use, and whether some
    /// export chain leaves the tree.
    ns_cache: Mutex<HashMap<FileId, (Arc<Namespace>, bool)>>,
}

impl<'a> LibIndex<'a> {
    pub(crate) fn build(graph: &'a Graph, facts: &'a [FileFacts]) -> Self {
        let mut ix = LibIndex::default();
        ix.build_zig(graph, facts);
        ix.build_perl(graph, facts);
        ix.build_scala(graph, facts);
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
    fn build_scala(&mut self, graph: &'a Graph, facts: &'a [FileFacts]) {
        if !facts.iter().any(|f| f.language == "scala") {
            return;
        }
        for f in facts.iter().filter(|f| f.language == "scala") {
            for t in &f.type_spans {
                let i = self.scala_types.len();
                self.scala_types.push(ScalaType {
                    file: f.file,
                    path: t.name.as_str(),
                    package: t.package.as_str(),
                    kind: t.kind.as_str(),
                    bases: &t.bases,
                    span: (t.start_byte, t.end_byte),
                });
                self.scala_file_types.entry(f.file).or_default().push(i);
                if !t.name.is_empty() {
                    let fqn = if t.package.is_empty() {
                        t.name.clone()
                    } else {
                        format!("{}.{}", t.package, t.name)
                    };
                    self.scala_by_fqn.entry(fqn).or_default().push(i);
                    let simple = t.name.rsplit('.').next().unwrap_or(&t.name);
                    self.scala_by_name.entry(simple).or_default().push(i);
                }
            }
            let mut im = ScalaImports::default();
            for imp in f.imports.iter().filter(|i| i.kind == "import") {
                scala_import(&imp.path, &mut im);
            }
            self.scala_imports.insert(f.file, im);
        }
        let mut pkgs: Vec<&str> = Vec::new();
        for f in facts {
            match f.language.as_str() {
                "scala" => {
                    pkgs.extend(f.type_spans.iter().map(|t| t.package.as_str()));
                    pkgs.extend(
                        f.imports
                            .iter()
                            .filter(|i| i.kind == "scala-package")
                            .map(|i| i.path.as_str()),
                    );
                }
                "java" | "kotlin" => pkgs.extend(f.packages.iter().map(String::as_str)),
                _ => {}
            }
        }
        for p in pkgs {
            let mut cur = p;
            while !cur.is_empty() && self.scala_packages.insert(cur.to_string()) {
                cur = cur.rsplit_once('.').map_or("", |(a, _)| a);
            }
        }
        let mut ids: Vec<&cgg_core::graph::CallableNode> = graph
            .callables
            .values()
            .filter(|c| c.language == "scala")
            .collect();
        ids.sort_by_key(|c| (c.file, c.start_byte, c.id));
        for c in ids {
            let Some(here) = self.scala_file_types.get(&c.file) else {
                continue;
            };
            let inner = here
                .iter()
                .copied()
                .filter(|&i| {
                    let (s, e) = self.scala_types[i].span;
                    s <= c.start_byte && c.end_byte <= e
                })
                .min_by_key(|&i| {
                    let (s, e) = self.scala_types[i].span;
                    (e - s, s)
                });
            if let Some(i) = inner {
                self.scala_members
                    .entry(i)
                    .or_default()
                    .entry(c.simple_name.as_str())
                    .or_default()
                    .push(c.id);
            }
        }
    }

    /// The class / trait a base name written in type `of` denotes:
    /// a type of the same file, an import (exact, then resolved against
    /// the file's wildcard imports and package — Scala imports may be
    /// relative), the type's package, a wildcard import; ambiguity is
    /// `None`. Only when none of those names a type in the tree, the one
    /// type of that simple name, flagged as a by-name fallback.
    fn scala_base(&self, of: usize, written: &str) -> Option<ScalaTypes> {
        let t = &self.scala_types[of];
        let name: String = written
            .split(['[', '('])
            .next()
            .unwrap_or(written)
            .split_whitespace()
            .collect();
        if name.is_empty() {
            return None;
        }
        let classes = |fqn: &str| -> Vec<usize> {
            self.scala_by_fqn
                .get(fqn)
                .map(|v| {
                    v.iter()
                        .copied()
                        .filter(|&i| self.scala_types[i].kind != "object")
                        .collect()
                })
                .unwrap_or_default()
        };
        let empty = ScalaImports::default();
        let im = self.scala_imports.get(&t.file).unwrap_or(&empty);
        let (head, rest) = match name.split_once('.') {
            Some((h, r)) => (h, Some(r)),
            None => (name.as_str(), None),
        };
        let with_rest = |p: &str| match rest {
            Some(r) => format!("{p}.{r}"),
            None => p.to_string(),
        };
        // The same file: a type it declares by this simple path.
        if rest.is_none() {
            let local: Vec<usize> = self
                .scala_file_types
                .get(&t.file)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&i| {
                    let u = &self.scala_types[i];
                    u.kind != "object"
                        && u.kind != "anon"
                        && u.path.rsplit('.').next() == Some(head)
                })
                .collect();
            if local.len() == 1 {
                return Some((local, false));
            }
        }
        // Candidate prefixes, in order of precedence.
        let mut tries: Vec<Vec<String>> = Vec::new();
        if let Some(target) = im.exact.get(head) {
            let mut v = vec![with_rest(target)];
            for w in &im.wild {
                v.push(format!("{w}.{}", with_rest(target)));
            }
            if !t.package.is_empty() {
                v.push(format!("{}.{}", t.package, with_rest(target)));
            }
            tries.push(v);
        }
        if !t.package.is_empty() {
            tries.push(vec![format!("{}.{name}", t.package)]);
        }
        tries.push(im.wild.iter().map(|w| format!("{w}.{name}")).collect());
        if rest.is_some() {
            tries.push(vec![name.clone()]);
        }
        for group in tries {
            let mut found: Vec<usize> = Vec::new();
            let mut fqns = 0;
            for fqn in &group {
                let c = classes(fqn);
                if !c.is_empty() {
                    fqns += 1;
                    found.extend(c);
                }
            }
            match fqns {
                0 => {}
                1 => return Some((found, false)),
                _ => return None,
            }
        }
        let last = name.rsplit('.').next().unwrap_or(&name);
        let by_name: Vec<usize> = self
            .scala_by_name
            .get(last)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| self.scala_types[i].kind != "object")
            .collect();
        let mut fqns: Vec<(&str, &str)> = by_name
            .iter()
            .map(|&i| (self.scala_types[i].package, self.scala_types[i].path))
            .collect();
        fqns.sort();
        fqns.dedup();
        (fqns.len() == 1).then_some((by_name, true))
    }

    /// A Scala bare call (`withTable(…)`, `this.m(…)`): the member of the
    /// innermost type around the site, else of its bases — each resolved
    /// through the declaring file's imports and package, never by simple
    /// name alone (`QueryTest` is the imported `org.x.sql.QueryTest`, not
    /// a same-named class in another package) — else of the enclosing
    /// types. A companion object's members are not the class's. `None`
    /// when no type around the site, or none on its chain, declares it.
    pub(crate) fn resolve_scala_bare(
        &self,
        file: FileId,
        r: &RefRecord,
    ) -> Option<Scoped> {
        let here = self.scala_file_types.get(&file)?;
        let mut around: Vec<usize> = here
            .iter()
            .copied()
            .filter(|&i| {
                let (s, e) = self.scala_types[i].span;
                s <= r.site_byte && r.site_byte < e
            })
            .collect();
        around.sort_by_key(|&i| {
            let (s, e) = self.scala_types[i].span;
            (e - s, s)
        });
        for t in around {
            if let Some((cids, guessed)) = self.scala_member(t, &r.name) {
                return Some(Scoped::Bound {
                    certain: !guessed && cids.len() == 1,
                    cids,
                });
            }
        }
        // Not a member: a name an explicit import brings in from outside
        // the tree (`import org.json4s.JsonMethods.{render}`) is the
        // dependency's.
        let im = self.scala_imports.get(&file)?;
        let target = im.exact.get(&r.name)?;
        let (owner, _) = target.rsplit_once('.')?;
        let pkg = here.first().map_or("", |&i| self.scala_types[i].package);
        let in_tree = |p: &str| {
            self.scala_packages.contains(p) || self.scala_by_fqn.contains_key(p)
        };
        let mut bases: Vec<String> = vec![String::new()];
        let mut cur = pkg;
        while !cur.is_empty() {
            bases.push(cur.to_string());
            cur = cur.rsplit_once('.').map_or("", |(a, _)| a);
        }
        bases.extend(im.wild.iter().cloned());
        let known = bases.iter().any(|b| {
            let full = if b.is_empty() {
                owner.to_string()
            } else {
                format!("{b}.{owner}")
            };
            in_tree(&full)
        });
        (!known).then(|| Scoped::External(target.clone()))
    }

    /// `name` declared in type `t` or up its bases (breadth-first, depth
    /// 8): the callables, and whether a base was found only by name.
    fn scala_member(&self, t: usize, name: &str) -> Option<(Vec<CallableId>, bool)> {
        let mut seen: HashSet<usize> = HashSet::new();
        let mut frontier: Vec<(usize, bool)> = vec![(t, false)];
        for _ in 0..8 {
            let mut found: Vec<CallableId> = Vec::new();
            let mut guessed = false;
            let mut next: Vec<(usize, bool)> = Vec::new();
            for (u, g) in frontier {
                if !seen.insert(u) {
                    continue;
                }
                if let Some(cids) = self.scala_members.get(&u).and_then(|m| m.get(name)) {
                    for c in cids {
                        if !found.contains(c) {
                            found.push(*c);
                        }
                    }
                    guessed |= g;
                    continue;
                }
                for b in self.scala_types[u].bases {
                    if let Some((ts, by_name)) = self.scala_base(u, b) {
                        next.extend(ts.into_iter().map(|x| (x, g || by_name)));
                    }
                }
            }
            if !found.is_empty() {
                return Some((found, guessed));
            }
            if next.is_empty() {
                return None;
            }
            frontier = next;
        }
        None
    }

    fn build_perl(&mut self, graph: &Graph, facts: &[FileFacts]) {
        if !facts.iter().any(|f| f.language == "perl") {
            return;
        }
        let mut ids: Vec<&cgg_core::graph::CallableNode> = graph
            .callables
            .values()
            .filter(|c| c.language == "perl")
            .collect();
        ids.sort_by_key(|c| (c.file, c.start_byte, c.id));
        for c in ids {
            let (pkg, name) = c
                .qualified_name
                .rsplit_once("::")
                .unwrap_or(("main", c.qualified_name.as_str()));
            self.perl_subs
                .entry((pkg.to_string(), name.to_string()))
                .or_default()
                .push(c.id);
        }
        for f in facts.iter().filter(|f| f.language == "perl") {
            for imp in &f.imports {
                match imp.kind.as_str() {
                    "perl-package" => {
                        self.perl_packages.insert(imp.path.clone());
                    }
                    "perl-isa" => {
                        let v = self.perl_isa.entry(imp.alias.clone()).or_default();
                        if !v.contains(&imp.path) {
                            v.push(imp.path.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// A Perl method call on the caller's invocant (`$self->m()` after
    /// `my $self = shift;`, `$self->SUPER::m()`): the sub `m` of the
    /// caller's package, else of its bases depth-first in declaration
    /// order (Perl's default method resolution). Not found, with a base
    /// outside the tree on the way: external — the method is inherited
    /// from the dependency. `None` when nothing decides.
    pub(crate) fn resolve_perl(
        &self,
        r: &RefRecord,
        caller_qn: Option<&str>,
    ) -> Option<Scoped> {
        if !r.invocant {
            return None;
        }
        let pkg = caller_qn
            .and_then(|q| q.rsplit_once("::"))
            .map_or("main", |(p, _)| p);
        let mut stack: Vec<String> = if r.receiver_hint == "SUPER" {
            self.perl_isa.get(pkg)?.iter().rev().cloned().collect()
        } else {
            vec![pkg.to_string()]
        };
        let mut seen: HashSet<String> = HashSet::new();
        let mut outside: Option<String> = None;
        let mut steps = 0;
        while let Some(p) = stack.pop() {
            steps += 1;
            if steps > 64 || !seen.insert(p.clone()) {
                continue;
            }
            if let Some(cids) = self.perl_subs.get(&(p.clone(), r.name.clone())) {
                return Some(Scoped::Bound {
                    cids: cids.clone(),
                    certain: cids.len() == 1,
                });
            }
            let declared = self.perl_packages.contains(&p);
            if !declared && outside.is_none() {
                outside = Some(p.clone());
            }
            if let Some(bases) = self.perl_isa.get(&p) {
                stack.extend(bases.iter().rev().cloned());
            }
        }
        outside.map(Scoped::External)
    }

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
        // Containers: each callable is a member of the innermost container
        // around it (an anonymous one included, so a generic's `return
        // struct { … }` keeps its methods to itself).
        let mut by_file: HashMap<FileId, Vec<&cgg_core::graph::CallableNode>> =
            HashMap::new();
        for c in graph.callables.values().filter(|c| c.language == "zig") {
            by_file.entry(c.file).or_default().push(c);
            self.zig_spans.insert(c.id, (c.start_byte, c.end_byte));
            if c.signature_hint.trim_end().ends_with(" type") {
                self.zig_type_fns.insert(c.id);
            }
            self.zig_file_of.insert(c.id, c.file);
            if let Some(ret) = zig_return_type(&c.signature_hint) {
                self.zig_returns.insert(c.id, ret);
            }
        }
        for f in facts.iter().filter(|f| f.language == "zig") {
            if f.type_spans.is_empty() {
                continue;
            }
            let named: Vec<(&'a str, u32, u32)> = f
                .type_spans
                .iter()
                .filter(|t| !t.name.is_empty())
                .map(|t| (t.name.as_str(), t.start_byte, t.end_byte))
                .collect();
            self.zig_types.insert(f.file, named);
            for t in f.type_spans.iter().filter(|t| !t.decls.is_empty()) {
                let slot = self
                    .zig_decls
                    .entry((f.file, t.start_byte, t.end_byte))
                    .or_default();
                for (n, v) in &t.decls {
                    slot.entry(n.as_str()).or_insert_with(|| v.clone());
                }
            }
            let mut cs = by_file.remove(&f.file).unwrap_or_default();
            cs.sort_by_key(|c| (c.start_byte, c.id));
            for c in cs {
                let inner = f
                    .type_spans
                    .iter()
                    .filter(|t| t.start_byte <= c.start_byte && c.end_byte <= t.end_byte)
                    .min_by_key(|t| (t.end_byte - t.start_byte, t.start_byte));
                if let Some(t) = inner {
                    self.zig_members
                        .entry((f.file, t.start_byte, t.end_byte))
                        .or_default()
                        .entry(c.simple_name.as_str())
                        .or_default()
                        .push(c.id);
                }
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
        if let Some(open) = rh.find('(') {
            return self.zig_generic_call(file, &rh[..open], &r.name, 0);
        }
        let segs: Vec<&str> = rh.split('.').collect();
        if !segs
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'))
        {
            return None;
        }
        let segs: Vec<String> = segs.into_iter().map(str::to_string).collect();
        let Some((g, _)) = self.zig_walk(file, &segs, 0) else {
            // The receiver is a value typed as a container (WS14b): the
            // call is that container's member.
            return self.zig_member_call(file, &segs, &r.name, Some(r.site_byte), 0);
        };
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

    /// A call on a receiver typed as a Zig container (`surface.close()`
    /// with `surface: *Surface`, `Surface = @import("surface.zig")
    /// .Surface`): the one member of that name the container declares —
    /// a function, or a declaration aliasing one (`pub const frame =
    /// Functions(@This()).frame;`, followed). `None` when the type is not
    /// found or the member is not one callable of it; refused when the
    /// container declares nothing of that name (a field holding a
    /// function pointer is not a same-named function elsewhere).
    fn zig_member_call(
        &self,
        file: FileId,
        segs: &[String],
        name: &str,
        site: Option<u32>,
        depth: u8,
    ) -> Option<Scoped> {
        if depth > 4 {
            return None;
        }
        let members = match self.zig_type(file, segs, site, 0)? {
            ZigTy::External(m) => return Some(Scoped::External(m)),
            ZigTy::File(g) => self.zig_top.get(&g)?.get(name)?,
            ZigTy::Container(g, s, e) => {
                match self.zig_members.get(&(g, s, e)).and_then(|m| m.get(name)) {
                    Some(m) => m,
                    None => {
                        return match self
                            .zig_decls
                            .get(&(g, s, e))
                            .and_then(|d| d.get(name))
                        {
                            Some(value) => {
                                self.zig_alias_call(g, (s, e), value, depth + 1)
                            }
                            None => Some(Scoped::Refused(segs.join("."))),
                        };
                    }
                }
            }
        };
        match members.as_slice() {
            [_] => Some(Scoped::Bound {
                cids: members.clone(),
                certain: true,
            }),
            _ => None,
        }
    }

    /// What a container-level alias of a function names, from the file
    /// `g` it is declared in: `Fn(args).member` (a type function's
    /// member) or `a.b.member` (a path). `None` when it is neither, or
    /// does not lead to one function.
    fn zig_alias_call(
        &self,
        g: FileId,
        within: (u32, u32),
        value: &str,
        depth: u8,
    ) -> Option<Scoped> {
        let value: String = value.split_whitespace().collect();
        if let Some(open) = value.find('(') {
            let close = value.rfind(')')?;
            let member = value[close + 1..].strip_prefix('.')?;
            if member.contains('.') || member.is_empty() {
                return None;
            }
            return match self.zig_generic_call(g, &value[..open], member, depth)? {
                b @ Scoped::Bound { .. } => Some(b),
                _ => None,
            };
        }
        let segs: Vec<String> = value.split('.').map(str::to_string).collect();
        let (member, prefix) = segs.split_last()?;
        if prefix.is_empty() {
            return None;
        }
        // `C.as` with `const C = Common(Self, Private);` in the same
        // container: the member of the type `Common` builds.
        if let [head] = prefix
            && let Some(built) = self
                .zig_decls
                .get(&(g, within.0, within.1))
                .and_then(|d| d.get(head.as_str()))
        {
            let built: String = built.split_whitespace().collect();
            let open = built.find('(')?;
            if !built.ends_with(')') {
                return None;
            }
            return match self.zig_generic_call(g, &built[..open], member, depth + 1)? {
                b @ Scoped::Bound { .. } => Some(b),
                _ => None,
            };
        }
        match self.zig_member_call(g, prefix, member, None, depth)? {
            b @ Scoped::Bound { .. } => Some(b),
            _ => None,
        }
    }

    /// A call on what a call returns (`callee` is the part before the
    /// arguments). Through a dependency, external. In the tree, when the
    /// callee is a type function (`list.Managed(T).init()`, returning
    /// `type`): the one member of that name of the container it returns
    /// (its outermost `struct { … }`), else refused — a same-named
    /// function elsewhere is not the callee. Any other callee: `None`.
    fn zig_generic_call(
        &self,
        file: FileId,
        callee: &str,
        name: &str,
        depth: u8,
    ) -> Option<Scoped> {
        if depth > 4 {
            return None;
        }
        let segs: Vec<String> = callee.split('.').map(str::to_string).collect();
        if !segs
            .iter()
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'))
        {
            return None;
        }
        let (fname, prefix) = segs.split_last()?;
        // The function called: of the file the prefix names (a module
        // path), or of the container it names (`IO.init`).
        let (g, fns) = if prefix.is_empty() {
            match self.zig_top.get(&file).and_then(|t| t.get(fname.as_str())) {
                Some(f) => (file, f),
                // `const Common = @import("../class.zig").Common;`: the
                // function the alias names, in its file.
                None => {
                    return match self.zig_binds.get(&file)?.get(fname.as_str())? {
                        ZigBind::ImportMember(spec, members) => {
                            match self.zig_target(file, spec) {
                                Target::File(f2) => {
                                    self.zig_generic_call(f2, members, name, depth + 1)
                                }
                                Target::External(m) => Some(Scoped::External(m)),
                                Target::Unknown => None,
                            }
                        }
                        ZigBind::Alias(path) if path != fname => {
                            self.zig_generic_call(file, path, name, depth + 1)
                        }
                        _ => None,
                    };
                }
            }
        } else {
            match self.zig_walk(file, prefix, 0).map(|(t, _)| t) {
                Some(Target::File(g)) => (g, self.zig_top.get(&g)?.get(fname.as_str())?),
                Some(Target::External(m)) => return Some(Scoped::External(m)),
                Some(Target::Unknown) => return None,
                None => match self.zig_type(file, prefix, None, 0)? {
                    ZigTy::External(m) => return Some(Scoped::External(m)),
                    ZigTy::File(g) => (g, self.zig_top.get(&g)?.get(fname.as_str())?),
                    ZigTy::Container(g, s, e) => {
                        (g, self.zig_members.get(&(g, s, e))?.get(fname.as_str())?)
                    }
                },
            }
        };
        let refused = || Some(Scoped::Refused(callee.to_string()));
        let [f] = fns.as_slice() else {
            return None;
        };
        // A plain function: the receiver is what it is declared to
        // return (`IO.init()` returning `!IO`, `Self`, `@This()`), WS14b.
        if !self.zig_type_fns.contains(f) {
            let ret = self.zig_returns.get(f)?;
            let fg = *self.zig_file_of.get(f)?;
            let &(fs, _) = self.zig_spans.get(f)?;
            let ty = if ret == "@This" {
                // The container around the function, else its file.
                let around = self
                    .zig_types
                    .get(&fg)
                    .into_iter()
                    .flatten()
                    .filter(|(_, s, e)| *s <= fs && fs < *e)
                    .min_by_key(|(_, s, e)| (e - s, *s))
                    .map(|&(_, s, e)| ZigTy::Container(fg, s, e));
                around.unwrap_or(ZigTy::File(fg))
            } else {
                let rsegs: Vec<String> = ret.split('.').map(str::to_string).collect();
                self.zig_type(fg, &rsegs, Some(fs), 0)?
            };
            let members = match ty {
                ZigTy::External(m) => return Some(Scoped::External(m)),
                ZigTy::File(h) => self.zig_top.get(&h)?.get(name)?,
                ZigTy::Container(h, s, e) => {
                    self.zig_members.get(&(h, s, e))?.get(name)?
                }
            };
            return match members.as_slice() {
                [_] => Some(Scoped::Bound {
                    cids: members.clone(),
                    certain: true,
                }),
                _ => None,
            };
        }
        let Some(&(fs, fe)) = self.zig_spans.get(f) else {
            return refused();
        };
        // The containers the function returns: those in its body not
        // nested in another container of it.
        let inside: Vec<(u32, u32)> = self
            .zig_members
            .keys()
            .filter(|(h, s, e)| *h == g && fs <= *s && *e <= fe)
            .map(|&(_, s, e)| (s, e))
            .chain(
                self.zig_decls
                    .keys()
                    .filter(|(h, s, e)| *h == g && fs <= *s && *e <= fe)
                    .map(|&(_, s, e)| (s, e)),
            )
            .collect();
        let outer = |(s, e): (u32, u32)| {
            !inside
                .iter()
                .any(|&(s2, e2)| s2 <= s && e <= e2 && (s2, e2) != (s, e))
        };
        let mut found: Vec<CallableId> = Vec::new();
        let mut aliased: Option<((u32, u32), &str)> = None;
        for &(s, e) in inside.iter().filter(|&&k| outer(k)) {
            if let Some(m) = self.zig_members.get(&(g, s, e)).and_then(|m| m.get(name)) {
                found.extend(m.iter().copied());
            } else if let Some(v) =
                self.zig_decls.get(&(g, s, e)).and_then(|d| d.get(name))
            {
                aliased = Some(((s, e), v.as_str()));
            }
        }
        found.sort();
        found.dedup();
        match (found.as_slice(), aliased) {
            ([_], _) => Some(Scoped::Bound {
                cids: found,
                certain: false,
            }),
            ([], Some((k, v))) => self.zig_alias_call(g, k, v, depth + 1),
            _ => refused(),
        }
    }

    /// The type `segs` names from `file`: through the file's bindings
    /// to another file, then a container that file declares. `site` (in
    /// `file`) lets a name resolve in the containers around it first
    /// (`Inner` inside `Outer` is `Outer.Inner`).
    fn zig_type(
        &self,
        file: FileId,
        segs: &[String],
        site: Option<u32>,
        depth: u8,
    ) -> Option<ZigTy> {
        if depth > MAX_DEPTH {
            return None;
        }
        let (head, rest) = segs.split_first()?;
        if let Some(bind) = self.zig_binds.get(&file).and_then(|b| b.get(head)) {
            let (g, path) = match bind {
                ZigBind::Alias(path) => {
                    let mut expanded: Vec<String> =
                        path.split('.').map(str::to_string).collect();
                    if expanded.first() == Some(head) {
                        return None;
                    }
                    expanded.extend(rest.iter().cloned());
                    return self.zig_type(file, &expanded, None, depth + 1);
                }
                ZigBind::Import(spec) => (self.zig_target(file, spec), rest.to_vec()),
                ZigBind::ImportMember(spec, members) => {
                    let mut p: Vec<String> =
                        members.split('.').map(str::to_string).collect();
                    p.extend(rest.iter().cloned());
                    (self.zig_target(file, spec), p)
                }
            };
            return match g {
                Target::File(g) if path.is_empty() => Some(ZigTy::File(g)),
                Target::File(g) => self.zig_type(g, &path, None, depth + 1),
                Target::External(m) => Some(ZigTy::External(m)),
                Target::Unknown => None,
            };
        }
        let types = self.zig_types.get(&file)?;
        let joined = segs.join(".");
        let find = |name: &str| {
            types
                .iter()
                .find(|(n, _, _)| *n == name)
                .map(|&(_, s, e)| ZigTy::Container(file, s, e))
        };
        if let Some(site) = site {
            let mut around: Vec<&(&str, u32, u32)> = types
                .iter()
                .filter(|(_, s, e)| *s <= site && site < *e)
                .collect();
            around.sort_by_key(|(_, s, e)| (e - s, *s));
            for (n, _, _) in around {
                if let Some(t) = find(&format!("{n}.{joined}")) {
                    return Some(t);
                }
            }
        }
        find(&joined)
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
    /// Functions declared inside a conditional (`if (!function_exists(…))`
    /// fallbacks), WS14b.
    conditional: HashSet<CallableId>,
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
        let conditional: HashSet<(FileId, u32)> = facts
            .iter()
            .filter(|f| f.language == "php")
            .flat_map(|f| {
                f.definitions
                    .iter()
                    .filter(|d| d.conditional)
                    .map(move |d| (f.file, d.start_byte))
            })
            .collect();
        for c in ids {
            if conditional.contains(&(c.file, c.start_byte)) {
                ix.conditional.insert(c.id);
            }
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
            // The chain does not decide (WS14b): a definition declared in
            // a conditional (`if (!function_exists('f'))`) is a fallback
            // that exists only when no other one is loaded. With exactly
            // one unconditional definition left among the candidates, that
            // one is the function — a guess at the loading order, so not
            // certain.
            pool => {
                let pool: &[FileId] = if pool.is_empty() { &files } else { pool };
                let firm: Vec<CallableId> = ids
                    .into_iter()
                    .filter(|c| {
                        !self.conditional.contains(c)
                            && self.file_of.get(c).is_some_and(|f| pool.contains(f))
                    })
                    .collect();
                match firm.as_slice() {
                    [_] => Scoped::Bound {
                        cids: firm,
                        certain: false,
                    },
                    _ => Scoped::Ambiguous,
                }
            }
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
            // `$this->m()`, `self::m()`, `static::m()`: the calling class's
            // method, or one it inherits or a trait mixes in; through a
            // dependency's base, external (WS8).
            "this" | "self" | "static" => {
                let (owner, _) = caller_qn?.split_once("::")?;
                let fqn = self.class_fqn(file, r.site_byte, owner);
                return match self.class_method(&fqn, &r.name, 0) {
                    ClassLookup::Found(cids, certain) => {
                        Some(match self.one_copy(file, cids) {
                            Scoped::Bound { cids, certain: c } => Scoped::Bound {
                                cids,
                                certain: c && certain,
                            },
                            other => other,
                        })
                    }
                    ClassLookup::External => Some(Scoped::External(fqn)),
                    // Undecided: `static::m()` binds late, to a subclass's
                    // `m`, and keeps the readings it had; `$this->m()` is
                    // left unbound rather than matched by name.
                    ClassLookup::Unknown if lower == "this" => Some(Scoped::Ambiguous),
                    ClassLookup::Unknown => None,
                };
            }
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

/// One Scala `import` (whitespace removed: `a.b.{X,Y=>Z}`, `a.b._`,
/// `a.b.*`, `a.b.X`) into the file's table.
fn scala_import(path: &str, im: &mut ScalaImports) {
    let path = path.trim();
    if let Some((prefix, sel)) = path.split_once(".{") {
        for item in sel.trim_end_matches('}').split(',') {
            let item = item.trim();
            match item.split_once("=>") {
                Some((_, "_")) => {}
                Some((from, to)) => {
                    im.exact.insert(
                        to.trim().to_string(),
                        format!("{prefix}.{}", from.trim()),
                    );
                }
                None if item == "_" || item == "*" => im.wild.push(prefix.to_string()),
                None if !item.is_empty() => {
                    im.exact
                        .insert(item.to_string(), format!("{prefix}.{item}"));
                }
                None => {}
            }
        }
        return;
    }
    if let Some(prefix) = path.strip_suffix("._").or_else(|| path.strip_suffix(".*")) {
        im.wild.push(prefix.to_string());
        return;
    }
    if let Some((_, last)) = path.rsplit_once('.') {
        im.exact.insert(last.to_string(), path.to_string());
    }
}

/// A Zig function's declared return type from its signature (`pub fn
/// init(a: u8) !?*IO` → `IO`, `… @This()` → `@This`): `None` unless it
/// is a plain name or path.
fn zig_return_type(sig: &str) -> Option<String> {
    let open = sig.find('(')?;
    let mut depth = 0i32;
    let mut close = None;
    for (i, ch) in sig[open..].char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(open + i);
                    break;
                }
            }
            _ => {}
        }
    }
    let rest = sig[close? + 1..].trim().trim_end_matches('{').trim();
    let mut t = rest.rsplit('!').next().unwrap_or(rest).trim();
    loop {
        let next = t
            .strip_prefix('?')
            .or_else(|| t.strip_prefix('*'))
            .or_else(|| t.strip_prefix("const "));
        match next {
            Some(n) => t = n.trim_start(),
            None => break,
        }
    }
    if t == "@This()" {
        return Some("@This".to_string());
    }
    let plain = !t.is_empty()
        && t.split('.')
            .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'));
    (plain && !matches!(t, "void" | "type" | "bool" | "noreturn" | "anytype"))
        .then(|| t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WS14b: a Zig function's declared return type, peeled to the type a
    /// receiver it initializes has; anything else is no type.
    #[test]
    fn zig_return_types_are_peeled() {
        assert_eq!(
            zig_return_type("pub fn init(n: u32) !IO").as_deref(),
            Some("IO")
        );
        assert_eq!(
            zig_return_type("fn a(self: *T) ?*const x.Surface").as_deref(),
            Some("x.Surface")
        );
        assert_eq!(
            zig_return_type("fn b(f: fn () void) E!*Self").as_deref(),
            Some("Self")
        );
        assert_eq!(zig_return_type("fn c() @This()").as_deref(), Some("@This"));
        assert_eq!(zig_return_type("fn d() void"), None);
        assert_eq!(zig_return_type("fn e() []const u8"), None);
        assert_eq!(zig_return_type("fn f(comptime T: type) type"), None);
    }
}
