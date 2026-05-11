//! C++ plugin — namespace/class-aware callable extraction.
//!
//! Extends the C model with:
//! * `namespace_definition` scope stack.
//! * `class_specifier` / `struct_specifier` type scope.
//! * Methods inside class bodies (including constructors/destructors).
//! * `qualified_identifier` in call expressions (`ns::fn()`).
//! * `field_expression` for `obj.method()` / `ptr->method()`.

use std::path::Path;

use cgg_core::{
    ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
};
use tree_sitter::{Node, Tree};

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct CppPlugin;

impl LanguagePlugin for CppPlugin {
    fn id(&self) -> &'static str {
        "cpp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".cc", ".cpp", ".cxx", ".C", ".hpp", ".hh", ".hxx"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::Custom
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_cpp::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "cpp");
        let mut w = CppWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct CppWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> CppWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}::{simple}", self.scope.join("::"))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "namespace_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    // anonymous namespace
                    self.walk_children(node);
                }
                return;
            }
            "class_specifier" | "struct_specifier" => {
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
            "function_definition" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "template_declaration" => {
                // template<typename T> void foo() {} — unwrap to find the function inside
                self.walk_children(node);
                return;
            }
            "declaration" => {
                self.try_record_prototype(node);
                self.walk_children(node);
                return;
            }
            "preproc_function_def" => {
                self.record_macro_def(node);
                return;
            }
            "preproc_include" => {
                self.record_include(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
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
        let Some(decl) = node.child_by_field_name("declarator") else { return };
        let (simple, variant) = self.fn_info_from_declarator(decl);
        if simple.is_empty() {
            return;
        }
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

    fn fn_info_from_declarator(&self, decl: Node) -> (String, DefVariant) {
        // function_declarator -> declarator: identifier | field_identifier |
        //   destructor_name | qualified_identifier
        if decl.kind() != "function_declarator" {
            return (String::new(), DefVariant::FreeFunction);
        }
        let Some(d) = decl.child_by_field_name("declarator") else {
            return (String::new(), DefVariant::FreeFunction);
        };
        match d.kind() {
            "identifier" => {
                let name = self.text(d).to_string();
                // If inside a class scope, it's a constructor if name == class name.
                let variant = if self.scope.last().map(|s| s.as_str()) == Some(name.as_str()) {
                    DefVariant::Constructor
                } else if self.scope.is_empty() {
                    DefVariant::FreeFunction
                } else {
                    DefVariant::InherentMethod
                };
                (name, variant)
            }
            "field_identifier" => {
                let name = self.text(d).to_string();
                (name, DefVariant::InherentMethod)
            }
            "destructor_name" => {
                // ~ClassName
                let name = format!("~{}", self.text(d).trim_start_matches('~'));
                (name, DefVariant::Destructor)
            }
            "qualified_identifier" => {
                // Out-of-line: `ClassName::method` or `ns::Class::method`.
                // We take the full text as the simple name (qualified
                // name will prepend current scope).
                let name_node = d.child_by_field_name("name");
                let simple = name_node
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_else(|| self.text(d).to_string());
                // Check if destructor
                if simple.starts_with('~') {
                    (simple, DefVariant::Destructor)
                } else {
                    (self.text(d).to_string(), DefVariant::InherentMethod)
                }
            }
            "operator_name" | "operator_cast" => {
                (self.text(d).to_string(), DefVariant::InherentMethod)
            }
            _ => (String::new(), DefVariant::FreeFunction),
        }
    }

    fn try_record_prototype(&mut self, node: Node) {
        let mut c = node.walk();
        for child in node.children(&mut c) {
            if child.kind() == "function_declarator" {
                let (simple, variant) = self.fn_info_from_declarator(child);
                if !simple.is_empty() {
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
                return;
            }
        }
    }

    fn record_macro_def(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else { return };
        let name = self.text(name_node).to_string();
        if name.is_empty() {
            return;
        }
        let qn = self.qn(&name);
        let (sl, el) = line_range(node);
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: single_line(self.text(node)),
            visibility: String::new(),
            attributes: vec!["macro".to_string()],
        });
    }

    fn record_include(&mut self, node: Node) {
        let Some(path_node) = node.child_by_field_name("path") else { return };
        if path_node.kind() == "string_literal" || path_node.kind() == "string_content" {
            let raw = self.text(path_node);
            let path = raw.trim_matches('"').to_string();
            if !path.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: "include".into(),
                    path,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
        }
    }

    fn record_call(&mut self, node: Node) {
        let Some(func) = node.child_by_field_name("function") else { return };
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "field_expression" => {
                let arg = func.child_by_field_name("argument")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let field = func.child_by_field_name("field")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (field, arg)
            }
            "qualified_identifier" => {
                // `ns::sub::fn()` — full text is the receiver+name.
                let full = self.text(func);
                if let Some(pos) = full.rfind("::") {
                    let recv = full[..pos].to_string();
                    let name = full[pos + 2..].to_string();
                    (name, recv)
                } else {
                    (full.to_string(), String::new())
                }
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
    s.lines().next().unwrap_or("").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_cpp::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        CppPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.cpp"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn namespace_qualified_names() {
        let src = "namespace math { namespace detail { void compute() {} } }\n";
        let f = extract(src);
        assert!(
            f.definitions.iter().any(|d| d.qualified_name == "math::detail::compute"),
            "got: {:?}",
            f.definitions.iter().map(|d| &d.qualified_name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn class_method_and_constructor() {
        let src = r#"
namespace ns {
class Foo {
public:
    Foo() {}
    ~Foo() {}
    int bar(int x) { return x; }
};
}
"#;
        let f = extract(src);
        let qns: Vec<&str> = f.definitions.iter().map(|d| d.qualified_name.as_str()).collect();
        assert!(qns.contains(&"ns::Foo::Foo"), "got: {qns:?}");
        assert!(qns.contains(&"ns::Foo::~Foo"), "got: {qns:?}");
        assert!(qns.contains(&"ns::Foo::bar"), "got: {qns:?}");
        let ctor = f.definitions.iter().find(|d| d.simple_name == "Foo").unwrap();
        assert_eq!(ctor.variant, DefVariant::Constructor);
        let dtor = f.definitions.iter().find(|d| d.simple_name == "~Foo").unwrap();
        assert_eq!(dtor.variant, DefVariant::Destructor);
    }

    #[test]
    fn qualified_call_expression() {
        let src = "void f() { math::detail::compute(); }\n";
        let f = extract(src);
        let r = f.references.iter().find(|r| r.name == "compute").unwrap();
        assert_eq!(r.receiver_hint, "math::detail");
    }

    #[test]
    fn field_expression_call() {
        let src = "void f() { obj.run(); ptr->exec(); }\n";
        let f = extract(src);
        let refs: Vec<(&str, &str)> = f.references.iter().map(|r| (r.name.as_str(), r.receiver_hint.as_str())).collect();
        assert!(refs.contains(&("run", "obj")), "got: {refs:?}");
        assert!(refs.contains(&("exec", "ptr")), "got: {refs:?}");
    }

    #[test]
    fn include_directive_captured() {
        let src = "#include \"base.h\"\n#include <vector>\nvoid f() {}\n";
        let f = extract(src);
        assert_eq!(f.imports.len(), 1);
        assert_eq!(f.imports[0].kind, "include");
        assert_eq!(f.imports[0].path, "base.h");
    }
}
