//! C# plugin.
//!
//! Two-phase AST pass over `tree-sitter-c-sharp`:
//!
//! * **Definitions** — `class_declaration`, `struct_declaration`,
//!   `record_declaration`, `interface_declaration` contribute to the
//!   scope stack (they don't themselves produce callables, but their
//!   methods do). `method_declaration`, `constructor_declaration`,
//!   `destructor_declaration`, and `local_function_statement` become
//!   callables. Accessors (`get`/`set`) of a property are emitted as
//!   `Property` variants.
//! * **References** — `invocation_expression` with callee either an
//!   `identifier` or a `member_access_expression` (`obj.Foo`,
//!   `Ns.Sub.Foo`).
//! * **Imports** — `using_directive` records: plain `using Ns;`,
//!   aliased `using A = Ns.X;`, and `using static Ns.X;` are all
//!   captured; v1 resolves only the plain and aliased forms.
//!
//! Limitations: partial classes are merged only by exact name
//! (behavior matches most IDEs at the scope level, but cross-file
//! overload resolution is not modeled); generics, extension methods
//! with special dispatch, and explicit interface implementations are
//! best-effort.

use std::path::Path;

use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn id(&self) -> &'static str {
        "csharp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".cs", ".csx"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            dyn_uses: true,
            visibility: true,
            attributes: true,
            impls: true,
            value_refs: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c_sharp::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "csharp");
        // A tree the grammar could not parse because `#if`/`#else` split a
        // statement or a declaration is re-read from a copy with the
        // directives blanked out (byte offsets and lines unchanged).
        let reparsed = reparse_without_conditionals(tree, source);
        let (tree, source) = match &reparsed {
            Some((t, s)) => (t, s.as_slice()),
            None => (tree, source),
        };
        let mut w = Walker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        if ctx.deadcode_signals {
            facts.dyn_uses = super::dynuse::extract(tree, source, "csharp");
        }
        // Handlers this file's sibling markup (`X.xaml`, `X.razor`,
        // `X.tscn`) binds to an event: entries, never edges.
        let handlers: Vec<super::markup::Handler<'_>> = facts
            .definitions
            .iter()
            .map(|d| super::markup::Handler {
                name: &d.simple_name,
                signature: &d.signature_hint,
            })
            .collect();
        let bindings = super::markup::bindings(path, &handlers);
        facts.markup_bindings = bindings;
        facts
    }
}

#[derive(Clone, Debug)]
enum Scope {
    Namespace(String),
    Type(String),
}

impl Scope {
    fn name(&self) -> &str {
        match self {
            Scope::Namespace(s) | Scope::Type(s) => s.as_str(),
        }
    }
}

struct Walker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    scope: Vec<Scope>,
    /// Base list of the enclosing type, innermost last — `: IJob` is
    /// what makes `Execute` an entry point, and only the type says so.
    bases: Vec<Vec<String>>,
}

