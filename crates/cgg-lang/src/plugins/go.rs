//! Go plugin.
//!
//! Two-phase AST pass over `tree-sitter-go`:
//!
//! * **Package root** — derived from the `package foo` clause. Module
//!   qualified names look like `pkg.Plain` (free functions) or
//!   `pkg.T.Do` (methods with receiver type `T` or `*T`).
//! * **Definitions** — `function_declaration` and
//!   `method_declaration` (receiver's type_identifier is folded into
//!   the qualified name; pointer-vs-value receivers are treated the
//!   same, matching Go's method-set rules for calls).
//! * **References** — `call_expression` with callee either a bare
//!   `identifier` or a `selector_expression` (`x.y`).
//! * **Imports** — `import_declaration` specs, including aliased
//!   (`al "path"`), dot-imports (`. "path"`), and blank imports
//!   (`_ "path"`). Alias becomes the import's `alias`; the path is
//!   the quoted string content.
//!
//! Limitations: embedded-struct method promotion, interface method
//! dispatch, and generics are not modeled — they fall through to
//! low-confidence cross-file lookups when the simple name is unique.

use std::path::Path;

use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct GoPlugin;

impl LanguagePlugin for GoPlugin {
    fn id(&self) -> &'static str {
        "go"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".go"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            test_defs: true,
            unreachable: true,
            value_refs: true,
            visibility: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_go::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "go");
        let root = tree.root_node();

        // Discover the package name first (default to file stem).
        let pkg = package_name(root, source).unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("main")
                .to_string()
        });

        // Synthetic marker so cross-file / other resolvers know the
        // file's package namespace without re-parsing.
        facts.imports.push(ImportRecord {
            kind: "package-root".into(),
            path: pkg.clone(),
            alias: String::new(),
            site_line: 1,
            site_byte: 0,
        });

        // The module path(s) this file is importable under, so the
        // resolver can tell `example.com/app/internal/jobs` (workspace)
        // from `github.com/google/uuid` (a dependency).
        let (modules, package_path) = super::manifest::go_module_info(path);
        facts.packages = modules;

        let mut w = Walker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            pkg,
        };
        w.walk(root);
        let mut out = facts;
        // The import path of this file's package, after every real
        // import so nothing keyed on "first import naming X" sees it.
        // The path rides in `alias`, never `path`: it is where the file
        // lives, not something it depends on.
        if let Some(p) = package_path {
            out.imports.push(ImportRecord {
                kind: "package-path".into(),
                path: String::new(),
                alias: p,
                site_line: 1,
                site_byte: 0,
            });
        }
        if ctx.deadcode_signals {
            out.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::GO);
        }
        if ctx.deadcode_signals {
            out.dyn_uses = super::dynuse::extract(tree, source, "go");
        }
        out
    }
}

fn package_name(root: Node, source: &[u8]) -> Option<String> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() == "package_clause" {
            for c2 in child.children(&mut child.walk()) {
                if c2.kind() == "package_identifier" {
                    return Some(c2.utf8_text(source).unwrap_or("").to_string());
                }
            }
        }
    }
    None
}

struct Walker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    pkg: String,
}

