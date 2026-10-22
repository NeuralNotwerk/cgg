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
        ctx: &crate::ExtractCtx<'_>,
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
            reference_sites: ctx.reference_sites,
        };
        w.walk(tree.root_node());
        facts
    }
}

struct HaskellWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
    /// Record functions used as values (`value_ref`):
    /// `crate::ExtractCtx::reference_sites`.
    reference_sites: bool,
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
        if self.reference_sites {
            self.capture_value_positions(node);
        }
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

        // An argument-less binding whose value is not a lambda is data
        // (`limit = 10`, `defaultConfig = …`): a value naming it reads it,
        // never passes a function (`value_refs::prune`).
        if node.kind() == "bind" {
            let lambda = node
                .child_by_field_name("match")
                .and_then(|m| m.child_by_field_name("expression"))
                .is_some_and(|e| matches!(e.kind(), "lambda" | "lambda_case"));
            if !lambda {
                self.facts.data_members.push(name.clone());
            }
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

impl HaskellWalker<'_> {
    /// Functions used as values, when the run records reference sites: an
    /// application's arguments (`map f xs`), an operator's operands
    /// (`f <$> x`, `m >>= f`, `f . g`), a record field's value, list and
    /// tuple elements, and the right side of an argument-less binding
    /// (`where handler = f`). Never the head of an application, never a
    /// constructor. Bare names only, as calls are recorded. Names a
    /// parameter or local binds are dropped through the Haskell locals
    /// (`value_refs::prune`).
    fn capture_value_positions(&mut self, node: Node) {
        let mut values: Vec<Node> = Vec::new();
        let mut c = node.walk();
        if matches!(node.kind(), "apply" | "infix" | "list" | "tuple")
            && in_type_or_pattern(node)
        {
            return;
        }
        match node.kind() {
            "apply" => values.extend(node.child_by_field_name("argument")),
            "infix" => {
                values.extend(node.child_by_field_name("left_operand"));
                values.extend(node.child_by_field_name("right_operand"));
            }
            "field_update" => values.extend(node.child_by_field_name("expression")),
            "list" | "tuple" => {
                values.extend(node.children_by_field_name("element", &mut c));
            }
            "bind" => values.extend(
                node.child_by_field_name("match")
                    .and_then(|m| m.child_by_field_name("expression")),
            ),
            _ => return,
        }
        for v in values {
            self.value_ref_in(v, 0);
        }
    }

    fn value_ref_in(&mut self, v: Node, depth: u8) {
        if depth > 4 {
            return;
        }
        match v.kind() {
            "parens" => {
                if let Some(e) = v.named_child(0) {
                    self.value_ref_in(e, depth + 1);
                }
            }
            "variable" => {
                let name = self.text(v).to_string();
                if name.is_empty() {
                    return;
                }
                self.facts.references.push(RefRecord {
                    name,
                    receiver_hint: String::new(),
                    site_line: (v.start_position().row as u32) + 1,
                    site_byte: v.start_byte() as u32,
                    value_ref: true,
                    ..Default::default()
                });
            }
            _ => {}
        }
    }
}

/// Whether `n` is part of a type (a signature, a data, newtype or type
/// declaration, an instance or class head) or of a pattern (`Just f ->`,
/// `(a, b) <- m`): type variables and pattern variables are `variable`s,
/// and type and constructor applications `apply`s, there too.
fn in_type_or_pattern(n: Node) -> bool {
    let mut cur = n;
    while let Some(p) = cur.parent() {
        let is_field =
            |f: &str| p.child_by_field_name(f).is_some_and(|t| t.id() == cur.id());
        if is_field("type") || is_field("pattern") || is_field("patterns") {
            return true;
        }
        match p.kind() {
            "data_type" | "newtype" | "type_synomym" | "type_family"
            | "type_instance" | "data_family" | "data_instance" | "deriving"
            | "deriving_instance" | "instance" | "class" | "context"
            | "kind_signature" | "patterns" => return true,
            "match"
            | "declarations"
            | "instance_declarations"
            | "class_declarations"
            | "local_binds"
            | "haskell"
            | "exp"
            | "guards" => return false,
            _ => cur = p,
        }
    }
    false
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

    fn extract_with_refs(src: &str, refs: bool) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_haskell::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        HaskellPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(refs),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.hs"),
            &tree,
            src.as_bytes(),
        )
    }

    /// WS26b: a function used as a value is a reference site only when the
    /// run asks for them; an application's head is a call, never a value.
    #[test]
    fn function_values_are_reference_sites() {
        let src = "module M where\nh = map g1 [1]\nk = g2 <$> Just 1\nc = g3 . g4\nb = Just 1 >>= g5\nr = Cfg { handler = g6 }\nl = [g7, (g8)]\nt = (g9, 1)\nw = q\n  where q = g10\nap = head 1\n";
        let f = extract_with_refs(src, true);
        let got: Vec<(String, String)> = f
            .references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| (r.receiver_hint.clone(), r.name.clone()))
            .collect();
        for (recv, name) in [
            ("", "g1"),
            ("", "g2"),
            ("", "g3"),
            ("", "g4"),
            ("", "g5"),
            ("", "g6"),
            ("", "g7"),
            ("", "g8"),
            ("", "g9"),
            ("", "g10"),
            ("", "q"),
        ] {
            assert!(
                got.contains(&(recv.to_string(), name.to_string())),
                "missing {recv}.{name} in {got:?}"
            );
        }
        let names: Vec<&str> = got.iter().map(|(_, n)| n.as_str()).collect();
        for n in ["head", "map", "Just", "Cfg", "handler"] {
            assert!(!names.contains(&n), "{n} recorded: {got:?}");
        }
        assert!(
            !extract_with_refs(src, false)
                .references
                .iter()
                .any(|r| r.value_ref)
        );
    }

    /// WS26b near misses: pattern variables (`(r, s) <- m`, `Just g ->`)
    /// and type variables in a signature are not values.
    #[test]
    fn patterns_and_types_are_not_values() {
        let f = extract_with_refs(
            "module M where\nf :: forall pv. C pv => S pv -> m a\nf = do\n  (rv, sv) <- mk\n  case m of\n    Just gv -> gv\n",
            true,
        );
        let values: Vec<&str> = f
            .references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| r.name.as_str())
            .collect();
        for n in ["pv", "rv", "sv", "gv"] {
            assert!(!values.contains(&n), "{n}: {values:?}");
        }
    }
}
