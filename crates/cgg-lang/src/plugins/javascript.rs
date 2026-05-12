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

use cgg_core::{
    ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
};
use tree_sitter::{Node, Tree};

use crate::{LanguagePlugin, ResolverKind};

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
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_javascript::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "javascript");
        let mut w = JsWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

pub(crate) struct JsWalker<'a> {
    source: &'a [u8],
    pub facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> JsWalker<'a> {
    pub(crate) fn new(source: &'a [u8], facts: &'a mut FileFacts) -> Self {
        Self { source, facts, scope: Vec::new() }
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
            "class_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "method_definition" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            "lexical_declaration" | "variable_declaration" => {
                self.try_record_named_fn(node);
                self.try_record_require(node);
                self.walk_children(node);
                return;
            }
            "export_statement" => {
                // Walk into the exported declaration.
                self.walk_children(node);
                return;
            }
            "expression_statement" => {
                // Handle `exports.foo = function() {}` and
                // `module.exports.foo = function() {}` patterns (CJS).
                self.try_record_exports_assign(node);
                self.walk_children(node);
                return;
            }
            "import_statement" => {
                self.record_import(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
                self.extract_named_fn_args(node);
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
        let Some(name_node) = node.child_by_field_name("name") else { return };
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
            signature_hint: single_line(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
        });
    }

    fn record_method(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else { return };
        let simple = self.text(name_node).to_string();
        if simple.is_empty() {
            return;
        }
        // Detect variant from node structure.
        let is_static = node.children(&mut node.walk())
            .any(|c| c.kind() == "static");
        let is_getter = node.children(&mut node.walk())
            .any(|c| c.kind() == "get");
        let is_setter = node.children(&mut node.walk())
            .any(|c| c.kind() == "set");
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
            signature_hint: single_line(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
        });
    }

    fn try_record_exports_assign(&mut self, node: Node) {
        // Handle various CJS patterns:
        // `exports.foo = function() {}`
        // `obj.method = function name() {}`
        // `req.get = req.header = function header() {}` (chained)
        let child = node.child(0);
        let Some(child) = child else { return };
        if child.kind() != "assignment_expression" { return; }
        self.extract_assignment_fn(child);
    }

    fn extract_assignment_fn(&mut self, node: Node) {
        let Some(left) = node.child_by_field_name("left") else { return };
        let Some(right) = node.child_by_field_name("right") else { return };

        // If RHS is another assignment, recurse (chained: a = b = function(){})
        if right.kind() == "assignment_expression" {
            self.extract_assignment_fn(right);
        }

        // Check if RHS is a function
        if !matches!(right.kind(), "arrow_function" | "function_expression" | "function" | "assignment_expression") {
            return;
        }
        // For chained assignments, the function is in the inner one
        let func_node = if right.kind() == "assignment_expression" {
            right.child_by_field_name("right")
                .filter(|r| matches!(r.kind(), "arrow_function" | "function_expression" | "function"))
        } else {
            Some(right)
        };
        let Some(_) = func_node else { return };

        if left.kind() != "member_expression" { return; }
        let prop = left.child_by_field_name("property")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if prop.is_empty() { return; }
        let qn = self.qn(&prop);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: prop,
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: self.text(node).lines().next().unwrap_or("").trim().to_string(),
            visibility: String::new(),
            attributes: Vec::new(),
        });
    }

    fn try_record_named_fn(&mut self, node: Node) {
        // `const foo = (x) => ...` or `const foo = function() {}`
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if child.kind() != "variable_declarator" {
                continue;
            }
            let Some(name_node) = child.child_by_field_name("name") else { continue };
            if name_node.kind() != "identifier" {
                continue;
            }
            let Some(value) = child.child_by_field_name("value") else { continue };
            if !matches!(value.kind(), "arrow_function" | "function_expression" | "generator_function") {
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
                signature_hint: single_line(self.text(child)),
                visibility: String::new(),
                attributes: Vec::new(),
            });
        }
    }

    fn try_record_require(&mut self, node: Node) {
        // `const foo = require('./mod')` or `const { bar } = require('./mod')`
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if child.kind() != "variable_declarator" { continue; }
            let Some(value) = child.child_by_field_name("value") else { continue };
            if value.kind() != "call_expression" { continue; }
            let func = value.child_by_field_name("function");
            if func.map(|f| self.text(f)) != Some("require") { continue; }
            // Extract the path from arguments
            let args = value.child_by_field_name("arguments");
            let path = args.and_then(|a| {
                let count = a.child_count();
                for i in 0..count {
                    let arg = a.child(i as u32).unwrap();
                    if arg.kind() == "string" {
                        return Some(self.text(arg).trim_matches(|c| c == '\'' || c == '"').to_string());
                    }
                }
                None
            });
            let Some(path) = path else { continue };
            if path.is_empty() { continue; }

            let Some(name_node) = child.child_by_field_name("name") else { continue };
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
        let source_node = node.child_by_field_name("source")
            .or_else(|| {
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
        let Some(source_node) = source_node else { return };
        let raw = self.text(source_node);
        let path = raw.trim_matches(|c| c == '\'' || c == '"').to_string();
        if path.is_empty() {
            return;
        }

        // Collect imported names from import_clause.
        let clause = node.children(&mut node.walk())
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
                    let alias = child.child_by_field_name("name")
                        .or_else(|| child.children(&mut child.walk()).find(|n| n.kind() == "identifier"))
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
                        let orig = spec.child_by_field_name("name")
                            .map(|n| self.text(n).to_string())
                            .unwrap_or_default();
                        let local = spec.child_by_field_name("alias")
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

    fn extract_named_fn_child(&mut self, node: Node) {
        // Find any named function_expression direct child
        let count = node.child_count();
        for i in 0..count as u32 {
            let Some(child) = node.child(i) else { continue };
            if child.kind() != "function_expression" && child.kind() != "function" { continue; }
            let name = child.child_by_field_name("name")
                .map(|n| self.text(n).to_string()).unwrap_or_default();
            if name.is_empty() { continue; }
            let qn = self.qn(&name);
            let (sl, el) = ((child.start_position().row as u32)+1, (child.end_position().row as u32)+1);
            self.facts.definitions.push(DefRecord {
                simple_name: name, qualified_name: qn,
                variant: DefVariant::FreeFunction,
                start_line: sl, end_line: el,
                start_byte: child.start_byte() as u32, end_byte: child.end_byte() as u32,
                signature_hint: self.text(child).lines().next().unwrap_or("").trim().to_string(),
                visibility: String::new(), attributes: Vec::new(),
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
            if arg.kind() != "function_expression" && arg.kind() != "function" { continue; }
            // Check if the function has a name
            let name = arg.child_by_field_name("name")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default();
            if name.is_empty() { continue; }
            let qn = self.qn(&name);
            let (sl, el) = ((arg.start_position().row as u32)+1, (arg.end_position().row as u32)+1);
            self.facts.definitions.push(DefRecord {
                simple_name: name, qualified_name: qn,
                variant: DefVariant::FreeFunction,
                start_line: sl, end_line: el,
                start_byte: arg.start_byte() as u32, end_byte: arg.end_byte() as u32,
                signature_hint: self.text(arg).lines().next().unwrap_or("").trim().to_string(),
                visibility: String::new(), attributes: Vec::new(),
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        let Some(func) = node.child_by_field_name("function") else { return };
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "member_expression" => {
                let obj = func.child_by_field_name("object")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let prop = func.child_by_field_name("property")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (prop, obj)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }
}

fn line_range(n: Node) -> (u32, u32) {
    ((n.start_position().row as u32) + 1, (n.end_position().row as u32) + 1)
}

fn single_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().trim_end_matches('{').trim_end_matches(':').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_javascript::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        JavaScriptPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.js"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn function_declarations() {
        let src = "function greet(name) {}\nasync function fetchData() {}\n";
        let f = extract(src);
        let names: Vec<&str> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(names.contains(&"fetchData"), "got: {names:?}");
    }

    #[test]
    fn arrow_function_named() {
        let src = "const add = (a, b) => a + b;\nlet mul = function(a, b) { return a*b; };\n";
        let f = extract(src);
        let names: Vec<&str> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
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
        let by: std::collections::HashMap<_, _> = f.definitions.iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(by["constructor"], DefVariant::Constructor);
        assert_eq!(by["run"], DefVariant::InherentMethod);
        assert_eq!(by["create"], DefVariant::StaticMethod);
        assert_eq!(by["label"], DefVariant::Property);
        // Qualified names include class.
        assert!(f.definitions.iter().any(|d| d.qualified_name == "Service.run"));
    }

    #[test]
    fn esm_imports_captured() {
        let src = "import { helper, scale as s } from './utils.js';\nimport * as math from './math.js';\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.kind == "from-import" && i.path == "./utils.js"));
        assert!(f.imports.iter().any(|i| i.kind == "import" && i.path == "./math.js" && i.alias == "math"));
    }

    #[test]
    fn call_expressions_captured() {
        let src = "function f() { greet('x'); obj.run(); }\n";
        let f = extract(src);
        let refs: Vec<(&str, &str)> = f.references.iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(refs.contains(&("greet", "")), "got: {refs:?}");
        assert!(refs.contains(&("run", "obj")), "got: {refs:?}");
    }
}
