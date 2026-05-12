//! Ruby plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct RubyPlugin;

impl LanguagePlugin for RubyPlugin {
    fn id(&self) -> &'static str { "ruby" }
    fn extensions(&self) -> &'static [&'static str] { &[".rb", ".rake", ".gemspec"] }
    fn shebangs(&self) -> &'static [&'static str] { &["ruby", "irb"] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_ruby::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "ruby");
        let mut w = RubyWalker { source, facts: &mut facts, scope: Vec::new() };
        w.walk(tree.root_node());
        facts
    }
}

struct RubyWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> RubyWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() { simple.to_string() }
        else { format!("{}::{simple}", self.scope.join("::")) }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "class" => {
                let name = node.child_by_field_name("name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else { self.walk_children(node); }
                return;
            }
            "module" => {
                let name = node.child_by_field_name("name")
                    .map(|n| self.text(n).to_string()).unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else { self.walk_children(node); }
                return;
            }
            "method" => {
                self.record_method(node, DefVariant::InherentMethod);
                self.walk_children(node);
                return;
            }
            "singleton_method" => {
                self.record_method(node, DefVariant::StaticMethod);
                self.walk_children(node);
                return;
            }
            "call" => {
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

    fn record_method(&mut self, node: Node, variant: DefVariant) {
        let name = node.child_by_field_name("name")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        if name.is_empty() { return; }
        let variant = if name == "initialize" { DefVariant::Constructor } else { variant };
        let qn = self.qn(&name);
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name, qualified_name: qn, variant,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: self.text(node).lines().next().unwrap_or("").trim().trim_end_matches('{').trim_end_matches(':').trim().to_string(),
            visibility: String::new(), attributes: Vec::new(),
        });
    }

    fn record_call(&mut self, node: Node) {
        let method = node.child_by_field_name("method")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        let recv = node.child_by_field_name("receiver")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        if method.is_empty() { return; }
        // require/require_relative -> import
        if (method == "require" || method == "require_relative") && recv.is_empty() {
            let arg = node.child_by_field_name("arguments")
                .and_then(|a| a.child(0))
                .map(|n| self.text(n).trim_matches('\'').trim_matches('"').to_string())
                .unwrap_or_default();
            if !arg.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: "require".into(), path: arg, alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
            return;
        }
        self.facts.references.push(RefRecord {
            name: method, receiver_hint: recv,
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
        p.set_language(&tree_sitter_ruby::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        RubyPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.rb"), &tree, src.as_bytes())
    }

    #[test]
    fn class_methods() {
        let src = "class Service\n  def initialize(name)\n    @name = name\n  end\n  def run\n    greet\n  end\n  def self.create\n    Service.new\n  end\nend\n";
        let f = extract(src);
        let qns: Vec<&str> = f.definitions.iter().map(|d| d.qualified_name.as_str()).collect();
        assert!(qns.contains(&"Service::initialize"), "got: {qns:?}");
        assert!(qns.contains(&"Service::run"), "got: {qns:?}");
        assert!(qns.contains(&"Service::create"), "got: {qns:?}");
        assert!(f.definitions.iter().any(|d| d.simple_name == "initialize" && d.variant == DefVariant::Constructor));
        assert!(f.definitions.iter().any(|d| d.simple_name == "create" && d.variant == DefVariant::StaticMethod));
    }

    #[test]
    fn require_is_import() {
        let src = "require './helper'\nrequire_relative 'utils'\ndef f; end\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.kind == "require" && i.path == "./helper"));
        assert!(f.imports.iter().any(|i| i.kind == "require" && i.path == "utils"));
    }

    #[test]
    fn call_expressions() {
        let src = "def f\n  greet('x')\n  obj.run\nend\n";
        let f = extract(src);
        assert!(f.references.iter().any(|r| r.name == "greet" && r.receiver_hint.is_empty()));
        assert!(f.references.iter().any(|r| r.name == "run" && r.receiver_hint == "obj"));
    }
}