impl<'a> Walker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        crate::closure_types::record("csharp", node, self.source, self.facts);
        match node.kind() {
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    // The full namespace, nested blocks joined, so the
                    // resolver knows which namespaces the workspace
                    // declares — including a file-scoped one, whose
                    // members' qualified names do not carry it.
                    let mut full: Vec<&str> = self
                        .scope
                        .iter()
                        .filter_map(|s| match s {
                            Scope::Namespace(n) => Some(n.as_str()),
                            Scope::Type(_) => None,
                        })
                        .collect();
                    full.push(name.as_str());
                    let full = full.join(".");
                    if !self.facts.packages.contains(&full) {
                        self.facts.packages.push(full);
                    }
                    self.scope.push(Scope::Namespace(name));
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "class_declaration"
            | "struct_declaration"
            | "record_declaration"
            | "record_struct_declaration"
            | "interface_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let bases = super::attrs::base_types(node, self.source);
                if !name.is_empty() {
                    // Every type, bodiless ones included: see the Java
                    // plugin. A chain that finds nothing then proves the
                    // member is outside the tree (`base.M()`).
                    let mut parts: Vec<&str> =
                        self.scope.iter().map(|s| s.name()).collect();
                    parts.push(&name);
                    self.facts.classes.push(cgg_core::ClassDecl {
                        class_qn: parts.join("."),
                        base_types: bases.clone(),
                        line: (node.start_position().row as u32) + 1,
                    });
                }
                self.bases.push(bases);
                if !name.is_empty() {
                    self.scope.push(Scope::Type(name));
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                self.bases.pop();
                return;
            }
            "method_declaration" | "local_function_statement" => {
                self.record_method(node, DefVariant::InherentMethod);
                // `partial void OnLoaded();` and its implementing part: a
                // hook whose other half is often generated (designer
                // files, source generators) or outside the tree; calls to
                // an unimplemented one are compiled away.
                if node.kind() == "method_declaration"
                    && has_modifier(node, "partial")
                    && let Some(d) = self.facts.definitions.last_mut()
                {
                    d.attributes.push(PARTIAL_DECLARATION.to_string());
                }
                self.walk_children(node);
                return;
            }
            "constructor_declaration" => {
                self.record_method(node, DefVariant::Constructor);
                self.walk_children(node);
                return;
            }
            "destructor_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| format!("~{}", self.text(n)))
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_named(node, &name, DefVariant::Destructor);
                }
                self.walk_children(node);
                return;
            }
            "accessor_declaration" => {
                // get/set accessors inside a property.
                let name = node
                    .children(&mut node.walk())
                    .find(|c| matches!(c.kind(), "get" | "set" | "init"))
                    .map(|n| n.kind().to_string())
                    .unwrap_or_else(|| "accessor".to_string());
                self.record_named(node, &name, DefVariant::Property);
                self.walk_children(node);
                return;
            }
            "using_directive" => {
                self.record_using(node);
                return;
            }
            "local_declaration_statement" => {
                self.record_local_type(node);
                self.walk_children(node);
                return;
            }
            // Value positions where a method group can stand: an argument
            // (`Select(F)`, `new EventHandler(this.H)`, `nameof(F)`), the
            // right of an assignment (`evt += H`, `f = F`), a variable
            // initializer (`Func<int> f = F`), a `return`, an
            // expression body, a collection initializer.
            "argument"
            | "assignment_expression"
            | "variable_declarator"
            | "return_statement"
            | "arrow_expression_clause"
            | "initializer_expression"
                if self.ctx.reference_sites =>
            {
                self.record_method_groups(node);
                self.walk_children(node);
                return;
            }
            "invocation_expression" => {
                if let Some(r) = self.ref_from_invoke(node) {
                    // Shape B: `app.MapGet("/x", Handler)` — the minimal
                    // API's whole routing surface lives in argument
                    // position, so the callee alone tells us nothing.
                    let context = if r.receiver_hint.is_empty() {
                        r.name.clone()
                    } else {
                        format!("{}.{}", r.receiver_hint, r.name)
                    };
                    self.facts.references.push(r);
                    let extra =
                        super::registrar::capture(&self.ctx, node, self.source, &context);
                    self.facts.references.extend(extra);
                }
                self.walk_children(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
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

    fn record_method(&mut self, node: Node, variant: DefVariant) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }
        self.record_named(node, &simple, variant);
    }

    fn record_named(&mut self, node: Node, simple: &str, variant: DefVariant) {
        let mut parts: Vec<&str> = self.scope.iter().map(|s| s.name()).collect();
        parts.push(simple);
        let qn = parts.join(".");
        let (sl, el) = line_range(node);
        self.facts.definitions.push(DefRecord {
            simple_name: simple.to_string(),
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            vis: csharp_vis(node),
            // Verbatim, so `[HttpGet("/users")]` keeps its route.
            attributes: {
                let mut a = super::attrs::collect(node, self.source);
                // Implements a member some base declares: the `override`
                // modifier, or an explicit interface implementation
                // (`void IFoo.Bar()`), callable only through `IFoo`. Like
                // Java's `@Override`, it says the runtime reaches this
                // through the base, which may be outside the tree.
                if has_modifier(node, "override")
                    || node
                        .children(&mut node.walk())
                        .any(|c| c.kind() == "explicit_interface_specifier")
                {
                    a.push("override".to_string());
                }
                a
            },
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_local_type(&mut self, node: Node) {
        // local_declaration_statement -> variable_declaration -> type + variable_declarator
        let var_decl = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "variable_declaration");
        let Some(var_decl) = var_decl else { return };

        // Check if explicit type or implicit (var)
        let mut children = var_decl.walk();
        let type_node = var_decl.children(&mut children).find(|c| {
            c.kind() == "identifier"
                || c.kind() == "qualified_name"
                || c.kind() == "generic_name"
                || c.kind() == "implicit_type"
                || c.kind() == "nullable_type"
        });
        let Some(mut type_node) = type_node else {
            return;
        };
        // `Foo? x` is a `Foo` for every call made on it (`x?.M()`).
        if type_node.kind() == "nullable_type" {
            match type_node
                .child_by_field_name("type")
                .or_else(|| type_node.named_child(0))
            {
                Some(inner) => type_node = inner,
                None => return,
            }
        }

        let declarator = var_decl
            .children(&mut var_decl.walk())
            .find(|c| c.kind() == "variable_declarator");
        let Some(declarator) = declarator else { return };
        let var_name = declarator
            .children(&mut declarator.walk())
            .find(|c| c.kind() == "identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if var_name.is_empty() {
            return;
        }

        if type_node.kind() == "implicit_type" {
            // `var x = new Foo(...)` — infer type from object_creation_expression
            let new_expr = declarator
                .children(&mut declarator.walk())
                .find(|c| c.kind() == "object_creation_expression");
            if let Some(new_expr) = new_expr {
                let type_id = new_expr
                    .child_by_field_name("type")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !type_id.is_empty() && type_id.starts_with(char::is_uppercase) {
                    self.facts.local_types.push(cgg_core::LocalType {
                        var_name,
                        type_name: type_id,
                        scope_byte: node.start_byte() as u32,
                    });
                }
            }
        } else {
            // Explicit type: `Service svc = ...`
            let type_name = self.text(type_node).to_string();
            if !type_name.is_empty() && type_name.starts_with(char::is_uppercase) {
                self.facts.local_types.push(cgg_core::LocalType {
                    var_name,
                    type_name,
                    scope_byte: node.start_byte() as u32,
                });
            }
        }
    }

    fn record_using(&mut self, node: Node) {
        // `using A;` | `using A = B;` | `using static A;`
        let text = self.text(node).trim().to_string();
        let is_static = text.starts_with("using static ");
        let body = text
            .trim_start_matches("using")
            .trim()
            .trim_start_matches("static")
            .trim()
            .trim_end_matches(';')
            .trim()
            .to_string();

        let (path, alias) = if let Some((lhs, rhs)) = body.split_once('=') {
            (rhs.trim().to_string(), lhs.trim().to_string())
        } else {
            (body.clone(), String::new())
        };

        let kind = if is_static { "using-static" } else { "using" };
        self.facts.imports.push(ImportRecord {
            kind: kind.into(),
            path,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    /// The value expressions of a value-position node, as listed on the
    /// match arm in `walk`.
    fn record_method_groups(&mut self, node: Node) {
        let values: Vec<Node> = match node.kind() {
            "assignment_expression" => {
                node.child_by_field_name("right").into_iter().collect()
            }
            "argument" | "variable_declarator" => {
                // The `name` field (`name: value`, the declared variable)
                // is a label, not a value.
                let label = node.child_by_field_name("name").map(|n| n.id());
                let mut c = node.walk();
                node.named_children(&mut c)
                    .filter(|n| Some(n.id()) != label)
                    .collect()
            }
            _ => {
                let mut c = node.walk();
                node.named_children(&mut c).collect()
            }
        };
        for v in values {
            self.record_method_group(v, 0);
        }
    }

    /// A method group: a name or member access in a value position,
    /// recorded as the call through that receiver it would be if invoked
    /// and marked [`RefRecord::value_ref`]. The resolver binds it only
    /// where a callable of that name exists, so a variable, field or
    /// property in the same position costs nothing.
    fn record_method_group(&mut self, v: Node, depth: u8) {
        let (name_node, recv) = match v.kind() {
            "identifier" => (v, String::new()),
            "member_access_expression" => {
                let (Some(expr), Some(name)) = (
                    v.child_by_field_name("expression"),
                    v.child_by_field_name("name"),
                ) else {
                    return;
                };
                if name.kind() != "identifier" {
                    return;
                }
                // Through `this`, `base` or a type (`Program.Handler`,
                // `Ns.Type.Handler`): a method group. Through a variable,
                // field or call (`value.Value`, `x.Name`) it is a
                // property read far more often than a method group, and
                // the receiver's type is rarely known, so resolving it
                // would bind an unrelated same-named method.
                let recv = match expr.kind() {
                    "base" => "super".to_string(),
                    "this" => "this".to_string(),
                    "identifier" | "qualified_name" | "generic_name"
                        if self
                            .text(expr)
                            .starts_with(|c: char| c.is_ascii_uppercase()) =>
                    {
                        self.text(expr).to_string()
                    }
                    "member_access_expression"
                        if self.text(expr).split('.').all(|seg| {
                            seg.starts_with(|c: char| c.is_ascii_uppercase())
                        }) =>
                    {
                        self.text(expr).to_string()
                    }
                    _ => return,
                };
                (name, recv)
            }
            // `(Del) F`: a cast of a method group to a delegate type.
            "cast_expression" if depth < 4 => {
                if let Some(val) = v.child_by_field_name("value") {
                    self.record_method_group(val, depth + 1);
                }
                return;
            }
            // `c ? F : G`, `(F)`: the branches are the values.
            "conditional_expression" | "parenthesized_expression" if depth < 4 => {
                let parts: Vec<Node> = if v.kind() == "conditional_expression" {
                    ["consequence", "alternative"]
                        .iter()
                        .filter_map(|f| v.child_by_field_name(f))
                        .collect()
                } else {
                    let mut c = v.walk();
                    v.named_children(&mut c).collect()
                };
                for p in parts {
                    self.record_method_group(p, depth + 1);
                }
                return;
            }
            _ => return,
        };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (name_node.start_position().row as u32) + 1,
            site_byte: name_node.start_byte() as u32,
            value_ref: true,
            ..Default::default()
        });
    }

    fn ref_from_invoke(&mut self, node: Node) -> Option<RefRecord> {
        let func = node.child_by_field_name("function")?;
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), (String::new(), false)),
            "member_access_expression" => {
                let expr = func.child_by_field_name("expression")?;
                let name = func.child_by_field_name("name")?;
                // `base.M()` is C#'s `super.m()`: the caller's base types,
                // never the caller's own class. Spelled the way every
                // other language spells it so the resolver's base-chain
                // reading applies.
                let recv = if expr.kind() == "base" {
                    ("super".to_string(), false)
                } else {
                    // `new Alpha().Run()`, `((Alpha)o).Run()`, `(o as
                    // Alpha).Run()`: the receiver is an `Alpha`, so the
                    // call is looked up on that type and up its bases, as
                    // a call through a variable of that type would be.
                    // `(x).Run()` / `(await t).Run()` are `x` / `t`.
                    self.shaped_receiver(expr)
                };
                (self.callee_name(name), recv)
            }
            // `Foo<T>()`: the name without its type arguments.
            "generic_name" => (self.callee_name(func), (String::new(), false)),
            // `x?.M()`: the null-conditional form of `x.M()`. The binding
            // (`.M`) names the method; the condition is the receiver.
            "conditional_access_expression" => {
                let cond = func.child_by_field_name("condition")?;
                let binding = func
                    .named_children(&mut func.walk())
                    .find(|c| c.kind() == "member_binding_expression")?;
                let name = binding.child_by_field_name("name")?;
                (self.callee_name(name), self.shaped_receiver(cond))
            }
            _ => return None,
        };
        let (recv, typed_receiver) = recv;
        if name.is_empty() {
            return None;
        }
        let site_line = (node.start_position().row as u32) + 1;
        Some(RefRecord {
            name,
            receiver_hint: recv,
            site_line,
            site_byte: node.start_byte() as u32,
            typed_receiver,
            ..Default::default()
        })
    }
}

