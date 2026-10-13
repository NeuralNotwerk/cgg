//! External module / package heads (FINDINGS A1).
//!
//! A call written through something the calling file imports from
//! *outside* the analyzed tree — `requests.post()` after `import
//! requests`, `uuid.NewString()` for a module outside every workspace
//! `go.mod`, `StringUtils.isBlank()` after `import
//! org.apache.commons.lang3.StringUtils`, `boost::split()` where no file
//! declares `boost` — is a call into that dependency. The workspace may
//! well have a callable of the same name (`app.util.post`, a local
//! `StringUtils`), and the name-matching steps of the cross-file resolver
//! (duck-typed fan-out, owner-type lookup, the bare-name fallbacks) would
//! bind the call to it. That edge is a coincidence of spelling, never a
//! call. Rust learnt this in 0.10.2 (`is_external_rust_head`,
//! `RefRecord::path_receiver`); this module is the same rule for the
//! other languages with import tables.
//!
//! "External" is decided structurally, from what the tree itself
//! declares, never from a list of package names:
//!
//! * **Python** — a module is the workspace's when its first segment is
//!   a directory or file stem of some analyzed Python file (or the head of
//!   a Python qualified name); a relative import always is.
//! * **JavaScript / TypeScript** — a relative specifier always is, and so
//!   is one no npm package can be named (`@/x`, `~/x`, `#x`: path
//!   aliases); a bare one when it names a `package.json` found in the
//!   tree, or when its first segment (after a `@` scope sigil) is a
//!   directory or file stem of an analyzed JS/TS file (tsconfig
//!   `paths`/`baseUrl` aliases resolve into the tree that way).
//! * **Go** — an import path is when it lies under a `module` path of a
//!   `go.mod` in the tree, or when some analyzed Go directory ends with
//!   the whole path (vendored or GOPATH-style). A file with no `go.mod`
//!   above it falls back to the package directory name.
//! * **Java / Kotlin** — a package is when the tree declares it, a
//!   sub-package of it, or a package beneath it.
//! * **C#** — a namespace is when the tree declares it (or one beneath
//!   it); a bare type name is looked up only in the namespaces the
//!   calling file can see (its own and enclosing namespaces, its `using`
//!   directives and every `global using`).
//! * **C++** — a `ns::` head is when some C/C++ qualified name has a scope
//!   segment of that name, after the file's namespace aliases.
//!
//! Every test errs toward "workspace": a wrongly-judged external head
//! would drop a real edge, a wrongly-judged workspace head only keeps the
//! behaviour cgg had before. Built sorted-insensitive (sets that are only
//! probed), so `--jobs` cannot change an answer.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path};

use cgg_core::{FileFacts, RefRecord, graph::Graph, ids::FileId};

/// What the whole tree declares, per language family. Built once per run.
#[derive(Debug, Default)]
pub(crate) struct WorkspaceVocab {
    /// Python: every directory name and file stem of an analyzed Python
    /// file, and the first segment of every Python qualified name.
    py_names: HashSet<String>,
    /// JS/TS: package names (`package.json`, `deno.json`) and exact path
    /// aliases found above analyzed files.
    pub(crate) npm_packages: HashSet<String>,
    /// JS/TS: path-alias prefixes (tsconfig `"@app/*"` → `@app/`, a
    /// Deno import-map key `"@/"`), sorted.
    pub(crate) js_alias_prefixes: Vec<String>,
    /// JS/TS: every directory name and file stem of an analyzed file.
    js_names: HashSet<String>,
    /// Go: `go.mod` module paths (and locally replaced modules), sorted.
    go_modules: Vec<String>,
    /// Go: every `/`-joined suffix of every directory holding a Go file.
    go_dir_suffixes: HashSet<String>,
    /// Java/Kotlin: declared packages.
    jvm_packages: HashSet<String>,
    /// Java/Kotlin: every declared package and each of its ancestors.
    jvm_prefixes: HashSet<String>,
    /// Java/Kotlin: first segments of declared packages.
    jvm_roots: HashSet<String>,
    /// C#: declared namespaces and each of their ancestors.
    cs_prefixes: HashSet<String>,
    /// C#: first segments of declared namespaces.
    cs_roots: HashSet<String>,
    /// C#: `global using N;` namespaces — visible in every C# file.
    cs_global_usings: Vec<String>,
    /// C#: namespaces each file declares.
    cs_file_namespaces: HashMap<FileId, Vec<String>>,
    /// C/C++: every scope segment (namespace or class) of a C-family
    /// qualified name, template arguments dropped, plus class names
    /// known only from their fields.
    cpp_scopes: HashSet<String>,
}

