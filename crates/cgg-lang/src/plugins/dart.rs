//! Dart plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct DartPlugin;

impl LanguagePlugin for DartPlugin {
    fn id(&self) -> &'static str {
        "dart"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".dart"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_dart::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "dart");
        let mut w = DartWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct DartWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> DartWalker<'a> {
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
            "function_declaration" | "local_function_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .or_else(|| {
                        node.child_by_field_name("signature")
                            .and_then(|sig| sig.child_by_field_name("name"))
                    })
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            "method_declaration" => {
                // method_declaration -> signature -> first identifier is the name
                let name = node
                    .child_by_field_name("signature")
                    .and_then(|sig| {
                        (0..sig.child_count())
                            .filter_map(|i| sig.child(i as u32))
                            .find(|ch| ch.kind() == "identifier")
                    })
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::InherentMethod);
                }
                self.walk_children(node);
                return;
            }
            "function_signature" | "method_signature" => {
                // Only record if not inside a declaration (avoid double-counting)
                if node
                    .parent()
                    .is_none_or(|p| !p.kind().ends_with("declaration"))
                {
                    let name = node
                        .child_by_field_name("name")
                        .or_else(|| {
                            (0..node.child_count())
                                .filter_map(|i| node.child(i as u32))
                                .find(|ch| ch.kind() == "identifier")
                        })
                        .map(|n| self.text(n).to_string())
                        .unwrap_or_default();
                    if !name.is_empty() {
                        let variant = if node.kind() == "method_signature" {
                            DefVariant::InherentMethod
                        } else {
                            DefVariant::FreeFunction
                        };
                        self.record_def(&name, node, variant);
                    }
                }
                self.walk_children(node);
                return;
            }
            "import_or_export" => {
                self.record_import(node);
                self.walk_children(node);
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

    fn record_def(&mut self, name: &str, node: Node, variant: DefVariant) {
        let qn = self.qn(name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name.to_string(),
            qualified_name: qn,
            variant,
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

    fn record_import(&mut self, node: Node) {
        // tree-sitter-dart: import_or_export -> library_import ->
        // import_specification -> configurable_uri -> uri -> string_literal.
        // The literal text we want sits inside template_chars_* under
        // string_literal_*. Descend to find it.
        fn find_uri_text<'a>(n: tree_sitter::Node<'a>, src: &[u8]) -> Option<String> {
            let mut stack = vec![n];
            while let Some(cur) = stack.pop() {
                let kind = cur.kind();
                if kind.starts_with("template_chars_") {
                    return Some(cur.utf8_text(src).ok()?.to_string());
                }
                if kind == "uri" {
                    // Grab text and strip outer quotes/braces.
                    let raw = cur.utf8_text(src).ok()?.trim();
                    let stripped = raw
                        .trim_matches(|c: char| c == '\'' || c == '"' || c == '`')
                        .to_string();
                    if !stripped.is_empty() {
                        return Some(stripped);
                    }
                }
                let mut c = cur.walk();
                if c.goto_first_child() {
                    loop {
                        stack.push(c.node());
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            None
        }
        // Find an `as <ident>` sibling for the alias.
        fn find_alias<'a>(n: tree_sitter::Node<'a>, src: &[u8]) -> String {
            let mut stack = vec![n];
            while let Some(cur) = stack.pop() {
                let mut c = cur.walk();
                if !c.goto_first_child() {
                    continue;
                }
                let mut saw_as = false;
                loop {
                    let ch = c.node();
                    if ch.kind() == "as" {
                        saw_as = true;
                    } else if saw_as && ch.kind() == "identifier" {
                        return ch.utf8_text(src).unwrap_or("").to_string();
                    } else {
                        stack.push(ch);
                    }
                    if !c.goto_next_sibling() {
                        break;
                    }
                }
            }
            String::new()
        }

        let uri = find_uri_text(node, self.source).unwrap_or_default();
        if uri.is_empty() {
            return;
        }
        let alias = find_alias(node, self.source);
        self.facts.imports.push(ImportRecord {
            kind: "import".into(),
            path: uri,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_call(&mut self, node: Node) {
        // call_expression layout:
        //   call_expression -> [identifier | member_expression] arguments
        //   member_expression -> [identifier | super | call_expression] . identifier
        let callee = node.child(0);
        let Some(callee) = callee else { return };
        let (name, receiver) = match callee.kind() {
            "identifier" => (self.text(callee).to_string(), String::new()),
            "member_expression" => {
                // Last identifier child is the called name; preceding text is the receiver.
                let mut last_ident: Option<Node> = None;
                let mut first_recv: Option<Node> = None;
                let mut c = callee.walk();
                if c.goto_first_child() {
                    loop {
                        let n = c.node();
                        if n.kind() == "identifier" {
                            if first_recv.is_none() && last_ident.is_none() {
                                // identifier in receiver position
                            }
                            last_ident = Some(n);
                        } else if matches!(n.kind(), "super" | "this" | "call_expression")
                            && first_recv.is_none()
                        {
                            first_recv = Some(n);
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                let name_node = last_ident;
                let recv_node = first_recv.or_else(|| callee.child(0));
                let name = name_node
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let mut receiver = recv_node
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // If receiver == name (only one identifier was found), receiver is empty.
                if name == receiver {
                    receiver.clear();
                }
                (name, receiver)
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }

        self.facts.references.push(RefRecord {
            name,
            receiver_hint: receiver,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
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
        p.set_language(&tree_sitter_dart::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        DartPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.dart"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn plugin_loads() {
        let plugin = DartPlugin;
        assert_eq!(plugin.id(), "dart");
        assert!(plugin.extensions().contains(&".dart"));
    }

    fn defs(f: &FileFacts) -> Vec<String> {
        f.definitions
            .iter()
            .map(|d| d.qualified_name.clone())
            .collect()
    }
    fn refs(f: &FileFacts) -> Vec<String> {
        f.references.iter().map(|r| r.name.clone()).collect()
    }

    #[test]
    fn extracts_definitions() {
        let src = "class Service {\n  void run() {}\n}\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn a_method_is_qualified_by_its_class() {
        // Bare `run` would collide with every other `run` in the tree;
        // the class scope is what makes the graph addressable.
        let f = extract("class Service {\n  void run() {}\n}\n");
        assert!(
            defs(&f)
                .iter()
                .any(|d| d.contains("Service") && d.ends_with("run")),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn a_top_level_function_is_captured() {
        let f = extract("void main() {}\n");
        assert!(
            f.definitions.iter().any(|d| d.simple_name == "main"),
            "{:?}",
            defs(&f)
        );
    }

    #[test]
    fn extracts_references() {
        // Was a no-op assertion (`let _ = f.references`), so a plugin
        // that stopped recording calls entirely still passed.
        let f = extract("void main() { greet(); }\n");
        assert!(
            refs(&f).iter().any(|r| r == "greet"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn a_method_call_on_a_receiver_is_recorded() {
        let f = extract("void main() { svc.run(); }\n");
        assert!(
            refs(&f).iter().any(|r| r == "run" || r == "svc.run"),
            "refs: {:?}",
            refs(&f)
        );
    }

    #[test]
    fn an_import_records_its_uri() {
        let f = extract("import 'package:foo/bar.dart';\nvoid main() {}\n");
        assert!(
            f.imports.iter().any(|i| i.path.contains("bar.dart")),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn an_aliased_import_keeps_the_alias() {
        let f = extract("import 'package:foo/bar.dart' as fb;\nvoid main() {}\n");
        let i = f
            .imports
            .iter()
            .find(|i| i.path.contains("bar.dart"))
            .unwrap_or_else(|| panic!("imports: {:?}", f.imports));
        assert_eq!(
            i.alias, "fb",
            "the `as` alias must survive: {:?}",
            f.imports
        );
    }

    #[test]
    fn an_empty_file_yields_nothing_and_does_not_panic() {
        let f = extract("");
        assert!(f.definitions.is_empty());
        assert!(f.imports.is_empty());
    }

    #[test]
    fn malformed_source_does_not_panic() {
        let f = extract("class Broken {\n  void run( {\n");
        let _ = defs(&f);
    }
}
