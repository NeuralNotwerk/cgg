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
        };
        walker.walk(tree.root_node());
        // Post-pass: emit synthetic LocalTypes of the form
        // `self.<field>` -> `<FieldType>` for each method in an impl
        // block, so the type propagator + cross-file resolver can map
        // `self.store.foo()` to `ChunkStore::foo()` etc.
        let struct_fields = std::mem::take(&mut walker.struct_fields);
        emit_self_field_local_types(&mut facts, &struct_fields);

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
}

impl<'a> Walker<'a> {
    fn text(&self, node: Node) -> &str {
        node.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        let kind = node.kind();

        match kind {
            // `mod foo { ... }` / `mod foo;`
            "mod_item" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
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
            if let Some(pos) = text.find("::") {
                let type_part = &text[..pos];
                let assoc_fn = &text[pos + 2..];
                if type_part.starts_with(char::is_uppercase) && !type_part.contains('<') {
                    // `Arc::new(<inner>)` / `Rc::from(<inner>)` etc. forward every
                    // method call to <inner> via Deref, so typing the local as the
                    // wrapper hides <inner>'s own methods from step 4's owner
                    // lookup and the call site is screened as stdlib. See through
                    // it when <inner>'s own constructor syntax names a type.
                    if matches!(assoc_fn, "new" | "from")
                        && is_deref_transparent_wrapper(type_part)
                    {
                        if let Some(inner_type) = self.inner_wrapped_type(call) {
                            self.facts.local_types.push(cgg_core::LocalType {
                                var_name,
                                type_name: inner_type,
                                scope_byte: node.start_byte() as u32,
                            });
                            return;
                        }
                        // <inner> isn't a `Type::assoc_fn()` call we can read —
                        // fall through to the old behaviour (type as the wrapper).
                    } else if assoc_fn == "clone"
                        && is_deref_transparent_wrapper(type_part)
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
    fn inner_wrapped_type(&self, call: Node) -> Option<String> {
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
        let text = self.text(func);
        let pos = text.find("::")?;
        let type_part = &text[..pos];
        if type_part.starts_with(char::is_uppercase) && !type_part.contains('<') {
            Some(type_part.to_string())
        } else {
            None
        }
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
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            let mut cur = n.walk();
            let kids: Vec<Node> = n.children(&mut cur).collect();
            for (i, k) in kids.iter().enumerate() {
                if k.kind() == "token_tree" {
                    stack.push(*k);
                }
                if k.kind() != "identifier" && k.kind() != "scoped_identifier" {
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
                    && ((kids[i - 1].kind() == "." && kids[i - 2].kind() == "identifier")
                        || kids[i - 1].kind() == "::")
                {
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
                    ..Default::default()
                });
            }
        }
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
            let ftype_s = simplify_rust_type(self.text(ftype));
            if !fname_s.is_empty() && !ftype_s.is_empty() {
                fields.push((fname_s, ftype_s));
            }
        }
        if !fields.is_empty() {
            self.struct_fields.insert(struct_name, fields);
        }
    }

    fn ref_from_call(&mut self, node: Node) -> Option<RefRecord> {
        let callee = node.child_by_field_name("function")?;
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
        let (name, receiver_hint) = match callee.kind() {
            "identifier" => (self.text(callee).to_string(), String::new()),
            "field_expression" => {
                // recv.method
                let field = callee.child_by_field_name("field")?;
                let recv = callee
                    .child_by_field_name("value")
                    .map(|n| self.text(n).to_string())
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
            receiver_hint,
            site_line: start_line,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        })
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
            matches!(
                d.variant,
                cgg_core::DefVariant::InherentMethod | cgg_core::DefVariant::TraitMethod
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

/// Pull the bare owner type out of a Rust qualified name. Handles both
/// inherent (`crate::mod::Type::method`) and trait-impl
/// (`crate::mod::<Type as Trait>::method`) forms.
fn self_type_from_qualified_name(qn: &str) -> Option<String> {
    let segs: Vec<&str> = qn.split("::").collect();
    if segs.len() < 2 {
        return None;
    }
    let owner = segs[segs.len() - 2];
    if let Some(stripped) = owner.strip_prefix('<') {
        // `<Type as Trait>` -> `Type`
        let t = stripped.split(" as ").next()?;
        return Some(t.trim_end_matches('>').to_string());
    }
    // Strip generic parameters: `Type<V>` -> `Type`.
    Some(owner.split('<').next()?.to_string())
}

/// Lightweight Rust type simplifier: strips references / lifetimes /
/// one level of common wrapper types so the cross-file resolver sees a
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
    // Unwrap one level of a common single-arg wrapper.
    for wrapper in [
        "Arc", "Rc", "Box", "Option", "Vec", "RefCell", "Mutex", "RwLock",
    ] {
        let prefix = format!("{wrapper}<");
        if let Some(inner) = s.strip_prefix(&prefix)
            && let Some(end) = matching_angle(inner)
        {
            s = inner[..end].trim().to_string();
            break;
        }
    }
    // Strip `dyn ` so `Arc<dyn Trait>` becomes `Trait`.
    if let Some(rest) = s.strip_prefix("dyn ") {
        s = rest.trim_start().to_string();
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
}