impl WorkspaceVocab {
    pub(crate) fn build(graph: &Graph, facts: &[FileFacts]) -> Self {
        let mut v = WorkspaceVocab::default();
        let mut go_modules: HashSet<String> = HashSet::new();
        let mut cs_global: HashSet<String> = HashSet::new();
        let mut js_alias: HashSet<String> = HashSet::new();
        for f in facts {
            match f.language.as_str() {
                "python" => path_names(&f.path, &mut v.py_names),
                "javascript" | "typescript" => {
                    path_names(&f.path, &mut v.js_names);
                    for p in &f.packages {
                        match p.split_once('*') {
                            Some((prefix, _)) => js_alias.insert(prefix.to_string()),
                            None if p.ends_with('/') => js_alias.insert(p.clone()),
                            None => v.npm_packages.insert(p.clone()),
                        };
                    }
                }
                "go" => {
                    go_modules.extend(f.packages.iter().cloned());
                    if let Some(dir) = f.path.parent() {
                        let segs: Vec<String> = normal_components(dir);
                        for i in 0..segs.len() {
                            v.go_dir_suffixes.insert(segs[i..].join("/"));
                        }
                    }
                }
                "java" | "kotlin" => {
                    for p in &f.packages {
                        v.jvm_packages.insert(p.clone());
                        insert_prefixes(p, &mut v.jvm_prefixes);
                        if let Some(root) = p.split('.').next() {
                            v.jvm_roots.insert(root.to_string());
                        }
                    }
                }
                "csharp" => {
                    for ns in &f.packages {
                        insert_prefixes(ns, &mut v.cs_prefixes);
                        if let Some(root) = ns.split('.').next() {
                            v.cs_roots.insert(root.to_string());
                        }
                    }
                    if !f.packages.is_empty() {
                        v.cs_file_namespaces.insert(f.file, f.packages.clone());
                    }
                    for imp in &f.imports {
                        if let Some(ns) = cs_global_using(imp) {
                            cs_global.insert(ns.to_string());
                        }
                    }
                }
                "cpp" | "c" | "objc" => {
                    for ft in &f.field_types {
                        v.cpp_scopes.insert(bare_scope(&ft.owner).to_string());
                    }
                    // `using json = nlohmann::json;` / `typedef … Foo;`: a
                    // type the tree names, whatever it aliases.
                    for a in &f.type_aliases {
                        v.cpp_scopes.insert(bare_scope(&a.name).to_string());
                    }
                }
                _ => {}
            }
        }
        for c in graph.callables.values() {
            match c.language.as_str() {
                "python" => {
                    if let Some(head) = c.qualified_name.split('.').next() {
                        v.py_names.insert(head.to_string());
                    }
                }
                "cpp" | "c" | "objc" => {
                    // Only a qualified name that is a scope path ending in
                    // the callable's own name says what its scopes are; a
                    // mis-split declaration (`std::string to_string`) does
                    // not declare a namespace `std`.
                    let qn = c.qualified_name.as_str();
                    if qn != c.simple_name
                        && !qn
                            .strip_suffix(c.simple_name.as_str())
                            .is_some_and(|p| p.ends_with("::"))
                    {
                        continue;
                    }
                    let segs: Vec<&str> = qn.split("::").collect();
                    for s in &segs[..segs.len().saturating_sub(1)] {
                        let s = bare_scope(s);
                        if !s.is_empty() {
                            v.cpp_scopes.insert(s.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        v.go_modules = go_modules.into_iter().collect();
        v.go_modules.sort();
        v.js_alias_prefixes = js_alias.into_iter().filter(|p| !p.is_empty()).collect();
        v.js_alias_prefixes.sort();
        let mut g: Vec<String> = cs_global.into_iter().collect();
        g.sort();
        v.cs_global_usings = g;
        v
    }

    pub(crate) fn py_workspace(&self, module: &str) -> bool {
        if module.starts_with('.') {
            return true;
        }
        let head = module.split('.').next().unwrap_or(module).trim();
        head.is_empty() || self.py_names.contains(head)
    }

    pub(crate) fn js_workspace(&self, spec: &str) -> bool {
        let spec = spec.trim();
        if spec.is_empty() || spec.starts_with('.') || spec.starts_with('/') {
            return true;
        }
        // No npm package can be named `@/x` (a scope is never empty), `~x`,
        // `#x` or `$x`: those are path aliases (tsconfig `paths`, bundler
        // `~`, package.json `imports`, SvelteKit `$lib`), and they resolve
        // into the tree.
        if spec.starts_with("@/") || spec.starts_with(['~', '#', '$']) {
            return true;
        }
        let pkg = npm_package_of(spec);
        if self.npm_packages.contains(pkg)
            || self.npm_packages.contains(spec)
            || self
                .js_alias_prefixes
                .iter()
                .any(|p| spec.starts_with(p.as_str()))
        {
            return true;
        }
        let stripped = spec
            .trim_start_matches(['@', '~', '#'])
            .trim_start_matches('/');
        let head = stripped.split('/').next().unwrap_or("");
        // `require('yapi.js')` through NODE_PATH names the file `yapi.js`.
        let head = [".js", ".mjs", ".cjs", ".jsx", ".ts", ".tsx"]
            .iter()
            .find_map(|ext| head.strip_suffix(ext))
            .unwrap_or(head);
        !head.is_empty() && !head.contains(':') && self.js_names.contains(head)
    }

    pub(crate) fn go_workspace(&self, path: &str, file_has_module: bool) -> bool {
        let path = path.trim();
        if path.is_empty() {
            return true;
        }
        if self.go_modules.iter().any(|m| {
            path == m
                || path
                    .strip_prefix(m.as_str())
                    .is_some_and(|r| r.starts_with('/'))
        }) {
            return true;
        }
        if self.go_dir_suffixes.contains(path) {
            return true;
        }
        // No `go.mod` above the calling file: there is no module graph
        // to judge by, so only a package directory of the same name
        // keeps the import in the workspace — the conservative reading.
        !file_has_module
            && self
                .go_dir_suffixes
                .contains(path.rsplit('/').next().unwrap_or(path))
    }

    /// A package (`a.b.c`) is the tree's when the tree declares it, one of
    /// its sub-packages, or a package it sits beneath.
    fn jvm_workspace(&self, pkg: &str) -> bool {
        if pkg.is_empty() || self.jvm_prefixes.contains(pkg) {
            return true;
        }
        let mut cut = pkg;
        while let Some((head, _)) = cut.rsplit_once('.') {
            if self.jvm_packages.contains(head) {
                return true;
            }
            cut = head;
        }
        false
    }

    /// A C# namespace or type path (`A.B` / `A.B.Type`) is the tree's when
    /// some prefix of it is a declared namespace, or it is the ancestor of
    /// one.
    fn cs_workspace(&self, path: &str) -> bool {
        if path.is_empty() || self.cs_prefixes.contains(path) {
            return true;
        }
        let mut cut = path;
        while let Some((head, _)) = cut.rsplit_once('.') {
            if self.cs_prefixes.contains(head) {
                return true;
            }
            cut = head;
        }
        false
    }
}

/// One name the calling file binds through an import.
#[derive(Debug, Clone)]
struct Binding {
    module: String,
    external: bool,
    /// The name is the module itself (`import requests`, `import * as
    /// fs`, a default import, a Go package), not a symbol taken out of
    /// one. Only then is a call chained off it (`requests.get(u).json()`)
    /// a call into the dependency too: a symbol can be an identity-like
    /// function (`verify(mock).run()`) whose result is a workspace value.
    module_binding: bool,
}

/// The calling file's import bindings, judged against [`WorkspaceVocab`].
#[derive(Debug, Default)]
pub(crate) struct FileHeads {
    /// Locally bound name (module alias, imported symbol, imported type)
    /// → where it comes from. A name two imports bind keeps the
    /// workspace reading if either is the workspace's.
    bound: HashMap<String, Binding>,
    /// Java/Kotlin: first segments of this file's own package and import
    /// paths — what a package-qualified receiver can start with.
    jvm_roots: HashSet<String>,
    /// C#: first segments of this file's `using` namespaces.
    cs_using_roots: HashSet<String>,
    /// C#: namespaces a bare type name is looked up in from this file.
    cs_visible: HashSet<String>,
    /// C++: namespace aliases, alias → target.
    cpp_aliases: HashMap<String, String>,
    /// Simple names this file defines. A bare call to one of them may be
    /// the local definition shadowing an import of the same name (a
    /// minified bundle reuses every short name), so it is never refused.
    own_defs: HashSet<String>,
    /// Parameters and locals: a receiver headed by one is that value,
    /// not an import of the same name.
    pub(crate) locals: crate::scope::FileLocals,
}

impl FileHeads {
    pub(crate) fn build(facts: &FileFacts, vocab: &WorkspaceVocab) -> Self {
        let mut h = FileHeads {
            own_defs: facts
                .definitions
                .iter()
                .map(|d| d.simple_name.clone())
                .collect(),
            locals: crate::scope::FileLocals::build(facts),
            ..FileHeads::default()
        };
        let lang = facts.language.as_str();
        let mut bind =
            |name: &str, module: &str, external: bool, module_binding: bool| {
                let name = name.trim();
                if name.is_empty() || name == "*" || name == "_" {
                    return;
                }
                h.bound
                    .entry(name.to_string())
                    .and_modify(|b| {
                        if !external {
                            b.external = false;
                            b.module = module.to_string();
                        }
                        b.module_binding &= module_binding;
                    })
                    .or_insert_with(|| Binding {
                        module: module.to_string(),
                        external,
                        module_binding,
                    });
            };
        match lang {
            "python" => {
                for imp in &facts.imports {
                    match imp.kind.as_str() {
                        "import" => {
                            // `import a.b`, `import a.b as c`, `import a, b as c`
                            // (the plugin keeps a comma list in `path`, and
                            // the last item's alias in `alias`).
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
                                        (item, item.split('.').next().unwrap_or(item))
                                    }
                                };
                                if module.is_empty() {
                                    continue;
                                }
                                bind(local, module, !vocab.py_workspace(module), true);
                            }
                        }
                        "from-import" => {
                            let module = imp.path.trim();
                            if module.is_empty() {
                                continue;
                            }
                            let external = !vocab.py_workspace(module);
                            for item in imp.alias.split(',') {
                                let item =
                                    item.trim().trim_matches(|c| c == '(' || c == ')');
                                let local = match item.split_once(" as ") {
                                    Some((_, a)) => a.trim(),
                                    None => item.trim(),
                                };
                                bind(local, module, external, false);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "javascript" | "typescript" => {
                for imp in &facts.imports {
                    let spec = imp.path.trim();
                    match imp.kind.as_str() {
                        "import" if !imp.alias.trim().is_empty() => {
                            bind(&imp.alias, spec, !vocab.js_workspace(spec), true);
                        }
                        "from-import" => {
                            let external = !vocab.js_workspace(spec);
                            for item in imp.alias.split(',') {
                                let item = item.trim();
                                // `import axios from 'axios'` arrives as
                                // `default as axios`: the module's own value.
                                let (local, module_binding) =
                                    match item.split_once(" as ") {
                                        Some((orig, a)) => {
                                            (a.trim(), orig.trim() == "default")
                                        }
                                        None => (item, false),
                                    };
                                bind(local, spec, external, module_binding);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "go" => {
                let has_module = !facts.packages.is_empty();
                for imp in facts.imports.iter().filter(|i| i.kind == "import") {
                    let path = imp.path.trim();
                    let external = !vocab.go_workspace(path, has_module);
                    let alias = imp.alias.trim();
                    if !alias.is_empty() {
                        // `_` (side effect) and `.` (dot import) bind no
                        // qualifier.
                        if alias != "_" && alias != "." {
                            bind(alias, path, external, true);
                        }
                        continue;
                    }
                    let raw_last = path.rsplit('/').next().unwrap_or(path);
                    bind(raw_last, path, external, true);
                    let pkg = go_package_name(path);
                    if pkg != raw_last {
                        bind(&pkg, path, external, true);
                    }
                }
            }
            "java" | "kotlin" => {
                for p in &facts.packages {
                    if let Some(root) = p.split('.').next() {
                        h.jvm_roots.insert(root.to_string());
                    }
                }
                for imp in &facts.imports {
                    let path = imp.path.trim();
                    if let Some(root) = path.split('.').next() {
                        h.jvm_roots.insert(root.to_string());
                    }
                    let last = path.rsplit('.').next().unwrap_or(path);
                    let typed = last.starts_with(char::is_uppercase);
                    let local = if imp.alias.trim().is_empty() {
                        last
                    } else {
                        imp.alias.trim()
                    };
                    let pkg = jvm_package_of(path);
                    match (lang, imp.kind.as_str()) {
                        // Java `import a.b.C;` binds a type; `import a.b.*;`
                        // reaches the plugin as `a.b` and binds nothing.
                        ("java", "import") if typed => {
                            bind(local, pkg, !vocab.jvm_workspace(pkg), false);
                        }
                        // `import static a.b.C.m;` binds `m`.
                        ("java", "from-import") => {
                            bind(local, pkg, !vocab.jvm_workspace(pkg), false);
                        }
                        // Kotlin imports types and top-level functions alike.
                        ("kotlin", "import") => {
                            bind(local, pkg, !vocab.jvm_workspace(pkg), false);
                        }
                        _ => {}
                    }
                }
            }
            "csharp" => {
                let own = vocab
                    .cs_file_namespaces
                    .get(&facts.file)
                    .cloned()
                    .unwrap_or_default();
                for ns in &own {
                    insert_prefixes(ns, &mut h.cs_visible);
                }
                h.cs_visible.extend(vocab.cs_global_usings.iter().cloned());
                for imp in &facts.imports {
                    if imp.kind != "using" {
                        continue;
                    }
                    let path = imp.path.trim();
                    let path = cs_global_using(imp).unwrap_or(path);
                    if let Some(root) = path.split('.').next() {
                        h.cs_using_roots.insert(root.to_string());
                    }
                    if imp.alias.trim().is_empty() {
                        // A `using` inside a namespace is resolved against
                        // every enclosing namespace too: `using Pipeline;`
                        // in `MediatR.Tests` can mean `MediatR.Pipeline`.
                        h.cs_visible.insert(path.to_string());
                        for ns in &own {
                            let mut cut = ns.as_str();
                            loop {
                                h.cs_visible.insert(format!("{cut}.{path}"));
                                match cut.rsplit_once('.') {
                                    Some((head, _)) => cut = head,
                                    None => break,
                                }
                            }
                        }
                    } else {
                        bind(&imp.alias, path, !vocab.cs_workspace(path), false);
                    }
                }
            }
            "cpp" => {
                for imp in facts.imports.iter().filter(|i| i.kind == "namespace-alias") {
                    h.cpp_aliases.insert(
                        imp.alias.trim().to_string(),
                        imp.path.trim().to_string(),
                    );
                }
            }
            _ => {}
        }
        h
    }

    /// The external module a call is written through, when it is one —
    /// `Some(module)` means the call must not be name-matched onto a
    /// workspace callable. `None` means "not provably external".
    pub(crate) fn external_head(
        &self,
        lang: &str,
        r: &RefRecord,
        vocab: &WorkspaceVocab,
    ) -> Option<String> {
        if !matches!(
            lang,
            "python"
                | "javascript"
                | "typescript"
                | "go"
                | "java"
                | "kotlin"
                | "csharp"
                | "cpp"
        ) {
            return None;
        }
        let rh = r.receiver_hint.trim();
        if rh == cgg_core::VALUE_REF_HINT || rh == cgg_core::STRING_REF_HINT {
            return None;
        }
        if !rh.is_empty() && self.locals.shadows_receiver_head(r) {
            return None;
        }
        if rh.is_empty() {
            if r.dot_call || lang == "cpp" || self.own_defs.contains(r.name.as_str()) {
                return None;
            }
            // A bare call to a name the file imports from a dependency
            // (`from requests import post; post()`).
            return self
                .bound
                .get(r.name.as_str())
                .filter(|b| b.external)
                .map(|b| b.module.clone());
        }
        let path_shaped = is_path_shaped(rh);
        if lang == "cpp" {
            // `rapidjson::Value(x).Move()` / `std::make_shared<T>(…)->run()`:
            // the object is built by the call, and may be a workspace one.
            if !(r.path_receiver || rh.contains("::")) || !path_shaped {
                return None;
            }
            let raw_head = rh.split("::").next().unwrap_or("").trim();
            // Read the path through a namespace alias (`fs::` →
            // `std::filesystem::`).
            let path = match self.cpp_aliases.get(raw_head) {
                Some(target) => format!("{target}{}", &rh[raw_head.len()..]),
                None => rh.to_string(),
            };
            let segs: Vec<&str> = path.split("::").map(bare_scope).collect();
            let head = segs.first().copied().unwrap_or("");
            // `::f()` (global scope) and anything not a plain name.
            if head.is_empty() || !is_ident(head) {
                return None;
            }
            // Any scope on the path the tree declares keeps the call: a
            // namespace opened through a macro (`namespace KWSYS_NAMESPACE`,
            // spelled `cmsys::` by its callers) never reaches a qualified
            // name, but its classes (`cmsys::SystemTools`) do.
            if segs.iter().any(|s| vocab.cpp_scopes.contains(*s)) {
                return None;
            }
            return Some(head.to_string());
        }
        let head = leading_ident(rh);
        if head.is_empty() || matches!(head, "self" | "this" | "cls" | "super" | "Self") {
            return None;
        }
        if let Some(b) = self.bound.get(head) {
            return (b.external && (path_shaped || b.module_binding))
                .then(|| b.module.clone());
        }
        if !path_shaped {
            return None;
        }
        match lang {
            "java" | "kotlin" => self.jvm_package_head(lang, rh, head, vocab),
            "csharp" => self.cs_namespace_head(rh, head, vocab),
            _ => None,
        }
    }

    /// `java.lang.Integer.compare()`, `org.apache.x.Y.m()`,
    /// `kotlinx.coroutines.runBlocking {}`: a receiver written as a
    /// package path. Only a head that is some package root this file or
    /// the tree names (or a standard-library root) can start one — a
    /// lowercase local variable cannot be told from a package otherwise.
    fn jvm_package_head(
        &self,
        lang: &str,
        rh: &str,
        head: &str,
        vocab: &WorkspaceVocab,
    ) -> Option<String> {
        if !rh.contains('.') || !head.starts_with(char::is_lowercase) {
            return None;
        }
        let segs: Vec<&str> = rh.split('.').collect();
        if !segs.iter().all(|s| is_ident(s)) {
            return None;
        }
        let pkg_len = segs
            .iter()
            .take_while(|s| s.starts_with(char::is_lowercase))
            .count();
        let typed = pkg_len < segs.len();
        let std_root =
            cgg_core::stdlib::stdlib_names(lang).is_some_and(|s| s.contains(head));
        let file_root = self.jvm_roots.contains(head);
        let ws_root = vocab.jvm_roots.contains(head);
        let package_like = if typed {
            // `pkg.Type`: a lowercase run then a type is a package path.
            std_root || file_root || ws_root
        } else {
            // All lowercase: `app.config` could be a variable chain, so
            // only a root the tree itself never declares qualifies.
            (std_root || file_root) && !ws_root
        };
        if !package_like || pkg_len == 0 {
            return None;
        }
        let pkg = segs[..pkg_len].join(".");
        (!vocab.jvm_workspace(&pkg)).then_some(pkg)
    }

    /// `System.IO.Path.GetTempPath()`: a dotted receiver headed by the
    /// root of a namespace this file `using`s (or a standard-library
    /// root) that no workspace namespace shares.
    fn cs_namespace_head(
        &self,
        rh: &str,
        head: &str,
        vocab: &WorkspaceVocab,
    ) -> Option<String> {
        if !rh.contains('.') || vocab.cs_roots.contains(head) {
            return None;
        }
        let std_root =
            cgg_core::stdlib::stdlib_names("csharp").is_some_and(|s| s.contains(head));
        if !(std_root || self.cs_using_roots.contains(head)) {
            return None;
        }
        let segs: Vec<&str> = rh.split('.').collect();
        if !segs.iter().all(|s| is_ident(s)) {
            return None;
        }
        (!vocab.cs_workspace(rh)).then(|| head.to_string())
    }

    /// C#: whether a workspace callable's type lives in a namespace a bare
    /// type name written in this file can reach. Unknown namespaces
    /// (a file that declares none) are visible.
    pub(crate) fn cs_candidate_visible(
        &self,
        vocab: &WorkspaceVocab,
        file: FileId,
        qualified_name: &str,
    ) -> bool {
        let Some(declared) = vocab.cs_file_namespaces.get(&file) else {
            return true;
        };
        // A block namespace prefixes the qualified name; a file-scoped one
        // does not, but is then the file's only namespace.
        let ns = declared
            .iter()
            .filter(|n| {
                qualified_name
                    .strip_prefix(n.as_str())
                    .is_some_and(|r| r.starts_with('.'))
            })
            .max_by_key(|n| n.len())
            .or_else(|| (declared.len() == 1).then(|| &declared[0]));
        match ns {
            Some(ns) => self.cs_visible.contains(ns.as_str()),
            None => true,
        }
    }
}

/// `Some(namespace)` for a `global using N;`. The C# plugin keeps the
/// directive's text after `using`, so a global one arrives as
/// `global using N` in `path`.
fn cs_global_using(imp: &cgg_core::ImportRecord) -> Option<&str> {
    if imp.kind != "using" || !imp.alias.trim().is_empty() {
        return None;
    }
    let rest = imp.path.trim().strip_prefix("global")?.trim_start();
    let rest = rest.strip_prefix("using")?.trim_start();
    if rest.starts_with("static ") || rest.is_empty() {
        return None;
    }
    Some(rest)
}

/// A receiver written as a name path — `requests`, `np.linalg`,
/// `ImmutableMap.Builder<K, V>`, `boost::algorithm` — rather than an
/// expression that computes a value (`zap.S()`, `` sql`…` ``, `a[0]`).
fn is_path_shaped(rh: &str) -> bool {
    let mut depth = 0i32;
    for c in rh.chars() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ if depth > 0 => {}
            c if c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | ':') => {}
            _ => return false,
        }
    }
    depth == 0
}

/// The leading identifier of a receiver expression (`axios` in
/// `axios.create()`, `zap` in `zap.S()`).
fn leading_ident(rh: &str) -> &str {
    let end = rh
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
        .map_or(rh.len(), |(i, _)| i);
    &rh[..end]
}

fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

/// `Foo<T>` → `Foo`; whitespace trimmed.
fn bare_scope(s: &str) -> &str {
    s.split('<').next().unwrap_or(s).trim()
}

/// The npm package a bare specifier names: `@scope/name` or `name`.
pub(crate) fn npm_package_of(spec: &str) -> &str {
    let mut it = spec.match_indices('/');
    if spec.starts_with('@') {
        it.next();
    }
    match it.next() {
        Some((i, _)) => &spec[..i],
        None => spec,
    }
}

/// The package part of a JVM import path: the lowercase segments before
/// the first type (`org.apache.commons.lang3` of
/// `org.apache.commons.lang3.StringUtils`), or everything but the last
/// segment when no segment is a type (a Kotlin top-level function).
fn jvm_package_of(path: &str) -> &str {
    let mut end = 0usize;
    let mut typed = false;
    for (i, seg) in path.split('.').enumerate() {
        if seg.starts_with(char::is_uppercase) {
            typed = true;
            break;
        }
        end += seg.len() + usize::from(i > 0);
    }
    if typed {
        &path[..end]
    } else {
        path.rsplit_once('.').map_or("", |(p, _)| p)
    }
}

/// The name a Go import binds when it has no alias: the package name,
/// which by near-universal convention is the last path element without a
/// major-version suffix (`…/v2`, `yaml.v3`) or a `go-` prefix.
pub(crate) fn go_package_name(path: &str) -> String {
    let mut segs: Vec<&str> = path.split('/').collect();
    let mut last = segs.pop().unwrap_or(path);
    let is_major = |s: &str| {
        s.len() >= 2 && s.starts_with('v') && s[1..].chars().all(|c| c.is_ascii_digit())
    };
    if is_major(last)
        && let Some(prev) = segs.pop()
    {
        last = prev;
    }
    if let Some((stem, ver)) = last.rsplit_once('.')
        && is_major(ver)
    {
        last = stem;
    }
    last.strip_prefix("go-").unwrap_or(last).to_string()
}

/// Every ancestor of a dotted name, and the name itself.
fn insert_prefixes(dotted: &str, out: &mut HashSet<String>) {
    let mut cut = dotted;
    out.insert(cut.to_string());
    while let Some((head, _)) = cut.rsplit_once('.') {
        out.insert(head.to_string());
        cut = head;
    }
}

fn normal_components(p: &Path) -> Vec<String> {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str().map(str::to_string),
            _ => None,
        })
        .collect()
}

/// Directory names along `path`, plus its file stem.
fn path_names(path: &Path, out: &mut HashSet<String>) {
    if let Some(dir) = path.parent() {
        out.extend(normal_components(dir));
    }
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        // `component.test.ts`, `index.d.ts` — the module is the first dot;
        // a loader that names modules by their last dots (`humhub.client.js`
        // required as `client`, `humhub.ui.loader.js` as `ui.loader`) reads
        // a dot suffix.
        out.insert(stem.split('.').next().unwrap_or(stem).to_string());
        let mut rest = stem;
        loop {
            out.insert(rest.to_string());
            match rest.split_once('.') {
                Some((_, tail)) if !tail.is_empty() => rest = tail,
                _ => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_package_names() {
        assert_eq!(npm_package_of("axios"), "axios");
        assert_eq!(npm_package_of("lodash/debounce"), "lodash");
        assert_eq!(npm_package_of("@tryghost/errors"), "@tryghost/errors");
        assert_eq!(npm_package_of("@scope/pkg/sub/x"), "@scope/pkg");
    }

    #[test]
    fn jvm_package_parts() {
        assert_eq!(
            jvm_package_of("org.apache.commons.lang3.StringUtils"),
            "org.apache.commons.lang3"
        );
        assert_eq!(jvm_package_of("com.acme.Outer.Inner"), "com.acme");
        assert_eq!(
            jvm_package_of("kotlinx.coroutines.runBlocking"),
            "kotlinx.coroutines"
        );
        assert_eq!(jvm_package_of("single"), "");
    }

    #[test]
    fn go_package_names() {
        assert_eq!(go_package_name("github.com/google/uuid"), "uuid");
        assert_eq!(go_package_name("github.com/go-redis/redis/v8"), "redis");
        assert_eq!(go_package_name("gopkg.in/yaml.v3"), "yaml");
        assert_eq!(go_package_name("github.com/mattn/go-sqlite3"), "sqlite3");
        assert_eq!(go_package_name("fmt"), "fmt");
    }

    fn facts(lang: &str, path: &str, imports: &[(&str, &str, &str)]) -> FileFacts {
        let mut f = FileFacts::new(FileId::new(0), std::path::PathBuf::from(path), lang);
        f.imports = imports
            .iter()
            .map(|(k, p, a)| cgg_core::ImportRecord {
                kind: (*k).into(),
                path: (*p).into(),
                alias: (*a).into(),
                site_line: 1,
                site_byte: 0,
            })
            .collect();
        f
    }

    fn call(name: &str, recv: &str) -> RefRecord {
        RefRecord {
            name: name.into(),
            receiver_hint: recv.into(),
            ..Default::default()
        }
    }

    #[test]
    fn path_shaped_receivers() {
        assert!(is_path_shaped("requests"));
        assert!(is_path_shaped("np.linalg"));
        assert!(is_path_shaped("ImmutableMap.Builder<K, V>"));
        assert!(is_path_shaped("boost::algorithm"));
        assert!(!is_path_shaped("zap.S()"));
        assert!(!is_path_shaped("sql`x`"));
        assert!(!is_path_shaped("a[0]"));
        assert!(!is_path_shaped("Foo<Bar"));
    }

    #[test]
    fn chained_calls_are_refused_only_off_a_module_binding() {
        let vocab = WorkspaceVocab::default();
        let py = facts(
            "python",
            "app/main.py",
            &[
                ("import", "requests", ""),
                ("from-import", "typing", "cast"),
            ],
        );
        let h = FileHeads::build(&py, &vocab);
        assert_eq!(
            h.external_head("python", &call("json", "requests.get(u)"), &vocab),
            Some("requests".into())
        );
        // `cast(Foo, x)` is an imported *function*; its result can be a
        // workspace value.
        assert_eq!(
            h.external_head("python", &call("run", "cast(Foo, x)"), &vocab),
            None
        );
        let java = facts(
            "java",
            "A.java",
            &[("from-import", "org.mockito.Mockito.verify", "verify")],
        );
        let h = FileHeads::build(&java, &vocab);
        assert_eq!(
            h.external_head("java", &call("run", "verify(m)"), &vocab),
            None
        );
        assert_eq!(
            h.external_head("java", &call("verify", ""), &vocab),
            Some("org.mockito".into())
        );
    }

    #[test]
    fn a_bare_name_the_file_defines_is_never_refused() {
        let vocab = WorkspaceVocab::default();
        let mut f = facts("javascript", "a.min.js", &[("import", "tty", "b")]);
        f.definitions.push(cgg_core::DefRecord {
            simple_name: "b".into(),
            qualified_name: "b".into(),
            ..Default::default()
        });
        let h = FileHeads::build(&f, &vocab);
        assert_eq!(h.external_head("javascript", &call("b", ""), &vocab), None);
    }

    #[test]
    fn js_path_aliases_and_workspace_packages_are_not_external() {
        let mut ws = facts("typescript", "src/libs/actions/Policy/Policy.ts", &[]);
        ws.packages = vec!["@acme/app".into(), "@userActions/*".into(), "@/".into()];
        let vocab = WorkspaceVocab::build(&Graph::new(), &[ws]);
        for spec in [
            "@userActions/Policy/Policy",
            "@acme/app",
            "@acme/app/sub",
            "@/x",
            "./y",
        ] {
            assert!(vocab.js_workspace(spec), "{spec}");
        }
        // SvelteKit `$lib`, a NODE_PATH `require('Policy.js')`, a loader
        // module named by a file's dot suffix (`humhub.ui.loader.js`).
        let mut more = facts("javascript", "static/js/humhub.ui.loader.js", &[]);
        more.packages = vec![];
        let vocab2 = WorkspaceVocab::build(&Graph::new(), &[more]);
        for spec in ["$lib/utils", "humhub.js", "ui.loader", "loader"] {
            assert!(vocab2.js_workspace(spec), "{spec}");
        }
        for spec in ["axios", "@tryghost/errors", "node:fs", "@acme/other"] {
            assert!(!vocab.js_workspace(spec), "{spec}");
        }
    }

    #[test]
    fn csharp_using_inside_a_namespace_is_relative_to_it() {
        let mut lib = facts("csharp", "Lib.cs", &[]);
        lib.file = FileId::new(1);
        lib.packages = vec!["MediatR.Pipeline".into()];
        let mut caller = facts("csharp", "T.cs", &[("using", "Pipeline", "")]);
        caller.packages = vec!["MediatR.Tests".into()];
        let vocab = WorkspaceVocab::build(&Graph::new(), &[lib, caller.clone()]);
        let h = FileHeads::build(&caller, &vocab);
        assert!(h.cs_candidate_visible(&vocab, FileId::new(1), "State.SetHandled"));
    }

    #[test]
    fn leading_identifier_of_a_receiver() {
        assert_eq!(leading_ident("axios"), "axios");
        assert_eq!(leading_ident("zap.S()"), "zap");
        assert_eq!(leading_ident("$http.get"), "$http");
        assert_eq!(leading_ident("(a).b"), "");
    }
}
