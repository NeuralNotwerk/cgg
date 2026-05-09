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

use cgg_core::{
    ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
};
use tree_sitter::{Node, Tree};

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct CSharpPlugin;

impl LanguagePlugin for CSharpPlugin {
    fn id(&self) -> &'static str {
        "csharp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".cs", ".csx"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_c_sharp::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "csharp");
        let mut w = Walker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
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
    facts: &'a mut FileFacts,
    scope: Vec<Scope>,
}

impl<'a> Walker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "namespace_declaration" | "file_scoped_namespace_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
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
                if !name.is_empty() {
                    self.scope.push(Scope::Type(name));
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "method_declaration" | "local_function_statement" => {
                self.record_method(node, DefVariant::InherentMethod);
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
            "invocation_expression" => {
                if let Some(r) = self.ref_from_invoke(node) {
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
            signature_hint: single_line(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
        });
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

    fn ref_from_invoke(&mut self, node: Node) -> Option<RefRecord> {
        let func = node.child_by_field_name("function")?;
        let (name, recv) = match func.kind() {
            "identifier" => (self.text(func).to_string(), String::new()),
            "member_access_expression" => {
                let expr = func.child_by_field_name("expression")?;
                let name = func.child_by_field_name("name")?;
                (
                    self.text(name).to_string(),
                    self.text(expr).to_string(),
                )
            }
            "generic_name" => {
                // Foo<T>() — take the base identifier.
                let base = func
                    .children(&mut func.walk())
                    .find(|c| c.kind() == "identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                (base, String::new())
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
        })
    }
}

fn line_range(n: Node) -> (u32, u32) {
    let s = (n.start_position().row as u32) + 1;
    let e = (n.end_position().row as u32) + 1;
    (s, e)
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
        p.set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        CSharpPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.cs"),
            &tree,
            src.as_bytes(),
        )
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
        let src = "using System; using Sys = System; using static System.Math;\nclass C {}\n";
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
            refs.iter().any(|r| r.name == "Helper" && r.receiver_hint.is_empty()),
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
}