impl Walker<'_> {
    /// A receiver read through its shape (`crate::receiver`): the type a
    /// cast or construction names, flagged as a type, or the text of the
    /// value inside parentheses, `await` and `!`.
    fn shaped_receiver(&self, expr: Node) -> (String, bool) {
        let p = crate::receiver::peel("csharp", expr, self.source);
        match p.type_name() {
            Some(t) => (t, true),
            None => (self.receiver_text(p.node), false),
        }
    }

    /// The written name of a callee, without explicit type arguments:
    /// `M<string>` is a call of `M`. Read from the tree (`generic_name`'s
    /// identifier), so a name that legitimately contains no type
    /// arguments is returned as written.
    fn callee_name(&self, n: Node) -> String {
        match n.kind() {
            "generic_name" => n
                .children(&mut n.walk())
                .find(|c| c.kind() == "identifier")
                .map(|c| self.text(c).to_string())
                .unwrap_or_default(),
            _ => self.text(n).to_string(),
        }
    }

    /// A receiver expression's text, with a null-conditional member read
    /// (`x?.y`) spelled as the plain member read (`x.y`) it is when `x`
    /// is not null. Rebuilt from the tree's condition and binding, so
    /// no other text is rewritten.
    fn receiver_text(&self, expr: Node) -> String {
        if expr.kind() == "conditional_access_expression"
            && let Some(cond) = expr.child_by_field_name("condition")
            && let Some(binding) = expr
                .named_children(&mut expr.walk())
                .find(|c| c.kind() == "member_binding_expression")
            && let Some(name) = binding.child_by_field_name("name")
        {
            return format!("{}.{}", self.receiver_text(cond), self.callee_name(name));
        }
        self.text(expr).to_string()
    }
}

