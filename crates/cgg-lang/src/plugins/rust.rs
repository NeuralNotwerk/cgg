//! Rust plugin.
//!
//! Two-phase AST pass over a `tree-sitter-rust` tree:
//!
//! * **Definitions** — `function_item` (plus its `async` / `unsafe` /
//!   `pub` flavors), inherent-`impl` methods, trait-`impl` methods,
//!   trait-default methods, free functions inside `mod` blocks, and
//!   named closures (`let foo = |x| { ... };`).
//! * **References** — every `call_expression` whose callee is an
//!   identifier, a path (`a::b::c()`), or a method (`recv.method()`).
//! * **Imports** — `use_declaration` records, flattened into one entry
//!   per imported path with optional alias.
//!
//! Qualified names are assembled from a scope stack seeded with
//! `"crate"`: each `mod_item` pushes its identifier; each `impl_item`
//! pushes the implemented type (prefixed with `"<trait> for "` for
//! trait impls so the path stays unique). A leaf `function_item` is
//! then emitted as `crate::mod_a::mod_b::Type::method` or
//! `crate::mod_a::free_fn` as appropriate.

use std::path::Path;

use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use tree_sitter::{Node, Tree, TreeCursor};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct RustPlugin;

impl LanguagePlugin for RustPlugin {
    fn id(&self) -> &'static str {
        "rust"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".rs"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            value_refs: true,
            attributes: true,
            exports: true,
            impls: true,
            test_defs: true,
            unreachable: true,
            visibility: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "rust");
        let (crate_root, module_segments) = rust_module_path(path);
        // Record the crate root as a synthetic import so downstream
        // consumers (cross-file resolver, re-export chain builder)
        // know which crate this file belongs to, even when the file
        // has no callable definitions of its own (e.g., a lib.rs
        // that only re-exports).
        facts.imports.push(ImportRecord {
            kind: "crate-root".into(),
            path: crate_root.clone(),
            alias: String::new(),
            site_line: 1,
            site_byte: 0,
        });
        // A dependency renamed in Cargo.toml (`common = { package =
        // "nkb-common" }`) is written `common::f()` in source; recorded as
        // `use nkb_common as common` so the resolver sees the real crate.
        for (alias, package) in cargo_dependency_renames(path) {
            facts.imports.push(ImportRecord {
                kind: "use".into(),
                path: package,
                alias,
                site_line: 1,
                site_byte: 0,
            });
        }
        let mut scope: Vec<ScopeSegment> = vec![ScopeSegment::Crate(crate_root)];
        for seg in module_segments {
            scope.push(ScopeSegment::Mod(seg));
        }
        let mut walker = Walker {
            source,
            facts: &mut facts,
            scope,
            struct_fields: std::collections::HashMap::new(),
            bases: Vec::new(),
            // Most files define no macro; skip the extra tree walk.
            macros: if mentions_macro_rules(source) {
                collect_macro_defs(tree.root_node(), source)
            } else {
                std::collections::HashMap::new()
            },
            expanding: false,
            parser: None,
        };
        walker.walk(tree.root_node());
        // Post-pass: emit synthetic LocalTypes of the form
        // `self.<field>` -> `<FieldType>` for each method in an impl
        // block, so the type propagator + cross-file resolver can map
        // `self.store.foo()` to `ChunkStore::foo()` etc.
        let struct_fields = std::mem::take(&mut walker.struct_fields);
        emit_self_field_local_types(&mut facts, &struct_fields);
        facts
            .classes
            .extend(rust_type_links(tree.root_node(), source));

        if ctx.deadcode_signals {
            facts.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::RUST);
        }
        // `pub use` is Rust's export surface: a re-exported name is part
        // of the crate's public API even though nothing inside the crate
        // calls it by that path.
        facts.exports = facts
            .imports
            .iter()
            .filter(|i| i.kind == "pub-use")
            .map(|i| {
                let name = if i.alias.is_empty() {
                    i.path.rsplit("::").next().unwrap_or(&i.path).to_string()
                } else {
                    i.alias.clone()
                };
                cgg_core::ExportRecord {
                    name,
                    kind: "pub-use".into(),
                    target: i.path.clone(),
                }
            })
            .filter(|e| !e.name.is_empty() && e.name != "*")
            .collect();
        facts
    }
}

/// Compute `(crate_root, [module_segment, ...])` for a Rust source file.
///
/// Walks up to find a `Cargo.toml`. The path from the crate's `src/`
/// directory down to the file becomes the module segments:
///
/// * `src/lib.rs` / `src/main.rs`       -> no module segments.
/// * `src/foo.rs`                       -> `foo`.
/// * `src/foo/mod.rs`                   -> `foo`.
/// * `src/foo/bar.rs`                   -> `foo::bar`.
/// * `src/bin/name.rs`                  -> `` (binary roots; no segments).
/// * `tests/name.rs`                    -> `tests::name`.
/// * `tests/dir/main.rs`                -> `tests::dir`.
/// * `benches/name.rs`, `examples/name.rs` -> `` (own roots; no segments).
fn rust_module_path(path: &Path) -> (String, Vec<String>) {
    let (crate_root, crate_dir) = match crate_dir_for(path) {
        Some((name, dir)) => (name.replace('-', "_"), dir),
        None => return ("crate".to_string(), Vec::new()),
    };

    // Relative path from the crate root to the file.
    let rel = match path.strip_prefix(&crate_dir) {
        Ok(p) => p.to_path_buf(),
        Err(_) => return (crate_root, Vec::new()),
    };

    let components: Vec<String> = rel
        .components()
        .filter_map(|c| c.as_os_str().to_str().map(|s| s.to_string()))
        .collect();

    // Drop the leading `src` or `tests`/`benches`/etc. container.
    let segs: Vec<String> = match components.first().map(|s| s.as_str()) {
        Some("src") => components[1..].to_vec(),
        Some("tests") => {
            // Each integration test file is its own compilation unit,
            // but (unlike benches/examples) it still lives in the same
            // workspace target graph as the library and is a common
            // place for a helper to share a name with something in
            // `src/` (`run`, `fixture`, `write`, …). Leaving these
            // unqualified at the bare crate root made `by_qn.insert`
            // last-write-wins onto whichever definition was walked
            // last, producing false edges from `src/` into test-file
            // helpers. Qualify under `<crate>::tests::` instead:
            //   `tests/<name>.rs`         -> tests::<name>
            //   `tests/<dir>/main.rs`     -> tests::<dir>  (cargo's own
            //                                convention: a directory
            //                                target's entry point)
            let rest = &components[1..];
            if rest.is_empty() {
                return (crate_root, Vec::new());
            }
            let mut mods: Vec<String> = vec!["tests".to_string()];
            let last = rest.last().map(|s| s.as_str()).unwrap_or("");
            if rest.len() > 1 && last == "main.rs" {
                mods.push(rest[rest.len() - 2].clone());
            } else if let Some(stem) = std::path::Path::new(last)
                .file_stem()
                .and_then(|s| s.to_str())
            {
                mods.push(stem.to_string());
            }
            return (crate_root, mods);
        }
        Some("benches") | Some("examples") => {
            // Each bench/example is its own compilation unit — treat
            // them as separate roots with no shared module path.
            return (crate_root, Vec::new());
        }
        _ => components,
    };

    if segs.is_empty() {
        return (crate_root, Vec::new());
    }

    // `lib.rs` and `main.rs` sit at the crate root.
    let last = segs.last().map(|s| s.as_str()).unwrap_or("");
    if segs.len() == 1 && matches!(last, "lib.rs" | "main.rs") {
        return (crate_root, Vec::new());
    }

    // `bin/<name>.rs` / `bin/<name>/main.rs` — binary target. Strip
    // the `bin` segment; the bin is its own crate-root-equivalent.
    if segs.first().map(|s| s.as_str()) == Some("bin") {
        return (crate_root, Vec::new());
    }

    // Drop the file extension on the last segment.
    let mut mods: Vec<String> = segs.iter().take(segs.len() - 1).cloned().collect();
    let last = segs.last().unwrap();
    if last == "mod.rs" {
        // `foo/mod.rs` -> [foo] (mods already contains ["foo"])
    } else if let Some(stem) = std::path::Path::new(last)
        .file_stem()
        .and_then(|s| s.to_str())
    {
        mods.push(stem.to_string());
    }
    (crate_root, mods)
}

/// Walk up to find the enclosing `Cargo.toml` and return (crate name, crate root dir).
fn crate_dir_for(path: &Path) -> Option<(String, std::path::PathBuf)> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        let cargo = d.join("Cargo.toml");
        if cargo.exists() {
            if let Ok(text) = std::fs::read_to_string(&cargo)
                && let Some(name) = extract_package_name(&text)
            {
                return Some((name, d.to_path_buf()));
            }
            return None;
        }
        dir = d.parent();
    }
    None
}

/// `(alias, package)` for every dependency of the file's crate that is
/// renamed (`alias = { package = "real-name", … }`, the table form
/// `[dependencies.alias]` + `package = …`, and `alias = { workspace = true
/// }` against a rename in the workspace root's `[workspace.dependencies]`).
/// Package names are returned as crate roots (`-` -> `_`).
fn cargo_dependency_renames(path: &Path) -> Vec<(String, String)> {
    let Some((_, crate_dir)) = crate_dir_for(path) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(crate_dir.join("Cargo.toml")) else {
        return Vec::new();
    };
    let (mut renames, inherited) = parse_dependency_renames(&text, false);
    // `[lib] name = "app_lib"`: the binary in the same package calls its
    // own library as `app_lib::run()`.
    if let Some(lib) = section_value(&text, "lib", "name")
        && let Some(pkg) = extract_package_name(&text)
    {
        let (lib, pkg) = (lib.replace('-', "_"), pkg.replace('-', "_"));
        if lib != pkg {
            renames.push((lib, pkg));
        }
    }
    if !inherited.is_empty() {
        // Nearest ancestor manifest that declares `[workspace]`.
        let mut dir = crate_dir.parent();
        while let Some(d) = dir {
            if let Ok(root) = std::fs::read_to_string(d.join("Cargo.toml"))
                && root.lines().any(|l| l.trim() == "[workspace]")
            {
                let (ws, _) = parse_dependency_renames(&root, true);
                renames.extend(ws.into_iter().filter(|(a, _)| inherited.contains(a)));
                break;
            }
            dir = d.parent();
        }
    }
    renames.sort();
    renames.dedup();
    renames
}

/// `key = "value"` inside `[section]`.
fn section_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let t = line.split('#').next().unwrap_or("").trim();
        if let Some(h) = t.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            inside = h.trim() == section;
            continue;
        }
        if inside
            && let Some((k, v)) = t.split_once('=')
            && k.trim() == key
        {
            let v = v.trim().strip_prefix('"')?;
            return Some(v[..v.find('"')?].to_string());
        }
    }
    None
}

/// Renames declared in one manifest's dependency tables, plus the aliases
/// that inherit their spec from the workspace (`{ workspace = true }`).
/// `workspace` selects `[workspace.dependencies]` instead of the crate's
/// own tables.
fn parse_dependency_renames(
    text: &str,
    workspace: bool,
) -> (Vec<(String, String)>, Vec<String>) {
    // `key = "value"`, at any occurrence of `key` followed by `=`
    // (`path = "../package-x"` must not hide a later `package = "…"`).
    let quoted = |v: &str, key: &str| -> Option<String> {
        v.match_indices(key).find_map(|(i, _)| {
            let before_ok = v[..i]
                .chars()
                .next_back()
                .is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'));
            let rest = v[i + key.len()..]
                .trim_start()
                .strip_prefix('=')?
                .trim_start();
            let rest = rest.strip_prefix('"')?;
            before_ok.then(|| rest[..rest.find('"').unwrap_or(rest.len())].to_string())
        })
    };
    let is_dep_table = |h: &str| {
        if workspace {
            h == "workspace.dependencies"
        } else {
            h.ends_with("dependencies") && !h.starts_with("workspace.")
        }
    };
    let (mut out, mut inherited) = (Vec::new(), Vec::new());
    let mut section = String::new();
    // `[dependencies.alias]`: the alias whose table we are in.
    let mut table_alias: Option<String> = None;
    for line in text.lines() {
        let t = line.split('#').next().unwrap_or("").trim();
        if t.is_empty() {
            continue;
        }
        if let Some(h) = t.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            let h = h.trim();
            table_alias = None;
            section.clear();
            if let Some((tab, alias)) = h.rsplit_once('.')
                && is_dep_table(tab)
            {
                table_alias = Some(alias.trim_matches('"').to_string());
            } else {
                section = h.to_string();
            }
            continue;
        }
        if let Some(alias) = &table_alias {
            if t.starts_with("package")
                && let Some(pkg) = quoted(t, "package")
            {
                out.push((alias.replace('-', "_"), pkg.replace('-', "_")));
            }
            continue;
        }
        if !is_dep_table(&section) {
            continue;
        }
        let Some((alias, spec)) = t.split_once('=') else {
            continue;
        };
        let alias = alias.trim().trim_matches('"').replace('-', "_");
        if let Some(pkg) = quoted(spec, "package") {
            let pkg = pkg.replace('-', "_");
            if pkg != alias {
                out.push((alias, pkg));
            }
        } else if spec.contains("workspace") && spec.contains("true") {
            inherited.push(alias);
        }
    }
    (out, inherited)
}

/// Best-effort extraction of `name = "…"` from a `[package]` section.
/// No full TOML parse to keep the plugin's dependency footprint small.
fn extract_package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = trimmed == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start_matches([' ', '\t']);
            if let Some(rest) = rest.strip_prefix('=') {
                let value = rest.trim();
                if let Some(stripped) =
                    value.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
                {
                    return Some(stripped.to_string());
                }
            }
        }
    }
    None
}

/// A scope-stack entry tagged by origin. Joined into qualified names
/// via `::`; the tag lets us classify a leaf function correctly
/// (inherent method vs trait method vs free function).
#[derive(Clone, Debug)]
enum ScopeSegment {
    Crate(String),
    Mod(String),
    InherentImpl(String),
    TraitImpl(String),
    Trait(String),
}

impl ScopeSegment {
    fn display(&self) -> &str {
        match self {
            ScopeSegment::Crate(s)
            | ScopeSegment::Mod(s)
            | ScopeSegment::InherentImpl(s)
            | ScopeSegment::TraitImpl(s)
            | ScopeSegment::Trait(s) => s.as_str(),
        }
    }
}

