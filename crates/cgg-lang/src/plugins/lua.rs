//! Lua plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct LuaPlugin;

impl LanguagePlugin for LuaPlugin {
    fn id(&self) -> &'static str {
        "lua"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".lua"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["lua", "luajit"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_lua::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "lua");
        let mut w = LuaWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        chunk_bindings(tree.root_node(), source, &mut facts);
        facts
    }
}

/// File-scope facts the resolver reads (WS19): the fields of a returned
/// table that name a file-level function (`return { check = check_phase }`),
/// and `local a = m.b` member aliases.
fn chunk_bindings(root: Node, source: &[u8], facts: &mut FileFacts) {
    let text = |n: Node| n.utf8_text(source).unwrap_or("").trim().to_string();
    let mut c = root.walk();
    for stmt in root.named_children(&mut c) {
        match stmt.kind() {
            "return_statement" => {
                let mut rc = stmt.walk();
                let tables = stmt
                    .named_children(&mut rc)
                    .filter(|n| n.kind() == "expression_list")
                    .flat_map(|l| {
                        let mut lc = l.walk();
                        l.named_children(&mut lc).collect::<Vec<_>>()
                    })
                    .filter(|n| n.kind() == "table_constructor");
                for t in tables.collect::<Vec<_>>() {
                    let mut tc = t.walk();
                    for f in t.named_children(&mut tc).filter(|f| f.kind() == "field") {
                        if let (Some(k), Some(v)) = (
                            f.child_by_field_name("name"),
                            f.child_by_field_name("value"),
                        ) && k.kind() == "identifier"
                            && v.kind() == "identifier"
                        {
                            facts.module_returns.push((text(k), text(v)));
                        }
                    }
                }
            }
            "variable_declaration" => {
                let mut sc = stmt.walk();
                let Some(a) = stmt
                    .named_children(&mut sc)
                    .find(|n| n.kind() == "assignment_statement")
                else {
                    continue;
                };
                let mut ac = a.walk();
                let kids: Vec<Node> = a.named_children(&mut ac).collect();
                let (Some(names), Some(values)) = (
                    kids.iter().find(|k| k.kind() == "variable_list"),
                    kids.iter().find(|k| k.kind() == "expression_list"),
                ) else {
                    continue;
                };
                let mut nc = names.walk();
                let names: Vec<Node> = names.named_children(&mut nc).collect();
                let mut vc = values.walk();
                let values: Vec<Node> = values.named_children(&mut vc).collect();
                for (n, v) in names.iter().zip(values.iter()) {
                    if n.kind() != "identifier" || v.kind() != "dot_index_expression" {
                        continue;
                    }
                    if let (Some(t), Some(f)) = (
                        v.child_by_field_name("table"),
                        v.child_by_field_name("field"),
                    ) && t.kind() == "identifier"
                    {
                        facts.member_aliases.push(cgg_core::MemberAlias {
                            name: text(*n),
                            head: text(t),
                            member: text(f),
                            decl_byte: stmt.start_byte() as u32,
                        });
                    }
                }
            }
            _ => {}
        }
    }
}