fn line_range(n: Node) -> (u32, u32) {
    let s = (n.start_position().row as u32) + 1;
    let e = (n.end_position().row as u32) + 1;
    (s, e)
}

/// Does this declaration carry the modifier keyword `kw`?
///
/// Each `modifier` child wraps one keyword token; matching the token
/// rather than the text means an attribute argument
/// (`[Obsolete("x")] public void F()`) can neither fake a modifier nor
/// hide one.
/// Attribute on a `partial` method part; see
/// `cgg_core::PARTIAL_DECLARATION`.
use cgg_core::PARTIAL_DECLARATION;

/// Re-parse a C# file whose tree has errors after blanking its
/// conditional-compilation directives — first keeping every branch, then,
/// if that still does not parse, keeping only the first branch of each
/// `#if` group. Returns `None` when the original tree is error-free, has
/// no `#if`, or neither copy parses cleanly (the original stands).
fn reparse_without_conditionals(tree: &Tree, source: &[u8]) -> Option<(Tree, Vec<u8>)> {
    if !tree.root_node().has_error() {
        return None;
    }
    let parse = |src: &[u8]| {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_c_sharp::LANGUAGE.into()).ok()?;
        p.parse(src, None).filter(|t| !t.root_node().has_error())
    };
    for first_branch_only in [false, true] {
        let masked = mask_conditionals(source, first_branch_only)?;
        if let Some(t) = parse(&masked) {
            return Some((t, masked));
        }
    }
    None
}