/// In-progress walker owning a scope stack.
struct Walker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<ScopeSegment>,
    /// Per-file map of struct definitions to their fields. Populated by
    /// the `struct_item` visit case; consumed in the post-pass that
    /// emits `self.<field>` LocalTypes for methods.
    struct_fields: std::collections::HashMap<String, Vec<(String, String)>>,
    /// Trait being implemented by each enclosing `impl Trait for Type`,
    /// innermost last. Recorded on every method inside so framework
    /// base-type rules (`actix` `Handler`/`StreamHandler`, tower
    /// `Service`) can match — the trait name is already parsed for the
    /// scope segment; this is the same string kept where a rule can see
    /// it. An inherent `impl Type` pushes an empty entry so the depth
    /// always mirrors the scope stack.
    bases: Vec<Vec<String>>,
    /// This file's own `macro_rules!` definitions that the expander can
    /// handle, by name. Only same-file definitions are expanded: a macro
    /// defined in the tree is code cgg can read, while `env!`/`concat!`/
    /// crates.io macros are not, and are never expanded by accident.
    macros: std::collections::HashMap<String, super::rust_macro::MacroDef>,
    /// Set inside an expansion, so an expansion never expands again.
    expanding: bool,
    /// One parser for every expansion in the file, made on first use.
    parser: Option<tree_sitter::Parser>,
}

impl<'a> Walker<'a> {
    fn text(&self, node: Node) -> &str {
        node.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        let kind = node.kind();

        match kind {
            // `#[serde(default = "default_hops")]` names a function that
            // serde calls; recorded as a mention (never an edge) and the
            // attribute is then walked exactly as before.
            "attribute_item" | "inner_attribute_item" => {
                self.record_attr_mentions(node);
            }
            // `mod foo { ... }` / `mod foo;`
            "mod_item" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // A declared module is in scope by name wherever its file
                // lives (`#[path = "../scripts/x.rs"] mod perf;`), so a call
                // through it is never a call into an external crate.
                if !name.is_empty() && !self.expanding {
                    self.facts.imports.push(ImportRecord {
                        kind: "use".into(),
                        path: format!("self::{name}"),
                        alias: name.clone(),
                        site_line: node.start_position().row as u32 + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
                if !name.is_empty() {
                    self.scope.push(ScopeSegment::Mod(name));
                }
                self.walk_children(node);
                if !self
                    .scope
                    .last()
                    .is_none_or(|s| matches!(s, ScopeSegment::Crate(_)))
                {
                    self.scope.pop();
                }
                return;
            }

            // `impl Type { ... }` or `impl Trait for Type { ... }`
            "impl_item" => {
                let type_name = node
                    .child_by_field_name("type")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_else(|| "<unknown>".into());
                let trait_name = node
                    .child_by_field_name("trait")
                    .map(|n| self.text(n).to_string());
                let segment = match &trait_name {
                    Some(t) => ScopeSegment::TraitImpl(format!("<{type_name} as {t}>")),
                    None => ScopeSegment::InherentImpl(type_name.clone()),
                };
                self.scope.push(segment);
                // Stored as written, generics included — the matcher
                // strips `<…>` itself, and the full form is what a
                // reader of the audit log needs to see.
                self.bases.push(trait_name.clone().into_iter().collect());
                self.walk_children(node);
                self.bases.pop();
                self.scope.pop();
                return;
            }

            // `trait T { fn default_m() { ... } }`
            "trait_item" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let pushed = !name.is_empty();
                if pushed {
                    self.scope.push(ScopeSegment::Trait(name));
                }
                self.walk_children(node);
                if pushed {
                    self.scope.pop();
                }
                return;
            }

            // `fn foo(...) { ... }`
            "function_item" => {
                self.record_function(node, /* is_trait_default */ false);
                self.walk_children(node);
                return;
            }

            // `fn foo(...);` — trait method signatures without bodies
            // appear inside trait_item as function_signature_item.
            "function_signature_item" => {
                self.record_function(node, /* sig only */ false);
                // No body to descend into.
                return;
            }

            // `macro_rules! name { ... }` — define a macro as a callable.
            "macro_definition" => {
                self.record_macro(node);
                // Don't walk into macro body (token trees aren't useful).
                return;
            }

            // `struct Foo { a: A, b: B }` — record field names+types so
            // the post-pass can emit `self.a` / `self.b` LocalTypes for
            // methods in `impl Foo`. Tuple structs (`struct Foo(A, B);`)
            // and unit structs are intentionally skipped — they have no
            // named field receivers.
            "struct_item" => {
                self.record_struct_fields(node);
                self.walk_children(node);
                return;
            }

            // `name!(args)` — macro invocation is a call site, and so is
            // anything called *inside* its arguments.
            "macro_invocation" => {
                if let Some(r) = self.ref_from_macro_invocation(node) {
                    self.facts.references.push(r);
                }
                // An item-position invocation of a macro defined in this
                // file is expanded: what it generates becomes callables,
                // and its calls are attributed to them.
                if self.expand_item_macro(node) {
                    return;
                }
                self.refs_from_token_tree(node);
                self.walk_children(node);
                return;
            }

            // `use a::b::c;`, `use a::b::c as d;`
            "use_declaration" => {
                self.record_use(node);
                // Still walk children for completeness.
                self.walk_children(node);
                return;
            }

            // `let foo = |x| { ... };` -> named closure.
            // `let x = Foo::new(...)` -> infer type Foo for x.
            "let_declaration" => {
                if let Some(rec) = self.named_closure(node) {
                    self.facts.definitions.push(rec);
                }
                self.infer_let_type(node);
                self.walk_children(node);
                return;
            }

            // Anonymous closure / async block. We only treat these as
            // standalone callables when they're being *spawned* — i.e.,
            // passed as the argument to `tokio::spawn`, `std::thread::spawn`,
            // `rayon::spawn`, `*::spawn_blocking`, `*::spawn_local`.
            // Tight callbacks like `.map(|x| x + 1)` stay inline.
            //
            // For a spawned closure we (a) emit a DefRecord so the body's
            // calls attribute to it (smallest-enclosing-byte-range), and
            // (b) emit a synthetic RefRecord at the spawn call site so the
            // graph carries an `enclosing_fn -> closure` edge. The reader
            // sees the disjoint subgraph plus an explicit "this is where
            // it gets spawned" pointer.
            "closure_expression" | "async_block" => {
                if let Some(spawn_call) = self.spawn_call_for(node) {
                    self.emit_spawned_closure(node, spawn_call);
                }
                self.walk_children(node);
                return;
            }

            // References.
            "call_expression" => {
                if let Some(r) = self.ref_from_call(node) {
                    self.facts.references.push(r);
                }
                self.refs_from_args(node);
                self.walk_children(node);
                return;
            }

            _ => {}
        }

        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut cursor: TreeCursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                self.walk(cursor.node());
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_function(&mut self, node: Node, _sig_only: bool) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }

        let visibility = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "visibility_modifier")
            .map(|c| self.text(c).to_string())
            .unwrap_or_default();

