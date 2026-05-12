//! Clojure plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct ClojurePlugin;

impl LanguagePlugin for ClojurePlugin {
    fn id(&self) -> &'static str { "clojure" }
    fn extensions(&self) -> &'static [&'static str] { &["clj", "cljs", "cljc", "edn"] }
    fn shebangs(&self) -> &'static [&'static str] { &[] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_lua::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "clojure");
        let mut w = ClojureWalker { source, facts: &mut facts, namespace: String::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct ClojureWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    namespace: String,
}

impl<'a> ClojureWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.namespace.is_empty() { simple.to_string() }
        else { format!("{}/{simple}", self.namespace) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "list_lit" => {
                if self.is_ns_form(node) {
                    self.extract_namespace(node);
                } else if self.is_defn_form(node) {
                    self.record_callable(node);
                } else {
                    self.record_call(node);
                }
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn is_ns_form(&self, node: Node) -> bool {
        if let Some(first) = node.child(0) {
            self.text(first) == "ns"
        } else {
            false
        }
    }

    fn is_defn_form(&self, node: Node) -> bool {
        if let Some(first) = node.child(0) {
            let text = self.text(first);
            text == "defn" || text == "defn-"
        } else {
            false
        }
    }

    fn extract_namespace(&mut self, node: Node) {
        // (ns name ...)
        if let Some(name_node) = node.child(1) {
            self.namespace = self.text(name_node).to_string();
            // Extract imports from :require vectors
            for i in 2..node.child_count() {
                if let Some(child) = node.child(i as u32) {
                    if self.text(child) == ":require" {
                        if let Some(next) = node.child((i + 1) as u32) {
                            self.extract_requires(next);
                        }
                    }
                }
            }
        }
    }

    fn extract_requires(&mut self, node: Node) {
        // Vector of requires: [lib1 lib2 ...]
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                let child = c.node();
                if child.kind() != "sym_lit" && child.kind() != "list_lit" {
                    if !c.goto_next_sibling() { break; }
                    continue;
                }
                let lib_name = self.text(child).to_string();
                if !lib_name.is_empty() && lib_name != "[" && lib_name != "]" {
                    self.facts.imports.push(ImportRecord {
                        kind: "require".to_string(),
                        path: lib_name,
                        alias: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
                if !c.goto_next_sibling() { break; }
            }
        }
    }

    fn record_callable(&mut self, node: Node) {
        // (defn name [...] ...)
        if let Some(name_node) = node.child(1) {
            let name = self.text(name_node).to_string();
            if name.is_empty() { return; }

            let qn = self.qn(&name);
            let variant = DefVariant::FreeFunction;
            let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
            self.facts.definitions.push(DefRecord {
                simple_name: name,
                qualified_name: qn,
                variant,
                start_line: sl,
                end_line: el,
                start_byte: node.start_byte() as u32,
                end_byte: node.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(node)),
                visibility: String::new(),
                attributes: Vec::new(),
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        // (func_name ...)
        if let Some(func_node) = node.child(0) {
            if func_node.kind() == "sym_lit" {
                let name = self.text(func_node).to_string();
                if name.is_empty() || name.starts_with(':') { return; }
                
                self.facts.references.push(RefRecord {
                    name,
                    receiver_hint: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_loads() {
        let p = ClojurePlugin;
        assert_eq!(p.id(), "clojure");
        assert_eq!(p.extensions(), &["clj", "cljs", "cljc", "edn"]);
        assert_eq!(p.resolver_kind(), ResolverKind::Custom);
    }
}
