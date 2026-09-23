//! Python plugin.
//!
//! Two-phase AST pass over a `tree-sitter-python` tree:
//!
//! * **Definitions** — `function_definition` (including async),
//!   `class_definition` methods, decorators treated as attribute
//!   annotations, and named lambdas bound via `assignment`
//!   (`foo = lambda x: x+1`).
//! * **References** — every `call` whose function is an identifier or
//!   an `attribute`.
//! * **Imports** — `import_statement` and `import_from_statement`,
//!   flattened with alias support.
//! * **Instance-field types** — `self.x = Foo(...)` and class
//!   annotations (`x: Foo`) become `self.x` LocalTypes on every method
//!   of that class, so the type propagator can rewrite `self.x.m()`.
//!
//! Module name for qualified names is derived from the file stem for
//! Task 4. Task 6 will refine this to the full dotted package path by
//! consulting `__init__.py` chains via stack-graphs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use cgg_core::{
    DefRecord, DefVariant, FileFacts, ImportRecord, LocalType, RefRecord, ids::FileId,
};
use tree_sitter::{Node, Tree};

use crate::LanguagePlugin;

#[derive(Debug)]
pub struct PythonPlugin;

impl LanguagePlugin for PythonPlugin {
    fn id(&self) -> &'static str {
        "python"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".py", ".pyi", ".ipynb"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["python3", "python", "python2"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            attributes: true,
            dyn_uses: true,
            exports: true,
            impls: true,
            test_defs: true,
            unreachable: true,
            value_refs: true,
            visibility: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "python");
        let mut walker = Walker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: vec![module_name(path)],
            scope_is_class: vec![false],
            bases: Vec::new(),
            class_field_types: Vec::new(),
            class_field_annotated: Vec::new(),
            function_local_classes: BTreeSet::new(),
        };
        walker.walk(tree.root_node());
        let local_classes = std::mem::take(&mut walker.function_local_classes);
        let mut out = facts;
        // A class defined inside a function whose name is defined more
        // than once in the file names no single type: two tests that each
        // define a local `_CrawlSpider` share an owner name, so typing
        // `spider = _CrawlSpider()` bound one test's call to the other
        // test's class. Those receivers stay untyped, as before. A
        // function-local class with a unique name is typed normally.
        if !local_classes.is_empty() {
            let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
            for c in &out.classes {
                let n = c.class_qn.rsplit('.').next().unwrap_or(&c.class_qn);
                *seen.entry(n).or_default() += 1;
            }
            let ambiguous: BTreeSet<String> = local_classes
                .into_iter()
                .filter(|n| seen.get(n.as_str()).copied().unwrap_or(0) > 1)
                .collect();
            if !ambiguous.is_empty() {
                out.local_types
                    .retain(|t| !ambiguous.contains(&t.type_name));
            }
        }
        if ctx.deadcode_signals {
            out.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::PYTHON);
        }
        if ctx.deadcode_signals {
            out.dyn_uses = super::dynuse::extract(tree, source, "python");
        }
        // `__all__` is Python's explicit export list; a name in it is
        // public API even when nothing in the package references it.
        out.exports = py_dunder_all(tree, source);
        out
    }
}

/// Derive a module name from the file path.
///
/// Walks the path's parents looking for directories that contain
/// `__init__.py`; those become module name segments, giving the full
/// dotted path (`pkg.sub.module`). If no package markers are found,
/// falls back to the file stem.
fn module_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");
    let mut parts: Vec<String> = vec![stem.to_string()];
    let mut dir = path.parent();
    while let Some(d) = dir {
        let init_py = d.join("__init__.py");
        if init_py.exists()
            && let Some(name) = d.file_name().and_then(|s| s.to_str())
        {
            parts.push(name.to_string());
            dir = d.parent();
            continue;
        }
        break;
    }
    parts.reverse();
    parts.join(".")
}

struct Walker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Parallel to `scope`: whether each entry is a `class` body. A
    /// `def` is a method only when its *immediate* scope is a class; a
    /// function nested inside a method is a plain function.
    scope_is_class: Vec<bool>,
    /// Base classes of the enclosing `class`, innermost last.
    bases: Vec<Vec<String>>,
    /// Instance-attribute types of the enclosing `class`, innermost last.
    /// `self.controller = Controller(...)` and `controller: Controller`
    /// both land here; a post-pass on the class copies them onto every
    /// method as `self.<field>` LocalTypes so the type propagator can
    /// rewrite `self.controller.open()`.
    class_field_types: Vec<BTreeMap<String, String>>,
    /// Fields of the enclosing `class` whose type came from a class-level
    /// annotation. The declared type wins over any assignment.
    class_field_annotated: Vec<BTreeSet<String>>,
    /// Names of classes defined inside a function body anywhere in the
    /// file. Their names are not unique across the file, so they are
    /// never used as a receiver type.
    function_local_classes: BTreeSet<String>,
}

