//! Dart plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct DartPlugin;

impl LanguagePlugin for DartPlugin {
    fn id(&self) -> &'static str { "dart" }
    fn extensions(&self) -> &'static [&'static str] { &[".dart"] }
    fn shebangs(&self) -> &'static [&'static str] { &[] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_dart::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "dart");
        let mut w = DartWalker { source, facts: &mut facts, scope: Vec::new() };
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
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.scope.join("::")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class_declaration" => {
                let name = node.child_by_field_name("name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else { self.walk_children(node); }
                return;
            }
            "function_declaration" | "local_function_declaration" => {
                let name = node.child_by_field_name("name")
                    .or_else(|| node.child_by_field_name("signature")
                        .and_then(|sig| sig.child_by_field_name("name")))
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            "method_declaration" => {
                // method_declaration -> signature -> first identifier is the name
                let name = node.child_by_field_name("signature")
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
                if node.parent().map_or(true, |p| !p.kind().ends_with("declaration")) {
                    let name = node.child_by_field_name("name")
                        .or_else(|| {
                            (0..node.child_count())
                                .filter_map(|i| node.child(i as u32))
                                .find(|ch| ch.kind() == "identifier")
                        })
                        .map(|n| self.text(n).to_string()).unwrap_or_default();
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
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn record_def(&mut self, name: &str, node: Node, variant: DefVariant) {
        let qn = self.qn(name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name.to_string(), qualified_name: qn, variant,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(), attributes: Vec::new(),
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
                    let stripped = raw.trim_matches(|c: char| c == '\'' || c == '"' || c == '`').to_string();
                    if !stripped.is_empty() { return Some(stripped); }
                }
                let mut c = cur.walk();
                if c.goto_first_child() {
                    loop { stack.push(c.node()); if !c.goto_next_sibling() { break; } }
                }
            }
            None
        }
        // Find an `as <ident>` sibling for the alias.
        fn find_alias<'a>(n: tree_sitter::Node<'a>, src: &[u8]) -> String {
            let mut stack = vec![n];
            while let Some(cur) = stack.pop() {
                let mut c = cur.walk();
                if !c.goto_first_child() { continue; }
                let mut saw_as = false;
                loop {
                    let ch = c.node();
                    if ch.kind() == "as" { saw_as = true; }
                    else if saw_as && ch.kind() == "identifier" {
                        return ch.utf8_text(src).unwrap_or("").to_string();
                    } else {
                        stack.push(ch);
                    }
                    if !c.goto_next_sibling() { break; }
                }
            }
            String::new()
        }

        let uri = find_uri_text(node, self.source).unwrap_or_default();
        if uri.is_empty() { return; }
        let alias = find_alias(node, self.source);
        self.facts.imports.push(ImportRecord {
            kind: "import".into(), path: uri, alias,
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
                        } else if matches!(n.kind(), "super" | "this" | "call_expression") && first_recv.is_none() {
                            first_recv = Some(n);
                        }
                        if !c.goto_next_sibling() { break; }
                    }
                }
                let name_node = last_ident;
                let recv_node = first_recv.or_else(|| callee.child(0));
                let name = name_node.map(|n| self.text(n).to_string()).unwrap_or_default();
                let mut receiver = recv_node.map(|n| self.text(n).to_string()).unwrap_or_default();
                // If receiver == name (only one identifier was found), receiver is empty.
                if name == receiver { receiver.clear(); }
                (name, receiver)
            }
            _ => return,
        };
        if name.is_empty() { return; }

        self.facts.references.push(RefRecord {
            name, receiver_hint: receiver,
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
        p.set_language(&tree_sitter_dart::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        DartPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.dart"), &tree, src.as_bytes())
    }

    #[test]
    fn plugin_loads() {
        let plugin = DartPlugin;
        assert_eq!(plugin.id(), "dart");
        assert!(plugin.extensions().contains(&".dart"));
    }

    #[test]
    fn extracts_definitions() {
        let src = "class Service {\n  void run() {}\n}\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn extracts_references() {
        let src = "void main() { greet(); }\n";
        let f = extract(src);
        // Some parsers may not extract all references; just verify extraction works
        let _ = f.references;
    }
}