        // Detect async and unsafe markers for the signature hint.
        let mut is_async = false;
        {
            let mut c = node.walk();
            for child in node.children(&mut c) {
                match child.kind() {
                    "async" => is_async = true,
                    "function_modifiers" => {
                        let mut mc = child.walk();
                        for m in child.children(&mut mc) {
                            if m.kind() == "async" {
                                is_async = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Classify variant from scope context.
        let variant = if is_async {
            DefVariant::AsyncFunction
        } else {
            match self.scope.last() {
                Some(ScopeSegment::TraitImpl(_)) => DefVariant::TraitMethod,
                Some(ScopeSegment::InherentImpl(_)) => DefVariant::InherentMethod,
                Some(ScopeSegment::Trait(_)) => DefVariant::TraitDefaultMethod,
                _ => DefVariant::FreeFunction,
            }
        };

        // A trait item carries no visibility of its own: its effective
        // visibility is the trait's, and for `impl Trait for T` the
        // trait may be declared in another file entirely. So an absent
        // token here means "inherited", not "private" — reporting a
        // method of a `pub trait` as private would tell a dead-code
        // reader that no out-of-tree caller can exist, which for a
        // library crate's own API is false. cgg does not compute the
        // inherited value, so it says `Unknown` and claims nothing.
        let vis = match self.scope.last() {
            Some(ScopeSegment::Trait(_) | ScopeSegment::TraitImpl(_))
                if visibility.is_empty() =>
            {
                cgg_core::Vis::Unknown
            }
            _ => rust_vis(&visibility),
        };

        let qn = qualified_name(&self.scope, &simple);
        let (sl, el) = line_range(node);
        let signature = super::extract_signature(self.text(node));

        let attributes = collect_attributes(node, self.source);

        // `extern "C"` is a modifier *inside* `function_item`, so
        // `collect_attributes` (which reads preceding siblings) can
        // never see it. Without this, `pub extern "C" fn` without an
        // explicit `#[no_mangle]` is invisible to the FFI classifier.
        let mut attributes = attributes;
        if self.text(node).contains("extern \"C\"") {
            attributes.push("extern:C".to_string());
        }
        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: signature,
            vis,
            test_role: rust_test_role(&attributes),
            visibility,
            attributes,
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    /// Detect `let NAME = |..| {..};` and treat the binding as a
    /// named closure definition.
    fn infer_let_type(&mut self, node: Node) {
        // Infer the type of `let x = Foo::new(...)` / `Foo::builder()` from
        // the initializer's syntax alone (Issue 5). Deliberately limited to
        // an *unqualified* `Type::assoc_fn()` call: typing more initializer
        // forms (qualified paths, struct/enum literals) over-types locals
        // and, with same-named locals of different builder types in one
        // file, mis-resolves their method calls.
        let pat = node.child_by_field_name("pattern");
        let val = node.child_by_field_name("value");
        let (Some(pat), Some(val)) = (pat, val) else {
            return;
        };
        let var_name = if pat.kind() == "identifier" {
            self.text(pat).to_string()
        } else {
            return;
        };
        if var_name.is_empty() {
            return;
        }
        // Check if value is a call_expression with a path that starts with a type
        let call = if val.kind() == "call_expression" {
            Some(val)
        } else if val.kind() == "try_expression" || val.kind() == "await_expression" {
            // `Foo::new()?` or `Foo::new().await`
            val.child(0).filter(|c| c.kind() == "call_expression")
        } else {
            None
        };
        let Some(call) = call else { return };
        let func = call.child_by_field_name("function");
        let Some(func) = func else { return };
        // Look for `Foo::new`, `Foo::builder`, `Foo::default`, `Foo::from`
        if func.kind() == "scoped_identifier" || func.kind() == "field_expression" {
            let text = self.text(func);
            if let Some((type_part, wrapper_name, assoc_fn)) = ctor_type_of(text) {
                let (type_part, wrapper_name, assoc_fn) =
                    (type_part.as_str(), wrapper_name.as_str(), assoc_fn.as_str());
                // `FormatBuilder::new().build()`: a chain headed by a
                // builder ends in the type it builds, which this syntax
                // does not name. (`Client::builder()...build()` is headed
                // by the product and still types as it.)
                // Only a chain that ends in the finishing call: builder
                // steps (`TarballBuilder::new().add_file(..)`) return the
                // builder itself.
                // The last method that is not `unwrap`/`expect`/`ok`
                // (`FormatBuilder::new().build().unwrap()` finishes too).
                let mut last = func;
                let mut method = self.text_field(last);
                while matches!(
                    method,
                    Some("unwrap" | "expect" | "ok" | "unwrap_or_default")
                ) && let Some(call) = last.child_by_field_name("value")
                    && call.kind() == "call_expression"
                    && let Some(f) = call.child_by_field_name("function")
                    && f.kind() == "field_expression"
                {
                    last = f;
                    method = self.text_field(last);
                }
                let finishes = method.is_some_and(|m| {
                    matches!(m, "build" | "finish" | "done" | "create" | "try_build")
                        || m.starts_with("build_")
                });
                if func.kind() == "field_expression"
                    && wrapper_name.ends_with("Builder")
                    && finishes
                {
                    return;
                }
                // `let b = Handler::builder(..);` binds the builder, not a
                // `Handler` (only a `…builder()…build()` chain is one).
                if func.kind() == "scoped_identifier" && assoc_fn == "builder" {
                    return;
                }
                {
                    // `Arc::new(<inner>)` / `Rc::from(<inner>)` etc. forward every
                    // method call to <inner> via Deref, so typing the local as the
                    // wrapper hides <inner>'s own methods from step 4's owner
                    // lookup and the call site is screened as stdlib. See through
                    // it when <inner>'s own constructor syntax names a type.
                    if matches!(assoc_fn, "new" | "from")
                        && is_deref_transparent_wrapper(wrapper_name)
                    {
                        let locked = is_lock_wrapper(wrapper_name);
                        if let Some((inner_type, inner_locked)) =
                            self.inner_wrapped_type(call, 3)
                        {
                            if locked || inner_locked {
                                self.facts.local_types.push(cgg_core::LocalType {
                                    var_name: format!("{var_name}{GUARD_SUFFIX}"),
                                    type_name: inner_type.clone(),
                                    scope_byte: node.start_byte() as u32,
                                });
                            }
                            self.facts.local_types.push(cgg_core::LocalType {
                                var_name,
                                type_name: inner_type,
                                scope_byte: node.start_byte() as u32,
                            });
                        }
                        // <inner> isn't a `Type::assoc_fn()` call we can read.
                        // Typing the local as the wrapper (`Arc`) turned a
                        // duck-resolvable lowercase receiver into a std type
                        // with no methods of interest — the site was lost
                        // to the tool's own typing. Leave it untyped.
                        return;
                    } else if assoc_fn == "clone"
                        && is_deref_transparent_wrapper(wrapper_name)
                    {
                        // `Arc::clone(&x)` / `Rc::clone(&x)` — same value as `x`,
                        // so it carries `x`'s already-known type, if any.
                        if let Some(arg_name) = self.first_call_arg_ident(call) {
                            if let Some(known) = self.known_local_type(&arg_name, node) {
                                self.facts.local_types.push(cgg_core::LocalType {
                                    var_name,
                                    type_name: known,
                                    scope_byte: node.start_byte() as u32,
                                });
                            }
                        }
                        return;
                    }
                    self.facts.local_types.push(cgg_core::LocalType {
                        var_name,
                        type_name: type_part.to_string(),
                        scope_byte: node.start_byte() as u32,
                    });
                    return;
                }
            }
        }
        // `let c = x.clone();` — method-call form. `x.clone` is a
        // `field_expression` whose text has no `::`, so it never reaches the
        // branch above. Same rule: `c` carries `x`'s already-known type.
        if func.kind() == "field_expression" {
            let field = func.child_by_field_name("field");
            let recv = func.child_by_field_name("value");
            if let (Some(field), Some(recv)) = (field, recv) {
                if self.text(field) == "clone" && recv.kind() == "identifier" {
                    let recv_name = self.text(recv).to_string();
                    if let Some(known) = self.known_local_type(&recv_name, node) {
                        self.facts.local_types.push(cgg_core::LocalType {
                            var_name,
                            type_name: known,
                            scope_byte: node.start_byte() as u32,
                        });
                    }
                }
            }
        }
    }

    /// The `field` (method name) of a `field_expression`.
    fn text_field(&self, node: Node) -> Option<&str> {
        node.child_by_field_name("field").map(|f| self.text(f))
    }

    /// Peel `?`, `.await`, `.unwrap()`, `.expect(..)` off an expression so a
    /// call wrapped in one of these can still be read. Loops so chained forms
    /// (`foo()?.unwrap()`, rare but cheap to handle) unwrap fully.
    fn peel_result_wrapper<'b>(&self, node: Node<'b>) -> Node<'b> {
        let mut cur = node;
        loop {
            match cur.kind() {
                "try_expression" | "await_expression" => {
                    if let Some(inner) = cur.child(0) {
                        cur = inner;
                        continue;
                    }
                }
                "call_expression" => {
                    if let Some(func) = cur.child_by_field_name("function") {
                        if func.kind() == "field_expression" {
                            let field_name = func
                                .child_by_field_name("field")
                                .map(|f| self.text(f))
                                .unwrap_or("");
                            if matches!(field_name, "unwrap" | "expect") {
                                if let Some(recv) = func.child_by_field_name("value") {
                                    cur = recv;
                                    continue;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
            break;
        }
        cur
    }

    /// `call` is `W::new(..)` / `W::from(..)`; read the type named by its
    /// first argument's own `Type::assoc_fn()` constructor syntax, if any.
    /// Wrappers nest (`Arc::new(Mutex::new(Store::new()))`) — up to
    /// `depth` of them are seen through. A path-qualified inner
    /// (`nkb_rules::Scheduler::new`) keeps its path, so the resolver can
    /// match it in full rather than by bare owner name.
    /// The bool is whether a lock (`Mutex`/`RwLock`/`RefCell`) was among
    /// the wrappers seen through.
    fn inner_wrapped_type(&self, call: Node, depth: u8) -> Option<(String, bool)> {
        let args = call.child_by_field_name("arguments")?;
        let mut cursor = args.walk();
        let first_arg = args.named_children(&mut cursor).next()?;
        let inner = self.peel_result_wrapper(first_arg);
        if inner.kind() != "call_expression" {
            return None;
        }
        let func = inner.child_by_field_name("function")?;
        if func.kind() != "scoped_identifier" && func.kind() != "field_expression" {
            return None;
        }
        let (type_part, last, assoc_fn) = ctor_type_of(self.text(func))?;
        if is_deref_transparent_wrapper(&last) {
            if depth == 0 || !matches!(assoc_fn.as_str(), "new" | "from") {
                return None;
            }
            return self
                .inner_wrapped_type(inner, depth - 1)
                .map(|(t, locked)| (t, locked || is_lock_wrapper(&last)));
        }
        Some((type_part, false))
    }

    /// `call` is `W::clone(<arg>)`; read `<arg>`'s bare identifier name, so
    /// the caller can look up whether that identifier already has a known
    /// local type (`Arc::clone(&x)` — the argument is `&x`).
    fn first_call_arg_ident(&self, call: Node) -> Option<String> {
        let args = call.child_by_field_name("arguments")?;
        let mut cursor = args.walk();
        let first_arg = args.named_children(&mut cursor).next()?;
        let ident = match first_arg.kind() {
            "identifier" => first_arg,
            "reference_expression" => first_arg
                .named_child(0)
                .filter(|c| c.kind() == "identifier")?,
            _ => return None,
        };
        Some(self.text(ident).to_string())
    }

    /// Find `name`'s most recently recorded `LocalType`, bounded to the
    /// function/closure enclosing `site` — a plain backward scan of
    /// `self.facts.local_types` without that bound would happily match a
    /// same-named local in an unrelated function elsewhere in the file.
    fn known_local_type(&self, name: &str, site: Node) -> Option<String> {
        let bounds = enclosing_fn_range(site);
        self.facts
            .local_types
            .iter()
            .rev()
            .find(|lt| {
                lt.var_name == name
                    && (lt.scope_byte as usize) < site.start_byte()
                    && bounds.is_none_or(|(s, e)| {
                        (lt.scope_byte as usize) >= s && (lt.scope_byte as usize) < e
                    })
            })
            .map(|lt| lt.type_name.clone())
    }

    fn named_closure(&mut self, node: Node) -> Option<DefRecord> {
        let pattern = node.child_by_field_name("pattern")?;
        if pattern.kind() != "identifier" {
            return None;
        }
        let value = node.child_by_field_name("value")?;
        if value.kind() != "closure_expression" {
            return None;
        }
        let simple = self.text(pattern).to_string();
        if simple.is_empty() {
            return None;
        }
        let qn = qualified_name(&self.scope, &simple);
        let (sl, el) = line_range(node);
        Some(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant: DefVariant::NamedClosure,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            vis: cgg_core::Vis::Private,
            attributes: Vec::new(),
            ..Default::default()
        })
    }

    /// Walk up from a closure / async-block node looking for the
    /// enclosing `spawn`-style call. Returns the `call_expression` node
    /// if the parent chain is `closure -> arguments -> call_expression`
    /// and the called function's last `::`-segment is `spawn`,
    /// `spawn_blocking`, or `spawn_local`. Stops at any enclosing
    /// function/closure boundary so we don't traverse out of scope.
    fn spawn_call_for<'b>(&self, node: Node<'b>) -> Option<Node<'b>> {
        let mut cur = node.parent()?;
        loop {
            match cur.kind() {
                "arguments" => {
                    let call = cur.parent()?;
                    if call.kind() != "call_expression" {
                        return None;
                    }
                    let fn_node = call.child_by_field_name("function")?;
                    let text = self.text(fn_node);
                    let last = text.rsplit("::").next().unwrap_or(text);
                    if matches!(last, "spawn" | "spawn_blocking" | "spawn_local") {
                        return Some(call);
                    }
                    return None;
                }
                // Stop at any enclosing callable boundary — a closure
                // that's nested two levels deep inside another closure
                // isn't "spawned" by the outer call.
                "function_item"
                | "function_signature_item"
                | "closure_expression"
                | "async_block" => return None,
                _ => cur = cur.parent()?,
            }
        }
    }

    /// Emit (a) a DefRecord for an anonymous closure/async-block so
    /// calls inside its body attribute to it, and (b) a synthetic
    /// RefRecord at the spawn call site so the graph carries an
    /// `enclosing_fn -> closure` edge once intra-file resolution runs.
    fn emit_spawned_closure(&mut self, node: Node, spawn_call: Node) {
        let line = (node.start_position().row as u32) + 1;
        // `closure_at_42` for `|args| body`, `async_at_42` for
        // `async move { ... }`. The line number makes the simple name
        // unique within the file, which is what intra-file resolution
        // matches on.
        let simple = if node.kind() == "async_block" {
            format!("async_at_{line}")
        } else {
            format!("closure_at_{line}")
        };
        // Nest under the enclosing function: the Walker's scope stack
        // tracks modules/impls/traits but not functions, so we find the
        // smallest-byte-range function-like definition already recorded
        // and append the closure's simple name to its qualified name.
        // Falls back to the module-scope path when no enclosing fn is
        // present (free-standing closures, etc.).
        let closure_byte = node.start_byte() as u32;
        let enclosing = self
            .facts
            .definitions
            .iter()
            .filter(|d| {
                d.start_byte <= closure_byte
                    && closure_byte < d.end_byte
                    && matches!(
                        d.variant,
                        DefVariant::FreeFunction
                            | DefVariant::AsyncFunction
                            | DefVariant::InherentMethod
                            | DefVariant::TraitMethod
                            | DefVariant::TraitDefaultMethod
                            | DefVariant::StaticMethod
                            | DefVariant::NamedClosure
                    )
            })
            .min_by_key(|d| d.end_byte - d.start_byte)
            .map(|d| d.qualified_name.clone());
        let qn = match enclosing {
            Some(parent_qn) => format!("{parent_qn}::{simple}"),
            None => qualified_name(&self.scope, &simple),
        };
        let end_line = (node.end_position().row as u32) + 1;
        let start_byte = node.start_byte() as u32;
        let end_byte = node.end_byte() as u32;
        self.facts.definitions.push(DefRecord {
            simple_name: simple.clone(),
            qualified_name: qn,
            variant: DefVariant::NamedClosure,
            start_line: line,
            end_line,
            start_byte,
            end_byte,
            signature_hint: String::new(),
            visibility: String::new(),
            vis: cgg_core::Vis::Private,
            // Tag it so downstream consumers can spot the synthetic
            // origin if they want to (audit log, custom filters).
            attributes: vec!["spawned".into()],
            ..Default::default()
        });

        // Synthetic "spawn" call edge: place the RefRecord at the spawn
        // call's start byte so it attributes to the enclosing function
        // (not to the closure body itself).
        let spawn_line = (spawn_call.start_position().row as u32) + 1;
        self.facts.references.push(RefRecord {
            name: simple,
            receiver_hint: String::new(),
            site_line: spawn_line,
            site_byte: spawn_call.start_byte() as u32,
            ..Default::default()
        });
    }

    fn record_macro(&mut self, node: Node) {
        // macro_definition has a child `identifier` (the macro name)
        // after the `macro_rules!` keyword.
        let name = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let qn = qualified_name(&self.scope, &name);
        let (sl, el) = line_range(node);
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            vis: cgg_core::Vis::Private,
            attributes: vec!["macro".to_string()],
            ..Default::default()
        });
    }

    /// Extract call sites from inside a macro's argument tokens.
    ///
    /// tree-sitter parses macro arguments as an unstructured
    /// `token_tree`, so a genuine call like `writeln!(out, "{}",
    /// xml_escape(s))` contains no `call_expression` node and is
    /// invisible to the ordinary walker. Every function used only from
    /// inside `format!`, `writeln!`, `vec!` or `assert_eq!` therefore
    /// looked unreferenced — measured on cgg's own source, that was the
    /// single largest false-positive class in the dead-code report.
    ///
    /// In token soup a call is an identifier immediately followed by a
    /// parenthesised group, so that is what this matches. It is a
    /// deliberate over-approximation: an extra edge can only mark
    /// something as used, which is the safe direction.
    fn refs_from_token_tree(&mut self, node: Node) {
        // Each group carries whether it is a call's argument list — a
        // group directly after an identifier (`map(…)`), not the macro's
        // own top-level arguments, where a bare identifier is nearly
        // always a variable (`format!("{}", name)`).
        let mut stack = vec![(node, false)];
        while let Some((n, call_args)) = stack.pop() {
            let mut cur = n.walk();
            let kids: Vec<Node> = n.children(&mut cur).collect();
            for (i, k) in kids.iter().enumerate() {
                if k.kind() == "token_tree" {
                    let after_ident = i > 0 && kids[i - 1].kind() == "identifier";
                    stack.push((*k, after_ident));
                }
                if k.kind() != "identifier" && k.kind() != "scoped_identifier" {
                    continue;
                }
                // A function passed as a value inside a macro's arguments
                // — `format!("{}", x.map(iso_day))`. Outside a macro this
                // is a value reference; inside one the token tree gives no
                // expression to bind, so it is recorded as a mention: the
                // dead-code report's reason to doubt, never an edge.
                if call_args
                    && i > 0
                    && matches!(kids[i - 1].kind(), "(" | ",")
                    && kids
                        .get(i + 1)
                        .is_some_and(|nx| matches!(nx.kind(), ")" | ","))
                {
                    let name = self.text(*k).to_string();
                    if name.starts_with(|c: char| c.is_lowercase() || c == '_') {
                        self.facts.name_mentions.push(cgg_core::NameMention {
                            name,
                            via: "macro-arg".into(),
                            site_line: k.start_position().row as u32 + 1,
                            site_byte: k.start_byte() as u32,
                        });
                    }
                    continue;
                }
                // `foo (` — the next token must open a group.
                let Some(next) = kids.get(i + 1) else {
                    continue;
                };
                if next.kind() != "token_tree" || !self.text(*next).starts_with('(') {
                    continue;
                }
                // `foo!(...)` is a nested macro, already handled above.
                let full_text = self.text(*k).to_string();
                if full_text.ends_with('!') {
                    continue;
                }
                let name = full_text
                    .rsplit("::")
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if name.is_empty() || is_rust_noise_macro(&name) {
                    continue;
                }
                // Skip obvious type constructors and enum variants: an
                // uppercase leading letter with no `::` path is far more
                // likely `Some(x)` or `Ok(v)` than a function call.
                if name.starts_with(char::is_uppercase) {
                    continue;
                }
                // Recover the receiver a token tree's flat lexing otherwise
                // discards, mirroring `ref_from_call`. Two shapes matter:
                // `recv . method (` — `.` and both identifiers are plain
                // token-tree siblings — and `A :: B :: f (` — tree-sitter
                // does NOT fold this into one `scoped_identifier` node
                // inside a token tree (verified: `A::new` lexes as three
                // flat siblings `identifier "A"`, `"::"`, `identifier
                // "new"`), so it is walked back segment by segment. The
                // `k.kind() == "scoped_identifier"` branch below is kept
                // for defensiveness in case some macro shape ever yields
                // one, mirroring `ref_from_call`'s handling of that node.
                //
                // The dot case is itself path-headed as often as not —
                // `TrustKind::Network.untrusted_input()`,
                // `OutputFormat::Mermaid.default_extension()` — so the
                // identifier immediately before `.` is only the LAST
                // segment of the receiver. Stopping there (as a first cut
                // did) handed the resolver `Network` instead of
                // `TrustKind::Network`; it cannot find that owner and the
                // edge is lost outright — worse than the pre-c3 bare name,
                // which at least fanned out and sometimes got lucky.
                // `path_back_from` is shared by both the dot and the `::`
                // branches so a path before `.` is walked exactly like a
                // path before a direct `::`-call.
                let receiver_hint = if k.kind() == "scoped_identifier" {
                    match full_text.rfind("::") {
                        Some(idx) => full_text[..idx].to_string(),
                        None => String::new(),
                    }
                } else if i >= 2
                    && kids[i - 1].kind() == "."
                    && kids[i - 2].kind() == "self"
                {
                    // `self . method (` — `self` lexes as its own kind, and
                    // dropping it made the call read as a bare `method()`.
                    "self".to_string()
                } else if i >= 2
                    && kids[i - 1].kind() == "."
                    && kids[i - 2].kind() == "identifier"
                {
                    // `self . signal . notified (` inside `select!`/`info!`:
                    // the receiver is the whole `.`-chain, not its last
                    // field — `signal` alone names no binding at all, and
                    // `self.signal` is what the self-field types answer.
                    // Only a chain rooted at `self`: `h.how` has no such
                    // answer, and its last field (`how`) is what the
                    // name-shape heuristics already resolve.
                    let chain = dotted_path_back_from(&kids, i - 2, self.source);
                    if chain.starts_with("self.") {
                        chain
                    } else {
                        path_back_from(&kids, i - 2, self.source)
                    }
                } else if i >= 2 && kids[i - 1].kind() == "::" {
                    path_back_from(&kids, i - 2, self.source)
                } else {
                    String::new()
                };
                // A receiver is only ever an identifier path here — never
                // a type argument list. Turbofish leaks through when the
                // walked-back token immediately before `::`/`.` is a
                // generic-argument delimiter rather than a path segment
                // (`Snapshot::<()>::load(..)`: the token before the final
                // `::` is `>`, not an identifier), which `path_back_from`
                // otherwise still stringifies verbatim. A receiver like
                // that resolves nowhere and is worse than none, so it is
                // dropped back to bare — exactly base's behaviour — rather
                // than emitted and left to mislead the resolver.
                let receiver_hint =
                    if receiver_hint.contains('<') || receiver_hint.contains('>') {
                        String::new()
                    } else {
                        receiver_hint
                    };
                let dot_call =
                    receiver_hint.is_empty() && i > 0 && kids[i - 1].kind() == ".";
                self.facts.references.push(RefRecord {
                    name,
                    site_line: k.start_position().row as u32 + 1,
                    site_byte: k.start_byte() as u32,
                    receiver_hint,
                    // The receiver above — when non-empty — was inferred
                    // from raw token adjacency (a type alias, a file-wide
                    // `var_types` guess), which can be wrong in ways a
                    // normal expression's receiver cannot. Lets the
                    // resolver retry name-only, once, if the qualified
                    // lookup fails outright.
                    from_macro_arg: true,
                    // `… ) . build (`: a method call whose receiver did not
                    // survive token-tree reconstruction.
                    dot_call,
                    arity: None,
                    ..Default::default()
                });
            }
        }
    }

    /// Expand `node` (a `macro_invocation`) in place when it sits where
    /// items go and names a `macro_rules!` of this file whose arms the
    /// expander handles. The expansion is extracted with the current
    /// module scope, then folded back into the file:
    ///
    /// * every generated definition is tagged `macro:<name>`: reachable by
    ///   path and owner but not by name alone, never rooted as an implicit
    ///   trait impl, and reported dead at no more than medium;
    /// * spans are remapped into disjoint slices of the invocation, so a
    ///   call inside a generated fn is attributed to that fn and the
    ///   `(file, start_byte)` keys stay unique;
    /// * lines all point at the invocation, the code a reader can open.
    ///
    /// Returns `false` — and changes nothing — whenever the expansion
    /// cannot be produced or parsed, or there is no room in the span.
    fn expand_item_macro(&mut self, node: Node) -> bool {
        if self.expanding || self.macros.is_empty() {
            return false;
        }
        let at_item = |n: Node| matches!(n.kind(), "source_file" | "declaration_list");
        let Some(parent) = node.parent() else {
            return false;
        };
        let item_position = at_item(parent)
            || (parent.kind() == "expression_statement"
                && parent.parent().is_some_and(at_item));
        if !item_position {
            return false;
        }
        let Some(name_node) = node.child_by_field_name("macro") else {
            return false;
        };
        let Some(def) = self.macros.get(self.text(name_node)) else {
            return false;
        };
        let Some(args) = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "token_tree")
            .map(|t| self.text(t))
            .filter(|t| t.len() >= 2)
        else {
            return false;
        };
        let Some(expanded) = super::rust_macro::expand(def, &args[1..args.len() - 1])
        else {
            return false;
        };
        if self.parser.is_none() {
            let mut p = tree_sitter::Parser::new();
            if p.set_language(&tree_sitter_rust::LANGUAGE.into()).is_err() {
                return false;
            }
            self.parser = Some(p);
        }
        let Some(tree) = self.parser.as_mut().and_then(|p| p.parse(&expanded, None))
        else {
            return false;
        };
        if tree.root_node().has_error() {
            return false;
        }
        let mut sub = FileFacts::new(self.facts.file, self.facts.path.clone(), "rust");
        let mut sub_fields = std::collections::HashMap::new();
        {
            let mut w = Walker {
                source: expanded.as_bytes(),
                facts: &mut sub,
                scope: self.scope.clone(),
                struct_fields: std::collections::HashMap::new(),
                bases: self.bases.clone(),
                macros: std::collections::HashMap::new(),
                expanding: true,
                parser: None,
            };
            w.walk(tree.root_node());
            std::mem::swap(&mut sub_fields, &mut w.struct_fields);
        }
        let (s, e) = (node.start_byte() as u32, node.end_byte() as u32);
        let line = node.start_position().row as u32 + 1;
        let n = sub.definitions.len() as u32;
        // Slot 0 (the invocation's first byte) belongs to no generated
        // definition, so a call outside every one keeps the caller it had.
        let k = if n == 0 {
            1
        } else {
            (e - s).saturating_sub(1) / n
        };
        if n > 0 && k == 0 {
            return false;
        }
        let orig: Vec<(u32, u32)> = sub
            .definitions
            .iter()
            .map(|d| (d.start_byte, d.end_byte))
            .collect();
        let slot = |i: u32| {
            let a = s + 1 + i * k;
            (a, if i + 1 == n { e } else { a + k })
        };
        // The innermost generated definition containing an expansion byte.
        let owner_of = |b: u32| {
            orig.iter()
                .enumerate()
                .filter(|(_, (a, z))| *a <= b && b < *z)
                .min_by_key(|(_, (a, z))| z - a)
                .map(|(i, _)| i as u32)
        };
        let remap = |b: u32| owner_of(b).map_or(s, |i| slot(i).0);
        let macro_name = self.text(name_node).to_string();
        for (i, mut d) in std::mem::take(&mut sub.definitions).into_iter().enumerate() {
            let (a, z) = slot(i as u32);
            d.start_byte = a;
            d.end_byte = z;
            d.start_line = line;
            d.end_line = line;
            d.attributes.push(format!("macro:{macro_name}"));
            self.facts.definitions.push(d);
        }
        for mut r in std::mem::take(&mut sub.references) {
            r.site_byte = remap(r.site_byte);
            r.site_line = line;
            self.facts.references.push(r);
        }
        for mut lt in std::mem::take(&mut sub.local_types) {
            lt.scope_byte = remap(lt.scope_byte);
            self.facts.local_types.push(lt);
        }
        for (k, v) in sub_fields {
            self.struct_fields.entry(k).or_insert(v);
        }
        true
    }

    fn ref_from_macro_invocation(&self, node: Node) -> Option<RefRecord> {
        // macro_invocation: first child is the macro name (identifier or scoped_identifier)
        let macro_node = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier" || c.kind() == "scoped_identifier")?;
        let name = self.text(macro_node).to_string();
        // Strip trailing ! if present (shouldn't be in the identifier, but defensive)
        let name = name.trim_end_matches('!').to_string();
        if name.is_empty() {
            return None;
        }
        // Filter common stdlib / logging / serde / anyhow macros that
        // are pure noise in a call graph — they expand to formatters,
        // panicking handlers, or value constructors, not user-defined
        // functions. Keeping them inflates the `external` bucket and
        // drowns out genuine unresolved calls in audit output.
        if is_rust_noise_macro(&name) {
            return None;
        }
        Some(RefRecord {
            name,
            site_line: node.start_position().row as u32 + 1,
            site_byte: node.start_byte() as u32,
            receiver_hint: String::new(),
            ..Default::default()
        })
    }

    fn record_use(&mut self, node: Node) {
        // Parse the tree-sitter AST of the use declaration properly
        // so we can split `use a::b::{X, Y as Z};` into per-item
        // import records, and track `pub use` re-exports.
        let text = self.text(node);
        let is_pub = text.trim_start().starts_with("pub");
        let start_line = (node.start_position().row as u32) + 1;
        let start_byte = node.start_byte() as u32;

        // The argument of `use_declaration` is always a single
        // `use_clause`-ish subtree. Rather than re-implementing the
        // grammar, strip the prose (`use`, optional `pub`, trailing
        // `;`) and parse the payload string.
        let payload = text
            .trim()
            .trim_start_matches("pub")
            .trim()
            .trim_start_matches("use")
            .trim()
            .trim_end_matches(';')
            .trim();
        let kind = if is_pub { "pub-use" } else { "use" };
        for (path, alias) in expand_use_payload(payload) {
            self.facts.imports.push(ImportRecord {
                kind: kind.into(),
                path,
                alias,
                site_line: start_line,
                site_byte: start_byte,
            });
        }
    }

    /// Record a `struct Foo { … }` definition's named fields into
    /// `self.struct_fields`. Tuple/unit structs produce no entries.
    fn record_struct_fields(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let struct_name = self.text(name_node).to_string();
        if struct_name.is_empty() {
            return;
        }
        // The body is a `field_declaration_list` containing
        // `field_declaration` nodes. tree-sitter-rust exposes it as the
        // `body` field on `struct_item`.
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        if body.kind() != "field_declaration_list" {
            return;
        }
        // The struct's own type parameters. A field typed `V` names no
        // owner anywhere; when `V` carries exactly one real trait bound
        // (`struct Memory<V: VectorIndex>`, or a `where` clause) the bound
        // is the type its methods dispatch through, otherwise the field
        // is left untyped so its call sites keep their written receiver.
        let mut generics: std::collections::HashMap<String, Option<String>> =
            std::collections::HashMap::new();
        let mut generic_src = String::new();
        if let Some(tp) = node.child_by_field_name("type_parameters") {
            let t = self.text(tp);
            generic_src.push_str(
                t.strip_prefix('<')
                    .and_then(|t| t.strip_suffix('>'))
                    .unwrap_or(t),
            );
        }
        for child in node.named_children(&mut node.walk()) {
            if child.kind() == "where_clause" {
                generic_src.push(',');
                generic_src.push_str(self.text(child).trim_start_matches("where"));
            }
        }
        for (param, bound) in rust_generic_bounds(&generic_src) {
            let slot = generics.entry(param).or_insert(None);
            if bound.is_some() {
                *slot = bound;
            }
        }
        let mut fields: Vec<(String, String)> = Vec::new();
        for child in body.named_children(&mut body.walk()) {
            if child.kind() != "field_declaration" {
                continue;
            }
            let Some(fname) = child.child_by_field_name("name") else {
                continue;
            };
            let Some(ftype) = child.child_by_field_name("type") else {
                continue;
            };
            let fname_s = self.text(fname).to_string();
            // Normalise the type: strip leading `&`, `&mut `, lifetimes,
            // and unwrap one level of `Arc<…>` / `Box<…>` / `Rc<…>` /
            // `Option<…>` / `Vec<…>` so the resolver sees a bare
            // nominal type to look up. This is a heuristic — anything
            // we can't simplify is left as-is and falls back to no-op.
            let raw = self.text(ftype);
            let mut ftype_s = simplify_rust_type(raw);
            if let Some(bound) = generics.get(&ftype_s) {
                let Some(bound) = bound else { continue };
                ftype_s = bound.clone();
            }
            // `Backend::Ticks`: an associated type of a type parameter names
            // no type cgg can look up.
            if ftype_s.contains("::")
                && ftype_s
                    .split("::")
                    .next()
                    .is_some_and(|head| generics.contains_key(head))
            {
                continue;
            }
            if !fname_s.is_empty() && !ftype_s.is_empty() {
                // `Arc<Mutex<Store>>`: `self.x.lock()` hands back the
                // `Store` through its guard.
                let guarded = ["Mutex<", "RwLock<", "RefCell<"]
                    .iter()
                    .any(|w| raw.contains(w));
                if guarded {
                    fields.push((format!("{fname_s}{GUARD_SUFFIX}"), ftype_s.clone()));
                }
                fields.push((fname_s, ftype_s));
            }
        }
        if !fields.is_empty() {
            self.struct_fields.insert(struct_name, fields);
        }
    }

    fn ref_from_call(&mut self, node: Node) -> Option<RefRecord> {
        let mut callee = node.child_by_field_name("function")?;
        // A turbofish callee — `f::<T>(…)`, `x.parse::<u32>()`,
        // `a::b::<T>(…)` — is a `generic_function` wrapping the callee
        // proper. Unwrap it and record the call exactly as the
        // un-turbofished form; falling through to `_ => None` dropped
        // the call with no reference and no unresolved record, so the
        // callee read as never-referenced with nothing in the audit.
        if callee.kind() == "generic_function" {
            callee = callee.child_by_field_name("function")?;
        }
        // Skip enum-variant constructors that look like calls but aren't
        // function calls in any useful graph-edge sense: `Ok(x)`,
        // `Err(e)`, `Some(v)`. Filtering here (rather than in
        // classify_external) keeps them out of `external_calls` too,
        // declutter audits.
        if callee.kind() == "identifier" {
            let n = self.text(callee);
            if matches!(n, "Ok" | "Err" | "Some") {
                return None;
            }
        }
        let path_receiver = callee.kind() == "scoped_identifier";
        let (name, receiver_hint) = match callee.kind() {
            "identifier" => (self.text(callee).to_string(), String::new()),
            "field_expression" => {
                // recv.method
                let field = callee.child_by_field_name("field")?;
                let recv = callee
                    .child_by_field_name("value")
                    .map(|n| self.receiver_text(n))
                    .unwrap_or_default();
                (self.text(field).to_string(), recv)
            }
            "scoped_identifier" => {
                // `a::b::c`
                let name = self.text(callee).to_string();
                // Last segment is the simple name; remainder is receiver path.
                let (receiver, simple) = match name.rfind("::") {
                    Some(idx) => (name[..idx].to_string(), name[idx + 2..].to_string()),
                    None => (String::new(), name.clone()),
                };
                (simple, receiver)
            }
            _ => return None,
        };
        if name.is_empty() {
            return None;
        }
        let start_line = (node.start_position().row as u32) + 1;
        Some(RefRecord {
            name,
            path_receiver: path_receiver && !receiver_hint.is_empty(),
            receiver_hint,
            site_line: start_line,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        })
    }

    /// Serde attribute keys whose string value is a path to a function
    /// serde calls — the allowlist the serde rule's own gap text names.
    /// Everything else in an attribute string (`rename`, `alias`, doc
    /// text) is data and is ignored.
    fn record_attr_mentions(&mut self, node: Node) {
        const KEYS: &[&str] = &[
            "default",
            "deserialize_with",
            "serialize_with",
            "skip_serializing_if",
            "getter",
            "with",
        ];
        let text = self.text(node).to_string();
        let Some(body) = text
            .find("serde(")
            .filter(|i| {
                text[..*i]
                    .trim_end()
                    .trim_start_matches("#!")
                    .trim_start_matches('#')
                    .trim_start_matches('[')
                    .trim()
                    .is_empty()
            })
            .map(|i| &text[i + "serde(".len()..])
        else {
            return;
        };
        let site_line = node.start_position().row as u32 + 1;
        let site_byte = node.start_byte() as u32;
        let mut rest = body;
        while let Some(eq) = rest.find('=') {
            let key = rest[..eq]
                .trim_end()
                .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
                .next()
                .unwrap_or("");
            let after = rest[eq + 1..].trim_start();
            let Some(value) = after.strip_prefix('"').and_then(|v| v.split_once('"'))
            else {
                rest = &rest[eq + 1..];
                continue;
            };
            let (path, tail) = value;
            if KEYS.contains(&key) && !path.is_empty() {
                let names: Vec<String> = if key == "with" {
                    vec![format!("{path}::serialize"), format!("{path}::deserialize")]
                } else {
                    vec![path.to_string()]
                };
                for name in names {
                    self.facts.name_mentions.push(cgg_core::NameMention {
                        name,
                        via: format!("attr:{key}"),
                        site_line,
                        site_byte,
                    });
                }
            }
            rest = tail;
        }
    }

    /// The receiver hint for `value.method(..)`: the receiver's source
    /// text, except where that text cannot name anything. Parentheses,
    /// `?` and `.await` are peeled (`(x).m()`, `x.lock().await.m()` reads
    /// as `x.lock().m()`); a struct literal (`admin_jobs::Runner { .. }
    /// .spawn()`) is its type's path, `::` kept so the resolver can tell a
    /// workspace path from an external one; and a receiver split across
    /// lines (`self\n    .vec_index`) is joined, since nothing downstream
    /// matches a hint with the source's indentation inside it.
    fn receiver_text(&self, mut node: Node) -> String {
        for _ in 0..4 {
            match node.kind() {
                "parenthesized_expression" | "try_expression" | "await_expression" => {
                    let Some(inner) = node.named_child(0) else {
                        break;
                    };
                    node = inner;
                }
                "struct_expression" => {
                    if let Some(name) = node.child_by_field_name("name") {
                        return join_lines(self.text(name));
                    }
                    break;
                }
                _ => break,
            }
        }
        join_lines(self.text(node))
    }

    /// Capture functions passed *by name* as arguments — `register(f)`,
    /// `route("/", handler)` (Issue 4). Each bare identifier in argument
    /// position is recorded as a value-reference (sentinel receiver), to
    /// be resolved into a `Via::Reference` edge if it names a known
    /// callable. Pure syntax; the value is not tracked through the call.
    fn refs_from_args(&mut self, call: Node) {
        // Axum's whole routing surface is shape B:
        // `Router::new().route("/users", get(list_users))`. The handler
        // sits inside `get(...)`, which carries no path of its own, so
        // the context/route slots inherit from the enclosing `.route`
        // call — without that the entry node is an anonymous `get`.
        let context = call
            .child_by_field_name("function")
            .and_then(|n| n.utf8_text(self.source).ok())
            .unwrap_or_default()
            .to_string();
        let route = super::registrar::route_of(call, self.source);
        let Some(args) = call.child_by_field_name("arguments") else {
            return;
        };
        let mut cursor = args.walk();
        for arg in args.named_children(&mut cursor) {
            // Only a bare `identifier` / `scoped_identifier` standing
            // alone is a function-as-value reference; calls, literals,
            // closures, field accesses etc. are not.
            let ident = match arg.kind() {
                "identifier" | "scoped_identifier" => self.text(arg),
                // `&handler` — a reference to the function item.
                "reference_expression" => match arg.named_child(0) {
                    Some(inner)
                        if matches!(inner.kind(), "identifier" | "scoped_identifier") =>
                    {
                        self.text(inner)
                    }
                    _ => continue,
                },
                _ => continue,
            };
            if ident.is_empty() || matches!(ident, "Ok" | "Err" | "Some" | "None") {
                continue;
            }
            // The simple name is the last path segment; the resolver
            // matches it against callable definitions by name.
            let simple = ident.rsplit("::").next().unwrap_or(ident).to_string();
            // Carry the registration context on the record itself
            // rather than emitting a second, richer copy: the two would
            // share a (name, site_byte) and whichever arrived first
            // would win, which on axum was the context-less one — every
            // real route lost its path.
            self.facts.references.push(RefRecord {
                from_macro_arg: false,
                path_receiver: false,
                dot_call: false,
                arrow: false,
                arity: None,
                name: simple,
                receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                site_line: (arg.start_position().row as u32) + 1,
                site_byte: arg.start_byte() as u32,
                context: context.clone(),
                route: route.clone(),
                kwargs: Vec::new(),
            });
        }
    }
}

/// Post-pass: for each inherent/trait method, push one LocalType per
/// known struct-field receiver, scoped to that method's body start.
/// Pure data plumbing — the type propagator does the lookup work.
fn emit_self_field_local_types(
    facts: &mut FileFacts,
    struct_fields: &std::collections::HashMap<String, Vec<(String, String)>>,
) {
    use cgg_core::LocalType;

    // Snapshot the defs we care about; iterating refs while mutating
    // facts.local_types is fine because the two collections are disjoint.
    let methods: Vec<(u32, String)> = facts
        .definitions
        .iter()
        .filter(|d| {
            // `async fn` in an impl is classified `AsyncFunction` (async
            // replaces the scope-derived variant), which left every async
            // method — all of `#[async_trait]` — with no self-field facts.
            // A free async fn's qualified name ends in a module, which
            // never matches a struct below.
            matches!(
                d.variant,
                cgg_core::DefVariant::InherentMethod
                    | cgg_core::DefVariant::TraitMethod
                    | cgg_core::DefVariant::AsyncFunction
            )
        })
        .map(|d| (d.start_byte, d.qualified_name.clone()))
        .collect();

    for (start_byte, qn) in methods {
        let Some(owner) = self_type_from_qualified_name(&qn) else {
            continue;
        };
        let Some(fields) = struct_fields.get(&owner) else {
            continue;
        };
        for (fname, ftype) in fields {
            facts.local_types.push(LocalType {
                var_name: format!("self.{fname}"),
                type_name: ftype.clone(),
                scope_byte: start_byte,
            });
        }
    }
}

/// `V: VectorIndex + Send, T, 'a, const N: usize` -> each type parameter
/// with its one non-marker trait bound (`None` when it has zero or
/// several). Bounds are reduced to the bare trait name.
fn rust_generic_bounds(src: &str) -> Vec<(String, Option<String>)> {
    const MARKERS: &[&str] = &[
        "Send",
        "Sync",
        "Sized",
        "?Sized",
        "Unpin",
        "Clone",
        "Copy",
        "Debug",
        "Default",
        "Eq",
        "PartialEq",
        "Ord",
        "PartialOrd",
        "Hash",
        "Display",
    ];
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in src.char_indices() {
        match c {
            '<' | '(' => depth += 1,
            '>' | ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&src[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&src[start..]);
    let mut out = Vec::new();
    for part in parts {
        let part = part.trim();
        if part.is_empty() || part.starts_with('\'') || part.starts_with("const ") {
            continue;
        }
        let (name, bounds) = part.split_once(':').unwrap_or((part, ""));
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        let real: Vec<&str> = bounds
            .split('+')
            .map(|b| b.trim())
            .filter(|b| !b.is_empty() && !b.starts_with('\''))
            .map(|b| b.split('<').next().unwrap_or(b).trim())
            .map(|b| b.rsplit("::").next().unwrap_or(b))
            .filter(|b| !MARKERS.contains(b))
            .collect();
        let bound = (real.len() == 1).then(|| real[0].to_string());
        out.push((name.to_string(), bound));
    }
    out
}

/// Pull the bare owner type out of a Rust qualified name. Handles both
/// inherent (`crate::mod::Type::method`) and trait-impl
/// (`crate::mod::<Type as Trait>::method`) forms.
fn self_type_from_qualified_name(qn: &str) -> Option<String> {
    // Trait impl: the wrapper opens a segment (`m::<Type<'a> as a::Trait>::f`)
    // and may contain `::` of its own, so it is located by its ` as `
    // rather than by splitting on `::`.
    if let Some(as_pos) = qn.find(" as ") {
        let open = qn[..as_pos]
            .rfind("::<")
            .map(|i| i + 3)
            .or_else(|| qn.starts_with('<').then_some(1))?;
        let ty = &qn[open..as_pos];
        let bare = ty.split('<').next()?.trim();
        return Some(bare.rsplit("::").next().unwrap_or(bare).to_string());
    }
    let segs: Vec<&str> = qn.split("::").collect();
    if segs.len() < 2 {
        return None;
    }
    // Strip generic parameters: `Type<V>` -> `Type`.
    Some(segs[segs.len() - 2].split('<').next()?.to_string())
}

/// Lightweight Rust type simplifier: strips references / lifetimes /
/// up to three nested pointer/lock wrappers so the cross-file resolver sees a
/// bare nominal name. Anything ambiguous is returned unchanged — the
/// resolver will then just fail the lookup (no harm done).
fn simplify_rust_type(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // Drop leading reference + mutability + lifetime.
    s = s.trim_start_matches('&').trim_start().to_string();
    if let Some(rest) = s.strip_prefix("mut ") {
        s = rest.trim_start().to_string();
    }
    if s.starts_with('\'') {
        // `'a Foo` — drop the lifetime token.
        if let Some(space) = s.find(' ') {
            s = s[space + 1..].trim_start().to_string();
        }
    }
    // Unwrap one level of a common single-arg wrapper — only those whose
    // contents a direct method call reaches (`Deref`, or a lock guard).
    // `Option<T>`/`Vec<T>` are deliberately absent: `self.opt.m()` is never
    // `T::m` — it is the container's own method or an extension trait
    // implemented FOR the container (`impl Helper for Option<&User>`),
    // which the owner index finds under `Option`.
    // Nested (`Arc<Mutex<Store>>`) up to three deep, each level's path
    // prefix ignored (`std::sync::Arc<…>`, `tokio::sync::Mutex<…>`).
    for _ in 0..3 {
        let Some(open) = s.find('<') else { break };
        let head = &s[..open];
        let head = head.rsplit("::").next().unwrap_or(head);
        if !["Arc", "Rc", "Box", "RefCell", "Mutex", "RwLock"].contains(&head) {
            break;
        }
        let Some(end) = matching_angle(&s[open + 1..]) else {
            break;
        };
        s = s[open + 1..open + 1 + end].trim().to_string();
        // Strip `dyn ` so `Arc<dyn Trait>` becomes `Trait`.
        if let Some(rest) = s.strip_prefix("dyn ") {
            s = rest.trim_start().to_string();
        }
    }
    if let Some(rest) = s.strip_prefix("dyn ") {
        s = rest.trim_start().to_string();
    }
    // `dyn Store + Send + Sync` -> `Store`.
    if let Some(idx) = s.find(" +") {
        s.truncate(idx);
    }
    // `State<AppState>`-style wrappers keep their one argument: the
    // resolver tries it when the wrapper itself has no such method.
    if cgg_core::rust_types::deref_wrapper_split(&s).is_some() {
        return s.trim().to_string();
    }
    // Drop a trailing generic suffix on the bare type so
    // `HashMap<K, V>` matches the unparameterised callable owner name.
    if let Some(idx) = s.find('<') {
        s.truncate(idx);
    }
    s.trim().to_string()
}

/// Given a string like `T, U>` (positioned just after the opening `<`),
/// return the byte index of the matching `>`. Balances nesting.
fn matching_angle(s: &str) -> Option<usize> {
    let mut depth: i32 = 1;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Reconstruct a `::`-joined path ending at `kids[idx]` (an `identifier`,
/// or a `crate`/`self`/`super`/`Self` keyword token) by walking the flat
/// token-tree siblings backwards while the token immediately before is
/// `::` and the one before THAT is itself a path segment. Used by
/// `refs_from_token_tree` for both `A :: B :: f (` and `A :: B . f (` —
/// in a token tree these are the same shape up to what follows the last
/// segment, since tree-sitter never folds a multi-segment path into one
/// `scoped_identifier` node here (see that function's comment).
fn path_back_from(kids: &[Node], idx: usize, source: &[u8]) -> String {
    let text_of = |n: Node| n.utf8_text(source).unwrap_or("");
    let mut segs: Vec<&str> = vec![text_of(kids[idx])];
    let mut j = idx;
    while j >= 2 && kids[j - 1].kind() == "::" {
        let seg = kids[j - 2];
        if matches!(
            seg.kind(),
            "identifier" | "crate" | "self" | "super" | "Self"
        ) {
            segs.push(text_of(seg));
            j -= 2;
        } else {
            break;
        }
    }
    segs.reverse();
    segs.join("::")
}

/// Like [`path_back_from`], but also walks back over `.` field steps:
/// `self . inner . field` -> `self.inner.field`, while a `::` step still
/// joins with `::` (`TrustKind::Network`).
fn dotted_path_back_from(kids: &[Node], idx: usize, source: &[u8]) -> String {
    let text_of = |n: Node| n.utf8_text(source).unwrap_or("");
    let mut out = text_of(kids[idx]).to_string();
    let mut j = idx;
    while j >= 2 {
        let sep = kids[j - 1].kind();
        let seg = kids[j - 2];
        if !matches!(sep, "." | "::")
            || !matches!(
                seg.kind(),
                "identifier" | "crate" | "self" | "super" | "Self"
            )
        {
            break;
        }
        out = format!("{}{sep}{out}", text_of(seg));
        j -= 2;
    }
    out
}

/// Macros whose expansion produces no user-callable target and which
/// therefore add only noise to a call graph. Names are matched after
/// stripping any leading path — both `format!` and `std::format!` map
/// to the bare `format` here.
///
/// Categories:
///   - core formatting: format, print*, eprint*, write*
///   - core panics/asserts: panic, assert, assert_eq, assert_ne, todo,
///     unimplemented, unreachable, debug_assert*, matches
///   - construction: vec
///   - tracing/log: trace, debug, info, warn, error, log, span
///   - anyhow / thiserror: anyhow, bail, ensure
///   - serde_json: json
///   - dbg
fn is_rust_noise_macro(name: &str) -> bool {
    let bare = name.rsplit("::").next().unwrap_or(name);
    matches!(
        bare,
        "format"
            | "print"
            | "println"
            | "eprint"
            | "eprintln"
            | "write"
            | "writeln"
            | "panic"
            | "assert"
            | "assert_eq"
            | "assert_ne"
            | "debug_assert"
            | "debug_assert_eq"
            | "debug_assert_ne"
            | "todo"
            | "unimplemented"
            | "unreachable"
            | "matches"
            | "vec"
            | "trace"
            | "debug"
            | "info"
            | "warn"
            | "error"
            | "log"
            | "span"
            | "anyhow"
            | "bail"
            | "ensure"
            | "json"
            | "dbg"
    )
}

fn qualified_name(scope: &[ScopeSegment], simple: &str) -> String {
    let mut parts: Vec<&str> = scope.iter().map(|s| s.display()).collect();
    parts.push(simple);
    parts.join("::")
}

fn line_range(node: Node) -> (u32, u32) {
    let start = (node.start_position().row as u32) + 1;
    let end = (node.end_position().row as u32) + 1;
    (start, end)
}

/// `W::new(x)` / `W::from(x)` / `W::clone(&x)` all hand back the SAME value
/// `x` wraps, forwarded through `Deref` for every method call — so typing a
/// `let` binding as `W` rather than as whatever `x` is hides `x`'s own
/// methods from resolution. Deliberately excludes `Vec`/`Option`/`HashMap`/
/// `String`: an earlier change that unwrapped those broke push/iter/clone
/// classification, because those constructors do NOT forward through Deref
/// to a single wrapped value the way this list does.
/// Types a method call on another type reaches, as `ClassDecl`s the
/// resolver's base chain (`resolve_via_bases`) already follows:
///
/// * `impl Deref for Io { type Target = IoRef; }` — `io.encode()` is
///   `IoRef::encode` when `Io` has none of its own;
/// * `type RealCollector = Collector<RealSource>;` — a module-level alias
///   names the aliased type.
///
/// A `Deref` target that is the impl's own type parameter is skipped: it
/// names no type.
fn rust_type_links(root: Node, source: &[u8]) -> Vec<cgg_core::ClassDecl> {
    let text = |n: Node| n.utf8_text(source).unwrap_or("").to_string();
    let bare = |t: &str| -> String {
        let t = t.trim().trim_start_matches('&').trim();
        let t = t.split('<').next().unwrap_or(t).trim();
        t.rsplit("::").next().unwrap_or(t).to_string()
    };
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "impl_item" => {
                let is_deref = n
                    .child_by_field_name("trait")
                    .is_some_and(|t| bare(&text(t)) == "Deref");
                if let (true, Some(ty), Some(body)) = (
                    is_deref,
                    n.child_by_field_name("type"),
                    n.child_by_field_name("body"),
                ) {
                    let params: Vec<String> = n
                        .child_by_field_name("type_parameters")
                        .map(text)
                        .unwrap_or_default()
                        .split([',', '<', '>', ':', ' '])
                        .map(str::to_string)
                        .collect();
                    for item in body.named_children(&mut body.walk()) {
                        if item.kind() == "type_item"
                            && item
                                .child_by_field_name("name")
                                .is_some_and(|nm| text(nm) == "Target")
                            && let Some(target) = item.child_by_field_name("type")
                        {
                            let target = bare(&text(target));
                            // `Target = T` (the impl's parameter): the type
                            // derefs to its argument, which the resolver
                            // reads from the receiver's `Cfg<WebAppConfig>`.
                            let base = if params.contains(&target) {
                                "#deref-arg".to_string()
                            } else {
                                target
                            };
                            if base.starts_with(|c: char| c.is_uppercase() || c == '#') {
                                out.push(cgg_core::ClassDecl {
                                    class_qn: bare(&text(ty)),
                                    base_types: vec![base],
                                    line: n.start_position().row as u32 + 1,
                                });
                            }
                        }
                    }
                }
                continue;
            }
            "type_item" => {
                if let (Some(name), Some(target)) =
                    (n.child_by_field_name("name"), n.child_by_field_name("type"))
                {
                    let target = bare(&text(target));
                    if target.starts_with(char::is_uppercase) {
                        out.push(cgg_core::ClassDecl {
                            class_qn: text(name),
                            base_types: vec![target],
                            line: n.start_position().row as u32 + 1,
                        });
                    }
                }
                continue;
            }
            // Trait bodies hold associated types, not aliases.
            "trait_item" => continue,
            _ => {}
        }
        stack.extend(n.children(&mut n.walk()));
    }
    out
}

/// Whether the source mentions `macro_rules!` at all.
fn mentions_macro_rules(source: &[u8]) -> bool {
    source.windows(12).any(|w| w == b"macro_rules!")
}

/// Every `macro_rules!` in the file the expander can handle, by name.
fn collect_macro_defs(
    root: Node,
    source: &[u8],
) -> std::collections::HashMap<String, super::rust_macro::MacroDef> {
    let mut out = std::collections::HashMap::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        if n.kind() == "macro_definition" {
            let name = n
                .child_by_field_name("name")
                .and_then(|c| c.utf8_text(source).ok());
            let text = n.utf8_text(source).unwrap_or("");
            if let Some(name) = name
                && let Some(idx) = text.find(['{', '(', '['])
                && let Some(def) = super::rust_macro::parse_macro_rules(&text[idx..])
            {
                out.insert(name.to_string(), def);
            }
            continue;
        }
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
    }
    out
}

/// Source text with its line breaks and the indentation after them
/// removed: `self\n        .vec_index` -> `self.vec_index`. Single-line
/// text is returned unchanged.
fn join_lines(text: &str) -> String {
    if !text.contains('\n') {
        return text.to_string();
    }
    text.lines().map(str::trim).collect()
}

/// `a::b::Type::new` -> (`a::b::Type`, `Type`, `new`): the constructed
/// type's path, its last segment, and the associated fn. `None` unless
/// that last segment looks like a type (uppercase, no generics).
/// Turbofish segments are dropped first (`Vec::<u8>::new` -> `Vec`). A
/// method chain is read at its head only — `Client::builder().url(Url::
/// parse(..)).build` is typed by `Client::builder`, never by a path inside
/// a later argument.
fn ctor_type_of(text: &str) -> Option<(String, String, String)> {
    let mut plain = String::with_capacity(text.len());
    let mut depth = 0u32;
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if depth == 0 && rest.starts_with("::<") {
            depth = 1;
            rest = &rest[3..];
            continue;
        }
        match c {
            '<' if depth > 0 => depth += 1,
            '>' if depth > 0 => depth -= 1,
            '(' | '.' | '{' if depth == 0 => break,
            _ if depth == 0 && c.is_whitespace() => break,
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
        rest = &rest[c.len_utf8()..];
    }
    let pos = plain.rfind("::")?;
    let (type_part, assoc_fn) = (&plain[..pos], &plain[pos + 2..]);
    let last = type_part.rsplit("::").next().unwrap_or(type_part);
    (last.starts_with(char::is_uppercase) && !type_part.contains('<')).then(|| {
        (
            type_part.to_string(),
            last.to_string(),
            assoc_fn.to_string(),
        )
    })
}

/// Appended to a variable or `self.<field>` name for the type its lock
/// guard dereferences to (`x.lock()` -> `x#guard`). Never valid Rust, so
/// it cannot collide with a real local.
pub(crate) const GUARD_SUFFIX: &str = "#guard";

fn is_lock_wrapper(name: &str) -> bool {
    matches!(name, "Mutex" | "RwLock" | "RefCell")
}

fn is_deref_transparent_wrapper(name: &str) -> bool {
    matches!(
        name,
        "Arc" | "Rc" | "Box" | "RefCell" | "Cell" | "Mutex" | "RwLock" | "Pin" | "Cow"
    )
}

/// The byte range of the nearest enclosing function/closure body around
/// `node`, if any. Used to bound a same-name local-type lookup to the
/// function it was actually declared in.
fn enclosing_fn_range(node: Node) -> Option<(usize, usize)> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if matches!(
            n.kind(),
            "function_item" | "function_signature_item" | "closure_expression"
        ) {
            return Some((n.start_byte(), n.end_byte()));
        }
        cur = n.parent();
    }
    None
}

fn collect_attributes(node: Node, source: &[u8]) -> Vec<String> {
    // Attributes appear as preceding siblings of `function_item` in
    // `declaration_list`; they look like `#[attr]` or `#![attr]`.
    let mut out = Vec::new();
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "attribute_item"
            | "inner_attribute_item"
            | "line_comment"
            | "block_comment" => {
                if s.kind().contains("attribute") {
                    out.push(s.utf8_text(source).unwrap_or("").trim().to_string());
                }
                sib = s.prev_sibling();
            }
            _ => break,
        }
    }
    out.reverse();
    out
}

/// Expand a `use_declaration` payload into `(path, alias)` pairs.
///
/// Handles:
///   * `a::b::c`                    -> [("a::b::c", "")]
///   * `a::b::c as d`               -> [("a::b::c", "d")]
///   * `a::b::{X, Y as Z}`          -> [("a::b::X", ""), ("a::b::Y", "Z")]
///   * `a::b::{X, self}`            -> [("a::b::X", ""), ("a::b", "")]
///   * `a::b::*` -> [("a::b::*", "")] — a marker; the cross-file
///     resolver treats `*` as a wildcard.
///
/// Nested groups (`a::{b::{X, Y}, Z}`) are flattened recursively.
fn expand_use_payload(payload: &str) -> Vec<(String, String)> {
    let payload = payload.trim();
    if payload.is_empty() {
        return Vec::new();
    }

    // Split at the `::{` that starts the first group, if any, at
    // depth 0.
    let bytes = payload.as_bytes();
    let mut depth: i32 = 0;
    let mut brace_start: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => {
                if depth == 0 && brace_start.is_none() {
                    brace_start = Some(i);
                }
                depth += 1;
            }
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }

