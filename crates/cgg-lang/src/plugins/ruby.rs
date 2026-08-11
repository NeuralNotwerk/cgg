//! Ruby plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct RubyPlugin;

impl LanguagePlugin for RubyPlugin {
    fn id(&self) -> &'static str {
        "ruby"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".rb", ".rake", ".gemspec"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["ruby", "irb"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            impls: true,
            value_refs: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_ruby::LANGUAGE.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "ruby");
        let mut w = RubyWalker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct RubyWalker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// `class X < Y` superclasses **and** `include M` mixins of the
    /// enclosing class. Sidekiq declares its contract with `include
    /// Sidekiq::Job`, which is a call rather than a superclass, so both
    /// forms have to land in the same slot.
    bases: Vec<Vec<String>>,
}

impl<'a> RubyWalker<'a> {
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
            "class" | "singleton_class" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let mut bases = node
                    .child_by_field_name("superclass")
                    .map(|n| self.text(n).trim_start_matches('<').trim().to_string())
                    .filter(|s| !s.is_empty())
                    .into_iter()
                    .collect::<Vec<_>>();
                bases.extend(self.included_modules(node));
                self.bases.push(bases);
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                self.bases.pop();
                return;
            }
            "module" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
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
        if c.goto_first_child() {
            loop {
                self.walk(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_method(&mut self, node: Node, variant: DefVariant) {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let variant = if name == "initialize" {
            DefVariant::Constructor
        } else {
            variant
        };
        let qn = self.qn(&name);
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
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    /// Modules mixed in at the top of a class body — `include
    /// Sidekiq::Job`, `extend ActiveSupport::Concern`. Only direct
    /// children of the body, so a conditional `include` deep inside a
    /// method is not mistaken for a declaration.
    fn included_modules(&self, class_node: Node) -> Vec<String> {
        let mut out = Vec::new();
        let Some(body) = class_node.child_by_field_name("body") else {
            return out;
        };
        let mut cursor = body.walk();
        for stmt in body.named_children(&mut cursor) {
            if stmt.kind() != "call" {
                continue;
            }
            let m = stmt
                .child_by_field_name("method")
                .map(|n| self.text(n))
                .unwrap_or("");
            if m != "include" && m != "extend" && m != "prepend" {
                continue;
            }
            let Some(args) = stmt.child_by_field_name("arguments") else {
                continue;
            };
            let mut ac = args.walk();
            for a in args.named_children(&mut ac) {
                let t = self.text(a).trim();
                if !t.is_empty() && t.starts_with(char::is_uppercase) {
                    out.push(t.to_string());
                }
            }
        }
        out
    }

    fn record_call(&mut self, node: Node) {
        let method = node
            .child_by_field_name("method")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        let recv = node
            .child_by_field_name("receiver")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if method.is_empty() {
            return;
        }
        // require/require_relative -> import
        if (method == "require" || method == "require_relative") && recv.is_empty() {
            let arg = node
                .child_by_field_name("arguments")
                .and_then(|a| a.child(0))
                .map(|n| {
                    self.text(n)
                        .trim_matches('\'')
                        .trim_matches('"')
                        .to_string()
                })
                .unwrap_or_default();
            if !arg.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: "require".into(),
                    path: arg,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
            return;
        }
        let context = if recv.is_empty() {
            method.clone()
        } else {
            format!("{recv}.{method}")
        };
        self.facts.references.push(RefRecord {
            name: method,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
        // `config/routes.rb` is ordinary Ruby: `get 'photos', to:
        // 'photos#index'`. The target is a string, so nothing here
        // becomes an edge — only an entry node, once a rule supplies
        // the premise that Rails invokes it.
        let extra = super::registrar::capture(&self.ctx, node, self.source, &context);
        self.facts.references.extend(extra);
        self.extract_inline_handler(node, &context);
    }

    /// Name the `do … end` block of a route registration so it can be a
    /// handler.
    ///
    /// Sinatra writes `get "/x" do … end`: the body is an anonymous
    /// block, so there is no callable for a rule to mark and the whole
    /// framework enumerates nothing. Mirrors the JavaScript inline-handler
    /// case, and is gated the same way — on the registrar verb and on a
    /// leading string literal — so an ordinary `items.each do |i| … end`
    /// mints nothing.
    fn extract_inline_handler(&mut self, node: Node, context: &str) {
        if !self
            .ctx
            .is_registrar_verb(super::registrar::last_segment(context))
        {
            return;
        }
        let Some(route) = super::registrar::is_registration_shape(node, self.source)
        else {
            return;
        };
        for closure in super::registrar::inline_closures(node) {
            let line = (closure.start_position().row as u32) + 1;
            let simple = format!("handler_at_{line}");
            let qn = self.qn(&simple);
            self.facts.definitions.push(DefRecord {
                simple_name: simple.clone(),
                qualified_name: qn,
                variant: DefVariant::NamedClosure,
                start_line: line,
                end_line: (closure.end_position().row as u32) + 1,
                start_byte: closure.start_byte() as u32,
                end_byte: closure.end_byte() as u32,
                signature_hint: super::extract_signature(self.text(closure)),
                visibility: String::new(),
                attributes: vec!["synthetic".to_string()],
                ..Default::default()
            });
            self.facts.references.push(RefRecord {
                name: simple,
                receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                site_line: line,
                site_byte: closure.start_byte() as u32,
                context: context.to_string(),
                route: route.clone(),
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
        p.set_language(&tree_sitter_ruby::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        RubyPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.rb"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn class_methods() {
        let src = "class Service\n  def initialize(name)\n    @name = name\n  end\n  def run\n    greet\n  end\n  def self.create\n    Service.new\n  end\nend\n";
        let f = extract(src);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"Service::initialize"), "got: {qns:?}");
        assert!(qns.contains(&"Service::run"), "got: {qns:?}");
        assert!(qns.contains(&"Service::create"), "got: {qns:?}");
        assert!(f.definitions.iter().any(
            |d| d.simple_name == "initialize" && d.variant == DefVariant::Constructor
        ));
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name == "create"
                    && d.variant == DefVariant::StaticMethod)
        );
    }

    #[test]
    fn require_is_import() {
        let src = "require './helper'\nrequire_relative 'utils'\ndef f; end\n";
        let f = extract(src);
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "require" && i.path == "./helper")
        );
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "require" && i.path == "utils")
        );
    }

    #[test]
    fn call_expressions() {
        let src = "def f\n  greet('x')\n  obj.run\nend\n";
        let f = extract(src);
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "greet" && r.receiver_hint.is_empty())
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "run" && r.receiver_hint == "obj")
        );
    }
}
