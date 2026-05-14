//! Clojure plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct ClojurePlugin;

impl LanguagePlugin for ClojurePlugin {
    fn id(&self) -> &'static str { "clojure" }
    fn extensions(&self) -> &'static [&'static str] { &[".clj", ".cljs", ".cljc", ".edn"] }
    fn shebangs(&self) -> &'static [&'static str] { &[] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_clojure_orchard::LANGUAGE.into() }

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
        if let Some(first) = node.named_child(0) {
            self.text(first) == "ns"
        } else {
            false
        }
    }

    fn is_defn_form(&self, node: Node) -> bool {
        if let Some(first) = node.named_child(0) {
            let text = self.text(first);
            matches!(
                text,
                "defn" | "defn-" | "defmacro" | "defmacro-"
                | "def" | "defonce"
                | "defprotocol" | "definterface"
                | "deftype" | "defrecord" | "defstruct"
                | "defmulti" | "defmethod"
            )
        } else {
            false
        }
    }

    fn extract_namespace(&mut self, node: Node) {
        // (ns name (:require [foo.bar :as fb] [baz]) (:use [qux]) ...)
        if let Some(name_node) = node.named_child(1) {
            self.namespace = self.text(name_node).to_string();
            // Each top-level form after the namespace name is its own
            // `list_lit` like `(:require ...)`. Walk siblings, look for
            // those that start with `:require` / `:use` / `:import`.
            let mut c = node.walk();
            if c.goto_first_child() {
                let _ = c.goto_next_sibling(); // past `(`
                let _ = c.goto_next_sibling(); // past `ns`
                let _ = c.goto_next_sibling(); // past name
                loop {
                    let n = c.node();
                    if n.kind() == "list_lit" {
                        // First inner child after `(` is the keyword.
                        let kw = n.named_child(0).map(|k| self.text(k).to_string()).unwrap_or_default();
                        if kw == ":require" || kw == ":use" || kw == ":import" {
                            // The rest of the form is a sequence of vec_lits / sym_lits.
                            for i in 1..n.named_child_count() {
                                if let Some(entry) = n.named_child(i as u32) {
                                    self.record_require_entry(entry, &kw);
                                }
                            }
                        }
                    }
                    if !c.goto_next_sibling() { break; }
                }
            }
        }
    }

    fn record_require_entry(&mut self, node: Node, kw: &str) {
        // Entry is either `sym_lit "foo.bar"` or `vec_lit "[foo.bar :as fb]"`.
        let kind = match kw {
            ":use" => "use",
            ":import" => "import",
            _ => "require",
        };
        let (path, alias) = match node.kind() {
            "sym_lit" => (self.text(node).to_string(), String::new()),
            "vec_lit" => {
                let mut path = String::new();
                let mut alias = String::new();
                let mut seen_as = false;
                let mut c = node.walk();
                if c.goto_first_child() {
                    loop {
                        let ch = c.node();
                        if ch.kind() == "sym_lit" {
                            let t = self.text(ch).to_string();
                            if seen_as { alias = t; seen_as = false; }
                            else if path.is_empty() { path = t; }
                        } else if ch.kind() == "kwd_lit" && self.text(ch) == ":as" {
                            seen_as = true;
                        }
                        if !c.goto_next_sibling() { break; }
                    }
                }
                (path, alias)
            }
            _ => return,
        };
        if path.is_empty() { return; }
        self.facts.imports.push(ImportRecord {
            kind: kind.into(), path, alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_callable(&mut self, node: Node) {
        // (defn name [...] ...)
        if let Some(name_node) = node.named_child(1) {
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
        if let Some(func_node) = node.named_child(0) {
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
        assert_eq!(p.extensions(), &[".clj", ".cljs", ".cljc", ".edn"]);
        assert_eq!(p.resolver_kind(), ResolverKind::Custom);
    }
}
