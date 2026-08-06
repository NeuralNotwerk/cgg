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
//!
//! Module name for qualified names is derived from the file stem for
//! Task 4. Task 6 will refine this to the full dotted package path by
//! consulting `__init__.py` chains via stack-graphs.

use std::path::Path;

use cgg_core::{
    ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
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
        crate::PluginSignals { attributes: true, dyn_uses: true, exports: true, impls: true, test_defs: true, unreachable: true, value_refs: true, visibility: true, ..Default::default() }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "python");
        let mut walker = Walker {
            source,
            facts: &mut facts,
            scope: vec![module_name(path)],
            bases: Vec::new(),
        };
        walker.walk(tree.root_node());
        let mut out = facts;
        if crate::deadcode_signals() {
            out.unreachable = super::cfg::unreachable_after_terminator(tree, &super::cfg::PYTHON);
        }
        if crate::deadcode_signals() {
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
        if init_py.exists() {
            if let Some(name) = d.file_name().and_then(|s| s.to_str()) {
                parts.push(name.to_string());
                dir = d.parent();
                continue;
            }
        }
        break;
    }
    parts.reverse();
    parts.join(".")
}

struct Walker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Base classes of the enclosing `class`, innermost last.
    bases: Vec<Vec<String>>,
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
                if !name.is_empty() {
                    self.scope.push(name);
                }
                // `class Encoder(nn.Module)` is the only thing that says
                // the runtime calls `forward`; nothing else in the file
                // does.
                self.bases.push(super::attrs::base_types(node, self.source));
                self.walk_children(node);
                self.bases.pop();
                if node.child_by_field_name("name").is_some() {
                    self.scope.pop();
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
                }
                self.walk_children(node);
                if !name.is_empty() {
                    self.scope.pop();
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
                self.walk_children(node);
                return;
            }
            "import_statement" | "import_from_statement" => {
                self.record_import(node);
                return;
            }
            "call" => {
                if let Some(r) = self.ref_from_call(node) {
                    // Django's `urls.py` is ordinary Python:
                    // `path("users/", views.list_users)` puts the
                    // handler in argument position, so the callee alone
                    // says nothing.
                    let context = if r.receiver_hint.is_empty() {
                        r.name.clone()
                    } else {
                        format!("{}.{}", r.receiver_hint, r.name)
                    };
                    self.facts.references.push(r);
                    let extra = super::registrar::capture(node, self.source, &context);
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

    fn record_function(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }

        let is_async = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "async");

        let decorators = collect_decorators(node, self.source);

        // Classify variant:
        //   * inside `class_definition`: method.
        //   * decorator @staticmethod / @classmethod / @property refine it.
        //   * `__init__` -> Constructor; `__del__` -> Destructor.
        let inside_class = self
            .scope
            .iter()
            .skip(1) // skip module name
            .any(|s| starts_uppercase(s));
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
        if assign.kind() != "assignment" { return; }
        let left = assign.child_by_field_name("left");
        let right = assign.child_by_field_name("right");
        let (Some(left), Some(right)) = (left, right) else { return };
        if left.kind() != "identifier" { return; }
        let var_name = self.text(left).to_string();
        if var_name.is_empty() { return; }
        // RHS must be a call where the function name starts with uppercase
        if right.kind() != "call" { return; }
        let func = right.child_by_field_name("function");
        let Some(func) = func else { return };
        let func_text = self.text(func);
        // Direct constructor: Foo(...)
        if func.kind() == "identifier" && func_text.starts_with(char::is_uppercase) {
            self.facts.local_types.push(cgg_core::LocalType {
                var_name: var_name.clone(), type_name: func_text.to_string(),
                scope_byte: node.start_byte() as u32,
            });
        }
        // Attribute constructor: module.Foo(...)
        if func.kind() == "attribute" {
            if let Some(attr) = func.child_by_field_name("attribute") {
                let attr_text = self.text(attr);
                if attr_text.starts_with(char::is_uppercase) {
                    self.facts.local_types.push(cgg_core::LocalType {
                        var_name, type_name: attr_text.to_string(),
                        scope_byte: node.start_byte() as u32,
                    });
                }
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
        Some(RefRecord {
            name,
            receiver_hint: receiver,
            site_line,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        })
    }
}

fn parse_import(text: &str) -> (String, String, String) {
    // `import a.b as c` | `import a, b` | `from x import y as z`
    if let Some(rest) = text.strip_prefix("from ") {
        if let Some((module, items)) = rest.split_once(" import ") {
            // For Task 4 we record the module + "import items" blob
            // under `path`. The resolver in Task 6 parses per-item.
            return (
                "from-import".into(),
                module.trim().to_string(),
                items.trim().to_string(),
            );
        }
    }
    if let Some(rest) = text.strip_prefix("import ") {
        let r = rest.trim();
        if let Some((lhs, alias)) = r.split_once(" as ") {
            return ("import".into(), lhs.trim().to_string(), alias.trim().to_string());
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
    if let Some(p) = parent {
        if p.kind() == "decorated_definition" {
            let mut c = p.walk();
            for child in p.children(&mut c) {
                if child.kind() == "decorator" {
                    out.push(child.utf8_text(source).unwrap_or("").trim().to_string());
                }
            }
            return out;
        }
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
    s.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract_with(path: &str, src: &str) -> FileFacts {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_python::LANGUAGE.into()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        PythonPlugin.extract(FileId::new(0), &PathBuf::from(path), &tree, src.as_bytes())
    }

    fn extract(src: &str) -> FileFacts {
        extract_with("m.py", src)
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
            refs.iter().any(|r| r.name == "n" && r.receiver_hint == "self"),
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
    "setUp", "tearDown", "setUpClass", "tearDownClass", "setup_module",
    "teardown_module", "setup_function", "teardown_function", "setup_class",
    "teardown_class", "setup_method", "teardown_method",
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
        let Some(lhs) = n.child_by_field_name("left") else { continue };
        if text(lhs).trim() != "__all__" {
            continue;
        }
        let Some(rhs) = n.child_by_field_name("right") else { continue };
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