impl<'a> Walker<'a> {
    fn text(&self, node: Node) -> &str {
        node.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // Checked before the push: is any enclosing scope a function?
                if !name.is_empty() && self.scope_is_class.iter().skip(1).any(|c| !c) {
                    self.function_local_classes.insert(name.clone());
                }
                if !name.is_empty() {
                    self.scope.push(name);
                    self.scope_is_class.push(true);
                }
                // `class Encoder(nn.Module)` is the only thing that says
                // the runtime calls `forward`; nothing else in the file
                // does.
                let bases = super::attrs::base_types(node, self.source);
                // Recorded for every class, methods or not, so an
                // inheritance chain can pass through
                // `class Middle(Base): pass`.
                self.facts.classes.push(cgg_core::ClassDecl {
                    class_qn: self.scope.join("."),
                    base_types: bases.clone(),
                    line: node.start_position().row as u32 + 1,
                });
                self.bases.push(bases);
                collect_class_fields(node, self.source, &self.scope, self.facts);
                self.class_field_types.push(BTreeMap::new());
                if let Some(fields) = self.class_field_types.last_mut() {
                    scan_class_annotations(node, self.source, fields);
                }
                self.class_field_annotated.push(
                    self.class_field_types
                        .last()
                        .map(|m| m.keys().cloned().collect())
                        .unwrap_or_default(),
                );
                self.walk_children(node);
                self.emit_instance_field_types();
                self.class_field_types.pop();
                self.class_field_annotated.pop();
                self.bases.pop();
                if node.child_by_field_name("name").is_some() {
                    self.scope.pop();
                    self.scope_is_class.pop();
                }
                return;
            }
            "function_definition" => {
                self.record_function(node);
                // Push the function name to enable nested-function qualified names.
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name.clone());
                    self.scope_is_class.push(false);
                }
                self.walk_children(node);
                if !name.is_empty() {
                    self.scope.pop();
                    self.scope_is_class.pop();
                }
                return;
            }
            "expression_statement" => {
                // Named lambdas: `foo = lambda x: x + 1`
                if let Some(rec) = self.named_lambda(node) {
                    self.facts.definitions.push(rec);
                }
                // Constructor inference: `x = Foo(...)`
                self.infer_assignment_type(node);
                self.capture_assignment_refs(node);
                self.walk_children(node);
                return;
            }
            "import_statement" | "import_from_statement" => {
                self.record_import(node);
                return;
            }
            "call" => {
                let context = match self.ref_from_call(node) {
                    Some(r) => {
                        // Django's `urls.py` is ordinary Python:
                        // `path("users/", views.list_users)` puts the
                        // handler in argument position, so the callee
                        // alone says nothing.
                        let context = if r.receiver_hint.is_empty() {
                            r.name.clone()
                        } else {
                            format!("{}.{}", r.receiver_hint, r.name)
                        };
                        self.facts.references.push(r);
                        let extra = super::registrar::capture(
                            &self.ctx,
                            node,
                            self.source,
                            &context,
                        );
                        self.facts.references.extend(extra);
                        context
                    }
                    // The callee is not a plain name —
                    // `ConfigAttribute[timedelta](...)` is a subscript,
                    // and a call on any other expression lands here too.
                    // Nothing names the *callee*, but the arguments are
                    // still references, and flask's two `_make_timedelta`
                    // definitions were reported dead on exactly this
                    // shape. An empty context matches no registrar verb,
                    // which is the right answer for an unnameable callee.
                    None => String::new(),
                };
                // A callable handed to an *ordinary* call — click's
                // `callback=`, SQLAlchemy's `event.listen`,
                // `staticmethod(...)` — is a real reference that no
                // framework rule covers. Without it the target reads as
                // never-referenced.
                let vals = super::registrar::capture_value_refs(
                    &self.ctx,
                    node,
                    self.source,
                    &context,
                );
                self.facts.references.extend(vals);
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

    /// Capture callables named on the right-hand side of an assignment.
    ///
    /// `DAEMONIZED_TASKS = {'check_status': _check_status}` and
    /// `__setitem__ = __delitem__ = _fail` are references, but no call
    /// encloses them, so the argument-slot pass never sees them.
    ///
    /// Only literal containers and bare names are followed. A call on
    /// the right (`x = compute()`) is left alone: the walker visits it
    /// on its own turn, and descending here as well would emit the
    /// callee twice.
    fn capture_assignment_refs(&mut self, node: Node) {
        let mut cursor = node.walk();
        let assignments: Vec<Node> = node
            .named_children(&mut cursor)
            .filter(|c| matches!(c.kind(), "assignment" | "augmented_assignment"))
            .collect();

        for a in assignments {
            let Some(rhs) = a.child_by_field_name("right") else {
                continue;
            };
            if !matches!(
                rhs.kind(),
                "dictionary"
                    | "list"
                    | "tuple"
                    | "set"
                    | "identifier"
                    | "attribute"
                    // `a = b = _fail` nests the second assignment on
                    // the right of the first.
                    | "assignment"
                    // `handler = x or _default_request` and
                    // `handler = a if c else b` name a callable without
                    // a call or a literal container.
                    | "boolean_operator"
                    | "conditional_expression"
                    | "parenthesized_expression"
            ) {
                continue;
            }
            let refs =
                super::registrar::capture_value_position(&self.ctx, rhs, self.source, "");
            self.facts.references.extend(refs);
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

        let is_async = node.children(&mut node.walk()).any(|c| c.kind() == "async");

        let decorators = collect_decorators(node, self.source);

        // Classify variant:
        //   * inside `class_definition`: method.
        //   * decorator @staticmethod / @classmethod / @property refine it.
        //   * `__init__` -> Constructor; `__del__` -> Destructor.
        // The immediate scope decides. Asking whether *any* enclosing
        // scope is a class made `def _scan()` nested inside a method a
        // method too, and a bare `_scan()` call then never bound to it.
        let inside_class = self.scope_is_class.last().copied().unwrap_or(false);
        let variant = if simple == "__init__" && inside_class {
            DefVariant::Constructor
        } else if simple == "__del__" && inside_class {
            DefVariant::Destructor
        } else if decorators.iter().any(|d| d.contains("staticmethod")) {
            DefVariant::StaticMethod
        } else if decorators.iter().any(|d| d.contains("classmethod")) {
            DefVariant::ClassMethod
        } else if decorators.iter().any(|d| d.contains("property")) {
            DefVariant::Property
        } else if is_async {
            DefVariant::AsyncFunction
        } else if inside_class {
            DefVariant::InherentMethod
        } else {
            DefVariant::FreeFunction
        };

        let qn = qualified_name(&self.scope, &simple);
        let (sl, el) = line_range(node);
        let simple_for_vis = simple.clone();

        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            vis: py_vis(&simple_for_vis),
            test_role: py_test_role(&simple_for_vis, &decorators),
            attributes: decorators,
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn infer_assignment_type(&mut self, node: Node) {
        // `x = Foo(...)` where Foo starts with uppercase -> x has type Foo
        let assign = node.named_child(0);
        let Some(assign) = assign else { return };
        if assign.kind() != "assignment" {
            return;
        }
        let left = assign.child_by_field_name("left");
        let right = assign.child_by_field_name("right");
        let (Some(left), Some(right)) = (left, right) else {
            return;
        };
        if right.kind() != "call" {
            return;
        }
        let func = right.child_by_field_name("function");
        let Some(func) = func else { return };
        let Some(type_name) = constructor_type(func, self.source) else {
            return;
        };

        if left.kind() == "identifier" {
            let var_name = self.text(left).to_string();
            if var_name.is_empty() {
                return;
            }
            self.facts.local_types.push(LocalType {
                var_name,
                type_name,
                scope_byte: node.start_byte() as u32,
            });
            return;
        }

        // `self.controller = Controller(...)` — instance field. Recorded
        // on the enclosing class and copied onto every method at class
        // exit; the assignment site itself is not a useful scope.
        if left.kind() == "attribute" {
            let Some(obj) = left.child_by_field_name("object") else {
                return;
            };
            let Some(attr) = left.child_by_field_name("attribute") else {
                return;
            };
            if self.text(obj) != "self" {
                return;
            }
            let field = self.text(attr).to_string();
            if field.is_empty() {
                return;
            }
            // A field assigned two different types (`self.game = Game()`
            // here, `self.game = BaseGame()` there) has no single type.
            // Last-write-wins sent every call to whichever assignment
            // came last, so it is left untyped instead (an empty name,
            // skipped at emit). A class-level annotation is the declared
            // type and is never overridden.
            let annotated = self
                .class_field_annotated
                .last()
                .is_some_and(|a| a.contains(&field));
            if let Some(fields) = self.class_field_types.last_mut()
                && !annotated
            {
                match fields.get(&field) {
                    None => {
                        fields.insert(field, type_name);
                    }
                    Some(t) if *t == type_name => {}
                    Some(_) => {
                        fields.insert(field, String::new());
                    }
                }
            }
        }
    }

    /// Copy the enclosing class's instance-field types onto every method
    /// of that class, keyed by the method's start byte so `self.x.m()`
    /// inside `connect` sees a type assigned in `__init__`.
    fn emit_instance_field_types(&mut self) {
        let Some(fields) = self.class_field_types.last() else {
            return;
        };
        if fields.is_empty() {
            return;
        }
        // Full class QN (`mod.Other.App`), not the bare last segment —
        // a nested `class App` must not inherit (or donate) field types
        // from a module-level `class App` in the same file.
        let class_qn = self.scope.join(".");
        if class_qn.is_empty() {
            return;
        }
        let fields: Vec<(String, String)> = fields
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let starts: Vec<u32> = self
            .facts
            .definitions
            .iter()
            .filter(|d| {
                d.qualified_name
                    .rsplit_once('.')
                    .is_some_and(|(owner, _)| owner == class_qn)
            })
            .map(|d| d.start_byte)
            .collect();
        for start in starts {
            for (fname, ty) in &fields {
                self.facts.local_types.push(LocalType {
                    var_name: format!("self.{fname}"),
                    type_name: ty.clone(),
                    scope_byte: start,
                });
            }
        }
    }

    fn named_lambda(&mut self, node: Node) -> Option<DefRecord> {
        // expression_statement -> assignment (left, right=lambda)
        let assignment = node.named_child(0)?;
        if assignment.kind() != "assignment" {
            return None;
        }
        let left = assignment.child_by_field_name("left")?;
        if left.kind() != "identifier" {
            return None;
        }
        let right = assignment.child_by_field_name("right")?;
        if right.kind() != "lambda" {
            return None;
        }
        let simple = self.text(left).to_string();
        if simple.is_empty() {
            return None;
        }
        let qn = qualified_name(&self.scope, &simple);
        let (sl, el) = line_range(node);
        let vis = py_vis(&simple);
        Some(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant: DefVariant::NamedLambda,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            vis,
            attributes: Vec::new(),
            ..Default::default()
        })
    }

    fn record_import(&mut self, node: Node) {
        let text = self.text(node).trim().to_string();
        let (kind, path, alias) = parse_import(&text);
        let site_line = (node.start_position().row as u32) + 1;
        self.facts.imports.push(ImportRecord {
            kind,
            path,
            alias,
            site_line,
            site_byte: node.start_byte() as u32,
        });
    }

    fn ref_from_call(&mut self, node: Node) -> Option<RefRecord> {
        let func = node.child_by_field_name("function")?;
        let (name, receiver) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "attribute" => {
                // a.b.c  -> name=c, receiver=a.b
                let attr = func.child_by_field_name("attribute")?;
                let recv = func
                    .child_by_field_name("object")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (self.text(attr).to_string(), recv)
            }
            _ => return None,
        };
        if name.is_empty() {
            return None;
        }
        let site_line = (node.start_position().row as u32) + 1;
        // Keyword names at the call site. They are the cheapest evidence
        // there is for narrowing duck-typed dispatch: a call passing
        // `data=`/`context=` cannot be reaching a method that takes
        // `w, x, y, z`, and cgg was emitting an edge to every one of them.
        let mut kwargs: Vec<String> = Vec::new();
        if let Some(args) = node.child_by_field_name("arguments") {
            let mut c = args.walk();
            for a in args.named_children(&mut c) {
                if a.kind() == "keyword_argument"
                    && let Some(n) = a.child_by_field_name("name")
                {
                    kwargs.push(self.text(n).to_string());
                }
            }
        }
        Some(RefRecord {
            name,
            receiver_hint: receiver,
            site_line,
            site_byte: node.start_byte() as u32,
            kwargs,
            ..Default::default()
        })
    }
}

