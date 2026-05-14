//! Verilog / SystemVerilog plugin — callable extraction.
//!
//! Definitions: `module_declaration`, `task_declaration`,
//! `function_declaration`. The "call graph" in Verilog comes mainly
//! from `module_instantiation` (one module wiring up another) plus
//! `system_tf_call` (`$display`, `$finish`, …). Plain task-enable
//! statements (`do_it(1);`) are sometimes misparsed by the grammar as
//! `data_declaration` and may not be captured.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct VerilogPlugin;

impl LanguagePlugin for VerilogPlugin {
    fn id(&self) -> &'static str { "verilog" }
    fn extensions(&self) -> &'static [&'static str] { &[".v", ".vh", ".sv", ".svh"] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_verilog::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "verilog");
        let mut w = VerilogWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct VerilogWalker<'a> { source: &'a [u8], facts: &'a mut FileFacts, scope: Vec<String> }

impl<'a> VerilogWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.into() } else { format!("{}.{simple}", self.scope.join(".")) }
    }

    /// Find the leaf `simple_identifier` somewhere under `node`. The
    /// grammar wraps task/function names in nested identifier nodes.
    fn first_simple_identifier(&self, node: Node) -> Option<String> {
        if node.kind() == "simple_identifier" {
            return Some(self.text(node).to_string());
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                if let Some(s) = self.first_simple_identifier(c.node()) { return Some(s); }
                if !c.goto_next_sibling() { break; }
            }
        }
        None
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "module_declaration" => {
                let name = node.children(&mut node.walk())
                    .find(|c| c.kind() == "module_header")
                    .and_then(|h| self.first_simple_identifier(h))
                    .unwrap_or_default();
                if !name.is_empty() {
                    let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
                    self.facts.definitions.push(DefRecord {
                        simple_name: name.clone(), qualified_name: self.qn(&name),
                        variant: DefVariant::FreeFunction,
                        start_line: sl, end_line: el,
                        start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
                        signature_hint: super::extract_signature(self.text(node)),
                        visibility: String::new(), attributes: vec!["module".into()],
                    });
                    self.scope.push(name); self.walk_children(node); self.scope.pop();
                } else { self.walk_children(node); }
                return;
            }
            "task_declaration" | "function_declaration" => {
                let id_kind = if node.kind() == "task_declaration" { "task_identifier" } else { "function_identifier" };
                let name = node.descendant_for_byte_range(node.start_byte(), node.end_byte())
                    .into_iter()
                    .find_map(|_| self.find_descendant(node, id_kind))
                    .and_then(|n| self.first_simple_identifier(n))
                    .unwrap_or_default();
                if !name.is_empty() {
                    let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
                    self.facts.definitions.push(DefRecord {
                        simple_name: name.clone(), qualified_name: self.qn(&name),
                        variant: if self.scope.is_empty() { DefVariant::FreeFunction } else { DefVariant::InherentMethod },
                        start_line: sl, end_line: el,
                        start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
                        signature_hint: super::extract_signature(self.text(node)),
                        visibility: String::new(),
                        attributes: vec![if node.kind() == "task_declaration" { "task".into() } else { "function".into() }],
                    });
                }
                self.walk_children(node);
                return;
            }
            // `checker_instantiation` is what the grammar emits when a
            // bare `foo bar();` is ambiguous with a module instance —
            // treat it the same way.
            "module_instantiation" | "checker_instantiation" | "program_instantiation"
            | "udp_instantiation" | "interface_instantiation" => {
                // First simple_identifier under module_instantiation is the
                // module-type name (e.g. `counter c0(...)` → "counter").
                if let Some(name) = self.first_simple_identifier(node) {
                    self.facts.references.push(RefRecord {
                        name, receiver_hint: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
                self.walk_children(node);
                return;
            }
            "system_tf_call" => {
                let name = node.children(&mut node.walk())
                    .find(|c| c.kind() == "system_tf_identifier")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.facts.references.push(RefRecord {
                        name, receiver_hint: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                }
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

    fn find_descendant<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if n.kind() == kind { return Some(n); }
            let mut c = n.walk();
            if c.goto_first_child() {
                loop { stack.push(c.node()); if !c.goto_next_sibling() { break; } }
            }
        }
        None
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
        p.set_language(&tree_sitter_verilog::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        VerilogPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/x.v"), &tree, src.as_bytes())
    }

    #[test]
    fn modules_are_callables() {
        let src = "module counter; endmodule\nmodule top;\n  counter c0();\nendmodule\n";
        let f = extract(src);
        let names: Vec<_> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
        assert!(names.contains(&"counter"), "got: {names:?}");
        assert!(names.contains(&"top"), "got: {names:?}");
        assert!(f.references.iter().any(|r| r.name == "counter"),
            "refs: {:?}", f.references);
    }

    #[test]
    fn task_and_function() {
        let src = "module M;\n  task do_it; endtask\n  function f; f = 0; endfunction\nendmodule\n";
        let f = extract(src);
        let names: Vec<_> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
        assert!(names.contains(&"do_it"), "got: {names:?}");
        assert!(names.contains(&"f"), "got: {names:?}");
    }

    #[test]
    fn system_call_captured() {
        let src = "module M; initial begin $display(\"x\"); end endmodule\n";
        let f = extract(src);
        assert!(f.references.iter().any(|r| r.name == "$display"),
            "refs: {:?}", f.references);
    }
}
