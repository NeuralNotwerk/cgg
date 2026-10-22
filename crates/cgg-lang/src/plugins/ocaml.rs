//! OCaml plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct OcamlPlugin;

impl LanguagePlugin for OcamlPlugin {
    fn id(&self) -> &'static str {
        "ocaml"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".ml", ".mli"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_ocaml::LANGUAGE_OCAML.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "ocaml");
        let mut w = OcamlWalker {
            source,
            facts: &mut facts,
            module: String::new(),
            lets: Vec::new(),
            reference_sites: ctx.reference_sites,
        };
        w.walk(tree.root_node());
        facts
    }
}

struct OcamlWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
    /// Enclosing `let` bindings, innermost last: name, start byte, and
    /// whether the name is in scope in its own body (`let rec`).
    lets: Vec<(String, u32, bool)>,
    /// Record functions used as values (`value_ref`):
    /// `crate::ExtractCtx::reference_sites`.
    reference_sites: bool,
}

impl<'a> OcamlWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.module.is_empty() {
            simple.to_string()
        } else {
            format!("{}::{simple}", self.module)
        }
    }

    fn walk(&mut self, node: Node) {
        // A `let`'s own value is read inside the binding (below).
        if self.reference_sites && node.kind() != "let_binding" {
            self.capture_value_positions(node);
        }
        match node.kind() {
            "module_definition" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            // `Alcotest.run`, `Dream.get`, `Cmdliner.Cmd.v`: OCaml reaches a
            // library through a qualified name, no `open` needed, so the
            // top module of every qualified path is recorded (once per file)
            // as a `module-use` import. It is what framework detection keys
            // on; the resolver ignores the kind.
            "value_path"
            | "constructor_path"
            | "field_path"
            | "type_constructor_path"
            | "module_path" => {
                self.record_module_use(node);
                self.walk_children(node);
            }
            "open_statement" | "open_module" => {
                self.extract_import(node);
                self.walk_children(node);
            }
            "let_binding" => {
                self.extract_function(node);
                let rec = node.parent().is_some_and(|p| {
                    let mut c = p.walk();
                    p.children(&mut c).any(|ch| ch.kind() == "rec")
                });
                let name = node
                    .child_by_field_name("pattern")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                self.lets.push((name, node.start_byte() as u32, rec));
                if self.reference_sites {
                    self.capture_value_positions(node);
                }
                self.walk_children(node);
                self.lets.pop();
            }
            "value_definition" => {
                self.extract_function(node);
                self.walk_children(node);
            }
            "application_expression" => {
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
        // module Name = struct ... end
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && child.kind() == "module_name"
            {
                self.module = self.text(child).to_string();
                break;
            }
        }
    }

    fn record_module_use(&mut self, node: Node) {
        let mut n = node;
        // Descend to the outermost module_name: `A.B.c` -> `A`.
        loop {
            let mut next = None;
            for i in 0..n.child_count() {
                if let Some(child) = n.child(i as u32)
                    && matches!(
                        child.kind(),
                        "module_path" | "extended_module_path" | "module_name"
                    )
                {
                    next = Some(child);
                    break;
                }
            }
            match next {
                Some(c) if c.kind() == "module_name" => {
                    let m = self.text(c).to_string();
                    if !m.is_empty()
                        && !self
                            .facts
                            .imports
                            .iter()
                            .any(|i| i.kind == "module-use" && i.path == m)
                    {
                        self.facts.imports.push(ImportRecord {
                            kind: "module-use".to_string(),
                            path: m,
                            alias: String::new(),
                            site_line: (c.start_position().row as u32) + 1,
                            site_byte: c.start_byte() as u32,
                        });
                    }
                    return;
                }
                Some(c) => n = c,
                None => return,
            }
        }
    }

    fn extract_import(&mut self, node: Node) {
        // open Module
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && (child.kind() == "module_path" || child.kind() == "module_name")
            {
                let module_name = self.text(child).to_string();
                if !module_name.is_empty() {
                    self.facts.imports.push(ImportRecord {
                        kind: "open".to_string(),
                        path: module_name,
                        alias: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
                break;
            }
        }
    }

    fn extract_function(&mut self, node: Node) {
        // let name args = expr or let name : type = expr
        let mut name = String::new();

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && (child.kind() == "value_name" || child.kind() == "identifier")
            {
                name = self.text(child).to_string();
                break;
            }
        }

        if name.is_empty() {
            return;
        }

        // A binding with no parameter whose value is not a `fun` or
        // `function` is data (`let name = …`): a value naming it reads
        // it, never passes a function (`value_refs::prune`).
        if node.kind() == "let_binding" && !self.binds_function(node) {
            self.facts.data_members.push(name.clone());
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
        // application: function applied to arguments
        if let Some(func_node) = node.child(0)
            && (func_node.kind() == "value_name"
                || func_node.kind() == "value_path"
                || func_node.kind() == "identifier")
        {
            let name = self.text(func_node).to_string();
            if !name.is_empty() {
                // `let f x = f (x + 1)` without `rec`: the innermost
                // binding of that name around the call does not see
                // itself (WS14b).
                let not_def = self
                    .lets
                    .iter()
                    .rev()
                    .find(|(n, _, _)| *n == name)
                    .filter(|(_, _, rec)| !rec)
                    .map(|(_, b, _)| *b);
                self.facts.references.push(RefRecord {
                    name,
                    receiver_hint: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                    not_def,
                    ..Default::default()
                });
            }
        }
    }
}

impl OcamlWalker<'_> {
    /// Whether a `let` binds a function: it has a parameter, or its value
    /// is a `fun` or `function` (through a type annotation too).
    fn binds_function(&self, node: Node) -> bool {
        let mut c = node.walk();
        if node.children(&mut c).any(|k| k.kind() == "parameter") {
            return true;
        }
        let mut body = node.child_by_field_name("body");
        while let Some(b) = body {
            match b.kind() {
                "fun_expression" | "function_expression" => return true,
                "parenthesized_expression" | "typed_expression" => {
                    body = b.named_child(0);
                }
                _ => return false,
            }
        }
        false
    }

    /// Functions used as values, when the run records reference sites: an
    /// application's arguments, labeled ones included (`List.map f xs`,
    /// `~f:g`), an operator's operands (`xs |> f`, `f @@ x`), a record
    /// field's value, list, array and tuple elements, and the value a
    /// `let` binds or returns (`let handler = f`, `let get () = f`). Never
    /// the head of an application. Recorded the way the call would be —
    /// the value path as written (`f`, `M.f`) — marked `value_ref`. Names
    /// a parameter or local binds are dropped through the OCaml locals
    /// (`value_refs::prune`).
    fn capture_value_positions(&mut self, node: Node) {
        let mut values: Vec<Node> = Vec::new();
        let mut c = node.walk();
        match node.kind() {
            "application_expression" => {
                values.extend(node.children_by_field_name("argument", &mut c));
            }
            "infix_expression" => {
                values.extend(node.child_by_field_name("left"));
                values.extend(node.child_by_field_name("right"));
            }
            "field_expression" | "let_binding" => {
                match node.child_by_field_name("body") {
                    Some(b) => values.push(b),
                    // A punned field (`{ M.encode; decode }`) is the value of
                    // the same-named binding in scope.
                    None if node.kind() == "field_expression" => {
                        if let Some(name) = node
                            .named_child(0)
                            .filter(|p| p.kind() == "field_path")
                            .and_then(|p| {
                                p.named_child(
                                    p.named_child_count().saturating_sub(1) as u32
                                )
                            })
                            .filter(|n| n.kind() == "field_name")
                        {
                            self.push_value(self.text(name).to_string(), name);
                        }
                    }
                    None => {}
                }
            }
            "list_expression" | "array_expression" | "tuple_expression" => {
                values.extend(node.named_children(&mut c));
            }
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
            "labeled_argument" => {
                if let Some(e) = v.child_by_field_name("expression") {
                    self.value_ref_in(e, depth + 1);
                }
            }
            "parenthesized_expression" => {
                if let Some(e) = v.child_by_field_name("expression") {
                    self.value_ref_in(e, depth + 1);
                }
            }
            "value_path" => {
                let name = self.text(v).to_string();
                self.push_value(name, v);
            }
            _ => {}
        }
    }

    /// Record `name` (as written at `at`) as a function value.
    fn push_value(&mut self, name: String, at: Node) {
        // A parenthesized operator (`( + )`) is no function of the tree.
        if name.is_empty() || name.contains(['(', ' ']) {
            return;
        }
        // `let f = f` without `rec` names the outer `f`, as a call does
        // (`record_call`).
        let not_def = self
            .lets
            .iter()
            .rev()
            .find(|(n, _, _)| *n == name)
            .filter(|(_, _, rec)| !rec)
            .map(|(_, b, _)| *b);
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: String::new(),
            site_line: (at.start_position().row as u32) + 1,
            site_byte: at.start_byte() as u32,
            not_def,
            value_ref: true,
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
        p.set_language(&tree_sitter_ocaml::LANGUAGE_OCAML.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        OcamlPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.ml"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn plugin_loads() {
        let plugin = OcamlPlugin;
        assert_eq!(plugin.id(), "ocaml");
        assert!(plugin.extensions().contains(&".ml"));
        assert!(plugin.extensions().contains(&".mli"));
    }

    #[test]
    fn let_bindings_and_call() {
        // No enclosing `module M = struct ... end`, so `OcamlWalker::module`
        // stays empty and qualified names are the bare simple names.
        let src = "let helper x = x + 1\n\nlet main () = helper 41\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(names.contains(&"helper"), "got: {names:?}");
        assert!(names.contains(&"main"), "got: {names:?}");
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"helper"), "got: {refs:?}");
    }

    #[test]
    fn open_is_recorded() {
        let f = extract("open Printf\n\nlet go () = ()\n");
        let paths: Vec<&str> = f.imports.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&"Printf"), "got: {paths:?}");
    }

    #[test]
    fn qualified_names_record_their_top_module_once() {
        let f = extract(
            "let t = Alcotest.run \"x\" [] ;; let u = Alcotest.check ;; let v = Cmdliner.Cmd.v i t\n",
        );
        let uses: Vec<_> = f
            .imports
            .iter()
            .filter(|i| i.kind == "module-use")
            .map(|i| i.path.as_str())
            .collect();
        assert_eq!(uses, ["Alcotest", "Cmdliner"], "{:?}", f.imports);
    }

    /// WS14b: a call to the name of the non-recursive `let` it sits in
    /// names that binding as one it cannot reach; under `let rec` it can.
    #[test]
    fn a_non_recursive_let_does_not_see_itself() {
        let f = extract(
            "let helper x = x + 1\nlet helper x = helper (x * 2)\n\
             let rec loop n = loop (n - 1)\n",
        );
        let second = f
            .definitions
            .iter()
            .filter(|d| d.simple_name == "helper")
            .nth(1)
            .map(|d| d.start_byte);
        let calls: Vec<(&str, Option<u32>)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.not_def))
            .collect();
        assert!(second.is_some(), "{:?}", f.definitions);
        assert_eq!(calls, vec![("helper", second), ("loop", None)], "{calls:?}");
    }

    fn extract_with_refs(src: &str, refs: bool) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_ocaml::LANGUAGE_OCAML.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        OcamlPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(refs),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.ml"),
            &tree,
            src.as_bytes(),
        )
    }

    /// WS26b: a function used as a value is a reference site only when the
    /// run asks for them; an application's head is a call, never a value.
    #[test]
    fn function_values_are_reference_sites() {
        let src = "let h = List.map g1 [1]\nlet k = [1] |> List.map g2 |> g3\nlet c = g4 @@ 1\nlet r = { handler = g5; other = M.g6; M.g14; g15 }\nlet l = [g7; g8]\nlet a = [| g9 |]\nlet t = (g10, 1)\nlet v = g11\nlet ret () = g12\nlet lab = run ~f:g13\nlet ap = head 1\n";
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
            ("", "M.g6"),
            ("", "g7"),
            ("", "g8"),
            ("", "g9"),
            ("", "g10"),
            ("", "g11"),
            ("", "g12"),
            ("", "g13"),
            ("", "g14"),
            ("", "g15"),
        ] {
            assert!(
                got.contains(&(recv.to_string(), name.to_string())),
                "missing {recv}.{name} in {got:?}"
            );
        }
        let names: Vec<&str> = got.iter().map(|(_, n)| n.as_str()).collect();
        for n in ["head", "run", "List.map", "Some"] {
            assert!(!names.contains(&n), "{n} recorded: {got:?}");
        }
        assert!(
            !extract_with_refs(src, false)
                .references
                .iter()
                .any(|r| r.value_ref)
        );
    }
}
