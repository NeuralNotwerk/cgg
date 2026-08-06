//! Clojure plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct ClojurePlugin;

impl LanguagePlugin for ClojurePlugin {
    fn id(&self) -> &'static str {
        "clojure"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".clj", ".cljs", ".cljc", ".edn"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_clojure_orchard::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "clojure");
        let mut w = ClojureWalker {
            source,
            facts: &mut facts,
            namespace: String::new(),
        };
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
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.namespace.is_empty() {
            simple.to_string()
        } else {
            format!("{}/{simple}", self.namespace)
        }
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
        if c.goto_first_child() {
            loop {
                self.walk(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
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
                "defn"
                    | "defn-"
                    | "defmacro"
                    | "defmacro-"
                    | "def"
                    | "defonce"
                    | "defprotocol"
                    | "definterface"
                    | "deftype"
                    | "defrecord"
                    | "defstruct"
                    | "defmulti"
                    | "defmethod"
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
                        let kw = n
                            .named_child(0)
                            .map(|k| self.text(k).to_string())
                            .unwrap_or_default();
                        if kw == ":require" || kw == ":use" || kw == ":import" {
                            // The rest of the form is a sequence of vec_lits / sym_lits.
                            for i in 1..n.named_child_count() {
                                if let Some(entry) = n.named_child(i as u32) {
                                    self.record_require_entry(entry, &kw);
                                }
                            }
                        }
                    }
                    if !c.goto_next_sibling() {
                        break;
                    }
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
                            if seen_as {
                                alias = t;
                                seen_as = false;
                            } else if path.is_empty() {
                                path = t;
                            }
                        } else if ch.kind() == "kwd_lit" && self.text(ch) == ":as" {
                            seen_as = true;
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                (path, alias)
            }
            _ => return,
        };
        if path.is_empty() {
            return;
        }
        self.facts.imports.push(ImportRecord {
            kind: kind.into(),
            path,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    fn record_callable(&mut self, node: Node) {
        // (defn name [...] ...)
        if let Some(name_node) = node.named_child(1) {
            let name = self.text(name_node).to_string();
            if name.is_empty() {
                return;
            }

            let qn = self.qn(&name);
            let variant = DefVariant::FreeFunction;
            let (sl, el) = (
                (node.start_position().row as u32) + 1,
                (node.end_position().row as u32) + 1,
            );
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
                ..Default::default()
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        // (func_name ...)
        if let Some(func_node) = node.named_child(0)
            && func_node.kind() == "sym_lit" {
                let name = self.text(func_node).to_string();
                if name.is_empty() || name.starts_with(':') {
                    return;
                }

                self.facts.references.push(RefRecord {
                    name,
                    receiver_hint: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                    ..Default::default()
                });
            }
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
        p.set_language(&tree_sitter_clojure_orchard::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        ClojurePlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/x.clj"),
            &tree,
            src.as_bytes(),
        )
    }

    fn defs(f: &FileFacts) -> Vec<String> {
        f.definitions
            .iter()
            .map(|d| d.qualified_name.clone())
            .collect()
    }

    #[test]
    fn plugin_loads() {
        let p = ClojurePlugin;
        assert_eq!(p.id(), "clojure");
        assert_eq!(p.extensions(), &[".clj", ".cljs", ".cljc", ".edn"]);
        assert!(p.shebangs().is_empty());
    }

    #[test]
    fn defn_is_a_callable() {
        let f = extract("(defn greet [n] (str \"hi \" n))\n");
        assert!(
            f.definitions.iter().any(|d| d.simple_name == "greet"),
            "defs: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn every_def_form_is_recognised() {
        // The plugin claims twelve `def*` heads; a head that silently
        // stops matching turns its whole construct invisible.
        let src = concat!(
            "(defn a [] 1)\n",
            "(defn- b [] 1)\n",
            "(defmacro c [] 1)\n",
            "(def d 1)\n",
            "(defonce e 1)\n",
            "(defprotocol f)\n",
            "(definterface g)\n",
            "(deftype h [x])\n",
            "(defrecord i [x])\n",
            "(defstruct j)\n",
            "(defmulti k identity)\n",
            "(defmethod l :x [_] 1)\n",
        );
        let f = extract(src);
        for name in ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"] {
            assert!(
                f.definitions.iter().any(|d| d.simple_name == name),
                "`{name}` was not captured; defs: {:?}",
                defs(&f)
            );
        }
    }

    #[test]
    fn the_namespace_qualifies_every_definition() {
        let f = extract("(ns my.app)\n(defn handler [] 1)\n");
        assert!(
            defs(&f).contains(&"my.app/handler".to_string()),
            "definitions must be namespace-qualified: {:?}",
            defs(&f)
        );
    }

    #[test]
    fn without_a_namespace_the_bare_name_is_used() {
        let f = extract("(defn handler [] 1)\n");
        assert!(defs(&f).contains(&"handler".to_string()), "{:?}", defs(&f));
    }

    #[test]
    fn require_with_an_alias_records_path_and_alias() {
        let f = extract("(ns my.app\n  (:require [foo.bar :as fb]))\n");
        let i = f
            .imports
            .iter()
            .find(|i| i.path == "foo.bar")
            .unwrap_or_else(|| panic!("imports: {:?}", f.imports));
        assert_eq!(i.kind, "require");
        assert_eq!(i.alias, "fb", "the `:as` alias must survive");
    }

    #[test]
    fn a_bare_require_vector_has_no_alias() {
        let f = extract("(ns my.app\n  (:require [baz]))\n");
        let i = f
            .imports
            .iter()
            .find(|i| i.path == "baz")
            .expect("baz imported");
        assert_eq!(i.kind, "require");
        assert!(i.alias.is_empty());
    }

    #[test]
    fn use_and_import_keep_their_own_kinds() {
        // `:use` and `:import` are different relationships from
        // `:require`; collapsing them would lose that in the audit.
        let f = extract("(ns my.app\n  (:use [qux])\n  (:import [java.util Date]))\n");
        assert!(
            f.imports.iter().any(|i| i.path == "qux" && i.kind == "use"),
            "imports: {:?}",
            f.imports
        );
        assert!(
            f.imports.iter().any(|i| i.kind == "import"),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn calls_inside_a_body_are_references() {
        let f = extract("(ns my.app)\n(defn outer [] (inner 1))\n(defn inner [x] x)\n");
        assert!(
            f.references.iter().any(|r| r.name == "inner"),
            "refs: {:?}",
            f.references.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn keywords_in_call_position_are_not_references() {
        // `(:key m)` is a map lookup, not a call to a function named
        // `:key`; recording it would invent a callee that cannot exist.
        let f = extract("(defn f [m] (:some-key m))\n");
        assert!(
            !f.references.iter().any(|r| r.name.starts_with(':')),
            "keyword lookups must not become references: {:?}",
            f.references.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn definition_spans_cover_the_whole_form() {
        // Spans drive `--since` seeding and dead-code line reporting, so
        // a multi-line form has to report its real extent.
        let f = extract("(defn multi\n  [x]\n  (+ x\n     1))\n");
        let d = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "multi")
            .unwrap();
        assert_eq!(d.start_line, 1);
        assert!(
            d.end_line >= 4,
            "end_line was {} for a 4-line form",
            d.end_line
        );
        assert!(d.end_byte > d.start_byte);
    }

    #[test]
    fn an_empty_file_yields_nothing_and_does_not_panic() {
        let f = extract("");
        assert!(f.definitions.is_empty());
        assert!(f.references.is_empty());
        assert!(f.imports.is_empty());
    }

    #[test]
    fn unbalanced_source_does_not_panic() {
        // tree-sitter yields an ERROR tree; extraction must survive it.
        let f = extract("(defn broken [x]\n  (+ x\n");
        let _ = defs(&f);
    }
}