    if let Some(bs) = brace_start {
        // Strip the trailing `::` (if any) before the group.
        let head = payload[..bs].trim_end_matches(':').trim_end_matches(':');
        let head = head.trim_end_matches("::").trim();
        // Find the matching closing brace.
        let mut d = 0i32;
        let mut be: Option<usize> = None;
        for (j, b) in bytes[bs..].iter().enumerate() {
            match b {
                b'{' => d += 1,
                b'}' => {
                    d -= 1;
                    if d == 0 {
                        be = Some(bs + j);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(be) = be else {
            // Malformed; emit the raw payload.
            return vec![(payload.to_string(), String::new())];
        };
        let inner = &payload[bs + 1..be];

        // Split inner by top-level commas.
        let mut out = Vec::new();
        for item in split_top_level(inner) {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            // `self` means "the head module itself".
            if item == "self" {
                out.push((head.to_string(), String::new()));
                continue;
            }
            for (sub_path, sub_alias) in expand_use_payload(item) {
                let joined = if head.is_empty() {
                    sub_path
                } else {
                    format!("{head}::{sub_path}")
                };
                out.push((joined, sub_alias));
            }
        }
        return out;
    }

    // No group — handle `a::b::c as d` and `a::b::c`.
    if let Some(idx) = payload.rfind(" as ") {
        let path = payload[..idx].trim().to_string();
        let alias = payload[idx + 4..].trim().to_string();
        return vec![(path, alias)];
    }
    vec![(payload.to_string(), String::new())]
}

/// Split a string by top-level commas (respecting nested braces).
fn split_top_level(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for ch in input.chars() {
        match ch {
            '{' => {
                depth += 1;
                cur.push(ch);
            }
            '}' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
            }
            _ => cur.push(ch),
        }
    }
    if !cur.trim().is_empty() {
        parts.push(cur);
    }
    parts
}

/// Project Rust's visibility syntax onto the shared vocabulary.
///
/// `pub` escapes the crate; `pub(crate)` / `pub(super)` / `pub(in ...)`
/// do not; absent means private to the module.
fn rust_vis(token: &str) -> cgg_core::Vis {
    let t = token.trim();
    if t.is_empty() {
        cgg_core::Vis::Private
    } else if t == "pub" {
        cgg_core::Vis::Public
    } else if t.starts_with("pub") {
        cgg_core::Vis::Internal
    } else {
        cgg_core::Vis::Private
    }
}

/// Attributes that mark a Rust test case or benchmark.
fn rust_test_role(attrs: &[String]) -> Option<cgg_core::TestRole> {
    for a in attrs {
        let k = a.trim().trim_start_matches("#[").trim_end_matches(']');
        let k = k.split('(').next().unwrap_or(k).trim();
        if matches!(
            k,
            "test"
                | "tokio::test"
                | "async_std::test"
                | "bench"
                | "rstest"
                | "proptest"
                | "quickcheck"
                | "test_case"
        ) {
            return Some(cgg_core::TestRole::Case);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ImportRecord;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        // Use an absolute path that lives outside any cargo workspace
        // so `crate_name_for` falls back to the literal `"crate"`
        // root — the legacy behavior the rest of the assertions
        // depend on. Task 6a's integration tests cover the real
        // crate-name path.
        RustPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.rs"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn free_function() {
        let f = extract("fn foo() { bar(); }\nfn bar() {}\n");
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"crate::foo"), "got: {names:?}");
        assert!(names.contains(&"crate::bar"), "got: {names:?}");
        let ref_names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"bar"));
    }

