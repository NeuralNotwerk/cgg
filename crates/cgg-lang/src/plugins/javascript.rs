//! JavaScript plugin — callable extraction.
//!
//! Handles:
//! * `function_declaration` / `generator_function_declaration`
//! * `arrow_function` and `function` in `variable_declarator`
//!   (named lambdas / arrow functions assigned to const/let/var)
//! * `method_definition` inside `class_body` (including
//!   constructor, get/set accessors, static methods)
//! * `call_expression` → RefRecord (bare identifier or
//!   member_expression)
//! * ESM imports: `import { x } from '...'`,
//!   `import * as ns from '...'`, `import x from '...'`
//! * CJS: `require('./...')` in destructuring patterns

use std::path::Path;

use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct JavaScriptPlugin;

impl LanguagePlugin for JavaScriptPlugin {
    fn id(&self) -> &'static str {
        "javascript"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".js", ".mjs", ".cjs", ".jsx"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["node"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            unreachable: true,
            attributes: true,
            impls: true,
            value_refs: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_javascript::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "javascript");
        // The names this file is imported under (`@acme/core`, a tsconfig
        // `paths` alias), so a bare specifier can be told apart from a
        // dependency's.
        let manifest = super::manifest::js_module_manifest(path);
        facts.packages = manifest.names;
        let own_package = manifest.own;
        let exported_as = manifest.exported_as;
        let mut w = JsWalker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        let mut out = facts;
        push_package_name(&mut out, own_package, exported_as);
        if ctx.deadcode_signals {
            out.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::JS);
        }
        if ctx.deadcode_signals {
            out.dyn_uses = super::dynuse::extract(tree, source, "javascript");
        }
        out
    }
}

/// The local name a `local-export` record gives an export cgg cannot
/// name (`export default createX()`): not an identifier, so it never
/// matches a definition, and the resolver reads it as "unknown".
pub(crate) const UNKNOWN_EXPORT: &str = "<value>";

/// The package the file belongs to (the nearest `package.json` /
/// `deno.json` name), after every real import and with the name in
/// `alias`, never `path`: it is where the file lives, not something it
/// depends on. The resolver finds a workspace package's root directory
/// through it.
pub(crate) fn push_package_name(
    facts: &mut FileFacts,
    name: Option<String>,
    exported_as: Vec<String>,
) {
    if let Some(name) = name {
        facts.imports.push(ImportRecord {
            kind: "package-name".into(),
            path: String::new(),
            alias: name,
            site_line: 1,
            site_byte: 0,
        });
    }
    // The specifiers the package's `exports` map gives this file.
    for spec in exported_as {
        facts.imports.push(ImportRecord {
            kind: "package-export".into(),
            path: String::new(),
            alias: spec,
            site_line: 1,
            site_byte: 0,
        });
    }
}

pub(crate) struct JsWalker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    pub facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// `extends`/`implements` of the enclosing class, innermost last.
    bases: Vec<Vec<String>>,
}

