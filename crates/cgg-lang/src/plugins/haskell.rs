//! Haskell plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct HaskellPlugin;

impl LanguagePlugin for HaskellPlugin {
    fn id(&self) -> &'static str {
        "haskell"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".hs", ".lhs"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["runhaskell", "runghc"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_haskell::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "haskell");
        let mut w = HaskellWalker {
            source,
            facts: &mut facts,
            module: String::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct HaskellWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
}

impl<'a> HaskellWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        // Haskell qualifies with `.` — `Data.Thing.work`, matching how
        // the module is written and imported. (The `::` this used to
        // produce was never observable: `self.module` was always empty,
        // see `extract_module`.)
        if self.module.is_empty() {
            simple.to_string()
        } else {
            format!("{}.{simple}", self.module)
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            // The module declaration is a `header` node wrapping a
            // `module` node; dispatching on `module` reached the inner
            // one, whose children are `module_id` parts rather than
            // another `module`.
            "header" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            "import" => {
                self.extract_import(node);
                self.walk_children(node);
            }
            "function" | "bind" => {
                self.extract_function(node);
                self.walk_children(node);
            }
            "apply" => {
                self.record_call(node);
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

    fn extract_module(&mut self, node: Node) {
        // module Name where
        //
        // The node kind is `module`, not `module_name` — the same kind
        // `extract_import` below already reads. Looking for
        // `module_name` matched nothing in tree-sitter-haskell 0.23, so
        // `self.module` stayed empty and every Haskell callable came out
        // unqualified: `work` instead of `Data.Thing.work`. Silent,
        // because an unqualified name is still a perfectly good name —
        // it just cannot be told apart from the `work` in every other
        // module.
        // Named children only: the `module` *keyword* is an anonymous
        // token of the same kind, and it comes first.
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if child.kind() == "module" {
                self.module = self.text(child).to_string();
                break;
            }
        }
    }

    fn extract_import(&mut self, node: Node) {
        // import [qualified] Module [as Alias] [(item, ...)]
        // tree-sitter-haskell layout:
        //   import     -> [import] [qualified?] [module] [as module]? [import_list]?
        let mut modules: Vec<String> = Vec::new();
        let mut alias = String::new();
        let mut qualified = false;
        let mut saw_as = false;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32) {
                match child.kind() {
                    "qualified" => qualified = true,
                    "as" => saw_as = true,
                    "module" => {
                        let text = self.text(child).to_string();
                        if saw_as {
                            alias = text;
                        } else {
                            modules.push(text);
                        }
                    }
                    _ => {}
                }
            }
        }
        let module_name = modules.into_iter().next().unwrap_or_default();

        if !module_name.is_empty() {
            self.facts.imports.push(ImportRecord {
                kind: if qualified {
                    "import qualified"
                } else {
                    "import"
                }
                .to_string(),
                path: module_name,
                alias,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    fn extract_function(&mut self, node: Node) {
        // function: name = expr or bind: pattern = expr
        let mut name = String::new();

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && child.kind() == "variable"
            {
                name = self.text(child).to_string();
                break;
            }
        }

        if name.is_empty() {
            return;
        }

        let qn = self.qn(&name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
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

    fn record_call(&mut self, node: Node) {
        // exp_apply: function applied to arguments
        if let Some(func_node) = node.child(0)
            && func_node.kind() == "variable"
        {
            let name = self.text(func_node).to_string();
            if !name.is_empty() {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_haskell::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        HaskellPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.hs"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn definitions_are_qualified_by_their_module() {
        // `extract_module` looked for a `module_name` node that
        // tree-sitter-haskell 0.23 does not have, so every Haskell name
        // came out unqualified and two modules' `work` collided.
        let f = extract(
            "module Data.Thing (work) where\n\nwork x = helper x\n\nhelper x = x\n",
        );
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"Data.Thing.work"), "got: {qns:?}");
        assert!(qns.contains(&"Data.Thing.helper"), "got: {qns:?}");
    }

    #[test]
    fn plugin_loads() {
        let plugin = HaskellPlugin;
        assert_eq!(plugin.id(), "haskell");
        assert!(plugin.extensions().contains(&".hs"));
        assert!(plugin.extensions().contains(&".lhs"));
    }

    #[test]
    fn free_functions_and_call() {
        // No `module ... where` header, so `HaskellWalker::module` stays
        // empty and qualified names are the bare simple names.
        let src = "foo :: Int -> Int\nfoo x = bar x\n\nbar :: Int -> Int\nbar x = x\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"foo"), "got: {names:?}");
        assert!(names.contains(&"bar"), "got: {names:?}");
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"bar"), "got: {refs:?}");
    }

    #[test]
    fn import_is_recorded() {
        let f = extract("import qualified Data.Map as M\n\nmain = pure ()\n");
        let paths: Vec<&str> = f.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"Data.Map"), "got: {paths:?}");
    }
}