    #[test]
    fn method_in_impl() {
        let src = r#"
struct T;
impl T {
    fn m(&self) { self.n(); }
    fn n(&self) {}
}
"#;
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.qualified_name.clone(), d.clone()))
            .collect();
        assert!(by.contains_key("crate::T::m"), "names: {by:?}");
        assert!(by.contains_key("crate::T::n"), "names: {by:?}");
        assert_eq!(
            by["crate::T::m"].variant,
            DefVariant::InherentMethod,
            "inherent-impl methods must be classified InherentMethod"
        );
        assert_eq!(by["crate::T::n"].variant, DefVariant::InherentMethod);
    }

    #[test]
    fn let_type_sees_through_deref_transparent_wrapper() {
        // Mirrors discord_safety_bot's src/main.rs:96 —
        // `let classifier = Arc::new(Classifier::from_env(assembled, grammar.to_string())?);`
        // then `classifier.model()` etc. Base (buggy) behaviour types
        // `classifier` as `Arc` — the OUTER constructor — so step 4's owner
        // lookup finds no `Arc::model` and the site is screened as stdlib.
        let src = r#"
struct Classifier;
impl Classifier {
    fn from_env(a: i32) -> Classifier { Classifier }
    fn model(&self) -> &str { "" }
}
fn boot() {
    let classifier = Arc::new(Classifier::from_env(1)?);
    let m = classifier.model();
}
"#;
        let f = extract(src);
        let got = f
            .local_types
            .iter()
            .find(|l| l.var_name == "classifier")
            .map(|l| l.type_name.as_str());
        assert_eq!(
            got,
            Some("Classifier"),
            "`classifier` must be typed as the DEREF target `Classifier`, not the \
             wrapper `Arc` — got local_types: {:?}",
            f.local_types
        );
    }

    #[test]
    fn let_type_clone_inherits_known_type() {
        // `Arc::clone(&x)` and `x.clone()` both hand back the same value as
        // `x`, so the new local should carry `x`'s already-known type.
        let src = r#"
struct Classifier;
impl Classifier {
    fn from_env(a: i32) -> Classifier { Classifier }
}
fn boot() {
    let classifier = Arc::new(Classifier::from_env(1)?);
    let c1 = Arc::clone(&classifier);
    let c2 = classifier.clone();
}
"#;
        let f = extract(src);
        let ty = |name: &str| {
            f.local_types
                .iter()
                .find(|l| l.var_name == name)
                .map(|l| l.type_name.as_str())
        };
        assert_eq!(
            ty("c1"),
            Some("Classifier"),
            "local_types: {:?}",
            f.local_types
        );
        assert_eq!(
            ty("c2"),
            Some("Classifier"),
            "local_types: {:?}",
            f.local_types
        );
    }

    // F10: `Arc::new(a::B::new())` typed the local as `Arc` — a std
    // wrapper with no methods of interest — whenever the inner constructor
    // was path-qualified, and the call site that was duck-resolvable while
    // untyped became unresolvable. Nested wrappers are seen through; an
    // unreadable inner leaves the local untyped instead of `Arc`.
    #[test]
    fn let_type_sees_through_wrappers_or_stays_untyped() {
        let src = r#"
fn boot() {
    let s = Arc::new(nkb_rules::Scheduler::new(1));
    let st = Arc::new(Mutex::new(Store::new()));
    let o = Arc::new(make_thing());
    let c = Client::builder().url(Url::parse("x")).build();
    let v = Vec::<u8>::with_capacity(4);
    let h = http::HeaderMap::new();
}
"#;
        let f = extract(src);
        let ty = |v: &str| {
            f.local_types
                .iter()
                .find(|l| l.var_name == v)
                .map(|l| l.type_name.as_str())
        };
        assert_eq!(ty("s"), Some("nkb_rules::Scheduler"), "{:?}", f.local_types);
        assert_eq!(ty("st"), Some("Store"), "{:?}", f.local_types);
        assert_eq!(ty("o"), None, "{:?}", f.local_types);
        assert_eq!(ty("c"), Some("Client"), "{:?}", f.local_types);
        assert_eq!(ty("v"), Some("Vec"), "{:?}", f.local_types);
        assert_eq!(ty("h"), Some("http::HeaderMap"), "{:?}", f.local_types);
    }

    // F1: async impl methods got no `self.<field>` facts at all, and a
    // field typed by the struct's own generic parameter named no owner.
    #[test]
    fn async_methods_get_self_field_types_and_generics_use_their_bound() {
        let src = r#"
struct Memory<V: VectorIndex + Send, W> where W: Clone + Store {
    vec_index: V,
    other: W,
    plain: Box<dyn Engine>,
    free: T,
}
impl<V: VectorIndex + Send, W> Memory<V, W> {
    async fn recall(&self) { self.vec_index.search(); }
}
"#;
        let f = extract(src);
        let mut got: Vec<(&str, &str)> = f
            .local_types
            .iter()
            .filter(|l| l.var_name.starts_with("self."))
            .map(|l| (l.var_name.as_str(), l.type_name.as_str()))
            .collect();
        got.sort();
        assert_eq!(
            got,
            [
                ("self.free", "T"),
                ("self.other", "Store"),
                ("self.plain", "Engine"),
                ("self.vec_index", "VectorIndex"),
            ]
        );
    }

    // F4: receivers reach the resolver as text it can match — a struct
    // literal as its type path, parens/`?`/`.await` peeled, a receiver
    // split across lines joined — and lock-wrapped bindings record the
    // type their guard dereferences to.
    #[test]
    fn receiver_hints_are_normalized_and_guards_typed() {
        let src = r#"
struct H { st: Arc<RwLock<Store>>, plain: Index }
impl H {
    async fn go(&self) {
        admin_jobs::Runner { gate: 1 }
            .spawn();
        (self.plain).len_of();
        self.st.write().await.put();
        self
            .plain
            .search();
        let m = Arc::new(Mutex::new(Store::new()));
    }
}
"#;
        let f = extract(src);
        let hint = |name: &str| {
            f.references
                .iter()
                .find(|r| r.name == name)
                .map(|r| r.receiver_hint.as_str())
                .unwrap_or_else(|| panic!("no ref {name}: {:?}", f.references))
        };
        assert_eq!(hint("spawn"), "admin_jobs::Runner");
        assert_eq!(hint("len_of"), "self.plain");
        assert_eq!(hint("put"), "self.st.write()");
        assert_eq!(hint("search"), "self.plain");
        let ty = |v: &str| {
            f.local_types
                .iter()
                .find(|l| l.var_name == v)
                .map(|l| l.type_name.as_str())
        };
        assert_eq!(ty("self.st#guard"), Some("Store"));
        assert_eq!(ty("self.plain#guard"), None);
        assert_eq!(ty("m#guard"), Some("Store"));
        assert_eq!(ty("m"), Some("Store"));
    }

    // F5/F13: names cgg cannot turn into edges are still recorded —
    // serde attribute strings (allowlisted keys only) and functions passed
    // as values inside macro call arguments.
    #[test]
    fn serde_attribute_strings_and_macro_arg_values_are_mentions() {
        let src = r#"
#[derive(Deserialize)]
struct Q {
    #[serde(default = "default_hops", rename = "not_a_fn")]
    hops: u32,
    #[serde(skip_serializing_if = "Vec::is_empty", with = "humantime_serde")]
    v: Vec<u32>,
    #[doc = "default = \"nope\""]
    w: u32,
}
fn f(x: Option<i64>, name: &str) {
    let s = format!("{} {}", name, x.map(iso_day).unwrap_or_default());
}
"#;
        let f = extract(src);
        let got: Vec<(&str, &str)> = f
            .name_mentions
            .iter()
            .map(|m| (m.name.as_str(), m.via.as_str()))
            .collect();
        assert_eq!(
            got,
            [
                ("default_hops", "attr:default"),
                ("Vec::is_empty", "attr:skip_serializing_if"),
                ("humantime_serde::serialize", "attr:with"),
                ("humantime_serde::deserialize", "attr:with"),
                ("iso_day", "macro-arg"),
            ]
        );
    }

    // F9: an item-position invocation of a same-file `macro_rules!`
    // generates callables — tagged with the macro, each with its
    // own slice of the invocation's span so calls inside it are attributed
    // to it. An invocation inside a function body is not an item position.
    #[test]
    fn same_file_item_macros_are_expanded_into_synthetic_callables() {
        let src = r#"
macro_rules! new_id {
    ($name:ident, $prefix:literal) => {
        pub struct $name(pub String);
        impl $name {
            pub fn new() -> Self { Self(make($prefix)) }
            pub fn as_str(&self) -> &str { &self.0 }
        }
    };
}
new_id!(DocId, "doc");
fn body() { new_id!(Inner, "x"); }
"#;
        let f = extract(src);
        let generated: Vec<&cgg_core::DefRecord> = f
            .definitions
            .iter()
            .filter(|d| d.attributes.iter().any(|a| a == "macro:new_id"))
            .collect();
        let names: Vec<&str> = generated
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("DocId::new")), "{names:?}");
        assert!(
            names.iter().any(|n| n.ends_with("DocId::as_str")),
            "{names:?}"
        );
        assert!(!names.iter().any(|n| n.contains("Inner")), "{names:?}");
        assert!(
            generated
                .iter()
                .all(|d| !d.attributes.iter().any(|a| a == "synthetic"))
        );
        let mut starts: Vec<u32> = generated.iter().map(|d| d.start_byte).collect();
        starts.sort();
        starts.dedup();
        assert_eq!(starts.len(), generated.len(), "spans must not collide");
        // `make(..)` inside `new` lands inside `new`'s slice.
        let new = generated.iter().find(|d| d.simple_name == "new").unwrap();
        let make = f
            .references
            .iter()
            .find(|r| r.name == "make")
            .expect("make ref");
        assert!(new.start_byte <= make.site_byte && make.site_byte < new.end_byte);
    }

    // F9c: a `.`-chain receiver inside a macro keeps its `self` prefix.
    #[test]
    fn macro_token_tree_dot_chain_keeps_self() {
        let f = extract("fn t(){ info!(q = self.signal.quiet_for_secs(), \"x\"); }");
        let r = f
            .references
            .iter()
            .find(|r| r.name == "quiet_for_secs")
            .unwrap_or_else(|| panic!("{:?}", f.references));
        assert_eq!(r.receiver_hint, "self.signal");
    }

    #[test]
    fn macro_token_tree_self_method_keeps_self() {
        let f = extract("fn t(){ json!({ \"url\": self.get_url(host) }); }");
        let r = f.references.iter().find(|r| r.name == "get_url").unwrap();
        assert_eq!(r.receiver_hint, "self");
    }

    #[test]
    fn lib_target_name_and_mod_declarations_are_in_scope() {
        assert_eq!(
            section_value(
                "[package]\nname = \"clash-verge\"\n[lib]\nname = \"app_lib\"\n",
                "lib",
                "name"
            ),
            Some("app_lib".to_string())
        );
        let f = extract("#[path = \"../x.rs\"] mod perf; fn main() { perf::run(); }");
        assert!(
            f.imports
                .iter()
                .any(|i| i.path == "self::perf" && i.alias == "perf"),
            "{:?}",
            f.imports
        );
    }

    #[test]
    fn cargo_dependency_renames_are_parsed() {
        let crate_toml = r#"
[package]
name = "server"
[dependencies]
common = { package = "nkb-common", path = "../common" }
serde = "1"
plain = { path = "../plain" }
shared = { workspace = true }
[dev-dependencies.fixtures]
package = "test-fixtures"
path = "../fx"
"#;
        let (renames, inherited) = parse_dependency_renames(crate_toml, false);
        assert_eq!(
            renames,
            [
                ("common".to_string(), "nkb_common".to_string()),
                ("fixtures".to_string(), "test_fixtures".to_string())
            ]
        );
        assert_eq!(inherited, ["shared"]);
        let ws = "[workspace]\n[workspace.dependencies]\nshared = { package = \"nkb-shared\", path = \"x\" }\n";
        let (ws_renames, _) = parse_dependency_renames(ws, true);
        assert_eq!(
            ws_renames,
            [("shared".to_string(), "nkb_shared".to_string())]
        );
    }

    #[test]
    fn builder_chain_is_not_typed_as_the_builder() {
        let f = extract(
            "fn t(){ let a = FormatBuilder::new().build(); let b = FormatBuilder::new(); let c = Client::builder().url(u).build(); let d = FormatBuilder::new().build().unwrap(); let e = TarballBuilder::new().add(x); let h = Handler::builder(); }",
        );
        let ty = |v: &str| {
            f.local_types
                .iter()
                .find(|l| l.var_name == v)
                .map(|l| l.type_name.as_str())
        };
        assert_eq!(ty("a"), None);
        assert_eq!(ty("b"), Some("FormatBuilder"));
        assert_eq!(ty("c"), Some("Client"));
        assert_eq!(ty("d"), None);
        assert_eq!(ty("e"), Some("TarballBuilder"));
        assert_eq!(ty("h"), None);
    }

    // Types a call reaches through: a `Deref` target (or the impl's own
    // argument), and a module-level type alias. A macro method call whose
    // receiver was lost is flagged so it is not read as a bare call.
    #[test]
    fn deref_targets_aliases_and_dot_calls() {
        let src = r#"
impl Deref for Io { type Target = IoRef; fn deref(&self) -> &IoRef { &self.0 } }
impl<T> Deref for Cfg<T> { type Target = T; fn deref(&self) -> &T { &self.0 } }
type Real = Collector<Source>;
trait Tr { type Assoc = Foo; }
fn t() { assert!(make().build(x)); }
"#;
        let f = extract(src);
        let links: Vec<(&str, &str)> = f
            .classes
            .iter()
            .map(|c| (c.class_qn.as_str(), c.base_types[0].as_str()))
            .collect();
        assert!(links.contains(&("Io", "IoRef")), "{links:?}");
        assert!(links.contains(&("Cfg", "#deref-arg")), "{links:?}");
        assert!(links.contains(&("Real", "Collector")), "{links:?}");
        assert_eq!(links.len(), 3, "{links:?}");
        let build = f.references.iter().find(|r| r.name == "build").unwrap();
        assert!(build.dot_call && build.receiver_hint.is_empty());
    }

    #[test]
    fn let_type_does_not_unwrap_collection_constructors() {
        // Vec/Option/HashMap/String are NOT deref-transparent wrappers over a
        // single inner value the way Arc/Rc/Box/... are — an earlier A/B
        // showed unwrapping them breaks push/iter/clone classification.
        // `Vec::new()` takes no inner-typed argument at all, so this is
        // mostly a guard against `is_deref_transparent_wrapper` growing to
        // include them by accident.
        let src = r#"
fn boot() {
    let v = Vec::new();
}
"#;
        let f = extract(src);
        let got = f
            .local_types
            .iter()
            .find(|l| l.var_name == "v")
            .map(|l| l.type_name.as_str());
        assert_eq!(got, Some("Vec"), "local_types: {:?}", f.local_types);
    }

    #[test]
    fn trait_impl_method() {
        let src = r#"
trait R { fn r(&self); }
struct S;
impl R for S { fn r(&self) {} }
"#;
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("<S as R>::r")),
            "got: {names:?}"
        );
    }

    #[test]
    fn trait_items_inherit_visibility_rather_than_claiming_private() {
        let src = r#"
pub trait R { fn decl(&self); fn dflt(&self) {} }
struct S;
impl R for S { fn decl(&self) {} }
impl S { fn inherent(&self) {} pub fn exported(&self) {} }
fn free() {}
"#;
        let f = extract(src);
        let vis_of = |simple: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == simple)
                .unwrap_or_else(|| panic!("no def named {simple}"))
                .vis
        };
        // A method of a `pub trait` is callable from outside the crate,
        // so claiming `Private` would let a dead-code report assert no
        // out-of-tree caller can exist. cgg does not resolve the
        // inherited value, so it claims nothing.
        assert_eq!(vis_of("decl"), cgg_core::Vis::Unknown);
        assert_eq!(vis_of("dflt"), cgg_core::Vis::Unknown);
        // Same for the implementing side, where the trait may not even
        // be declared in this file.
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name.contains("<S as R>::decl")
                    && d.vis == cgg_core::Vis::Unknown)
        );
        // Items that own their visibility are unaffected.
        assert_eq!(vis_of("inherent"), cgg_core::Vis::Private);
        assert_eq!(vis_of("exported"), cgg_core::Vis::Public);
        assert_eq!(vis_of("free"), cgg_core::Vis::Private);
    }

    #[test]
    fn mod_prefix() {
        let f = extract("mod a { mod b { fn c() {} } }");
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(names, vec!["crate::a::b::c"]);
    }

    #[test]
    fn integration_test_file_qualified_under_crate_tests_module() {
        // VERIFIED.md (h): `tests/<name>.rs` files used to map to a bare
        // `(crate_root, [])`, so `fn cgg()` in `crates/cgg/tests/rollup.rs`
        // collided (by qualified name) with any same-named item in the
        // library crate itself, producing false cross-file edges.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"cgg\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let tests_dir = tmp.path().join("tests");
        std::fs::create_dir_all(&tests_dir).unwrap();
        let file = tests_dir.join("rollup.rs");
        std::fs::write(&file, b"").unwrap();

        let (crate_root, mods) = rust_module_path(&file);
        assert_eq!(crate_root, "cgg");
        assert_eq!(
            mods,
            vec!["tests".to_string(), "rollup".to_string()],
            "tests/<name>.rs must be qualified under <crate>::tests::<name>, not the bare crate root"
        );
    }

    #[test]
    fn integration_test_main_rs_under_subdir_qualified_by_dir_name() {
        // `tests/<dir>/main.rs` is its own compilation unit named after
        // `<dir>` (cargo convention), so it must be qualified as
        // `<crate>::tests::<dir>`, not `<crate>::tests::main`.
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"cgg\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        let sub_dir = tmp.path().join("tests").join("harness");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let file = sub_dir.join("main.rs");
        std::fs::write(&file, b"").unwrap();

        let (crate_root, mods) = rust_module_path(&file);
        assert_eq!(crate_root, "cgg");
        assert_eq!(mods, vec!["tests".to_string(), "harness".to_string()]);
    }

    #[test]
    fn named_closure_is_callable() {
        let src = "fn wrap() {\n    let inc = |x: i32| x + 1;\n    inc(1);\n}\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"crate::wrap"));
        assert!(names.iter().any(|n| n.ends_with("inc")), "got: {names:?}");
        let ref_names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"inc"));
    }

    #[test]
    fn call_expressions_captured() {
        let src = r#"
fn main() {
    std::println!("hi");
    helper();
    self.m();
    a::b::c();
}
fn helper() {}
"#;
        let f = extract(src);
        let names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        // Expect at minimum: helper, m, c.
        assert!(names.contains(&"helper"), "got: {names:?}");
        assert!(names.contains(&"m"), "got: {names:?}");
        assert!(names.contains(&"c"), "got: {names:?}");
    }

    #[test]
    fn use_imports_captured() {
        let src = "use a::b::c;\nuse x::y as z;\nfn main() {}\n";
        let f = extract(src);
        let uses: Vec<&ImportRecord> = f
            .imports
            .iter()
            .filter(|i| i.kind == "use" || i.kind == "pub-use")
            .collect();
        assert_eq!(uses.len(), 2);
        assert_eq!(uses[0].kind, "use");
        assert_eq!(uses[0].path, "a::b::c");
        assert_eq!(uses[0].alias, "");
        assert_eq!(uses[1].path, "x::y");
        assert_eq!(uses[1].alias, "z");
    }

    #[test]
    fn use_block_imports_flatten() {
        let src = "use a::b::{X, Y as Z};\nfn main() {}\n";
        let f = extract(src);
        let pairs: Vec<(String, String)> = f
            .imports
            .iter()
            .map(|i| (i.path.clone(), i.alias.clone()))
            .collect();
        assert!(
            pairs.contains(&("a::b::X".into(), "".into())),
            "got: {pairs:?}"
        );
        assert!(
            pairs.contains(&("a::b::Y".into(), "Z".into())),
            "got: {pairs:?}"
        );
    }

    #[test]
    fn use_self_in_block() {
        let src = "use a::b::{self, X};\nfn main() {}\n";
        let f = extract(src);
        let paths: Vec<&str> = f.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"a::b"), "got: {paths:?}");
        assert!(paths.contains(&"a::b::X"), "got: {paths:?}");
    }

    #[test]
    fn pub_use_is_tagged() {
        let src = "pub use a::b::c;\nfn main() {}\n";
        let f = extract(src);
        let pub_uses: Vec<&ImportRecord> =
            f.imports.iter().filter(|i| i.kind == "pub-use").collect();
        assert_eq!(pub_uses.len(), 1);
        assert_eq!(pub_uses[0].path, "a::b::c");
    }

    #[test]
    fn nested_use_block_flatten() {
        let src = "use a::{b::{X, Y}, c::Z};\nfn main() {}\n";
        let f = extract(src);
        let paths: Vec<&str> = f.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"a::b::X"), "got: {paths:?}");
        assert!(paths.contains(&"a::b::Y"), "got: {paths:?}");
        assert!(paths.contains(&"a::c::Z"), "got: {paths:?}");
    }

    #[test]
    fn visibility_and_async_captured() {
        let src = "pub async fn f() {}\nasync fn g() {}\npub(crate) fn h() {}\n";
        let f = extract(src);
        let by_name: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.clone()))
            .collect();
        assert_eq!(by_name["f"].visibility, "pub");
        assert_eq!(by_name["f"].variant, DefVariant::AsyncFunction);
        assert_eq!(by_name["g"].variant, DefVariant::AsyncFunction);
        assert!(by_name["h"].visibility.starts_with("pub"));
    }

    #[test]
    fn tokio_spawn_extracts_async_block_as_disjoint_callable() {
        let src = "\
async fn outer() {
    tokio::spawn(async move {
        inner_called().await;
    });
}
async fn inner_called() {}
";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        // The async block becomes its own callable; line 2 is the
        // `tokio::spawn(async move {` line.
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("crate::outer::async_at_")),
            "expected an async_at_* callable under outer; got {names:?}"
        );
        // The synthetic spawn-edge RefRecord must point at the
        // synthesized closure name from inside `outer`.
        let async_simple = f
            .definitions
            .iter()
            .find(|d| d.simple_name.starts_with("async_at_"))
            .unwrap()
            .simple_name
            .clone();
        let ref_names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(
            ref_names.contains(&async_simple.as_str()),
            "expected a synthetic RefRecord referencing {async_simple}; got {ref_names:?}"
        );
        // Tag survives so callers can filter.
        let asyncs = f
            .definitions
            .iter()
            .filter(|d| d.simple_name.starts_with("async_at_"))
            .collect::<Vec<_>>();
        assert!(asyncs[0].attributes.contains(&"spawned".to_string()));
    }

    #[test]
    fn inline_callback_closure_not_extracted_as_disjoint() {
        // `.map(|x| x + 1)` is a tight callback — keep it inline so the
        // graph doesn't drown in single-expression closure noise.
        let src = "\
fn outer() {
    let v: Vec<i32> = vec![1, 2, 3].into_iter().map(|x| x + 1).collect();
}
";
        let f = extract(src);
        assert!(
            !f.definitions
                .iter()
                .any(|d| d.simple_name.starts_with("closure_at_")),
            "inline .map closure should NOT become a separate callable"
        );
    }

    #[test]
    fn thread_spawn_also_extracts_closure() {
        // `std::thread::spawn` should be detected by the
        // last-segment-is-`spawn` heuristic, same as tokio.
        let src = "\
fn outer() {
    std::thread::spawn(|| {
        worker();
    });
}
fn worker() {}
";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(
            names
                .iter()
                .any(|n| n.starts_with("crate::outer::closure_at_")),
            "expected std::thread::spawn closure to be extracted; got {names:?}"
        );
    }

    // c3 (VERIFIED.md §1b, §3.3): a call inside a macro's token tree loses
    // its receiver — `refs_from_token_tree` records only the last path
    // segment with `receiver_hint: String::new()`. `p.id()` inside
    // `assert_eq!` becomes a bare `id` reference and `A::new(2)` becomes a
    // bare `new`, so cross-file resolution fans out by simple name instead
    // of following the receiver, exactly like `node_ids.rs:177`'s
    // `assert_eq!(NodeIds::resolve(..))`.
    #[test]
    fn macro_token_tree_call_keeps_receiver() {
        let src = "fn t(){ assert_eq!(p.id(), 1); assert!(A::new(2).ok()); }";
        let f = extract(src);
        let id_ref = f
            .references
            .iter()
            .find(|r| r.name == "id")
            .unwrap_or_else(|| panic!("no reference named `id`; got {:?}", f.references));
        assert_eq!(
            id_ref.receiver_hint, "p",
            "p.id() inside assert_eq! must keep receiver `p`, got {:?}",
            f.references
        );
        let new_ref = f
            .references
            .iter()
            .find(|r| r.name == "new")
            .unwrap_or_else(|| {
                panic!("no reference named `new`; got {:?}", f.references)
            });
        assert_eq!(
            new_ref.receiver_hint, "A",
            "A::new(2) inside assert! must keep receiver `A`, got {:?}",
            f.references
        );
    }

    // Multi-segment scoped path inside a macro's token tree: tree-sitter
    // never produces a `scoped_identifier` node here (verified with a
    // probe dump — `A::new` lexes as flat `identifier "A"`, `"::"`,
    // `identifier "new"` siblings, not one typed node), so the fix must
    // walk the flat `identifier "::" identifier "::" ...` run backwards,
    // not just branch on `k.kind() == "scoped_identifier"`.
    #[test]
    fn macro_token_tree_call_keeps_multi_segment_receiver() {
        let src = "fn t(){ assert_eq!(NodeIds::resolve(1), crate::a::b::helper(2)); }";
        let f = extract(src);
        let resolve_ref = f
            .references
            .iter()
            .find(|r| r.name == "resolve")
            .unwrap_or_else(|| {
                panic!("no reference named `resolve`; got {:?}", f.references)
            });
        assert_eq!(
            resolve_ref.receiver_hint, "NodeIds",
            "got {:?}",
            f.references
        );
        let helper_ref = f
            .references
            .iter()
            .find(|r| r.name == "helper")
            .unwrap_or_else(|| {
                panic!("no reference named `helper`; got {:?}", f.references)
            });
        assert_eq!(
            helper_ref.receiver_hint, "crate::a::b",
            "got {:?}",
            f.references
        );
    }

    // Amendment: the dot branch must walk the FULL path before `.`, not
    // just the identifier immediately preceding it. `assert!(TrustKind::
    // Network.untrusted_input())` (a real cgg-core call site) first cut
    // this to receiver_hint `Network`, which sent the resolver looking for
    // an owner named `Network` instead of `TrustKind::Network` — no such
    // owner exists, so the edge was LOST outright (worse than the pre-c3
    // bare name, which at least fanned out and sometimes got lucky).
    #[test]
    fn macro_token_tree_dot_call_keeps_full_path_receiver() {
        let src = "fn t(){ assert!(TrustKind::Network.untrusted_input()); }";
        let f = extract(src);
        let r = f
            .references
            .iter()
            .find(|r| r.name == "untrusted_input")
            .unwrap_or_else(|| {
                panic!(
                    "no reference named `untrusted_input`; got {:?}",
                    f.references
                )
            });
        assert_eq!(
            r.receiver_hint, "TrustKind::Network",
            "got {:?}",
            f.references
        );
    }

    // Second amendment, part (a): turbofish inside a macro token tree
    // (`Snapshot::<()>::load(..)` in `matches!`) leaks a bare `>` as
    // receiver_hint — verified by probe: the token immediately before
    // the final `::` is the closing angle bracket `>`, not a path
    // segment, and `path_back_from` otherwise stringifies it verbatim.
    // A receiver that is not an identifier path resolves nowhere and is
    // worse than none, so it must be dropped back to bare.
    #[test]
    fn macro_token_tree_turbofish_receiver_is_dropped_not_emitted() {
        let src = "fn t(){ matches!(Snapshot::<()>::load(x), Ok(_)); }";
        let f = extract(src);
        let r = f
            .references
            .iter()
            .find(|r| r.name == "load")
            .unwrap_or_else(|| {
                panic!("no reference named `load`; got {:?}", f.references)
            });
        assert_eq!(
            r.receiver_hint, "",
            "a turbofish-mangled receiver must be dropped to bare, not emitted; got {:?}",
            f.references
        );
    }

    // Turbofish callees (`generic_function`) were dropped with no
    // reference and no unresolved record, so `f::<T>()`'s callee read as
    // never-referenced and the audit held no trace of the call.
    #[test]
    fn turbofish_callee_is_recorded_like_the_plain_call() {
        let src = "fn t(){ call_llm_typed::<Foo>(a); m::parse_it::<u8>(); x.collect::<Vec<_>>(); }";
        let f = extract(src);
        let got: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(got.contains(&("call_llm_typed", "")), "got {got:?}");
        assert!(got.contains(&("parse_it", "m")), "got {got:?}");
        assert!(got.contains(&("collect", "x")), "got {got:?}");
    }
}