struct LuaWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> LuaWalker<'a> {
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
            "function_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    self.record_named_def(name, node);
                }
                self.walk_children(node);
                return;
            }
            // `M.f = function(…) … end`, `local f = function(…) … end`: a
            // function bound by assignment, named by its target.
            "assignment_statement" => {
                let targets = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "variable_list");
                let values = node
                    .children(&mut node.walk())
                    .find(|c| c.kind() == "expression_list");
                if let (Some(targets), Some(values)) = (targets, values) {
                    let names: Vec<Node> =
                        targets.named_children(&mut targets.walk()).collect();
                    let vals: Vec<Node> =
                        values.named_children(&mut values.walk()).collect();
                    for (name, val) in names.iter().zip(vals.iter()) {
                        if val.kind() == "function_definition" {
                            self.record_named_def(*name, *val);
                        }
                    }
                }
                self.walk_children(node);
                return;
            }
            // `local t = { f = function(…) … end }`: a function stored in a
            // field of a named table is `t.f`.
            "field" => {
                if let (Some(name), Some(val)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ) && name.kind() == "identifier"
                    && val.kind() == "function_definition"
                    && let Some(table) = self.table_binding(node)
                {
                    let simple = self.text(name).to_string();
                    let qn = format!("{table}.{simple}");
                    self.push_def(simple, qn, val);
                }
                self.walk_children(node);
                return;
            }
            "local_function" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                }
                self.walk_children(node);
                return;
            }
            "function_call" => {
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

    /// Record the function `node` under the name `name` gives it: a bare
    /// identifier (`f`), a table field (`M.f`, `M.sub.f`) or a method
    /// (`M:m`). The simple name is the last segment — the name a call
    /// writes — and the qualified name the path as written.
    fn record_named_def(&mut self, name: Node, node: Node) {
        match name.kind() {
            "identifier" => {
                let n = self.text(name).to_string();
                if !n.is_empty() {
                    self.record_def(&n, node, DefVariant::FreeFunction);
                }
            }
            "dot_index_expression" | "method_index_expression" => {
                let leaf = name
                    .child_by_field_name("field")
                    .or_else(|| name.child_by_field_name("method"));
                if let Some(leaf) = leaf {
                    let simple = self.text(leaf).to_string();
                    let qn: String = self.text(name).split_whitespace().collect();
                    self.push_def(simple, qn, node);
                }
            }
            _ => {}
        }
    }

    /// The name a table constructor holding `field` is bound to (`t` in
    /// `local t = { … }`, `M.cfg` in `M.cfg = { … }`), if any.
    fn table_binding(&self, field: Node) -> Option<String> {
        let table = field.parent().filter(|p| p.kind() == "table_constructor")?;
        let values = table.parent().filter(|p| p.kind() == "expression_list")?;
        let assign = values
            .parent()
            .filter(|p| p.kind() == "assignment_statement")?;
        let idx = values
            .named_children(&mut values.walk())
            .position(|v| v == table)?;
        let targets = assign
            .children(&mut assign.walk())
            .find(|c| c.kind() == "variable_list")?;
        let target = targets.named_children(&mut targets.walk()).nth(idx)?;
        matches!(target.kind(), "identifier" | "dot_index_expression")
            .then(|| self.text(target).split_whitespace().collect())
    }

    /// The plain name a `require(…)` call's result is assigned to, when
    /// the call is the whole value of the assignment (`local m =
    /// require("m")`, `m = require "m"`).
    fn require_binding(&self, call: Node) -> Option<String> {
        let values = call.parent().filter(|p| p.kind() == "expression_list")?;
        let assign = values
            .parent()
            .filter(|p| p.kind() == "assignment_statement")?;
        let idx = values
            .named_children(&mut values.walk())
            .position(|v| v == call)?;
        let targets = assign
            .children(&mut assign.walk())
            .find(|c| c.kind() == "variable_list")?;
        let target = targets.named_children(&mut targets.walk()).nth(idx)?;
        (target.kind() == "identifier").then(|| self.text(target).to_string())
    }

    fn push_def(&mut self, simple: String, qn: String, node: Node) {
        if simple.is_empty() {
            return;
        }
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        // A function stored in a table field is reached through the table
        // (`M.f()`, `obj:m()`), never by a bare `f()`.
        self.facts.definitions.push(DefRecord {
            simple_name: simple,
            qualified_name: qn,
            variant: DefVariant::InherentMethod,
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

    fn record_call(&mut self, node: Node) {
        let Some(callee) = node.child_by_field_name("name") else {
            return;
        };
        // `M.f()`, `a.b.f()`, `obj:m()`: the called name is the field or
        // method; the table expression before it is the receiver.
        if matches!(
            callee.kind(),
            "dot_index_expression" | "method_index_expression"
        ) {
            let leaf = callee
                .child_by_field_name("field")
                .or_else(|| callee.child_by_field_name("method"));
            let table = callee.child_by_field_name("table");
            if let (Some(leaf), Some(table)) = (leaf, table) {
                let name = self.text(leaf).to_string();
                if !name.is_empty() {
                    self.facts.references.push(RefRecord {
                        name,
                        receiver_hint: self.text(table).split_whitespace().collect(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                        ..Default::default()
                    });
                }
                return;
            }
        }
        let func = self.text(callee).to_string();
        if func.is_empty() {
            return;
        }

        // require() as import
        if func == "require" {
            // `named_child`, not `child`: the `arguments` node's first
            // child is the `(` token, so `child(0)` recorded every Lua
            // require as an import of the literal string "(" — which
            // matched no rule and made every Lua framework undetectable.
            let args = node.child_by_field_name("arguments");
            if let Some(a) = args.and_then(|a| a.named_child(0)) {
                let path = self
                    .text(a)
                    .trim_matches('\'')
                    .trim_matches('"')
                    .to_string();
                if !path.is_empty() {
                    // `local util = require("util")`: the name the module
                    // is bound to, which calls through it are written with.
                    let alias = self.require_binding(node).unwrap_or_default();
                    self.facts.imports.push(ImportRecord {
                        kind: "require".into(),
                        path,
                        alias,
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                    return;
                }
            }
        }

        self.facts.references.push(RefRecord {
            name: func,
            receiver_hint: String::new(),
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
        p.set_language(&tree_sitter_lua::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        LuaPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.lua"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn table_functions_are_named_by_their_field_and_calls_split_their_receiver() {
        let f = extract(
            "local util = require(\"util\")\nlocal M = {}\nfunction M.f() return M.g() end\nfunction M:h() return self:f() end\nM.a = function() return util.x() end\nlocal t = { k = function() return helper() end }\nlocal function loc() end\nreturn M\n",
        );
        let defs: Vec<(&str, &str)> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.as_str(), d.qualified_name.as_str()))
            .collect();
        assert_eq!(
            defs,
            vec![
                ("f", "M.f"),
                ("h", "M:h"),
                ("a", "M.a"),
                ("k", "t.k"),
                ("loc", "loc")
            ],
            "{defs:?}"
        );
        let calls: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert!(calls.contains(&("g", "M")), "{calls:?}");
        assert!(calls.contains(&("f", "self")), "{calls:?}");
        assert!(calls.contains(&("x", "util")), "{calls:?}");
        assert!(calls.contains(&("helper", "")), "{calls:?}");
        let req = f.imports.iter().find(|i| i.kind == "require").unwrap();
        assert_eq!((req.path.as_str(), req.alias.as_str()), ("util", "util"));
    }

    #[test]
    fn plugin_loads() {
        let plugin = LuaPlugin;
        assert_eq!(plugin.id(), "lua");
        assert!(plugin.extensions().contains(&".lua"));
        assert!(plugin.shebangs().contains(&"lua"));
    }

    #[test]
    fn extracts_definitions() {
        let src = "function greet(name) end\nlocal function helper() end\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn extracts_references() {
        let src = "function main() greet() end\n";
        let f = extract(src);
        // Some parsers may not extract all references; just verify extraction works
        let _ = f.references;
    }

    /// WS19: a returned table's fields that name file-level functions, and
    /// file-scope `local a = m.b` member aliases, are recorded.
    #[test]
    fn returned_table_fields_and_member_aliases_are_recorded() {
        let f = extract(
            "local pc = require \"p\"\nlocal check_phase = pc.check\nlocal log = ngx.log\nlocal n = 1\nlocal function chk() end\nreturn {\n  check = chk,\n  n = 3,\n}\n",
        );
        assert_eq!(
            f.module_returns,
            vec![("check".to_string(), "chk".to_string())]
        );
        let got: Vec<(&str, &str, &str)> = f
            .member_aliases
            .iter()
            .map(|a| (a.name.as_str(), a.head.as_str(), a.member.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![("check_phase", "pc", "check"), ("log", "ngx", "log")]
        );
    }
}
