//! Ruby plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::collections::HashSet;
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
    fn filenames(&self) -> &'static [&'static str] {
        &["Gemfile", "Rakefile", "config.ru"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["ruby", "irb", "jruby"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            dyn_uses: true,
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
            locals: vec![HashSet::new()],
        };
        w.walk(tree.root_node());
        if ctx.deadcode_signals {
            facts.dyn_uses = super::dynuse::extract(tree, source, "ruby");
        }
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
    /// Local variables in scope, innermost last. Ruby's own parser decides
    /// that a bare `x` is a variable when an assignment to `x` (or a
    /// parameter `x`) precedes it in the scope, and a method call
    /// otherwise; the grammar does not, so the walker does. `def`, `class`
    /// and `module` open a fresh scope; a block sees its enclosing one.
    locals: Vec<HashSet<String>>,
}

impl<'a> RubyWalker<'a> {
    /// Run `f` in a new local scope: fresh, or (`inherit`) a copy of the
    /// enclosing one, with the names `params` binds already in it.
    fn in_scope(
        &mut self,
        inherit: bool,
        params: Option<Node>,
        f: impl FnOnce(&mut Self),
    ) {
        let mut set = if inherit {
            self.locals.last().cloned().unwrap_or_default()
        } else {
            HashSet::new()
        };
        if let Some(p) = params {
            self.collect_param_names(p, &mut set);
        }
        self.locals.push(set);
        f(self);
        self.locals.pop();
    }

    /// The names a parameter list binds: its identifiers and the `name`
    /// of each optional/keyword/splat/block parameter, through
    /// destructuring — never a default value's expression.
    fn collect_param_names(&self, n: Node, out: &mut HashSet<String>) {
        let mut c = n.walk();
        for ch in n.named_children(&mut c) {
            match ch.kind() {
                "identifier" => {
                    out.insert(self.text(ch).to_string());
                }
                "destructured_parameter" => self.collect_param_names(ch, out),
                _ => {
                    if let Some(name) = ch.child_by_field_name("name") {
                        out.insert(self.text(name).to_string());
                    }
                }
            }
        }
    }

    /// Bind the variables an assignment target names (`x = …`,
    /// `a, (b, *c) = …`).
    fn bind_target(&mut self, n: Node) {
        match n.kind() {
            "identifier" => {
                let name = self.text(n).to_string();
                if let Some(top) = self.locals.last_mut() {
                    top.insert(name);
                }
            }
            "left_assignment_list"
            | "destructured_left_assignment"
            | "rest_assignment" => {
                let mut c = n.walk();
                let kids: Vec<Node> = n.named_children(&mut c).collect();
                for k in kids {
                    self.bind_target(k);
                }
            }
            _ => {}
        }
    }

    /// A bare identifier that is not a variable in scope: Ruby calls the
    /// method of that name with no arguments.
    fn record_bare_identifier(&mut self, node: Node) {
        let name = self.text(node);
        if name.is_empty() || self.locals.last().is_some_and(|l| l.contains(name)) {
            return;
        }
        self.facts.references.push(RefRecord {
            name: name.to_string(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            paren_less: true,
            ..Default::default()
        });
    }

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
                self.in_scope(false, None, |w| {
                    if !name.is_empty() {
                        w.scope.push(name);
                        w.walk_children(node);
                        w.scope.pop();
                    } else {
                        w.walk_children(node);
                    }
                });
                self.bases.pop();
                return;
            }
            "module" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                self.in_scope(false, None, |w| {
                    if !name.is_empty() {
                        w.scope.push(name);
                        w.walk_children(node);
                        w.scope.pop();
                    } else {
                        w.walk_children(node);
                    }
                });
                return;
            }
            "method" | "singleton_method" => {
                let variant = if node.kind() == "method" {
                    DefVariant::InherentMethod
                } else {
                    DefVariant::StaticMethod
                };
                self.record_method(node, variant);
                let params = node.child_by_field_name("parameters");
                self.in_scope(false, params, |w| w.walk_children(node));
                return;
            }
            "block" | "do_block" | "lambda" => {
                let params = node.child_by_field_name("parameters");
                self.in_scope(true, params, |w| w.walk_children(node));
                return;
            }
            "assignment" | "operator_assignment" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.bind_target(left);
                }
                self.walk_children(node);
                return;
            }
            "exception_variable" => {
                let mut c = node.walk();
                let kids: Vec<Node> = node.named_children(&mut c).collect();
                for k in kids {
                    self.bind_target(k);
                }
                return;
            }
            "for" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    self.bind_target(pattern);
                }
                self.walk_children(node);
                return;
            }
            "identifier" if bare_call_position(node) => {
                self.record_bare_identifier(node);
                return;
            }
            "call" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            // A bare `super`: the enclosing method's own name, up the
            // class's ancestry.
            "super" => {
                self.record_super(node);
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

    /// `super` / `super(…)`: a call of the enclosing method's name on
    /// the class's ancestors, never on the class itself.
    fn record_super(&mut self, node: Node) {
        let Some(name) = std::iter::successors(node.parent(), |n| n.parent())
            .find(|n| matches!(n.kind(), "method" | "singleton_method"))
            .and_then(|m| m.child_by_field_name("name"))
            .map(|n| self.text(n).to_string())
            .filter(|n| !n.is_empty())
        else {
            return;
        };
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: "super".to_string(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        if let Some(m) = node.child_by_field_name("method")
            && m.kind() == "super"
        {
            self.record_super(node);
            return;
        }
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
        // `(x).m`, `Foo.new.m`: the value or the class the receiver's
        // shape stands for (`crate::receiver`).
        let (recv, typed_receiver) = match node.child_by_field_name("receiver") {
            Some(r) => {
                let p = crate::receiver::peel("ruby", r, self.source);
                match p.type_name() {
                    Some(t) => (t, true),
                    None => (self.text(p.node).to_string(), false),
                }
            }
            None => (recv, false),
        };
        // `Foo.new(…)` builds a `Foo`: besides `new` itself (a class may
        // define `self.new`), it calls `Foo#initialize`, looked up on the
        // class and its ancestors.
        if let Some(ty) = crate::receiver::constructed_type("ruby", node, self.source) {
            self.facts.references.push(RefRecord {
                name: "initialize".to_string(),
                receiver_hint: ty,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
                typed_receiver: true,
                ..Default::default()
            });
        }
        self.facts.references.push(RefRecord {
            name: method,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver,
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
                arg_type: None,
                from_macro_arg: false,
                path_receiver: false,
                dot_call: false,
                arrow: false,
                macro_call: false,
                invocant: false,
                implicit_receiver: false,
                not_def: None,
                arity: None,
                value_ref: false,
                paren_less: false,
                typed_receiver: false,
                weak_receiver_type: false,
                name: simple,
                receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
                site_line: line,
                site_byte: closure.start_byte() as u32,
                context: context.to_string(),
                route: route.clone(),
                kwargs: Vec::new(),
            });
        }
    }
}

