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

use cgg_core::{
    ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
};
use tree_sitter::{Node, Tree, TreeCursor};

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct RustPlugin;

impl LanguagePlugin for RustPlugin {
    fn id(&self) -> &'static str {
        "rust"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".rs"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_rust::LANGUAGE.into()
    }

    fn extract(
        &self,
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
        };
        walker.walk(tree.root_node());
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
/// * `tests/name.rs`                    -> `` (integration test root).
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
        Some("tests") | Some("benches") | Some("examples") => {
            // Each test/bench/example is its own compilation unit —
            // treat them as separate roots with no shared module path.
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
    let mut mods: Vec<String> = segs
        .iter()
        .take(segs.len() - 1)
        .cloned()
        .collect();
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
            if let Ok(text) = std::fs::read_to_string(&cargo) {
                if let Some(name) = extract_package_name(&text) {
                    return Some((name, d.to_path_buf()));
                }
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
                if let Some(stripped) = value
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
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
                if !self.scope.last().map_or(true, |s| matches!(s, ScopeSegment::Crate(_))) {
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
                    Some(t) => {
                        ScopeSegment::TraitImpl(format!("<{type_name} as {t}>"))
                    }
                    None => ScopeSegment::InherentImpl(type_name.clone()),
                };
                self.scope.push(segment);
                self.walk_children(node);
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

            // References.
            "call_expression" => {
                if let Some(r) = self.ref_from_call(node) {
                    self.facts.references.push(r);
                }
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

        let qn = qualified_name(&self.scope, &simple);
        let (sl, el) = line_range(node);
        let signature = super::extract_signature(self.text(node));

        let attributes = collect_attributes(node, self.source);

        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: signature,
            visibility,
            attributes,
        });
    }

    /// Detect `let NAME = |..| {..};` and treat the binding as a
    /// named closure definition.
    fn infer_let_type(&mut self, node: Node) {
        // `let x = Foo::new(...)` or `let x = Foo::builder()`
        // Pattern field: let_declaration has "pattern" (identifier) and "value" (call_expression)
        let pat = node.child_by_field_name("pattern");
        let val = node.child_by_field_name("value");
        let (Some(pat), Some(val)) = (pat, val) else { return };
        // Get variable name
        let var_name = if pat.kind() == "identifier" {
            self.text(pat).to_string()
        } else { return; };
        if var_name.is_empty() { return; }
        // Check if value is a call_expression with a path that starts with a type
        let call = if val.kind() == "call_expression" {
            Some(val)
        } else if val.kind() == "try_expression" || val.kind() == "await_expression" {
            // `Foo::new()?` or `Foo::new().await`
            val.child(0).filter(|c| c.kind() == "call_expression")
        } else { None };
        let Some(call) = call else { return };
        let func = call.child_by_field_name("function");
        let Some(func) = func else { return };
        // Look for `Foo::new`, `Foo::builder`, `Foo::default`, `Foo::from`
        if func.kind() == "scoped_identifier" || func.kind() == "field_expression" {
            let text = self.text(func);
            if let Some(pos) = text.find("::") {
                let type_part = &text[..pos];
                if type_part.starts_with(char::is_uppercase) && !type_part.contains('<') {
                    self.facts.local_types.push(cgg_core::LocalType {
                        var_name,
                        type_name: type_part.to_string(),
                        scope_byte: node.start_byte() as u32,
                    });
                }
            }
        }
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
            attributes: Vec::new(),
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

    fn ref_from_call(&mut self, node: Node) -> Option<RefRecord> {
        let callee = node.child_by_field_name("function")?;
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
        })
    }
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


fn collect_attributes(node: Node, source: &[u8]) -> Vec<String> {
    // Attributes appear as preceding siblings of `function_item` in
    // `declaration_list`; they look like `#[attr]` or `#![attr]`.
    let mut out = Vec::new();
    let mut sib = node.prev_sibling();
    while let Some(s) = sib {
        match s.kind() {
            "attribute_item" | "inner_attribute_item" | "line_comment" | "block_comment" => {
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
///   * `a::b::*`                    -> [("a::b::*", "")] (marker; the
///                                     cross-file resolver treats `*`
///                                     as a wildcard).
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

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use cgg_core::ImportRecord;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        // Use an absolute path that lives outside any cargo workspace
        // so `crate_name_for` falls back to the literal `"crate"`
        // root — the legacy behavior the rest of the assertions
        // depend on. Task 6a's integration tests cover the real
        // crate-name path.
        RustPlugin.extract(
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
        let ref_names: Vec<&str> =
            f.references.iter().map(|r| r.name.as_str()).collect();
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
        assert!(pairs.contains(&("a::b::X".into(), "".into())), "got: {pairs:?}");
        assert!(pairs.contains(&("a::b::Y".into(), "Z".into())), "got: {pairs:?}");
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
}