impl<'a> JsWalker<'a> {
    pub(crate) fn new(
        ctx: crate::ExtractCtx<'a>,
        source: &'a [u8],
        facts: &'a mut FileFacts,
    ) -> Self {
        Self {
            source,
            ctx,
            facts,
            scope: Vec::new(),
            bases: Vec::new(),
        }
    }

    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
    }

    pub(crate) fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            // `abstract class` is its own node kind in TypeScript; it is a
            // class, and its members are owned by it.
            "class_declaration" | "class" | "abstract_class_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                self.bases.push(super::attrs::base_types(node, self.source));
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                self.bases.pop();
                return;
            }
            "method_definition" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            // TypeScript `interface I extends J { m(): void }`: a type
            // whose members are declarations. Owned like a class's, so a
            // call typed `I` binds to `I.m` — the declaration the
            // implementations below it are dispatched from.
            "interface_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if name.is_empty() {
                    return;
                }
                self.bases.push(super::attrs::base_types(node, self.source));
                self.scope.push(name);
                self.walk_children(node);
                self.scope.pop();
                self.bases.pop();
                return;
            }
            // A member with no body: an interface's method, or an
            // `abstract` method of an abstract class. Not a method
            // signature inside an object type (`type T = { m(): void }`),
            // which belongs to no class.
            "method_signature" | "abstract_method_signature" => {
                let owned = node.kind() == "abstract_method_signature"
                    || node.parent().is_some_and(|p| p.kind() == "interface_body");
                if owned && !self.scope.is_empty() {
                    self.record_signature(node);
                }
                return;
            }
            "lexical_declaration" | "variable_declaration" => {
                self.try_record_named_fn(node);
                self.try_record_require(node);
                self.walk_children(node);
                return;
            }
            "export_statement" => {
                // `export * as oak from "…/oak/mod.ts"` / `export { x } from
                // "m"` re-export a module, and are the only place some
                // modules name a dependency (Deno's `deps.ts` convention).
                if node.child_by_field_name("source").is_some() {
                    self.record_import(node);
                    self.record_reexport(node);
                } else {
                    self.record_local_export(node);
                }
                // Walk into the exported declaration.
                self.walk_children(node);
                return;
            }
            "expression_statement" => {
                // Handle `exports.foo = function() {}` and
                // `module.exports.foo = function() {}` patterns (CJS).
                self.try_record_exports_assign(node);
                self.try_record_require_assign(node);
                self.try_record_cjs_export(node);
                self.walk_children(node);
                return;
            }
            "import_statement" => {
                self.record_import(node);
                return;
            }
            "call_expression" | "new_expression" => {
                self.record_call(node);
                self.extract_named_fn_args(node);
                self.extract_inline_handler(node);
                self.walk_children(node);
                return;
            }
            "return_statement" | "pair" => {
                // return function name() {...} OR { key: function name() {...} }
                self.extract_named_fn_child(node);
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                self.walk(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_function(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }
        let qn = self.qn(&simple);
        let (sl, el) = line_range(node);
        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: super::attrs::collect_with_preceding(node, self.source),
            ..Default::default()
        });
    }

    fn record_method(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }
        // Detect variant from node structure.
        let is_static = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "static");
        let is_getter = node.children(&mut node.walk()).any(|c| c.kind() == "get");
        let is_setter = node.children(&mut node.walk()).any(|c| c.kind() == "set");
        let variant = if simple == "constructor" {
            DefVariant::Constructor
        } else if is_getter || is_setter {
            DefVariant::Property
        } else if is_static {
            DefVariant::StaticMethod
        } else {
            DefVariant::InherentMethod
        };
        let qn = self.qn(&simple);
        let (sl, el) = line_range(node);
        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            // Deliberately no language-native token: the dead-code
            // capability table reads a non-empty `visibility` as "this
            // language extracts visibility", and only class members are
            // covered here — module-level visibility (`export`) is not.
            visibility: String::new(),
            vis: class_member_vis(node),
            // TypeScript decorators (`@Get('/users')`) are shape A and
            // carry the whole NestJS routing surface.
            attributes: super::attrs::collect_with_preceding(node, self.source),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    /// A bodiless member declaration: an interface method or an abstract
    /// method. Tagged `abstract` so duck-typed fan-out prefers the
    /// implementations, as it does a Python `@abstractmethod`.
    fn record_signature(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }
        let qn = self.qn(&simple);
        let (sl, el) = line_range(node);
        let mut attributes = super::attrs::collect_with_preceding(node, self.source);
        attributes.push("abstract".to_string());
        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant: DefVariant::TraitMethod,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // Interface members are public; an abstract method says.
            vis: if node.kind() == "method_signature" {
                cgg_core::Vis::Public
            } else {
                class_member_vis(node)
            },
            attributes,
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn try_record_exports_assign(&mut self, node: Node) {
        // Handle various CJS patterns:
        // `exports.foo = function() {}`
        // `obj.method = function name() {}`
        // `req.get = req.header = function header() {}` (chained)
        let child = node.child(0);
        let Some(child) = child else { return };
        if child.kind() != "assignment_expression" {
            return;
        }
        self.extract_assignment_fn(child);
    }

    fn extract_assignment_fn(&mut self, node: Node) {
        let Some(left) = node.child_by_field_name("left") else {
            return;
        };
        let Some(right) = node.child_by_field_name("right") else {
            return;
        };

        // If RHS is another assignment, recurse (chained: a = b = function(){})
        if right.kind() == "assignment_expression" {
            self.extract_assignment_fn(right);
        }

        // Check if RHS is a function
        if !matches!(
            right.kind(),
            "arrow_function"
                | "function_expression"
                | "function"
                | "assignment_expression"
        ) {
            return;
        }
        // For chained assignments, the function is in the inner one
        let func_node = if right.kind() == "assignment_expression" {
            right.child_by_field_name("right").filter(|r| {
                matches!(
                    r.kind(),
                    "arrow_function" | "function_expression" | "function"
                )
            })
        } else {
            Some(right)
        };
        let Some(_) = func_node else { return };

        if left.kind() != "member_expression" {
            return;
        }
        let prop = left
            .child_by_field_name("property")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if prop.is_empty() {
            return;
        }
        let qn = self.qn(&prop);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: prop,
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn try_record_named_fn(&mut self, node: Node) {
        // `const foo = (x) => ...` or `const foo = function() {}`
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if child.kind() != "variable_declarator" {
                continue;
            }
            let Some(name_node) = child.child_by_field_name("name") else {
                continue;
            };
            if name_node.kind() != "identifier" {
                continue;
            }
            let Some(value) = child.child_by_field_name("value") else {
                continue;
            };
            if !matches!(
                value.kind(),
                "arrow_function" | "function_expression" | "generator_function"
            ) {
                continue;
            }
            let simple = self.text(name_node).to_string();
            if simple.is_empty() {
                continue;
            }
            let qn = self.qn(&simple);
            let (sl, el) = line_range(child);
            self.facts.definitions.push(DefRecord {
                simple_name: simple,
                qualified_name: qn,
                variant: DefVariant::FreeFunction,
                start_line: sl,
                end_line: el,
                start_byte: child.start_byte() as u32,
                end_byte: child.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(child)),
                visibility: String::new(),
                attributes: Vec::new(),
                ..Default::default()
            });
        }
    }

    /// `sails = require('sails')` — a `require` assigned to a variable
    /// declared earlier (commonly inside `try { … }` so a missing optional
    /// dependency can be reported). Same import as the declaration form.
    fn try_record_require_assign(&mut self, node: Node) {
        let Some(expr) = node.named_child(0) else {
            return;
        };
        if expr.kind() != "assignment_expression" {
            return;
        }
        let (Some(left), Some(right)) = (
            expr.child_by_field_name("left"),
            expr.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "identifier" || right.kind() != "call_expression" {
            return;
        }
        if right.child_by_field_name("function").map(|f| self.text(f)) != Some("require")
        {
            return;
        }
        let Some(args) = right.child_by_field_name("arguments") else {
            return;
        };
        let mut c = args.walk();
        let path = args
            .children(&mut c)
            .find(|a| a.kind() == "string")
            .map(|a| {
                self.text(a)
                    .trim_matches(|c| c == '\'' || c == '"')
                    .to_string()
            });
        let Some(path) = path.filter(|p| !p.is_empty()) else {
            return;
        };
        self.facts.imports.push(ImportRecord {
            kind: "import".into(),
            path,
            alias: self.text(left).to_string(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn try_record_require(&mut self, node: Node) {
        // `const foo = require('./mod')` or `const { bar } = require('./mod')`
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if child.kind() != "variable_declarator" {
                continue;
            }
            let Some(value) = child.child_by_field_name("value") else {
                continue;
            };
            if value.kind() != "call_expression" {
                continue;
            }
            let func = value.child_by_field_name("function");
            if func.map(|f| self.text(f)) != Some("require") {
                continue;
            }
            // Extract the path from arguments
            let args = value.child_by_field_name("arguments");
            let path = args.and_then(|a| {
                let count = a.child_count();
                for i in 0..count {
                    let arg = a.child(i as u32).unwrap();
                    if arg.kind() == "string" {
                        return Some(
                            self.text(arg)
                                .trim_matches(|c| c == '\'' || c == '"')
                                .to_string(),
                        );
                    }
                }
                None
            });
            let Some(path) = path else { continue };
            if path.is_empty() {
                continue;
            }

            let Some(name_node) = child.child_by_field_name("name") else {
                continue;
            };
            match name_node.kind() {
                "identifier" => {
                    // `const foo = require('./mod')` -> namespace import
                    let alias = self.text(name_node).to_string();
                    self.facts.imports.push(ImportRecord {
                        kind: "import".into(),
                        path,
                        alias,
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
                "object_pattern" => {
                    // `const { bar, baz } = require('./mod')` -> from-import
                    let mut names = Vec::new();
                    let mut ic = name_node.walk();
                    for prop in name_node.children(&mut ic) {
                        match prop.kind() {
                            "shorthand_property_identifier_pattern" => {
                                names.push(self.text(prop).to_string());
                            }
                            "pair_pattern" => {
                                if let Some(v) = prop.child_by_field_name("value") {
                                    names.push(self.text(v).to_string());
                                }
                            }
                            _ => {}
                        }
                    }
                    if !names.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "from-import".into(),
                            path,
                            alias: names.join(", "),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                _ => {}
            }
        }
    }

    fn record_import(&mut self, node: Node) {
        // TypeScript `import x = require('./m')`: the binding and the
        // specifier both sit inside the clause, not on the statement.
        if let Some(clause) = node
            .children(&mut node.walk())
            .find(|n| n.kind() == "import_require_clause")
        {
            let alias = clause
                .children(&mut clause.walk())
                .find(|n| n.kind() == "identifier")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            let path = clause
                .child_by_field_name("source")
                .map(|n| self.unquote(n))
                .unwrap_or_default();
            if !alias.is_empty() && !path.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: "import".into(),
                    path,
                    alias,
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
            return;
        }
        let source_node = node.child_by_field_name("source").or_else(|| {
            // Fallback: find the string child directly.
            let count = node.child_count();
            for i in 0..count {
                let child = node.child(i as u32).unwrap();
                if child.kind() == "string" {
                    return Some(child);
                }
            }
            None
        });
        let Some(source_node) = source_node else {
            return;
        };
        let raw = self.text(source_node);
        let path = raw.trim_matches(|c| c == '\'' || c == '"').to_string();
        if path.is_empty() {
            return;
        }

        // Collect imported names from import_clause.
        let clause = node
            .children(&mut node.walk())
            .find(|n| n.kind() == "import_clause");
        let Some(clause) = clause else {
            // Side-effect import: `import './polyfill'`
            self.facts.imports.push(ImportRecord {
                kind: "import".into(),
                path,
                alias: String::new(),
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
            return;
        };

        let mut names: Vec<(String, String)> = Vec::new(); // (local, original)
        let mut c = clause.walk();
        for child in clause.children(&mut c) {
            match child.kind() {
                "identifier" => {
                    // default import: `import foo from '...'`
                    let name = self.text(child).to_string();
                    names.push((name, "default".to_string()));
                }
                "namespace_import" => {
                    // `import * as ns from '...'`
                    let alias = child
                        .child_by_field_name("name")
                        .or_else(|| {
                            child
                                .children(&mut child.walk())
                                .find(|n| n.kind() == "identifier")
                        })
                        .map(|n| self.text(n).to_string())
                        .unwrap_or_default();
                    if !alias.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "import".into(),
                            path: path.clone(),
                            alias,
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                "named_imports" => {
                    let mut ic = child.walk();
                    for spec in child.children(&mut ic) {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let orig = spec
                            .child_by_field_name("name")
                            .map(|n| self.text(n).to_string())
                            .unwrap_or_default();
                        let local = spec
                            .child_by_field_name("alias")
                            .map(|n| self.text(n).to_string())
                            .unwrap_or_else(|| orig.clone());
                        if !orig.is_empty() {
                            names.push((local, orig));
                        }
                    }
                }
                _ => {}
            }
        }

        // Emit from-import records for named imports.
        if !names.is_empty() {
            let items: String = names
                .iter()
                .map(|(local, orig)| {
                    if local == orig {
                        orig.clone()
                    } else {
                        format!("{orig} as {local}")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            self.facts.imports.push(ImportRecord {
                kind: "from-import".into(),
                path,
                alias: items,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    /// A string literal's text without its quotes.
    fn unquote(&self, n: Node) -> String {
        self.text(n)
            .trim_matches(|c| c == '\'' || c == '"' || c == '`')
            .to_string()
    }

    /// `export * from './m'`, `export * as ns from './m'`, `export { a, b
    /// as c } from './m'`: names this module provides that live in
    /// another one. Recorded as a `re-export` import (the plain `import`
    /// record above keeps the dependency visible) so the resolver can
    /// follow a call through a barrel file to the module that defines
    /// it. `alias` is `*`, `* as ns`, or the specifier list.
    fn record_reexport(&mut self, node: Node) {
        let Some(path) = node
            .child_by_field_name("source")
            .map(|n| self.unquote(n))
            .filter(|p| !p.is_empty())
        else {
            return;
        };
        let mut alias = String::new();
        let mut star = false;
        let mut c = node.walk();
        for child in node.children(&mut c) {
            match child.kind() {
                "*" => star = true,
                "namespace_export" => {
                    let name = child
                        .children(&mut child.walk())
                        .filter(|n| n.is_named())
                        .last()
                        .map(|n| self.unquote(n))
                        .unwrap_or_default();
                    if !name.is_empty() {
                        alias = format!("* as {name}");
                    }
                }
                "export_clause" => alias = self.export_specifiers(child),
                _ => {}
            }
        }
        if alias.is_empty() && star {
            alias = "*".to_string();
        }
        if alias.is_empty() {
            return;
        }
        self.push_reexport(node, path, alias);
    }

    /// The specifiers of an `export { … }` clause as `name` / `name as
    /// alias`, comma-joined.
    fn export_specifiers(&self, clause: Node) -> String {
        let mut items: Vec<String> = Vec::new();
        let mut c = clause.walk();
        for spec in clause.children(&mut c) {
            if spec.kind() != "export_specifier" {
                continue;
            }
            let name = spec
                .child_by_field_name("name")
                .map(|n| self.unquote(n))
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            match spec.child_by_field_name("alias").map(|n| self.unquote(n)) {
                Some(alias) if !alias.is_empty() && alias != name => {
                    items.push(format!("{name} as {alias}"))
                }
                _ => items.push(name),
            }
        }
        items.join(", ")
    }

    /// Names this module exports under a different name than it defines
    /// them, or as its default: `export { a, b as c }`, `export default
    /// foo`, `export default function foo() {}`. Recorded as a
    /// `local-export` (empty path) — `local` or `local as exported` —
    /// so an import of `c` or of the default can be followed to the
    /// definition this file holds.
    fn record_local_export(&mut self, node: Node) {
        let mut c = node.walk();
        let children: Vec<Node> = node.children(&mut c).collect();
        let mut alias = String::new();
        if let Some(clause) = children.iter().find(|n| n.kind() == "export_clause") {
            alias = self.export_specifiers(*clause);
        } else if children.iter().any(|n| n.kind() == "default") {
            let name = node
                .child_by_field_name("declaration")
                .and_then(|d| d.child_by_field_name("name"))
                .or_else(|| {
                    node.child_by_field_name("value")
                        .filter(|v| v.kind() == "identifier")
                })
                .map(|n| self.text(n).to_string())
                .or_else(|| {
                    // `export default new Api()`.
                    node.child_by_field_name("value")
                        .and_then(|v| self.constructed_class(v))
                })
                // A value cgg cannot name (`export default createX()`,
                // an anonymous function, an object): the resolver must
                // not take a miss through it as proof of absence.
                .unwrap_or_else(|| UNKNOWN_EXPORT.to_string());
            alias = format!("{name} as default");
        } else if let Some(eq) = children.iter().position(|n| n.kind() == "=") {
            // TypeScript `export = value`: CommonJS `module.exports`.
            let value = children[eq + 1..].iter().find(|n| n.is_named());
            match value {
                Some(v) if v.kind() == "identifier" => {
                    alias = format!("{} as default", self.text(*v));
                }
                Some(v) => {
                    if let Some(name) = self.constructed_class(*v) {
                        alias = format!("{name} as default");
                    } else if let Some(path) = self.required_path(*v) {
                        self.push_reexport(node, path, "*, default".to_string());
                        return;
                    } else {
                        alias = format!("{UNKNOWN_EXPORT} as default");
                    }
                }
                None => {}
            }
        }
        if alias.is_empty() {
            return;
        }
        self.push_local_export(node, alias);
    }

    fn push_local_export(&mut self, node: Node, alias: String) {
        self.facts.imports.push(ImportRecord {
            kind: "local-export".into(),
            path: String::new(),
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    /// CommonJS exports: `module.exports = foo`, `module.exports = { a,
    /// b: c }`, `module.exports = require('./m')`, `exports.k = v`,
    /// `module.exports.k = require('./m')`. The same `local-export` /
    /// `re-export` records the ES forms produce. A function assigned to
    /// `module.exports` is already a definition named `exports` (see
    /// `extract_assignment_fn`), and is the module's default.
    fn try_record_cjs_export(&mut self, node: Node) {
        let Some(expr) = node.named_child(0) else {
            return;
        };
        if expr.kind() != "assignment_expression" {
            return;
        }
        let (Some(left), Some(right)) = (
            expr.child_by_field_name("left"),
            expr.child_by_field_name("right"),
        ) else {
            return;
        };
        if left.kind() != "member_expression" {
            return;
        }
        let object = left.child_by_field_name("object");
        let property = left
            .child_by_field_name("property")
            .map(|p| self.text(p).to_string())
            .unwrap_or_default();
        if self.is_module_exports(left) {
            // `module.exports = …`: the module's default value.
            match right.kind() {
                "identifier" => {
                    let name = self.text(right).to_string();
                    self.push_local_export(node, format!("{name} as default"));
                }
                "function_expression"
                | "function"
                | "arrow_function"
                | "generator_function" => {
                    self.push_local_export(node, "exports as default".to_string());
                }
                "class" => {
                    if let Some(n) = right.child_by_field_name("name") {
                        let name = self.text(n).to_string();
                        self.push_local_export(node, format!("{name} as default"));
                    }
                }
                // `module.exports = new SettingsHelpers(…)`: an instance,
                // whose members are its class's.
                "new_expression" => {
                    if let Some(name) = self.constructed_class(right) {
                        self.push_local_export(node, format!("{name} as default"));
                    }
                }
                "object" => {
                    let mut items: Vec<String> = Vec::new();
                    let mut c = right.walk();
                    for prop in right.children(&mut c) {
                        match prop.kind() {
                            // `{ ...require('./a') }`: every export of
                            // `./a`; any other spread is unreadable.
                            "spread_element" => {
                                let required = prop
                                    .named_child(0)
                                    .and_then(|v| self.required_path(v));
                                match required {
                                    Some(path) => {
                                        self.push_reexport(node, path, "*".to_string())
                                    }
                                    None => {
                                        items.push(format!("{UNKNOWN_EXPORT} as default"))
                                    }
                                }
                            }
                            "shorthand_property_identifier" => {
                                items.push(self.text(prop).to_string());
                            }
                            "pair" => {
                                let key = prop
                                    .child_by_field_name("key")
                                    .map(|k| self.unquote(k))
                                    .unwrap_or_default();
                                let Some(value) = prop.child_by_field_name("value")
                                else {
                                    continue;
                                };
                                if key.is_empty() {
                                    continue;
                                }
                                if value.kind() == "identifier" {
                                    let v = self.text(value).to_string();
                                    if v == key {
                                        items.push(v);
                                    } else {
                                        items.push(format!("{v} as {key}"));
                                    }
                                } else if let Some(path) = self.required_path(value) {
                                    self.push_reexport(node, path, format!("* as {key}"));
                                }
                            }
                            _ => {}
                        }
                    }
                    if !items.is_empty() {
                        self.push_local_export(node, items.join(", "));
                    }
                }
                _ => {
                    if let Some(path) = self.required_path(right) {
                        // `module.exports = require('./m')`: `./m`'s
                        // names, and its value (`module.exports = new
                        // Store()` there) as this module's.
                        self.push_reexport(node, path, "*, default".to_string());
                    } else {
                        // `module.exports = makeThing()`: a value cgg
                        // cannot read.
                        self.push_local_export(
                            node,
                            format!("{UNKNOWN_EXPORT} as default"),
                        );
                    }
                }
            }
            return;
        }
        // `exports.k = …` / `module.exports.k = …`.
        let on_exports = object.is_some_and(|o| {
            (o.kind() == "identifier" && self.text(o) == "exports")
                || self.is_module_exports(o)
        });
        if !on_exports || property.is_empty() {
            return;
        }
        if right.kind() == "identifier" {
            let v = self.text(right).to_string();
            if v != property {
                self.push_local_export(node, format!("{v} as {property}"));
            }
        } else if let Some(path) = self.required_path(right) {
            self.push_reexport(node, path, format!("* as {property}"));
        }
    }

    /// The class a `new Klass(…)` expression constructs, when it is named
    /// by a plain identifier.
    fn constructed_class(&self, n: Node) -> Option<String> {
        if n.kind() != "new_expression" {
            return None;
        }
        let ctor = n.child_by_field_name("constructor")?;
        (ctor.kind() == "identifier").then(|| self.text(ctor).to_string())
    }

    /// `module.exports` as an expression.
    fn is_module_exports(&self, n: Node) -> bool {
        n.kind() == "member_expression"
            && n.child_by_field_name("object")
                .is_some_and(|o| self.text(o) == "module")
            && n.child_by_field_name("property")
                .is_some_and(|p| self.text(p) == "exports")
    }

    /// The specifier of a `require('…')` call.
    fn required_path(&self, n: Node) -> Option<String> {
        if n.kind() != "call_expression"
            || n.child_by_field_name("function").map(|f| self.text(f)) != Some("require")
        {
            return None;
        }
        let args = n.child_by_field_name("arguments")?;
        let mut c = args.walk();
        let s = args.children(&mut c).find(|a| a.kind() == "string")?;
        Some(self.unquote(s)).filter(|p| !p.is_empty())
    }

    fn push_reexport(&mut self, node: Node, path: String, alias: String) {
        self.facts.imports.push(ImportRecord {
            kind: "re-export".into(),
            path,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn extract_named_fn_child(&mut self, node: Node) {
        // Find any named function_expression direct child
        let count = node.child_count();
        for i in 0..count as u32 {
            let Some(child) = node.child(i) else { continue };
            if child.kind() != "function_expression" && child.kind() != "function" {
                continue;
            }
            let name = child
                .child_by_field_name("name")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let qn = self.qn(&name);
            let (sl, el) = (
                (child.start_position().row as u32) + 1,
                (child.end_position().row as u32) + 1,
            );
            self.facts.definitions.push(DefRecord {
                simple_name: name,
                qualified_name: qn,
                variant: DefVariant::FreeFunction,
                start_line: sl,
                end_line: el,
                start_byte: child.start_byte() as u32,
                end_byte: child.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(child)),
                visibility: String::new(),
                attributes: Vec::new(),
                ..Default::default()
            });
        }
    }

    fn extract_named_fn_args(&mut self, node: Node) {
        // defineGetter(obj, 'name', function name() {...})
        // or any call with a named function_expression argument
        let args = node.child_by_field_name("arguments");
        let Some(args) = args else { return };
        let count = args.child_count();
        for i in 0..count as u32 {
            let Some(arg) = args.child(i) else { continue };
            if arg.kind() != "function_expression" && arg.kind() != "function" {
                continue;
            }
            // Check if the function has a name
            let name = arg
                .child_by_field_name("name")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            let qn = self.qn(&name);
            let (sl, el) = (
                (arg.start_position().row as u32) + 1,
                (arg.end_position().row as u32) + 1,
            );
            self.facts.definitions.push(DefRecord {
                simple_name: name,
                qualified_name: qn,
                variant: DefVariant::FreeFunction,
                start_line: sl,
                end_line: el,
                start_byte: arg.start_byte() as u32,
                end_byte: arg.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(arg)),
                visibility: String::new(),
                attributes: Vec::new(),
                ..Default::default()
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        // `f()` names the callee under `function`; `new C()` names it
        // under `constructor`. `new Worker('./w.js')` is the only
        // reference to a worker module anywhere, so missing the second
        // form leaves that whole file with no caller.
        let Some(func) = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("constructor"))
        else {
            return;
        };
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "member_expression" => {
                let obj = func
                    .child_by_field_name("object")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let prop = func
                    .child_by_field_name("property")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (prop, obj)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        let context = if recv.is_empty() {
            name.clone()
        } else {
            format!("{recv}.{name}")
        };
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
        // Shape B/E: `app.get('/x', listUsers)` and
        // `new Worker('./w.js')`. Both put the handler in argument
        // position, where the callee alone says nothing.
        let extra = super::registrar::capture(&self.ctx, node, self.source, &context);
        self.facts.references.extend(extra);
    }

    /// Shape C — an anonymous handler written in place.
    ///
    /// The body is already reachable, so this adds no edge the graph was
    /// missing. It exists so the *route* has something to point at:
    /// `POST /admin/users` is the fact a reader wants, and roughly two
    /// thirds of Express handlers are written anonymously.
    ///
    /// Gated on the registration shape so ordinary callbacks — `.map(x
    /// => …)`, promise chains, `setTimeout` — mint nothing.
    fn extract_inline_handler(&mut self, node: Node) {
        let Some(func) = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("constructor"))
        else {
            return;
        };
        let context = self.text(func).to_string();
        // `describe('...', () => {})` has exactly the shape of a route
        // registration. Gating on the verb is what keeps a test suite
        // from minting a synthesized handler per block — four thousand
        // of them on TypeORM, none of which any rule could use.
        if !self
            .ctx
            .is_registrar_verb(super::registrar::last_segment(&context))
        {
            return;
        }
        let Some(route) = super::registrar::is_registration_shape(node, self.source)
        else {
            return;
        };
        // The shared helper, not a local re-scan: it also finds a
        // closure sitting in an options object, which is how Azure
        // Functions' v4 model writes every handler
        // (`app.http("name", { handler: async () => … })`). Scanning
        // argument position alone left that whole runtime enumerating
        // nothing.
        let closures = super::registrar::inline_closures(node);
        for closure in closures {
            let line = (closure.start_position().row as u32) + 1;
            let simple = format!("handler_at_{line}");
            let qn = self.qn(&simple);
            let (sl, el) = line_range(closure);
            self.facts.definitions.push(DefRecord {
                simple_name: simple.clone(),
                qualified_name: qn,
                variant: DefVariant::NamedClosure,
                start_line: sl,
                end_line: el,
                start_byte: closure.start_byte() as u32,
                end_byte: closure.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(closure)),
                visibility: String::new(),
                attributes: vec!["synthetic".to_string()],
                ..Default::default()
            });
            self.facts.references.push(RefRecord {
                from_macro_arg: false,
                path_receiver: false,
                dot_call: false,
                arrow: false,
                macro_call: false,
                invocant: false,
                implicit_receiver: false,
                not_def: None,
                arity: None,
                name: simple,
                receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                site_line: line,
                site_byte: closure.start_byte() as u32,
                context: context.clone(),
                route: route.clone(),
                kwargs: Vec::new(),
            });
        }
    }
}

fn line_range(n: Node) -> (u32, u32) {
    (
        (n.start_position().row as u32) + 1,
        (n.end_position().row as u32) + 1,
    )
}

/// Visibility of a `method_definition`, read from the grammar.
///
/// Only a class member has one: TypeScript's `accessibility_modifier`
/// (`public`/`protected`/`private`), or an ECMAScript `#name`, which is
/// private at runtime as well. A class member with neither is public —
/// the language default in both TypeScript and JavaScript. A method in
/// an object literal has no visibility of its own (what the object is
/// exported or assigned as decides it), so it stays `Vis::Unknown`, as
/// does every free function: this plugin does not model module-level
/// visibility.
///
/// TypeScript's `private`/`protected` are compile-time only — bracket
/// access (`obj["m"]`) and `as any` reach them — which is one reason the
/// TypeScript plugin still does not declare the visibility signal, and
/// its dead-code findings stay capped below `High`.
fn class_member_vis(node: Node) -> cgg_core::Vis {
    if node.parent().map(|p| p.kind()) != Some("class_body") {
        return cgg_core::Vis::Unknown;
    }
    let modifier = node
        .children(&mut node.walk())
        .find(|c| c.kind() == "accessibility_modifier")
        .and_then(|m| m.child(0))
        .map(|t| t.kind());
    match modifier {
        Some("private") => cgg_core::Vis::Private,
        Some("protected") => cgg_core::Vis::Protected,
        Some("public") => cgg_core::Vis::Public,
        _ if node
            .child_by_field_name("name")
            .is_some_and(|n| n.kind() == "private_property_identifier") =>
        {
            cgg_core::Vis::Private
        }
        _ => cgg_core::Vis::Public,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        JavaScriptPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.js"),
            &tree,
            src.as_bytes(),
        )
    }

    fn imports_of(f: &FileFacts, kind: &str) -> Vec<(String, String)> {
        f.imports
            .iter()
            .filter(|i| i.kind == kind)
            .map(|i| (i.path.clone(), i.alias.clone()))
            .collect()
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    /// FINDINGS A4: what a module re-exports from another, so a call
    /// through a barrel file can be followed to the module defining it.
    /// The plain `import` record of the same statement stays.
    #[test]
    fn es_reexports_are_recorded() {
        let f = extract(
            "export * from './impl';\n\
             export * as u from './util';\n\
             export { a, b as c, default as D } from './re';\n",
        );
        assert_eq!(
            imports_of(&f, "re-export"),
            pairs(&[
                ("./impl", "*"),
                ("./util", "* as u"),
                ("./re", "a, b as c, default as D"),
            ])
        );
        assert_eq!(imports_of(&f, "import").len(), 3);
    }

    /// A default export cgg cannot name is recorded as unknown, so the
    /// resolver does not take a miss through it as proof of absence.
    #[test]
    fn unreadable_default_exports_are_marked_unknown() {
        let f = extract(
            "function make() { return {}; }\n\
             export default make();\n\
             module.exports = make();\n\
             module.exports = { ...require('./a'), ...other, b };\n",
        );
        let locals: Vec<String> = imports_of(&f, "local-export")
            .into_iter()
            .map(|(_, a)| a)
            .collect();
        assert_eq!(
            locals,
            vec![
                "<value> as default".to_string(),
                "<value> as default".to_string(),
                "<value> as default, b".to_string(),
            ]
        );
        // A spread `require()` re-exports that module's names.
        assert_eq!(
            imports_of(&f, "re-export"),
            vec![("./a".to_string(), "*".to_string())]
        );
    }

    /// FINDINGS A4: names a module exports under another name or as its
    /// default, ES and CommonJS alike.
    #[test]
    fn local_exports_and_commonjs_exports_are_recorded() {
        let f = extract(
            "function run() {}\nfunction impl2() {}\n\
             export { run as go };\n\
             export default function main() {}\n\
             module.exports = { run, other: impl2, sub: require('./sub') };\n\
             exports.k = impl2;\n\
             module.exports = require('./proxy');\n\
             module.exports = function () {};\n\
             module.exports = new Store(cfg);\n",
        );
        assert_eq!(
            imports_of(&f, "local-export"),
            pairs(&[
                ("", "run as go"),
                ("", "main as default"),
                ("", "run, impl2 as other"),
                ("", "impl2 as k"),
                ("", "exports as default"),
                ("", "Store as default"),
            ])
        );
        assert_eq!(
            imports_of(&f, "re-export"),
            pairs(&[("./sub", "* as sub"), ("./proxy", "*, default")])
        );
    }

    #[test]
    fn function_declarations() {
        let src = "function greet(name) {}\nasync function fetchData() {}\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(names.contains(&"fetchData"), "got: {names:?}");
    }

    #[test]
    fn arrow_function_named() {
        let src =
            "const add = (a, b) => a + b;\nlet mul = function(a, b) { return a*b; };\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"add"), "got: {names:?}");
        assert!(names.contains(&"mul"), "got: {names:?}");
    }

    #[test]
    fn class_methods() {
        let src = r#"
class Service {
    constructor(name) {}
    run() {}
    static create() {}
    get label() { return ""; }
}
"#;
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(by["constructor"], DefVariant::Constructor);
        assert_eq!(by["run"], DefVariant::InherentMethod);
        assert_eq!(by["create"], DefVariant::StaticMethod);
        assert_eq!(by["label"], DefVariant::Property);
        // Qualified names include class.
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name == "Service.run")
        );
    }

    #[test]
    fn class_member_visibility_follows_private_names() {
        // The walker is shared with TypeScript; JavaScript's only access
        // syntax is the ECMAScript `#name`, and a class member without
        // one is public. Object-literal methods and functions stay
        // unknown.
        use cgg_core::Vis;
        let src = "class S {\n  #hidden() {}\n  shown() {}\n}\nconst o = { m() {} };\nfunction g() {}\n";
        let f = extract(src);
        let vis = |n: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == n)
                .unwrap()
                .vis
        };
        assert_eq!(vis("#hidden"), Vis::Private);
        assert_eq!(vis("shown"), Vis::Public);
        assert_eq!(vis("m"), Vis::Unknown);
        assert_eq!(vis("g"), Vis::Unknown);
    }

    #[test]
    fn esm_imports_captured() {
        let src = "import { helper, scale as s } from './utils.js';\nimport * as math from './math.js';\n";
        let f = extract(src);
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "from-import" && i.path == "./utils.js")
        );
        assert!(
            f.imports.iter().any(|i| i.kind == "import"
                && i.path == "./math.js"
                && i.alias == "math")
        );
    }

    #[test]
    fn call_expressions_captured() {
        let src = "function f() { greet('x'); obj.run(); }\n";
        let f = extract(src);
        let refs: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(refs.contains(&("greet", "")), "got: {refs:?}");
        assert!(refs.contains(&("run", "obj")), "got: {refs:?}");
    }

    #[test]
    fn export_from_and_assigned_require_are_imports() {
        let f = extract(
            "export * as oak from 'https://deno.land/x/oak@v12.1.0/mod.ts';\nvar sails;\ntry { sails = require('sails'); } catch (e) {}\n",
        );
        let paths: Vec<_> = f.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(
            paths.contains(&"https://deno.land/x/oak@v12.1.0/mod.ts"),
            "{paths:?}"
        );
        assert!(paths.contains(&"sails"), "{paths:?}");
    }
}
