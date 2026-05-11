//! Kotlin plugin — full callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct KotlinPlugin;

impl LanguagePlugin for KotlinPlugin {
    fn id(&self) -> &'static str { "kotlin" }
    fn extensions(&self) -> &'static [&'static str] { &[".kt", ".kts"] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_kotlin_sg::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "kotlin");
        let mut w = KtWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct KtWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> KtWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}.{simple}", self.scope.join(".")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "package_header" => {
                // package com.example
                let name = node.children(&mut node.walk())
                    .find(|c| c.kind() == "identifier")
                    .map(|n| self.text(n).replace(" ", "").replace("\n", ""))
                    .unwrap_or_default();
                if !name.is_empty() {
                    // identifier node contains simple_identifier children joined by dots
                    let pkg = name.replace(".", ".");
                    self.scope.push(pkg);
                }
                return;
            }
            "import_header" => {
                self.record_import(node);
                return;
            }
            "class_declaration" => {
                let name = node.children(&mut node.walk())
                    .find(|c| c.kind() == "type_identifier")
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
            "object_declaration" => {
                let name = node.children(&mut node.walk())
                    .find(|c| c.kind() == "type_identifier")
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
            "companion_object" => {
                // companion object { ... } — methods are static-like
                self.walk_children(node);
                return;
            }
            "function_declaration" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "call_expression" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            "property_declaration" => {
                // val svc: Service = ... / var helper: Helper = ...
                self.record_local_type(node);
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
                if !c.goto_next_sibling() { break; }
            }
        }
    }

    fn record_function(&mut self, node: Node) {
        // function_declaration has simple_identifier as name
        let name = node.children(&mut node.walk())
            .find(|c| c.kind() == "simple_identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() { return; }

        // Check if it's an extension function (has receiver_type field)
        let is_extension = node.children(&mut node.walk())
            .any(|c| c.kind() == "receiver_type");
        let variant = if is_extension {
            DefVariant::InherentMethod
        } else if self.scope.is_empty() {
            DefVariant::FreeFunction
        } else {
            DefVariant::InherentMethod
        };

        let qn = self.qn(&name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: self.text(node).lines().next().unwrap_or("").trim().to_string(),
            visibility: String::new(),
            attributes: Vec::new(),
        });
    }

    fn record_import(&mut self, node: Node) {
        let ident = node.children(&mut node.walk())
            .find(|c| c.kind() == "identifier")
            .map(|n| self.text(n).replace(" ", "").replace("\n", ""))
            .unwrap_or_default();
        if ident.is_empty() { return; }

        // Check for alias: `import ... as Alias`
        let explicit_alias = node.children(&mut node.walk())
            .find(|c| c.kind() == "import_alias")
            .and_then(|a| a.children(&mut a.walk()).find(|c| c.kind() == "type_identifier" || c.kind() == "simple_identifier"))
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();

        // For non-aliased imports, the binding name is the last segment.
        // `import com.example.Helper` -> alias "Helper"
        let alias = if !explicit_alias.is_empty() {
            explicit_alias
        } else {
            ident.rsplit('.').next().unwrap_or("").to_string()
        };

        self.facts.imports.push(ImportRecord {
            kind: "import".into(),
            path: ident,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_local_type(&mut self, node: Node) {
        // property_declaration -> variable_declaration -> simple_identifier + user_type -> type_identifier
        let var_decl = node.children(&mut node.walk())
            .find(|c| c.kind() == "variable_declaration");
        let Some(var_decl) = var_decl else { return };
        let var_name = var_decl.children(&mut var_decl.walk())
            .find(|c| c.kind() == "simple_identifier")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if var_name.is_empty() { return; }

        // Try explicit type annotation first
        let type_name = var_decl.children(&mut var_decl.walk())
            .find(|c| c.kind() == "user_type")
            .and_then(|ut| ut.children(&mut ut.walk()).find(|c| c.kind() == "type_identifier"))
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if !type_name.is_empty() && type_name.starts_with(char::is_uppercase) {
            self.facts.local_types.push(cgg_core::LocalType {
                var_name, type_name, scope_byte: node.start_byte() as u32,
            });
            return;
        }

        // Infer from RHS constructor: val x = Foo(...) or val x = Foo.create(...)
        // The RHS is a sibling of variable_declaration in property_declaration
        let call = node.children(&mut node.walk())
            .find(|c| c.kind() == "call_expression");
        if let Some(call) = call {
            let callee = call.child(0);
            if let Some(callee) = callee {
                let callee_text = self.text(callee);
                // Direct constructor: Foo(...)
                if callee_text.starts_with(char::is_uppercase) && !callee_text.contains('.') {
                    self.facts.local_types.push(cgg_core::LocalType {
                        var_name, type_name: callee_text.to_string(),
                        scope_byte: node.start_byte() as u32,
                    });
                }
            }
        }
    }

    fn record_call(&mut self, node: Node) {
        // call_expression -> first child is the callee
        let callee = node.child(0);
        let Some(callee) = callee else { return };
        let (name, recv) = match callee.kind() {
            "simple_identifier" => (self.text(callee).to_string(), String::new()),
            "navigation_expression" => {
                // obj.method or Obj.method
                let parts: Vec<&str> = callee.children(&mut callee.walk())
                    .filter(|c| c.kind() == "simple_identifier" || c.kind() == "navigation_suffix")
                    .map(|c| {
                        if c.kind() == "navigation_suffix" {
                            c.children(&mut c.walk())
                                .find(|cc| cc.kind() == "simple_identifier")
                                .map(|n| self.text(n))
                                .unwrap_or("")
                        } else {
                            self.text(c)
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .collect();
                if parts.len() >= 2 {
                    let name = parts.last().unwrap().to_string();
                    let recv = parts[..parts.len()-1].join(".");
                    (name, recv)
                } else if parts.len() == 1 {
                    (parts[0].to_string(), String::new())
                } else {
                    return;
                }
            }
            _ => return,
        };
        if name.is_empty() { return; }
        self.facts.references.push(RefRecord {
            name, receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
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
        p.set_language(&tree_sitter_kotlin_sg::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        KotlinPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.kt"), &tree, src.as_bytes())
    }

    #[test]
    fn class_methods_with_package() {
        let src = "package com.example\nclass Service {\n  fun run() {}\n  fun process(s: String): Int = s.length\n}\n";
        let f = extract(src);
        let qns: Vec<&str> = f.definitions.iter().map(|d| d.qualified_name.as_str()).collect();
        assert!(qns.iter().any(|q| q.ends_with("Service.run")), "got: {qns:?}");
        assert!(qns.iter().any(|q| q.ends_with("Service.process")), "got: {qns:?}");
    }

    #[test]
    fn top_level_function() {
        let src = "fun topLevel() {}\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "topLevel" && d.variant == DefVariant::FreeFunction));
    }

    #[test]
    fn extension_function() {
        let src = "fun String.greet() { println(this) }\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "greet"));
    }

    #[test]
    fn imports_captured() {
        let src = "import com.example.Helper\nimport com.example.format as fmt\nfun f() {}\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path.contains("Helper")));
        assert!(f.imports.iter().any(|i| i.alias == "fmt"));
    }

    #[test]
    fn call_expressions() {
        let src = "fun f() { helper(); obj.run() }\n";
        let f = extract(src);
        assert!(f.references.iter().any(|r| r.name == "helper"), "refs: {:?}", f.references);
        assert!(f.references.iter().any(|r| r.name == "run" && r.receiver_hint == "obj"), "refs: {:?}", f.references);
    }
}