/// The directive word of a preprocessor line (`#if`, `# endif`), if any.
fn directive(line: &[u8]) -> Option<&[u8]> {
    let t = line.trim_ascii_start();
    let rest = t.strip_prefix(b"#")?.trim_ascii_start();
    let end = rest
        .iter()
        .position(|b| !b.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// A copy of `source` with every `#if`/`#elif`/`#else`/`#endif` line
/// replaced by spaces; with `first_branch_only`, the lines of every
/// `#elif`/`#else` branch too. Newlines are kept, so every byte offset
/// and line number is the original's. `None` when there is no `#if`.
fn mask_conditionals(source: &[u8], first_branch_only: bool) -> Option<Vec<u8>> {
    let mut out = source.to_vec();
    // Per open `#if` group: whether it has reached an `#elif`/`#else`.
    let mut groups: Vec<bool> = Vec::new();
    let mut saw_if = false;
    let mut start = 0usize;
    while start < source.len() {
        let end = source[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(source.len(), |i| start + i);
        let line = &source[start..end];
        let skipping = first_branch_only && groups.iter().any(|&in_else| in_else);
        let blank = match directive(line) {
            Some(b"if") => {
                saw_if = true;
                groups.push(false);
                true
            }
            Some(b"elif") | Some(b"else") => {
                if let Some(g) = groups.last_mut() {
                    *g = true;
                }
                true
            }
            Some(b"endif") => {
                groups.pop();
                true
            }
            _ => skipping,
        };
        if blank {
            for b in &mut out[start..end] {
                if *b != b'\r' {
                    *b = b' ';
                }
            }
        }
        start = end + 1;
    }
    saw_if.then_some(out)
}

fn has_modifier(node: Node, kw: &str) -> bool {
    node.children(&mut node.walk())
        .filter(|c| c.kind() == "modifier")
        .any(|m| m.children(&mut m.walk()).any(|t| t.kind() == kw))
}

/// Project the declaration's access modifiers onto the shared
/// vocabulary. The *absent* case is the interesting one and differs per
/// language — and, in C#, per enclosing declaration — which is exactly
/// why this normalization belongs in the plugin.
///
/// Read from the `modifier` nodes. It used to be read from the signature
/// text cut at the first `(`, which an attribute argument ends early:
/// `[Obsolete("x")] public void F()` read as private, and so as
/// corroborated dead code.
fn csharp_vis(node: Node) -> cgg_core::Vis {
    // `protected internal` and `private protected` both reach subtypes,
    // so `protected` wins over the keyword it is combined with.
    if has_modifier(node, "public") {
        return cgg_core::Vis::Public;
    }
    if has_modifier(node, "protected") {
        return cgg_core::Vis::Protected;
    }
    if has_modifier(node, "private") {
        return cgg_core::Vis::Private;
    }
    if has_modifier(node, "internal") {
        return cgg_core::Vis::Internal;
    }
    let parent = node.parent();
    match parent.map(|p| p.kind()) {
        // An accessor without its own modifier has the accessibility of
        // the property, indexer or event that declares it
        // (`accessor_list` → declaration).
        Some("accessor_list") => parent
            .and_then(|l| l.parent())
            .map(csharp_vis)
            .unwrap_or(cgg_core::Vis::Private),
        // Interface members are public unless they say otherwise (C# 8
        // lets a default-implemented member be `private`/`protected`,
        // handled above). Only the interface's own body counts; a type
        // nested inside it has a body of its own.
        Some("declaration_list")
            if parent
                .and_then(|l| l.parent())
                .is_some_and(|d| d.kind() == "interface_declaration") =>
        {
            cgg_core::Vis::Public
        }
        // Class, struct and record members default to private, and so
        // do declarations that cannot carry an access modifier at all:
        // local functions, static constructors, finalizers, and explicit
        // interface implementations (`void IFoo.Bar()`), which are
        // reachable only through the interface.
        _ => cgg_core::Vis::Private,
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
        p.set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        CSharpPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.cs"),
            &tree,
            src.as_bytes(),
        )
    }

    fn extract_refs(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        CSharpPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(true),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.cs"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn a_nullable_local_is_typed_as_its_type() {
        let f = extract(
            "class M { void F() { Foo? a = null; Pt? b = null; int? n = null; Foo c = null; } }",
        );
        let ty = |v: &str| {
            f.local_types
                .iter()
                .find(|l| l.var_name == v)
                .map(|l| l.type_name.clone())
        };
        assert_eq!(ty("a").as_deref(), Some("Foo"));
        assert_eq!(ty("b").as_deref(), Some("Pt"));
        assert_eq!(ty("c").as_deref(), Some("Foo"));
        // Near miss: a nullable primitive is no class.
        assert_eq!(ty("n"), None);
    }

    /// B3: a method group in a value position is recorded through its
    /// receiver; an invoked member and a declared variable's name are not.
    #[test]
    fn conditional_compilation_split_code_is_reparsed() {
        // `#if` in an else-if chain and `#if`/`#else` method headers: the
        // grammar reads a callable named `if`, or two headers; blanking
        // the directives (and, if needed, all but the first branch) gives
        // the code one configuration compiles.
        let src = "namespace N {\n  public class D {\n    public bool Has(int k)\n    {\n        if (a) { return Foo(k); }\n#if X\n        else if (b) { return Bar(k); }\n#endif\n        else { return Baz(k); }\n    }\n#if ASYNC\n    public async Task<int> Load()\n#else\n    public int Load()\n#endif\n    {\n      return Foo(1);\n    }\n  }\n}\n";
        let f = extract(src);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(qns, vec!["N.D.Has", "N.D.Load"], "{qns:?}");
        let has = &f.definitions[0];
        let bar = f.references.iter().find(|r| r.name == "Bar").unwrap();
        assert!(has.start_byte <= bar.site_byte && bar.site_byte < has.end_byte);
        // Byte offsets are the original file's.
        let load = &f.definitions[1];
        assert_eq!(load.start_line, 12);
    }

    #[test]
    fn construction_receivers_name_their_type_and_partial_methods_are_marked() {
        let f = extract(
            "class U { void G() { new Alpha().Run(); new N.Box<int>(1).Get(); (new Beta()).Go(); x.Run(); } }\npartial class P { partial void OnLoaded(); void M() {} }\n",
        );
        let recv = |n: &str| {
            f.references
                .iter()
                .find(|r| r.name == n)
                .map(|r| r.receiver_hint.clone())
                .unwrap()
        };
        assert_eq!(recv("Run"), "Alpha");
        assert_eq!(recv("Get"), "N.Box");
        assert_eq!(recv("Go"), "Beta");
        let partial = |n: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == n)
                .unwrap()
                .attributes
                .iter()
                .any(|a| a == cgg_core::PARTIAL_DECLARATION)
        };
        assert!(partial("OnLoaded"));
        assert!(!partial("M"));
    }

    #[test]
    fn method_groups_are_value_refs() {
        let src = "class P { event System.EventHandler E; void Run(int[] xs) {\n\
            xs.Select(Double); E += OnE; var h = new System.EventHandler(this.OnO);\n\
            System.Func<int,int> f = Triple; Log(nameof(Named)); System.Action a = P.S;\n\
            E -= c ? OnA : OnB; Go(base.Up); Call(); Agg((Del) Local); } }";
        let f = extract_refs(src);
        let got: Vec<(String, String)> = f
            .references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| (r.receiver_hint.clone(), r.name.clone()))
            .collect();
        let want: Vec<(String, String)> = [
            ("", "Double"),
            ("", "OnE"),
            ("", "Named"),
            ("this", "OnO"),
            ("", "Triple"),
            ("P", "S"),
            ("", "OnA"),
            ("", "OnB"),
            ("super", "Up"),
            ("", "Local"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
        let mut got_sorted = got.clone();
        got_sorted.sort();
        let mut want_sorted = want;
        want_sorted.sort();
        assert_eq!(got_sorted, want_sorted, "got {got:?}");
        // The declared names `h`, `f`, `a` are not values.
        assert!(!got.iter().any(|(_, n)| n == "h" || n == "f" || n == "a"));
    }

    /// Near miss: a member read through a variable (`value.Value`) is a
    /// property far more often than a method group, so it is not one.
    #[test]
    fn member_reads_through_a_variable_are_not_method_groups() {
        let f = extract_refs(
            "class P { void Run(J value, K x) { Use(value.Value); var n = x.Name; Use(this.H); } }",
        );
        let got: Vec<&str> = f
            .references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(got, vec!["H"]);
    }

    #[test]
    fn method_groups_need_reference_sites() {
        let f = extract("class P { void Run(int[] xs) { xs.Select(Double); } }");
        assert!(f.references.iter().all(|r| !r.value_ref));
    }

    #[test]
    fn method_qualified_name_with_namespace_and_class() {
        let src = r#"
namespace App.Core {
    class Service {
        public void Run() {}
        void Helper() {}
    }
}
"#;
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"App.Core.Service.Run"), "got: {names:?}");
        assert!(names.contains(&"App.Core.Service.Helper"), "got: {names:?}");
    }

    #[test]
    fn constructor_and_destructor_variants() {
        let src = r#"
class Foo {
    public Foo() {}
    ~Foo() {}
}
"#;
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(by["Foo"], DefVariant::Constructor);
        assert_eq!(by["~Foo"], DefVariant::Destructor);
    }

    #[test]
    fn using_directive_captured() {
        let src =
            "using System; using Sys = System; using static System.Math;\nclass C {}\n";
        let f = extract(src);
        let usings: Vec<(String, String, String)> = f
            .imports
            .iter()
            .map(|i| (i.kind.clone(), i.path.clone(), i.alias.clone()))
            .collect();
        assert!(usings.contains(&("using".into(), "System".into(), "".into())));
        assert!(usings.contains(&("using".into(), "System".into(), "Sys".into())));
        assert!(usings.contains(&(
            "using-static".into(),
            "System.Math".into(),
            "".into()
        )));
    }

    #[test]
    fn invocation_references_extracted() {
        let src = r#"
using System;
class Service {
    public void Run() {
        Helper();
        Console.WriteLine("hi");
    }
    void Helper() {}
}
"#;
        let f = extract(src);
        let refs: Vec<&RefRecord> = f.references.iter().collect();
        assert!(
            refs.iter()
                .any(|r| r.name == "Helper" && r.receiver_hint.is_empty()),
            "no bare Helper call: {refs:?}"
        );
        assert!(
            refs.iter()
                .any(|r| r.name == "WriteLine" && r.receiver_hint == "Console"),
            "no Console.WriteLine ref: {refs:?}"
        );
    }

    #[test]
    fn nested_namespaces_join_dots() {
        let src = r#"
namespace A.B.C {
    class T { void M() {} }
}
"#;
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        // `A.B.C` is a qualified_name node; our extractor uses the
        // text of the name field which includes the dots already.
        assert!(names.iter().any(|n| n.ends_with(".T.M")), "got: {names:?}");
        assert!(names.iter().any(|n| n.contains("A.B.C")), "got: {names:?}");
    }

    #[test]
    fn declared_namespaces_are_recorded_in_full() {
        let f = extract(
            "namespace A.B { namespace C { class T { void M() {} } } }\n\
             namespace D { }\n",
        );
        assert_eq!(f.packages, vec!["A.B", "A.B.C", "D"]);
        let scoped =
            extract("namespace Acme.Web;\npublic class App { public void Run() {} }\n");
        assert_eq!(scoped.packages, vec!["Acme.Web"]);
    }

    fn vis_of(f: &FileFacts, qn_suffix: &str) -> cgg_core::Vis {
        f.definitions
            .iter()
            .find(|d| d.qualified_name.ends_with(qn_suffix))
            .unwrap_or_else(|| panic!("{qn_suffix} not extracted"))
            .vis
    }

    #[test]
    fn an_attribute_argument_does_not_hide_the_access_modifier() {
        // FINDINGS B1: the signature was cut at the first `(`, which here
        // is the attribute's, so every modifier after it was lost and the
        // method read as private — corroborated dead code.
        use cgg_core::Vis;
        let f = extract(
            "class X {\n\
             \x20 [Obsolete(\"x\")]\n  public void Pub() {}\n\
             \x20 [Obsolete(\"x\", false)] protected void Prot() {}\n\
             \x20 [Obsolete(\"x\")] internal void Intl() {}\n\
             \x20 [Obsolete(\"x\")] private void Priv() {}\n\
             \x20 [Obsolete(\"x\")] void Dflt() {}\n\
             }\n",
        );
        assert_eq!(vis_of(&f, ".Pub"), Vis::Public);
        assert_eq!(vis_of(&f, ".Prot"), Vis::Protected);
        assert_eq!(vis_of(&f, ".Intl"), Vis::Internal);
        // Near-misses: an explicit `private`, or no modifier at all,
        // behind the same attribute must not change.
        assert_eq!(vis_of(&f, ".Priv"), Vis::Private);
        assert_eq!(vis_of(&f, ".Dflt"), Vis::Private);
    }

    #[test]
    fn modifier_words_outside_modifier_nodes_are_not_modifiers() {
        use cgg_core::Vis;
        let f = extract(
            "class X {\n  void A(string publicId) {}\n  [Named(\"public\")] void B() {}\n}\n",
        );
        assert_eq!(vis_of(&f, ".A"), Vis::Private);
        assert_eq!(vis_of(&f, ".B"), Vis::Private);
    }

    #[test]
    fn combined_access_modifiers_reach_subtypes() {
        use cgg_core::Vis;
        let f = extract(
            "class X {\n  protected internal void A() {}\n  private protected void B() {}\n}\n",
        );
        assert_eq!(vis_of(&f, ".A"), Vis::Protected);
        assert_eq!(vis_of(&f, ".B"), Vis::Protected);
    }

    #[test]
    fn interface_members_without_a_modifier_are_public() {
        use cgg_core::Vis;
        let f = extract(
            "public interface IRepo {\n\
             \x20 void FindAll();\n\
             \x20 [Obsolete(\"x\")] void Annotated();\n\
             \x20 void WithBody() {}\n\
             \x20 private void Helper() {}\n\
             \x20 protected void Hook() {}\n\
             \x20 int Count { get; }\n\
             \x20 class Nested { void Inner() {} }\n\
             }\n",
        );
        assert_eq!(vis_of(&f, "IRepo.FindAll"), Vis::Public);
        assert_eq!(vis_of(&f, "IRepo.Annotated"), Vis::Public);
        assert_eq!(vis_of(&f, "IRepo.WithBody"), Vis::Public);
        assert_eq!(vis_of(&f, "IRepo.get"), Vis::Public);
        // An explicit modifier still wins in an interface.
        assert_eq!(vis_of(&f, "IRepo.Helper"), Vis::Private);
        assert_eq!(vis_of(&f, "IRepo.Hook"), Vis::Protected);
        // Near-miss: a type nested in the interface has class rules.
        assert_eq!(vis_of(&f, "Nested.Inner"), Vis::Private);
    }

    #[test]
    fn class_struct_and_record_members_default_to_private() {
        use cgg_core::Vis;
        let f = extract(
            "class C { void M() {} }\n\
             struct S { void M() {} }\n\
             record R(int X) { void M() {} }\n\
             class Impl : IRepo { void IRepo.FindAll() {} }\n",
        );
        assert_eq!(vis_of(&f, "C.M"), Vis::Private);
        assert_eq!(vis_of(&f, "S.M"), Vis::Private);
        assert_eq!(vis_of(&f, "R.M"), Vis::Private);
        // An explicit interface implementation carries no modifier and
        // keeps its old reading here; dispatch to it is FINDINGS B2.
        assert_eq!(vis_of(&f, "Impl.FindAll"), Vis::Private);
    }

    #[test]
    fn an_accessor_takes_its_property_accessibility_unless_it_has_its_own() {
        use cgg_core::Vis;
        let f = extract(
            "class C {\n  public int P { get; private set; }\n  int Q { get; set; }\n}\n",
        );
        let acc = |name: &str, line: u32| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == name && d.start_line == line)
                .unwrap_or_else(|| panic!("{name}@{line}"))
                .vis
        };
        assert_eq!(acc("get", 2), Vis::Public);
        assert_eq!(acc("set", 2), Vis::Private);
        assert_eq!(acc("get", 3), Vis::Private);
    }

    #[test]
    fn base_receiver_reads_as_super_and_overrides_are_marked() {
        let f = extract(
            "namespace N {\n  class W : Ext, IThing {\n    protected override void Paint() { base.Paint(); this.Paint(); }\n    void IThing.Tick() { }\n    void Plain() { }\n  }\n}\n",
        );
        let attrs = |name: &str| {
            f.definitions
                .iter()
                .find(|d| d.simple_name == name)
                .map(|d| d.attributes.clone())
                .unwrap_or_default()
        };
        assert!(attrs("Paint").iter().any(|a| a == "override"));
        assert!(attrs("Tick").iter().any(|a| a == "override"));
        assert!(!attrs("Plain").iter().any(|a| a == "override"));
        let recv: Vec<&str> = f
            .references
            .iter()
            .filter(|r| r.name == "Paint")
            .map(|r| r.receiver_hint.as_str())
            .collect();
        assert_eq!(recv, vec!["super", "this"]);
        assert_eq!(f.classes.len(), 1);
        assert_eq!(f.classes[0].class_qn, "N.W");
        assert_eq!(f.classes[0].base_types, vec!["Ext", "IThing"]);
    }

    /// B5/C1: `a.M<T>()` is a call of `M` (it kept the name `M<string>`
    /// and was screened as external), and `x?.M()` is a call of `M`
    /// through `x` (it was dropped).
    #[test]
    fn generic_member_and_null_conditional_calls() {
        let f = extract(
            "class C {\n  void Caller(B b) {\n    b.M<string>(\"a\");\n    B.S<int>(1);\n    G<int>(2);\n    b?.Plain();\n    b?.M<int>();\n    b?.Child.Run();\n    this.x?.Go();\n  }\n}\n",
        );
        let calls: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        for want in [
            ("M", "b"),
            ("S", "B"),
            ("G", ""),
            ("Plain", "b"),
            ("Run", "b.Child"),
            ("Go", "this.x"),
        ] {
            assert!(calls.contains(&want), "{want:?} missing from {calls:?}");
        }
        assert_eq!(calls.iter().filter(|c| **c == ("M", "b")).count(), 2);
        assert!(!calls.iter().any(|(n, _)| n.contains('<')), "{calls:?}");
    }
}
