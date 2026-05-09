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

use cgg_core::{
    ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord,
};
use tree_sitter::{Node, Tree};

use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct GoPlugin;

impl LanguagePlugin for GoPlugin {
    fn id(&self) -> &'static str {
        "go"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".go"]
    }
    fn resolver_kind(&self) -> ResolverKind {
        ResolverKind::StackGraphs
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_go::LANGUAGE.into()
    }

    fn extract(
        &self,
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

        let mut w = Walker {
            source,
            facts: &mut facts,
            pkg,
        };
        w.walk(root);
        facts
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
                // Continue into body for call sites.
                self.walk_children(node);
                return;
            }
            "method_declaration" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            "import_declaration" => {
                self.record_imports(node);
                return;
            }
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
                        match t.kind() {
                            "type_identifier" => {
                                out = self.text(t).to_string();
                            }
                            "pointer_type" => {
                                let mut cc = t.walk();
                                for n in t.children(&mut cc) {
                                    if n.kind() == "type_identifier" {
                                        out = self.text(n).to_string();
                                        break;
                                    }
                                }
                            }
                            _ => {}
                        }
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
        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant: DefVariant::InherentMethod,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: single_line(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
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
        p.set_language(&tree_sitter_go::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        GoPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.go"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn free_function() {
        let f = extract("package pkg\n\nfunc Plain() {}\n");
        assert!(f
            .definitions
            .iter()
            .any(|d| d.qualified_name == "pkg.Plain"));
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
        let fmt_ref = f
            .references
            .iter()
            .find(|r| r.name == "Println")
            .unwrap();
        assert_eq!(fmt_ref.receiver_hint, "fmt");
    }

    #[test]
    fn package_root_marker_emitted() {
        let f = extract("package alpha\n\nfunc X() {}\n");
        let pkg = f.imports.iter().find(|i| i.kind == "package-root").unwrap();
        assert_eq!(pkg.path, "alpha");
    }
}