fn parse_import(text: &str) -> (String, String, String) {
    // `import a.b as c` | `import a, b` | `from x import y as z`
    if let Some(rest) = text.strip_prefix("from ")
        && let Some((module, items)) = rest.split_once(" import ")
    {
        // The module goes under `path`; the items list, as one blob the
        // resolver splits on commas, under `alias`. A parenthesised list —
        // `from m import (a, b as c)`, and its multi-line form with a
        // trailing comma — must lose its parentheses and line breaks here,
        // or the first item reaches the resolver as `(a` and the last as
        // `b)` and neither ever binds. Every call through such a name then
        // fell to the same-name fan-out, and past `--fanout-cap` to
        // nothing, which reported live functions as unreferenced.
        let items = items
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .replace('\\', " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let items = items.trim().trim_end_matches(',').trim().to_string();
        return ("from-import".into(), module.trim().to_string(), items);
    }
    if let Some(rest) = text.strip_prefix("import ") {
        let r = rest.trim();
        if let Some((lhs, alias)) = r.split_once(" as ") {
            return (
                "import".into(),
                lhs.trim().to_string(),
                alias.trim().to_string(),
            );
        }
        return ("import".into(), r.to_string(), String::new());
    }
    ("import".into(), text.to_string(), String::new())
}

fn qualified_name(scope: &[String], simple: &str) -> String {
    let mut parts: Vec<&str> = scope.iter().map(|s| s.as_str()).collect();
    parts.push(simple);
    parts.join(".")
}

fn line_range(node: Node) -> (u32, u32) {
    let start = (node.start_position().row as u32) + 1;
    let end = (node.end_position().row as u32) + 1;
    (start, end)
}

fn collect_decorators(node: Node, source: &[u8]) -> Vec<String> {
    // In tree-sitter-python a decorated function lives inside
    // `decorated_definition`, where previous siblings are `decorator`
    // nodes. The `function_definition` child stands on its own here;
    // we check the parent.
    let mut out = Vec::new();
    let mut parent = node.parent();
    if let Some(p) = parent
        && p.kind() == "decorated_definition"
    {
        let mut c = p.walk();
        for child in p.children(&mut c) {
            if child.kind() == "decorator" {
                out.push(child.utf8_text(source).unwrap_or("").trim().to_string());
            }
        }
        return out;
    }
    parent = node.prev_sibling();
    while let Some(s) = parent {
        if s.kind() == "decorator" {
            out.push(s.utf8_text(source).unwrap_or("").trim().to_string());
            parent = s.prev_sibling();
        } else {
            break;
        }
    }
    out.reverse();
    out
}

fn starts_uppercase(s: &str) -> bool {
    cgg_core::looks_like_type_name(s)
}

fn constructor_type(func: Node, source: &[u8]) -> Option<String> {
    let name = match func.kind() {
        "identifier" => func.utf8_text(source).ok()?.to_string(),
        "attribute" => func
            .child_by_field_name("attribute")
            .and_then(|a| a.utf8_text(source).ok())
            .map(|s| s.to_string())?,
        _ => return None,
    };
    if starts_uppercase(&name) {
        Some(name)
    } else {
        None
    }
}

/// Type-level wrappers whose first argument *is* the instance type.
/// A container (`list[Foo]`, `dict[str, Foo]`, `Sequence[Foo]`) is not:
/// `self.items.clear()` on a `list[Foo]` runs `list.clear`, and typing
/// the field as `Foo` would bind it to `Foo.clear` if one exists.
const TRANSPARENT_WRAPPERS: &[&str] = &[
    "Optional",
    "Union",
    "ClassVar",
    "Final",
    "Annotated",
    "InitVar",
];

fn annotation_type_stem(raw: &str) -> Option<String> {
    let mut t = raw.trim().trim_matches(|c| c == '"' || c == '\'');
    if t.is_empty() {
        return None;
    }
    // `Optional[Foo]` — the argument is the instance type. `list[Foo]`
    // is a list; a subscripted name that is not a transparent wrapper
    // is a container, and the field's type is the container.
    if let Some(open) = t.find('[')
        && let Some(close) = t.rfind(']')
        && close > open + 1
    {
        let head = t[..open].trim().rsplit('.').next().unwrap_or("");
        if !TRANSPARENT_WRAPPERS.contains(&head) {
            return None;
        }
        t = t[open + 1..close].trim();
    }
    // `Foo | None` is `Foo`; `Foo | Bar` names two types and is
    // neither, so it is left untyped rather than guessed.
    let mut members = t
        .split(['|', ','])
        .map(|m| m.trim().trim_matches(|c| c == '"' || c == '\''))
        .filter(|m| !m.is_empty() && *m != "None");
    let first = members.next()?;
    if members.next().is_some() {
        return None;
    }
    let stem = first.rsplit('.').next().unwrap_or(first).trim();
    if starts_uppercase(stem) {
        Some(stem.to_string())
    } else {
        None
    }
}

/// Scan the class body for `name: Type` annotations and record them as
/// instance-field types.  This is the standalone class-body scan that
/// does not require `ClassFieldDecl` infrastructure.
fn scan_class_annotations(
    class: tree_sitter::Node,
    source: &[u8],
    fields: &mut BTreeMap<String, String>,
) {
    let Some(body) = class.child_by_field_name("body") else {
        return;
    };
    let mut c = body.walk();
    for child in body.named_children(&mut c) {
        let stmt = if child.kind() == "expression_statement" {
            child.named_child(0)
        } else {
            Some(child)
        };
        let Some(stmt) = stmt else {
            continue;
        };
        if stmt.kind() != "assignment" {
            continue;
        }
        record_annotated_instance_field(stmt, source, fields);
    }
}

fn record_annotated_instance_field(
    node: tree_sitter::Node,
    source: &[u8],
    fields: &mut BTreeMap<String, String>,
) {
    let left = node
        .child_by_field_name("left")
        .or_else(|| node.named_child(0));
    let Some(left) = left else {
        return;
    };
    if left.kind() != "identifier" {
        return;
    }
    let Ok(name) = left.utf8_text(source) else {
        return;
    };
    if name.is_empty() {
        return;
    }
    let Some(ty) = node.child_by_field_name("type") else {
        return;
    };
    let Ok(raw) = ty.utf8_text(source) else {
        return;
    };
    let Some(stem) = annotation_type_stem(raw) else {
        return;
    };
    fields.insert(name.to_string(), stem);
}

/// Generic class-level `name = Type(...)` extraction.
///
/// Records every assignment at class body level whose RHS is a call.
/// Framework-agnostic: the Python plugin records what it sees, and
/// framework rules interpret it (e.g. Kivy's rule uses these to
/// recognise `on_<field>` methods as property observers).
fn collect_class_fields(
    class: tree_sitter::Node,
    source: &[u8],
    scope: &[String],
    facts: &mut FileFacts,
) {
    let Some(body) = class.child_by_field_name("body") else {
        return;
    };
    let class_qn = scope.join(".");
    let mut c = body.walk();
    for child in body.named_children(&mut c) {
        if child.kind() != "expression_statement" {
            continue;
        }
        let Some(assign) = child.named_child(0) else {
            continue;
        };
        if assign.kind() != "assignment" {
            continue;
        }
        let Some(left) = assign.child_by_field_name("left") else {
            continue;
        };
        if left.kind() != "identifier" {
            continue;
        }
        let Some(right) = assign.child_by_field_name("right") else {
            continue;
        };
        if right.kind() != "call" {
            continue;
        }
        let Some(func) = right.child_by_field_name("function") else {
            continue;
        };
        let type_name = match func.kind() {
            "identifier" => func.utf8_text(source).unwrap_or(""),
            "attribute" => func
                .child_by_field_name("attribute")
                .and_then(|a| a.utf8_text(source).ok())
                .unwrap_or(""),
            _ => "",
        };
        if type_name.is_empty() {
            continue;
        }
        let Ok(field_name) = left.utf8_text(source) else {
            continue;
        };
        if field_name.is_empty() {
            continue;
        }
        facts.class_fields.push(cgg_core::ClassFieldDecl {
            class_qn: class_qn.clone(),
            field_name: field_name.to_string(),
            type_name: type_name.to_string(),
            line: (assign.start_position().row as u32) + 1,
        });
    }
}

/// Python has no visibility keyword; the underscore convention is the
/// language's actual, universally-followed rule.
fn py_vis(simple: &str) -> cgg_core::Vis {
    if simple.starts_with("__") && simple.ends_with("__") {
        cgg_core::Vis::Public // dunder: part of the protocol surface
    } else if simple.starts_with('_') {
        cgg_core::Vis::Private
    } else {
        cgg_core::Vis::Public
    }
}

/// pytest / unittest lifecycle hook names.
const PY_FIXTURES: &[&str] = &[
    "setUp",
    "tearDown",
    "setUpClass",
    "tearDownClass",
    "setup_module",
    "teardown_module",
    "setup_function",
    "teardown_function",
    "setup_class",
    "teardown_class",
    "setup_method",
    "teardown_method",
];

/// Decide a Python definition's test role.
///
/// Decorator evidence applies everywhere — `@pytest.fixture` is
/// unambiguous wherever it appears. Name evidence is weaker, so it is a
/// separate, softer signal.
fn py_test_role(simple: &str, decorators: &[String]) -> Option<cgg_core::TestRole> {
    for d in decorators {
        let k = d.trim().trim_start_matches('@');
        let k = k.split('(').next().unwrap_or(k).trim();
        if k == "pytest.fixture" || k == "fixture" {
            return Some(cgg_core::TestRole::Fixture);
        }
        if k.starts_with("pytest.mark") {
            return Some(cgg_core::TestRole::Case);
        }
    }
    if PY_FIXTURES.contains(&simple) {
        return Some(cgg_core::TestRole::Fixture);
    }
    if simple.starts_with("test_") {
        return Some(cgg_core::TestRole::Case);
    }
    None
}

/// Names listed in a module's `__all__`.
///
/// Only the literal list/tuple form is read. `__all__ += [...]` and
/// `__all__.extend(...)` are deliberately out of scope: following them
/// means evaluating the module, and a wrong answer here would silently
/// mark real findings as exported.
fn py_dunder_all(tree: &tree_sitter::Tree, source: &[u8]) -> Vec<cgg_core::ExportRecord> {
    let text = |n: tree_sitter::Node| -> String {
        String::from_utf8_lossy(&source[n.byte_range()]).to_string()
    };
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
        if n.kind() != "assignment" {
            continue;
        }
        let Some(lhs) = n.child_by_field_name("left") else {
            continue;
        };
        if text(lhs).trim() != "__all__" {
            continue;
        }
        let Some(rhs) = n.child_by_field_name("right") else {
            continue;
        };
        let mut rc = rhs.walk();
        for e in rhs.children(&mut rc) {
            if !e.kind().contains("string") {
                continue;
            }
            let name = text(e).trim().trim_matches(['"', '\'']).to_string();
            if !name.is_empty() {
                out.push(cgg_core::ExportRecord {
                    name,
                    kind: "__all__".into(),
                    target: String::new(),
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract_with(path: &str, src: &str) -> FileFacts {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(src, None).unwrap();
        PythonPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from(path),
            &tree,
            src.as_bytes(),
        )
    }

    fn extract(src: &str) -> FileFacts {
        extract_with("m.py", src)
    }

    /// Names captured as value references (a callable passed as a
    /// value), not as calls.
    fn value_refs(f: &FileFacts) -> Vec<&str> {
        f.references
            .iter()
            .filter(|r| r.receiver_hint == cgg_core::VALUE_REF_HINT)
            .map(|r| r.name.as_str())
            .collect()
    }

    /// A callable passed to an ordinary, non-registrar call is a real
    /// reference. Before this was captured, every one of these read as
    /// a `High`-confidence dead-code finding.
    #[test]
    fn value_ref_in_ordinary_call_is_captured() {
        let f = extract(
            "def _validate_key(ctx, param, value):\n    return value\n\n\
             click.option('--key', callback=_validate_key)\n",
        );
        assert!(
            value_refs(&f).contains(&"_validate_key"),
            "`callback=_validate_key` is a reference; got {:?}",
            value_refs(&f)
        );
    }

    /// `ConfigAttribute[timedelta](get_converter=_make_timedelta)` —
    /// the callee is a subscript, so nothing names it. The arguments
    /// are still references; flask's two `_make_timedelta` definitions
    /// were reported dead on exactly this shape.
    #[test]
    fn value_ref_survives_an_unnameable_callee() {
        let f = extract(
            "def _make_timedelta(v):\n    return v\n\n\
             x = ConfigAttribute[timedelta]('KEY', get_converter=_make_timedelta)\n",
        );
        assert!(
            value_refs(&f).contains(&"_make_timedelta"),
            "a call on a subscript still has arguments; got {:?}",
            value_refs(&f)
        );
    }

    /// The ungated pass must not manufacture routing claims: strings in
    /// an ordinary call are data, and only the framework rules can tell
    /// `'photos#index'` from a log message.
    #[test]
    fn ordinary_call_strings_are_not_captured() {
        let f = extract("def h():\n    pass\n\nlogger.warning('h')\n");
        let strings: Vec<&str> = f
            .references
            .iter()
            .filter(|r| r.receiver_hint == cgg_core::STRING_REF_HINT)
            .map(|r| r.name.as_str())
            .collect();
        assert!(
            strings.is_empty(),
            "an ordinary call must not emit string refs; got {strings:?}"
        );
    }

    /// A registrar verb goes through `capture`, which emits a superset.
    /// Running both passes would double every record.
    #[test]
    fn registrar_call_value_refs_are_not_duplicated() {
        let f = extract(
            "def list_users(req):\n    pass\n\n\
             app.route('/users', list_users)\n",
        );
        let n = value_refs(&f)
            .iter()
            .filter(|n| **n == "list_users")
            .count();
        assert_eq!(n, 1, "expected exactly one record, got {n}");
    }

    /// A dispatch table names its handlers in a value position that no
    /// call encloses.
    #[test]
    fn value_refs_in_an_assigned_dict_are_captured() {
        let f = extract(
            "def _check_status():\n    pass\n\ndef _fetch_updates():\n    pass\n\n\
             DAEMONIZED_TASKS = {\n    'check_status': _check_status,\n\
             \x20   'fetch_updates': _fetch_updates,\n}\n",
        );
        let v = value_refs(&f);
        assert!(
            v.contains(&"_check_status") && v.contains(&"_fetch_updates"),
            "dispatch-table values are references; got {v:?}"
        );
    }

    /// A call on the right-hand side is visited by the walker on its own
    /// turn. Descending into it here as well would emit the callee twice.
    #[test]
    fn assignment_to_a_call_does_not_double_count() {
        let f = extract("def compute():\n    return 1\n\nx = compute()\n");
        let n = f.references.iter().filter(|r| r.name == "compute").count();
        assert_eq!(n, 1, "expected one record for `compute`, got {n}");
    }

    #[test]
    fn free_functions() {
        let f = extract("def a():\n    b()\n\ndef b():\n    pass\n");
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"m.a"), "got: {names:?}");
        assert!(names.contains(&"m.b"), "got: {names:?}");
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"b"));
    }

    #[test]
    fn class_methods_get_class_in_name() {
        let src = r#"
class Foo:
    def bar(self):
        self.baz()
    def baz(self):
        pass
"#;
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"m.Foo.bar"), "got: {names:?}");
        assert!(names.contains(&"m.Foo.baz"), "got: {names:?}");
    }

    #[test]
    fn static_and_class_methods_are_variants() {
        let src = r#"
class C:
    @staticmethod
    def s(): pass
    @classmethod
    def c(cls): pass
    @property
    def p(self): return 1
"#;
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(by["s"], DefVariant::StaticMethod);
        assert_eq!(by["c"], DefVariant::ClassMethod);
        assert_eq!(by["p"], DefVariant::Property);
    }

    #[test]
    fn named_lambda_is_callable() {
        let f = extract("inc = lambda x: x + 1\ninc(1)\n");
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"m.inc"), "got: {names:?}");
        let defs_by_name: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(defs_by_name["inc"], DefVariant::NamedLambda);
    }

    #[test]
    fn parenthesised_from_import_binds_every_name() {
        // The reporter's exact shape: first name unaliased, the rest
        // aliased, multi-line, trailing comma.
        let (kind, path, items) = parse_import(
            "from client import (\n    create_remediation_request,\n    _create_client as _create_local_client,\n    _get_endpoint as _get_local_endpoint,\n)",
        );
        assert_eq!(kind, "from-import");
        assert_eq!(path, "client");
        assert_eq!(
            items,
            "create_remediation_request, _create_client as _create_local_client, _get_endpoint as _get_local_endpoint"
        );
        let (_, _, one_line) = parse_import("from m import (a, b as c)");
        assert_eq!(one_line, "a, b as c");
        let (_, _, plain) = parse_import("from m import a, b as c");
        assert_eq!(plain, "a, b as c");
    }

    #[test]
    fn imports_parsed() {
        let src = "import a.b\nimport c as d\nfrom x import y as z\n";
        let f = extract(src);
        assert_eq!(f.imports.len(), 3);
        assert_eq!(f.imports[0].kind, "import");
        assert_eq!(f.imports[0].path, "a.b");
        assert_eq!(f.imports[1].path, "c");
        assert_eq!(f.imports[1].alias, "d");
        assert_eq!(f.imports[2].kind, "from-import");
        assert_eq!(f.imports[2].path, "x");
        assert_eq!(f.imports[2].alias, "y as z");
    }

    #[test]
    fn method_calls_captured() {
        let src = "class C:\n    def m(self): self.n()\n    def n(self): pass\n";
        let f = extract(src);
        let refs: Vec<&RefRecord> = f.references.iter().collect();
        assert!(
            refs.iter()
                .any(|r| r.name == "n" && r.receiver_hint == "self"),
            "got: {refs:?}"
        );
    }

    #[test]
    fn init_is_constructor() {
        let src = "class X:\n    def __init__(self): pass\n    def __del__(self): pass\n";
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(by["__init__"], DefVariant::Constructor);
        assert_eq!(by["__del__"], DefVariant::Destructor);
    }

    #[test]
    fn async_function() {
        let f = extract("async def a():\n    pass\n");
        assert_eq!(f.definitions[0].variant, DefVariant::AsyncFunction);
    }

    #[test]
    fn nested_function_qualified_name() {
        let src = "def outer():\n    def inner():\n        pass\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"m.outer.inner"), "got: {names:?}");
    }

    #[test]
    fn class_field_declarations_are_extracted() {
        let f = extract(
            "class Label:\n\
             \x20   text = StringProperty('')\n\
             \x20   size = NumericProperty(0)\n\
             \x20   plain = 42\n\
             \x20   def on_text(self, instance, value):\n\
             \x20       pass\n",
        );
        assert_eq!(
            f.class_fields.len(),
            2,
            "only RHS-is-call assignments: {:?}",
            f.class_fields
        );
        assert!(
            f.class_fields
                .iter()
                .any(|cf| cf.field_name == "text" && cf.type_name == "StringProperty"),
            "text field: {:?}",
            f.class_fields
        );
        assert!(
            f.class_fields
                .iter()
                .any(|cf| cf.field_name == "size" && cf.type_name == "NumericProperty"),
            "size field: {:?}",
            f.class_fields
        );
        assert!(
            f.class_fields
                .iter()
                .all(|cf| cf.class_qn.contains("Label")),
            "class_qn must name the class: {:?}",
            f.class_fields
        );
    }

    #[test]
    fn underscore_prefixed_subclass_records_base_types() {
        let f = extract(
            "class _MarkerHoverToolTip(ToolTipButton):\n    def on_mouse_pos(self, *args):\n        pass\n",
        );
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "on_mouse_pos")
            .expect("on_mouse_pos");
        assert_eq!(d.base_types, ["ToolTipButton"], "bases: {:?}", d.base_types);
        assert_eq!(
            d.variant,
            DefVariant::InherentMethod,
            "a `_Class` method must not be classified as a free function"
        );
    }

    #[test]
    fn self_field_constructor_is_copied_onto_every_method() {
        let f = extract(
            "class App:\n    def __init__(self):\n        self.controller = Controller()\n    def connect(self):\n        self.controller.open()\n",
        );
        let connect = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "connect")
            .expect("connect");
        assert!(
            f.local_types.iter().any(|t| {
                t.var_name == "self.controller"
                    && t.type_name == "Controller"
                    && t.scope_byte == connect.start_byte
            }),
            "connect should see self.controller: Controller; types: {:?}",
            f.local_types
        );
        let open = f
            .references
            .iter()
            .find(|r| r.name == "open")
            .expect("open");
        assert_eq!(open.receiver_hint, "self.controller");
    }

    #[test]
    fn class_annotation_is_an_instance_field_type() {
        let f = extract(
            "class Panel:\n    controller: Controller\n    def go(self):\n        self.controller.open()\n",
        );
        let go = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "go")
            .expect("go");
        assert!(
            f.local_types.iter().any(|t| {
                t.var_name == "self.controller"
                    && t.type_name == "Controller"
                    && t.scope_byte == go.start_byte
            }),
            "annotation missed; types: {:?}",
            f.local_types
        );
    }

    #[test]
    fn annotation_type_stem_skips_none_in_union() {
        assert_eq!(annotation_type_stem("None | Foo"), Some("Foo".to_string()));
        assert_eq!(annotation_type_stem("Foo | None"), Some("Foo".to_string()));
        assert_eq!(
            annotation_type_stem("Optional[Foo]"),
            Some("Foo".to_string())
        );
        assert_eq!(annotation_type_stem("module.Foo"), Some("Foo".to_string()));
        assert_eq!(annotation_type_stem("int"), None);
        assert_eq!(annotation_type_stem("None"), None);
        assert_eq!(annotation_type_stem(""), None);
    }

    #[test]
    fn nested_class_does_not_share_instance_field_types_with_same_named_outer() {
        let f = extract(
            "class App:\n    def __init__(self):\n        self.controller = Controller()\n    def connect(self):\n        pass\n\
             class Other:\n    class App:\n        def __init__(self):\n            self.panel = Panel()\n        def go(self):\n            pass\n",
        );
        let connect = f
            .definitions
            .iter()
            .find(|d| {
                d.qualified_name.ends_with(".App.connect")
                    && !d.qualified_name.contains(".Other.")
            })
            .expect("App.connect");
        let go = f
            .definitions
            .iter()
            .find(|d| d.qualified_name.contains(".Other.App.go"))
            .expect("Other.App.go");
        assert!(
            f.local_types.iter().any(|t| {
                t.var_name == "self.controller"
                    && t.type_name == "Controller"
                    && t.scope_byte == connect.start_byte
            }),
            "outer App.connect should see controller; types: {:?}",
            f.local_types
        );
        assert!(
            !f.local_types.iter().any(|t| {
                t.var_name == "self.controller" && t.scope_byte == go.start_byte
            }),
            "nested Other.App.go must not inherit outer App fields; types: {:?}",
            f.local_types
        );
        assert!(
            f.local_types.iter().any(|t| {
                t.var_name == "self.panel"
                    && t.type_name == "Panel"
                    && t.scope_byte == go.start_byte
            }),
            "nested Other.App.go should see panel; types: {:?}",
            f.local_types
        );
        assert!(
            !f.local_types.iter().any(|t| {
                t.var_name == "self.panel" && t.scope_byte == connect.start_byte
            }),
            "outer App.connect must not inherit nested App fields; types: {:?}",
            f.local_types
        );
    }

    #[test]
    fn value_ref_on_the_right_of_or_is_captured() {
        let f = extract(
            "def _default_request():\n    return 1\n\n\
             handler = x or _default_request\n",
        );
        assert!(
            value_refs(&f).contains(&"_default_request"),
            "`x or _default_request` is a reference; got {:?}",
            value_refs(&f)
        );
    }

    #[test]
    fn a_field_assigned_two_types_is_left_untyped_unless_annotated() {
        let f = extract(
            "class App:\n    def a(self):\n        self.game = Game()\n    def b(self):\n        self.game = BaseGame()\n    def c(self):\n        self.same = Game()\n    def d(self):\n        self.same = Game()\n",
        );
        assert!(
            !f.local_types.iter().any(|t| t.var_name == "self.game"),
            "{:?}",
            f.local_types
        );
        assert!(
            f.local_types
                .iter()
                .any(|t| t.var_name == "self.same" && t.type_name == "Game")
        );
        let f = extract(
            "class App:\n    game: BaseGame\n    def a(self):\n        self.game = Game()\n",
        );
        assert!(
            f.local_types
                .iter()
                .all(|t| t.var_name != "self.game" || t.type_name == "BaseGame"),
            "{:?}",
            f.local_types
        );
        assert!(f.local_types.iter().any(|t| t.var_name == "self.game"));
    }

    #[test]
    fn a_duplicated_function_local_class_is_never_a_receiver_type() {
        let f = extract(
            "class Top:\n    pass\n\ndef test_a():\n    class _Local(Top):\n        pass\n    s = _Local()\n    t = Top()\n    s.run()\n\ndef test_b():\n    class _Local(Top):\n        pass\n\ndef test_c():\n    class _Unique(Top):\n        pass\n    u = _Unique()\n",
        );
        assert!(
            f.local_types
                .iter()
                .any(|t| t.var_name == "u" && t.type_name == "_Unique"),
            "a unique function-local class is typed: {:?}",
            f.local_types
        );
        assert!(
            !f.local_types.iter().any(|t| t.type_name == "_Local"),
            "{:?}",
            f.local_types
        );
        assert!(
            f.local_types
                .iter()
                .any(|t| t.var_name == "t" && t.type_name == "Top"),
            "{:?}",
            f.local_types
        );
    }

    #[test]
    fn function_nested_in_a_method_is_not_a_method() {
        let f = extract(
            "class Plain:\n    def build(self):\n        def _scan(x):\n            return x\n        return _scan(1)\n\nclass _Private:\n    def build(self):\n        def _scan(x):\n            return x\n        return _scan(1)\n",
        );
        for d in f.definitions.iter().filter(|d| d.simple_name == "_scan") {
            assert_eq!(d.variant, DefVariant::FreeFunction, "{}", d.qualified_name);
        }
        for d in f.definitions.iter().filter(|d| d.simple_name == "build") {
            assert_eq!(
                d.variant,
                DefVariant::InherentMethod,
                "{}",
                d.qualified_name
            );
        }
    }

    #[test]
    fn annotation_stem_unwraps_optional_but_not_containers() {
        assert_eq!(
            annotation_type_stem("Controller").as_deref(),
            Some("Controller")
        );
        assert_eq!(
            annotation_type_stem("Optional[Controller]").as_deref(),
            Some("Controller")
        );
        assert_eq!(
            annotation_type_stem("typing.Optional[Controller]").as_deref(),
            Some("Controller")
        );
        assert_eq!(
            annotation_type_stem("Controller | None").as_deref(),
            Some("Controller")
        );
        assert_eq!(
            annotation_type_stem("\"Controller\"").as_deref(),
            Some("Controller")
        );
        assert_eq!(
            annotation_type_stem("Optional['Controller']").as_deref(),
            Some("Controller")
        );
        assert_eq!(
            annotation_type_stem("ui.Controller").as_deref(),
            Some("Controller")
        );
        // A container's element type is not the field's type.
        assert_eq!(annotation_type_stem("list[Controller]"), None);
        assert_eq!(annotation_type_stem("dict[str, Controller]"), None);
        assert_eq!(annotation_type_stem("Sequence[Controller]"), None);
        // Two candidate types is no type.
        assert_eq!(annotation_type_stem("Controller | Other"), None);
        assert_eq!(annotation_type_stem("Union[Controller, Other]"), None);
        assert_eq!(annotation_type_stem("int"), None);
    }

    #[test]
    fn container_annotation_does_not_type_the_field() {
        let f = extract(
            "class Panel:\n    items: list[Controller]\n    def go(self):\n        self.items.clear()\n",
        );
        assert!(
            !f.local_types.iter().any(|t| t.var_name == "self.items"),
            "list[Controller] must not type self.items as Controller: {:?}",
            f.local_types
        );
    }

    #[test]
    fn every_class_records_its_bases_even_without_methods() {
        let f = extract(
            "class Base:\n    def run(self):\n        pass\n\nclass Middle(Base):\n    pass\n\nclass Leaf(Middle):\n    def run(self):\n        pass\n",
        );
        let by_qn: std::collections::BTreeMap<_, _> = f
            .classes
            .iter()
            .map(|c| (c.class_qn.as_str(), c.base_types.clone()))
            .collect();
        assert_eq!(by_qn.get("m.Base"), Some(&vec![]));
        assert_eq!(by_qn.get("m.Middle"), Some(&vec!["Base".to_string()]));
        assert_eq!(by_qn.get("m.Leaf"), Some(&vec!["Middle".to_string()]));
    }
}