/// Whether an identifier stands where Ruby evaluates it: a statement, an
/// operand, an argument, an assigned value, a condition or branch, the
/// receiver of a call. Names, parameters and assignment targets are not.
fn bare_call_position(node: Node) -> bool {
    let Some(p) = node.parent() else {
        return false;
    };
    let is = |field: &str| p.child_by_field_name(field) == Some(node);
    match p.kind() {
        "body_statement"
        | "then"
        | "else"
        | "begin"
        | "ensure"
        | "parenthesized_statements"
        | "argument_list"
        | "interpolation"
        | "array"
        | "if_modifier"
        | "unless_modifier"
        | "while_modifier"
        | "until_modifier"
        | "splat_argument"
        | "hash_splat_argument"
        | "block_argument"
        | "block_body" => true,
        "binary" => is("left") || is("right"),
        "unary" => is("operand"),
        "assignment" | "operator_assignment" => is("right"),
        "if" | "unless" | "while" | "until" | "elsif" => is("condition"),
        "conditional" => is("condition") || is("consequence") || is("alternative"),
        "call" => is("receiver"),
        "element_reference" => is("object"),
        "pair" => is("value"),
        "range" => is("begin") || is("end"),
        _ => false,
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
    fn a_bare_identifier_that_is_no_local_is_a_paren_less_call() {
        // Ruby's parser rule: an identifier assigned (or a parameter)
        // earlier in the scope is a variable; otherwise a method call.
        let f = extract(
            "class Foo\n  def run(a)\n    x = 1\n    helper\n    x\n    a\n    y = helper + x\n    [1].each { |e| e; helper }\n  end\n  def other\n    x\n  end\nend\n",
        );
        let bare: Vec<(&str, u32)> = f
            .references
            .iter()
            .filter(|r| r.paren_less)
            .map(|r| (r.name.as_str(), r.site_line))
            .collect();
        assert_eq!(
            bare,
            vec![("helper", 4), ("helper", 7), ("helper", 8), ("x", 11)],
            "{bare:?}"
        );
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

    #[test]
    fn new_reaches_initialize_and_super_names_the_enclosing_method() {
        let f = extract(
            "class C < B\n  def initialize(a)\n    super\n  end\n  def run\n    super(1)\n    x = Mod::K.new(2)\n    y = k.new\n  end\nend\n",
        );
        let refs: Vec<(&str, &str, bool)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str(), r.typed_receiver))
            .collect();
        assert!(refs.contains(&("initialize", "super", false)), "{refs:?}");
        assert!(refs.contains(&("run", "super", false)), "{refs:?}");
        assert!(refs.contains(&("initialize", "Mod::K", true)), "{refs:?}");
        assert!(refs.contains(&("new", "Mod::K", false)), "{refs:?}");
        // Near miss: `new` on a value is an ordinary call.
        assert!(refs.contains(&("new", "k", false)), "{refs:?}");
        assert!(!refs.iter().any(|r| r.0 == "super"), "{refs:?}");
    }
}