impl<'a> Walker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_declaration" => {
                self.record_function(node);
                self.record_param_shadows(node);
                // Continue into body for call sites.
                self.walk_children(node);
                return;
            }
            "method_declaration" => {
                self.record_method(node);
                self.record_param_shadows(node);
                self.walk_children(node);
                return;
            }
            "func_literal" => {
                self.record_param_shadows(node);
                self.walk_children(node);
                return;
            }
            "import_declaration" => {
                self.record_imports(node);
                return;
            }
            "type_declaration" => {
                // Extract interface method specs
                self.extract_interface_methods(node);
                self.walk_children(node);
                return;
            }
            "call_expression" => {
                if let Some(r) = self.ref_from_call(node) {
                    // One `receiver.VERB(string, handler)` matcher covers
                    // Gin, Echo, Fiber, Chi and net/http at once — the
                    // handler is always in argument position.
                    let context = if r.receiver_hint.is_empty() {
                        r.name.clone()
                    } else {
                        format!("{}.{}", r.receiver_hint, r.name)
                    };
                    self.facts.references.push(r);
                    let extra =
                        super::registrar::capture(&self.ctx, node, self.source, &context);
                    self.facts.references.extend(extra);
                    self.extract_inline_handler(node, &context);
                }
                self.walk_children(node);
                return;
            }
            "var_declaration" => {
                self.record_var_type(node);
                self.walk_children(node);
                return;
            }
            "short_var_declaration" => {
                self.record_short_var_type(node);
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    /// Shape C — a handler written in place: `m.Get("/", func() string
    /// { … })`. Martini's README writes every route this way, and a
    /// `func_literal` has no name, so without a node here the route has
    /// nothing to point at. The body's calls are already attributed to
    /// the enclosing function; this adds the node the entry names.
    ///
    /// Gated on a registrar verb and the registration shape, as in the
    /// JavaScript plugin, so `sort.Slice(xs, func…)`, goroutines and
    /// `defer func(){}()` mint nothing.
    fn extract_inline_handler(&mut self, node: Node, context: &str) {
        if !self
            .ctx
            .is_registrar_verb(super::registrar::last_segment(context))
        {
            return;
        }
        let Some(route) = super::registrar::is_registration_shape(node, self.source)
        else {
            return;
        };
        for closure in super::registrar::inline_closures(node) {
            let line = (closure.start_position().row as u32) + 1;
            let simple = format!("handler_at_{line}");
            self.facts.definitions.push(cgg_core::DefRecord {
                simple_name: simple.clone(),
                qualified_name: format!("{}.{simple}", self.pkg),
                variant: cgg_core::DefVariant::NamedClosure,
                start_line: line,
                end_line: (closure.end_position().row as u32) + 1,
                start_byte: closure.start_byte() as u32,
                end_byte: closure.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(closure)),
                visibility: String::new(),
                attributes: vec!["synthetic".to_string()],
                ..Default::default()
            });
            self.facts.references.push(cgg_core::RefRecord {
                name: simple,
                receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                site_line: line,
                site_byte: closure.start_byte() as u32,
                context: context.to_string(),
                route: route.clone(),
                ..Default::default()
            });
        }
    }

    fn walk_children(&mut self, node: Node) {
        let mut cursor = node.walk();
        if cursor.goto_first_child() {
            loop {
                self.walk(cursor.node());
                if !cursor.goto_next_sibling() {
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
        let qn = format!("{pkg}.{simple}", pkg = self.pkg, simple = simple);
        let (sl, el) = line_range(node);
        let (vis, test_role) = (go_vis(&simple), go_test_role(&simple));
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
            vis,
            test_role,
            attributes: Vec::new(),
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
        // Receiver type extraction: parameter_list ->
        // parameter_declaration -> type_identifier (or pointer_type ->
        // type_identifier).
        let recv_type = {
            let mut out = String::new();
            if let Some(recv) = node.child_by_field_name("receiver") {
                let mut c = recv.walk();
                for p in recv.children(&mut c) {
                    if p.kind() != "parameter_declaration" {
                        continue;
                    }
                    if let Some(t) = p.child_by_field_name("type") {
                        out = self.receiver_type_name(t).unwrap_or_default();
                    }
                    break;
                }
            }
            out
        };
        let qn = if recv_type.is_empty() {
            format!("{pkg}.{simple}", pkg = self.pkg)
        } else {
            format!("{pkg}.{recv_type}.{simple}", pkg = self.pkg)
        };
        let (sl, el) = line_range(node);
        let (vis, test_role) = (go_vis(&simple), go_test_role(&simple));
        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant: DefVariant::InherentMethod,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            vis,
            test_role,
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_imports(&mut self, node: Node) {
        // import_declaration has either a single import_spec or an
        // import_spec_list wrapping many specs.
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "import_spec" => self.record_import_spec(child),
                "import_spec_list" => {
                    for s in child.children(&mut child.walk()) {
                        if s.kind() == "import_spec" {
                            self.record_import_spec(s);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn record_import_spec(&mut self, spec: Node) {
        let path_node = spec.child_by_field_name("path");
        let Some(path_node) = path_node else { return };
        // Extract the string content (strip quotes).
        let raw = self.text(path_node);
        let path = raw.trim_matches('"').to_string();

        let alias_node = spec.child_by_field_name("name");
        let alias = alias_node
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();

        let site_line = (spec.start_position().row as u32) + 1;
        self.facts.imports.push(ImportRecord {
            kind: "import".into(),
            path,
            alias,
            site_line,
            site_byte: spec.start_byte() as u32,
        });
    }

    fn extract_interface_methods(&mut self, node: Node) {
        // type_declaration -> type_spec -> type_identifier + interface_type -> method_spec_list
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "type_spec" {
                continue;
            }
            let type_name = child
                .child_by_field_name("name")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            let type_node = child.child_by_field_name("type");
            let Some(type_node) = type_node else { continue };
            if type_node.kind() != "interface_type" {
                continue;
            }
            // Walk method specs inside the interface
            let mut ic = type_node.walk();
            for spec in type_node.children(&mut ic) {
                if spec.kind() != "method_elem" && spec.kind() != "method_spec" {
                    continue;
                }
                let method_name = spec
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if method_name.is_empty() {
                    continue;
                }
                let qn = format!("{}.{method_name}", self.package_prefix(&type_name));
                let (sl, el) = (
                    (spec.start_position().row as u32) + 1,
                    (spec.end_position().row as u32) + 1,
                );
                let vis = go_vis(&method_name);
                self.facts.definitions.push(cgg_core::DefRecord {
                    simple_name: method_name,
                    qualified_name: qn,
                    variant: cgg_core::DefVariant::InherentMethod,
                    start_line: sl,
                    end_line: el,
                    start_byte: spec.start_byte() as u32,
                    end_byte: spec.end_byte() as u32,
                    signature_hint: self.text(spec).trim().to_string(),
                    visibility: String::new(),
                    vis,
                    attributes: Vec::new(),
                    ..Default::default()
                });
            }
        }
    }

    /// The named type of a method receiver: `T`, `*T`, and a generic
    /// `Map[K, V]` / `*Map[K, V]` (its type parameters are not part of
    /// the owner's name).
    fn receiver_type_name(&self, t: Node) -> Option<String> {
        match t.kind() {
            "type_identifier" => Some(self.text(t).to_string()),
            "pointer_type" | "parenthesized_type" => {
                let mut c = t.walk();
                let inner = t.named_children(&mut c).next()?;
                self.receiver_type_name(inner)
            }
            "generic_type" => t
                .child_by_field_name("type")
                .and_then(|n| self.receiver_type_name(n)),
            _ => None,
        }
    }

    fn package_prefix(&self, type_name: &str) -> String {
        format!("{}.{type_name}", self.pkg)
    }

    fn record_var_type(&mut self, node: Node) {
        // var_declaration -> var_spec -> name + type
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "var_spec" {
                continue;
            }
            let var_name = child
                .child_by_field_name("name")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            let type_name = child
                .child_by_field_name("type")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            if var_name.is_empty() || type_name.is_empty() {
                return;
            }
            let type_name = type_name.trim_start_matches('*').to_string();
            if !type_name.starts_with(char::is_uppercase) {
                return;
            }
            self.facts.local_types.push(cgg_core::LocalType {
                var_name,
                type_name,
                scope_byte: node.start_byte() as u32,
            });
        }
    }

    fn record_short_var_type(&mut self, node: Node) {
        // short_var_declaration: `helper := NewHelper()` or `add := func() {}`
        let left = node.child_by_field_name("left");
        let right = node.child_by_field_name("right");
        let (Some(left), Some(right)) = (left, right) else {
            return;
        };
        let var_name = self.text(left).to_string();
        if var_name.is_empty() {
            return;
        }
        // `r := jobs.NewRunner()` / `r, err := jobs.Open(p)` /
        // `r := &jobs.Runner{}`: a value made through an imported
        // package. Its type is recorded as written — the call itself
        // (`jobs.NewRunner()`) or the package-qualified type — and the
        // resolver reads the callee's declared result type inside that
        // package. Naming conventions are not consulted.
        // The variable is in scope only after its declaration (Go spec):
        // in `x, err := x.Open()` the right side's `x` is the package, so
        // the type takes effect from the statement's end.
        if let Some(first) = Self::first_assigned(left)
            && let Some(rhs) = Self::first_value(right)
            && let Some(ty) = self.package_made_type(rhs)
        {
            let var_name = self.text(first).to_string();
            self.facts.local_types.push(cgg_core::LocalType {
                var_name: var_name.clone(),
                type_name: ty,
                scope_byte: node.end_byte() as u32,
            });
            // ...and out of scope where its block ends (an `if`/`for`
            // body, a `case` clause): an untyped entry there ends it, so a
            // sibling block's `pp.New()` is the package again.
            if let Some(scope) = node.parent().filter(|p| p.kind() != "source_file") {
                self.facts.local_types.push(cgg_core::LocalType {
                    var_name,
                    type_name: String::new(),
                    scope_byte: scope.end_byte() as u32,
                });
            }
            return;
        }

        // A local named like an import binding shadows the package from
        // here to the end of its block.
        if let Some(scope) = node.parent().filter(|p| p.kind() != "source_file") {
            let mut c = left.walk();
            let names: Vec<String> = if left.kind() == "expression_list" {
                left.named_children(&mut c)
                    .filter(|n| n.kind() == "identifier")
                    .map(|n| self.text(n).to_string())
                    .collect()
            } else {
                vec![var_name.clone()]
            };
            let shadowing: Vec<String> =
                names.into_iter().filter(|n| self.imports_bind(n)).collect();
            for name in shadowing {
                self.push_scoped_local(
                    name,
                    LOCAL_SHADOW.to_string(),
                    node.end_byte() as u32,
                    scope.end_byte() as u32,
                );
            }
        }
        // Check if RHS is a func_literal — emit as a callable
        let rhs = if right.kind() == "expression_list" {
            right.child(0)
        } else {
            Some(right)
        };
        if let Some(rhs) = rhs
            && rhs.kind() == "func_literal"
        {
            let qn = format!("{}.{var_name}", self.pkg);
            let (sl, el) = (
                (node.start_position().row as u32) + 1,
                (node.end_position().row as u32) + 1,
            );
            let vis = go_vis(&var_name);
            self.facts.definitions.push(cgg_core::DefRecord {
                simple_name: var_name.clone(),
                qualified_name: qn,
                variant: cgg_core::DefVariant::FreeFunction,
                start_line: sl,
                end_line: el,
                start_byte: node.start_byte() as u32,
                end_byte: node.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(node)),
                visibility: String::new(),
                vis,
                attributes: Vec::new(),
                ..Default::default()
            });
            return;
        }

        // Heuristic: if RHS is a call to NewFoo(), type is Foo.
        let call = if right.kind() == "expression_list" {
            right.child(0).filter(|c| c.kind() == "call_expression")
        } else if right.kind() == "call_expression" {
            Some(right)
        } else {
            None
        };
        let Some(call) = call else { return };
        let func = call.child_by_field_name("function");
        let Some(func) = func else { return };
        let func_name = self.text(func);
        let type_name = if func_name.starts_with("New") && func_name.len() > 3 {
            func_name[3..].to_string()
        } else if func_name.starts_with("new") && func_name.len() > 3 {
            let rest = &func_name[3..];
            if rest.starts_with(char::is_uppercase) {
                rest.to_string()
            } else {
                return;
            }
        } else {
            return;
        };
        if !type_name.starts_with(char::is_uppercase) {
            return;
        }
        self.facts.local_types.push(cgg_core::LocalType {
            var_name,
            type_name,
            scope_byte: node.start_byte() as u32,
        });
    }

    /// A parameter named like an import binding (`c *app.RequestContext`
    /// in a file importing `c ".../client"`) shadows the package for the
    /// function's body: recorded as a local from the body's start — its
    /// package-qualified type when it has one, else `<local>` — ended at
    /// the body's end by an untyped entry.
    fn record_param_shadows(&mut self, node: Node) {
        let Some(body) = node.child_by_field_name("body") else {
            return;
        };
        let lists = [
            node.child_by_field_name("receiver"),
            node.child_by_field_name("parameters"),
        ];
        let mut shadows: Vec<(String, String)> = Vec::new();
        for list in lists.into_iter().flatten() {
            let mut c = list.walk();
            for p in list.named_children(&mut c) {
                let ty = p
                    .child_by_field_name("type")
                    .map(|t| {
                        let t = self.text(t).trim_start_matches('*');
                        t.split('[').next().unwrap_or(t).trim().to_string()
                    })
                    .unwrap_or_default();
                let typed = ty.split_once('.').is_some_and(|(pkg, t)| {
                    self.imports_bind(pkg) && t.starts_with(char::is_uppercase)
                });
                let mut nc = p.walk();
                for n in p.children_by_field_name("name", &mut nc) {
                    let name = self.text(n).to_string();
                    if self.imports_bind(&name) {
                        let ty = if typed {
                            ty.clone()
                        } else {
                            LOCAL_SHADOW.to_string()
                        };
                        shadows.push((name, ty));
                    }
                }
            }
        }
        for (name, ty) in shadows {
            self.push_scoped_local(
                name,
                ty,
                body.start_byte() as u32,
                body.end_byte() as u32,
            );
        }
    }

    /// A local typed `ty` from `start` to `end` (its block's end, where an
    /// untyped entry ends it).
    fn push_scoped_local(
        &mut self,
        var_name: String,
        type_name: String,
        start: u32,
        end: u32,
    ) {
        self.facts.local_types.push(cgg_core::LocalType {
            var_name: var_name.clone(),
            type_name,
            scope_byte: start,
        });
        self.facts.local_types.push(cgg_core::LocalType {
            var_name,
            type_name: String::new(),
            scope_byte: end,
        });
    }

    /// The first identifier an assignment's left side binds (`r` in `r,
    /// err := …`), when it is a plain identifier.
    fn first_assigned(left: Node) -> Option<Node> {
        let first = if left.kind() == "expression_list" {
            left.named_child(0)?
        } else {
            left
        };
        (first.kind() == "identifier").then_some(first)
    }

    /// The first value of an assignment's right side.
    fn first_value(right: Node) -> Option<Node> {
        if right.kind() == "expression_list" {
            right.named_child(0)
        } else {
            Some(right)
        }
    }

    /// `pkg.F(…)` → `pkg.F()`, `pkg.T{…}` / `&pkg.T{…}` → `pkg.T`, where
    /// `pkg` is a name this file's imports bind (a method call on a
    /// variable, `r.Clone()`, is not a package's doing).
    fn package_made_type(&self, rhs: Node) -> Option<String> {
        match rhs.kind() {
            "call_expression" => {
                let func = rhs.child_by_field_name("function")?;
                if func.kind() != "selector_expression" {
                    return None;
                }
                let operand = func.child_by_field_name("operand")?;
                let field = func.child_by_field_name("field")?;
                if operand.kind() != "identifier"
                    || !self.imports_bind(self.text(operand))
                {
                    return None;
                }
                Some(format!("{}.{}()", self.text(operand), self.text(field)))
            }
            "unary_expression" => {
                let operand = rhs.child_by_field_name("operand")?;
                if rhs.child(0).map(|c| c.kind()) != Some("&") {
                    return None;
                }
                (operand.kind() == "composite_literal")
                    .then(|| self.package_made_type(operand))
                    .flatten()
            }
            "composite_literal" => {
                let ty = rhs.child_by_field_name("type")?;
                if ty.kind() != "qualified_type" {
                    return None;
                }
                let t = self.text(ty);
                let t = t.split('[').next().unwrap_or(t).trim();
                let pkg = t.split('.').next().unwrap_or("");
                self.imports_bind(pkg).then(|| t.to_string())
            }
            _ => None,
        }
    }

    /// Whether an `import` of this file binds `name`: its alias, or the
    /// package name the path implies (last element, a `.vN`/`/vN` major
    /// version and a `go-` prefix dropped).
    fn imports_bind(&self, name: &str) -> bool {
        self.facts
            .imports
            .iter()
            .filter(|i| i.kind == "import")
            .any(|i| {
                let alias = i.alias.trim();
                if !alias.is_empty() {
                    return alias == name;
                }
                let path = i.path.trim();
                let mut segs = path.rsplit('/');
                let mut last = segs.next().unwrap_or(path);
                let is_major = |s: &str| {
                    s.len() >= 2
                        && s.starts_with('v')
                        && s[1..].chars().all(|c| c.is_ascii_digit())
                };
                if is_major(last) {
                    last = segs.next().unwrap_or(last);
                }
                if let Some((stem, v)) = last.rsplit_once('.')
                    && is_major(v)
                {
                    last = stem;
                }
                last == name || last.strip_prefix("go-") == Some(name)
            })
    }

    fn ref_from_call(&mut self, node: Node) -> Option<RefRecord> {
        let func = node.child_by_field_name("function")?;
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "selector_expression" => {
                let operand = func.child_by_field_name("operand")?;
                let field = func.child_by_field_name("field")?;
                (self.text(field).to_string(), self.text(operand).to_string())
            }
            _ => return None,
        };
        if name.is_empty() {
            return None;
        }
        let site_line = (node.start_position().row as u32) + 1;
        Some(RefRecord {
            name,
            receiver_hint: recv,
            site_line,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        })
    }
}

/// The type a Go plugin records for a local whose type is unknown but
/// which shadows an imported package's name (not an identifier, so it
/// never names a type).
pub(crate) const LOCAL_SHADOW: &str = "<local>";

fn line_range(n: Node) -> (u32, u32) {
    let s = (n.start_position().row as u32) + 1;
    let e = (n.end_position().row as u32) + 1;
    (s, e)
}

/// Go's visibility rule *is* the identifier's first letter — this is
/// exact, not a heuristic.
fn go_vis(simple: &str) -> cgg_core::Vis {
    match simple.chars().next() {
        Some(c) if c.is_uppercase() => cgg_core::Vis::Public,
        _ => cgg_core::Vis::Internal,
    }
}

/// `TestXxx`/`BenchmarkXxx`/`FuzzXxx`/`ExampleXxx` — the harness
/// conventions. Requires more than the bare prefix so `Test` itself and
/// `Testing` do not qualify.
fn go_test_role(simple: &str) -> Option<cgg_core::TestRole> {
    if simple == "TestMain" {
        return Some(cgg_core::TestRole::Fixture);
    }
    for pfx in ["Test", "Benchmark", "Fuzz", "Example"] {
        if let Some(rest) = simple.strip_prefix(pfx)
            && rest.chars().next().is_some_and(|c| c.is_uppercase())
        {
            return Some(cgg_core::TestRole::Case);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_go::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        GoPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.go"),
            &tree,
            src.as_bytes(),
        )
    }

    /// R7: a generic type's methods keep their owner — `func (m *Map[K,
    /// V]) ReplaceKey` is `pkg.Map.ReplaceKey`, not `pkg.ReplaceKey`.
    #[test]
    fn generic_receiver_keeps_its_type() {
        let f = extract(
            "package ordered\n\ntype Map[K comparable, V any] struct{}\n\n\
             func (m *Map[K, V]) ReplaceKey(a, b K) bool { return true }\n\
             func (m Map[K, V]) Len() int { return 0 }\n",
        );
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(qns, vec!["ordered.Map.ReplaceKey", "ordered.Map.Len"]);
    }

    /// FINDINGS R9: a parameter named like an import binding shadows the
    /// package over the function's body — typed when its type is another
    /// package's, else `<local>` — and an untyped entry ends it there.
    #[test]
    fn a_parameter_shadowing_an_import_is_scoped_to_the_body() {
        let src = "package main\n\nimport (\n\tc \"example.com/pkg/client\"\n\t\"example.com/pkg/app\"\n)\n\n\
                   func h(c *app.RequestContext) { c.JSON(200) }\n\
                   func k(c int) { _ = c }\n\
                   func u(app int) {}\n";
        let f = extract(src);
        let body = |name: &str| {
            let at = src.find(name).unwrap();
            let open = at + src[at..].find('{').unwrap();
            let close = at + src[at..].find('}').unwrap() + 1;
            (open as u32, close as u32)
        };
        let entries: Vec<(&str, &str, u32)> = f
            .local_types
            .iter()
            .map(|t| (t.var_name.as_str(), t.type_name.as_str(), t.scope_byte))
            .collect();
        let (h0, h1) = body("func h");
        let (k0, k1) = body("func k");
        let (u0, u1) = body("func u");
        assert_eq!(
            entries,
            vec![
                ("c", "app.RequestContext", h0),
                ("c", "", h1),
                ("c", LOCAL_SHADOW, k0),
                ("c", "", k1),
                ("app", LOCAL_SHADOW, u0),
                ("app", "", u1),
            ]
        );
    }

    /// R1: `x, err := x.Open()` types `x` from the end of the statement —
    /// the right side's `x` is the package.
    #[test]
    fn package_made_local_starts_after_its_statement() {
        let src = "package main\n\nimport \"example.com/x\"\n\nfunc main() {\n\tx, err := x.Open()\n\t_ = err\n}\n";
        let f = extract(src);
        let lt = f
            .local_types
            .iter()
            .find(|t| t.type_name == "x.Open()")
            .expect("typed");
        let stmt_end = src.find("x.Open()").unwrap() + "x.Open()".len();
        assert_eq!(lt.scope_byte as usize, stmt_end);
    }

    /// FINDINGS A5: a value made through an imported package is typed as
    /// written — the call, or the package-qualified type — for the
    /// resolver to follow into that package. A bare `NewFoo()` keeps the
    /// older reading; a value a variable's method returned is untyped.
    #[test]
    fn values_made_through_a_package_are_typed_as_written() {
        let f = extract(
            "package main\n\nimport \"example.com/app/jobs\"\n\n\
             func main() {\n\
             \tr := jobs.NewRunner()\n\
             \tq, err := jobs.Open(\"x\")\n\
             \tw := &jobs.Runner{}\n\
             \tz := jobs.Runner{X: 1}\n\
             \tv := NewThing()\n\
             \tm := r.Clone()\n\
             \t_, _, _, _ = q, err, w, m\n\
             }\n",
        );
        let got: Vec<(&str, &str)> = f
            .local_types
            .iter()
            .filter(|t| !t.type_name.is_empty())
            .map(|t| (t.var_name.as_str(), t.type_name.as_str()))
            .collect();
        // Each package-made value is also closed where its block ends.
        assert!(
            f.local_types
                .iter()
                .any(|t| t.var_name == "r" && t.type_name.is_empty())
        );
        assert_eq!(
            got,
            vec![
                ("r", "jobs.NewRunner()"),
                ("q", "jobs.Open()"),
                ("w", "jobs.Runner"),
                ("z", "jobs.Runner"),
                ("v", "Thing"),
            ]
        );
    }

    #[test]
    fn inline_route_handler_gets_a_node() {
        let f = extract(
            "package main\n\nfunc main() {\n\tm := martini.Classic()\n\tm.Get(\"/\", func() string {\n\t\treturn render()\n\t})\n\tsort.Slice(xs, func(i, j int) bool { return xs[i] < xs[j] })\n}\n",
        );
        let synth: Vec<_> = f
            .definitions
            .iter()
            .filter(|d| d.attributes.iter().any(|a| a == "synthetic"))
            .collect();
        assert_eq!(synth.len(), 1, "{:?}", f.definitions);
        assert_eq!(synth[0].qualified_name, "main.handler_at_5");
        let r = f
            .references
            .iter()
            .find(|r| r.name == "handler_at_5")
            .expect("value ref to the handler");
        assert_eq!(r.context, "m.Get");
        assert_eq!(r.route, "/");
    }

    #[test]
    fn free_function() {
        let f = extract("package pkg\n\nfunc Plain() {}\n");
        assert!(
            f.definitions
                .iter()
                .any(|d| d.qualified_name == "pkg.Plain")
        );
    }

    #[test]
    fn method_qualified_name_has_receiver_type() {
        let src = r#"
package pkg

type T struct {}
func (t *T) Do() {}
func (t T) Helper() {}
"#;
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"pkg.T.Do"), "got: {names:?}");
        assert!(names.contains(&"pkg.T.Helper"), "got: {names:?}");
    }

    #[test]
    fn imports_captured() {
        let src = r#"
package pkg
import (
    "fmt"
    al "other/lib"
    _ "blanked"
)
"#;
        let f = extract(src);
        let imports: Vec<(String, String)> = f
            .imports
            .iter()
            .filter(|i| i.kind == "import")
            .map(|i| (i.path.clone(), i.alias.clone()))
            .collect();
        assert!(imports.contains(&("fmt".into(), "".into())));
        assert!(imports.contains(&("other/lib".into(), "al".into())));
        assert!(imports.contains(&("blanked".into(), "_".into())));
    }

    #[test]
    fn call_expressions_captured() {
        let src = r#"
package pkg
import "fmt"
func Do() {
    Helper()
    fmt.Println("hi")
}
func Helper() {}
"#;
        let f = extract(src);
        let ref_names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(ref_names.contains(&"Helper"));
        assert!(ref_names.contains(&"Println"));
        let fmt_ref = f.references.iter().find(|r| r.name == "Println").unwrap();
        assert_eq!(fmt_ref.receiver_hint, "fmt");
    }

    #[test]
    fn package_root_marker_emitted() {
        let f = extract("package alpha\n\nfunc X() {}\n");
        let pkg = f.imports.iter().find(|i| i.kind == "package-root").unwrap();
        assert_eq!(pkg.path, "alpha");
    }
}
