//! Groovy plugin — callable extraction for Groovy and Gradle.
//!
//! Parses with the dekobon/tree-sitter-groovy grammar vendored under
//! `vendor/groovy` (its C symbols renamed `tree_sitter_groovy_dekobon*`;
//! see `vendor/groovy/PROVENANCE.md`), which reads Groovy's own forms —
//! command expressions, closures, `?.`/`*.`, method pointers, string-named
//! methods, slashy strings — as nodes of their own.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};
use tree_sitter_language::LanguageFn;

unsafe extern "C" {
    fn tree_sitter_groovy_dekobon() -> *const ();
}

/// The vendored grammar's C entry point, renamed from `tree_sitter_groovy`.
const GROOVY_LANGUAGE: LanguageFn =
    unsafe { LanguageFn::from_raw(tree_sitter_groovy_dekobon) };

/// The Groovy grammar.
pub(crate) fn language() -> tree_sitter::Language {
    GROOVY_LANGUAGE.into()
}

#[derive(Debug)]
pub struct GroovyPlugin;

impl LanguagePlugin for GroovyPlugin {
    fn id(&self) -> &'static str {
        "groovy"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".groovy", ".gradle"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        language()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "groovy");
        let mut w = GroovyWalker {
            bases: Vec::new(),
            source,
            facts: &mut facts,
            scope: Vec::new(),
            reference_sites: ctx.reference_sites,
        };
        w.walk(tree.root_node());
        facts
    }
}

/// Attribute of a method declared with a string name (`def "…"()`), which
/// the `spock` rule reads as a feature method.
const STRING_NAMED: &str = "string-named";

/// Type declarations: each names a scope for the members inside it.
const TYPE_DECLARATIONS: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "trait_declaration",
    "record_declaration",
    "annotation_type_declaration",
];

/// Member accesses through which a call names its method: the receiver is
/// `object`, the name `field` or `property`.
const MEMBER_ACCESSES: &[&str] = &[
    "field_access",
    "safe_navigation_expression",
    "safe_chain_dot_expression",
    "spread_dot_expression",
];

/// The name a method declaration or a member access writes: an identifier,
/// or the content of a string (`def "stops when …"()`, `obj.'NAME'()`),
/// with whether it was a string.
fn member_name(n: Node, src: &[u8]) -> (String, bool) {
    let text = n.utf8_text(src).unwrap_or("");
    if n.kind() == "quoted_identifier" {
        let inner = text
            .strip_prefix(['"', '\''])
            .and_then(|t| t.strip_suffix(['"', '\'']))
            .unwrap_or(text);
        (inner.to_string(), true)
    } else {
        (text.to_string(), false)
    }
}

/// Whether `n` is a statement that continues a command chain the grammar
/// splits (`module('a') tries('1')`, Groovy's `module('a').tries('1')`):
/// the statement before it ends on its line, with no `;` between them.
fn continues_a_command_chain(n: Node, src: &[u8]) -> bool {
    let stmt = match n.parent() {
        Some(p) if p.kind() == "expression_statement" => p,
        _ if n.kind() == "command_chain" => n,
        _ => return false,
    };
    let Some(prev) = stmt.prev_sibling() else {
        return false;
    };
    if !matches!(prev.kind(), "expression_statement" | "command_chain") {
        return false;
    }
    let between = src.get(prev.end_byte()..stmt.start_byte()).unwrap_or(&[]);
    !between.is_empty() && between.iter().all(|b| matches!(b, b' ' | b'\t'))
}

struct GroovyWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Supertypes of the enclosing class, innermost last.
    bases: Vec<Vec<String>>,
    /// Record method pointers as value references (WS26); see
    /// `crate::ExtractCtx`.
    reference_sites: bool,
}

impl<'a> GroovyWalker<'a> {
    fn text(&self, n: Node) -> &'a str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "package_declaration" => {
                if let Some(name) = node.child_by_field_name("name") {
                    self.scope.push(self.text(name).trim().to_string());
                }
                return;
            }
            "import_declaration" => {
                self.record_import(node);
                return;
            }
            k if TYPE_DECLARATIONS.contains(&k) => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let class_bases = super::attrs::base_types(node, self.source);
                let named = !name.is_empty();
                if named {
                    self.scope.push(name);
                }
                self.bases.push(class_bases);
                self.walk_children(node);
                self.bases.pop();
                if named {
                    self.scope.pop();
                }
                return;
            }
            "method_declaration" => self.record_method(node),
            // `Box(int x) { … }`: a constructor, named after its class as
            // Java's is, so `new Box(…)` and `this(…)` reach it.
            "constructor_declaration" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n);
                    if !name.is_empty() {
                        self.record_def(node, name, DefVariant::Constructor);
                    }
                }
            }
            // `new Box(…)`: a call of `Box`'s constructor, looked up on the
            // type as a construction receiver is.
            "object_creation_expression" => self.record_construction(node),
            "method_invocation" => self.record_invocation(node),
            "command_chain" => self.record_command(node),
            // `def t = make()`: the bare call's result, recorded as
            // `make()` for type propagation to read the callee's declared
            // return type (WS23). An untyped field's too (WS27): type
            // propagation reads a declaration no local binding records as
            // a field, visible where no local or parameter of its name is.
            "local_variable_declaration" | "field_declaration" => {
                self.record_call_result_locals(node);
            }
            // `this.&m`, `obj.&m`, `Util.&m`: a method pointer.
            "method_pointer_expression" if self.reference_sites => {
                if let (Some(recv), Some(name)) = (
                    node.child_by_field_name("object"),
                    node.child_by_field_name("method"),
                ) {
                    self.push_pointer(recv, name);
                }
            }
            // Groovy 3 `this::m` / `Util::m`.
            "method_reference_expression" if self.reference_sites => {
                if let (Some(recv), Some(name)) = (
                    node.child_by_field_name("target"),
                    node.child_by_field_name("name"),
                ) && name.kind() == "identifier"
                {
                    self.push_pointer(recv, name);
                }
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

    fn record_method(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let (simple, string_named) = member_name(name_node, self.source);
        if simple.is_empty() {
            return;
        }
        let keyword = |kw: &str| {
            node.children(&mut node.walk())
                .any(|c| !c.is_named() && c.kind() == kw)
        };
        // A method outside every type is a script's own function.
        let variant = if self.bases.is_empty()
            && node
                .parent()
                .is_some_and(|p| matches!(p.kind(), "source_file" | "block"))
        {
            DefVariant::FreeFunction
        } else if keyword("static") {
            DefVariant::StaticMethod
        } else {
            DefVariant::InherentMethod
        };
        let from = self.facts.definitions.len();
        self.record_def(node, &simple, variant);
        // `def "…"()`: a Spock feature method (`spock` rule).
        if string_named && let Some(d) = self.facts.definitions.get_mut(from) {
            d.attributes.push(STRING_NAMED.to_string());
        }
        // An interface or `abstract` method: a declaration. An
        // `@interface` element (`String value() default 'x'`) has no body
        // either, and declares nothing a call runs.
        super::attrs::mark_abstract_from(
            &mut self.facts.definitions,
            from,
            node.child_by_field_name("body").is_none() && !keyword("native"),
        );
    }

    fn record_def(&mut self, node: Node, simple: &str, variant: DefVariant) {
        let qn = self.qn(simple);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        let bases = self.bases.last().cloned().unwrap_or_default();
        self.facts.definitions.push(DefRecord {
            base_types: bases,
            simple_name: simple.to_string(),
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // Annotations are `@FunctionName("X")` here exactly as in
            // Java, and the shared collector reads them the same way.
            attributes: super::attrs::collect_with_preceding(node, self.source),
            ..Default::default()
        });
    }

    /// `new Box(…)` / `new pkg.Box<T>(…)`: the constructor is named after
    /// the type and looked up on it, as a typed receiver is. An anonymous
    /// class body (`new Base() { … }`) still constructs `Base`.
    fn record_construction(&mut self, node: Node) {
        let Some(ty) = node.child_by_field_name("type") else {
            return;
        };
        let path = self.text(ty);
        let path = path.split('<').next().unwrap_or(path).trim().to_string();
        let name = crate::receiver::type_simple_name(&path).to_string();
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: path,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver: true,
            arity: node.child_by_field_name("arguments").map(super::arg_count),
            ..Default::default()
        });
    }

    fn record_import(&mut self, node: Node) {
        let Some(name) = node.child_by_field_name("name") else {
            return;
        };
        let path = self.text(name).to_string();
        if path.is_empty() {
            return;
        }
        let token = |kw: &str| {
            node.children(&mut node.walk())
                .any(|c| !c.is_named() && c.kind() == kw)
        };
        let wildcard = token(".*");
        let alias = match node.child_by_field_name("alias") {
            Some(a) => self.text(a).to_string(),
            // `import static a.b.C.*` imports every static member of `C`:
            // recorded as `*`, not as the class name, as Java's is.
            None if wildcard && token("static") => "*".to_string(),
            None => path.rsplit('.').next().unwrap_or("").to_string(),
        };
        self.facts.imports.push(ImportRecord {
            kind: if token("static") {
                "from-import"
            } else {
                "import"
            }
            .into(),
            path,
            alias,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }

    /// `f(a)`, `obj.m(a) { … }`, `a?.m()`, `xs*.m()`, `obj."name"()`, and
    /// `this(…)`/`super(…)` in a constructor.
    fn record_invocation(&mut self, node: Node) {
        let Some(callee) = node.child_by_field_name("function") else {
            return;
        };
        let args = node.child_by_field_name("arguments");
        // `this(…)`/`super(…)`: a constructor's delegation, or a call of
        // the object itself (`this()` invokes its `call` method), never a
        // method named `this` or `super`.
        if callee.kind() == "identifier" && matches!(self.text(callee), "this" | "super")
        {
            if let Some(r) = self.constructor_delegation(node, callee, args) {
                self.facts.references.push(r);
            }
            return;
        }
        // The argument count, for overload narrowing (WS23): the listed
        // arguments plus a trailing closure. Named arguments (`f(a: 1)`)
        // are collected into one map parameter, and a spread passes an
        // unknown number, so neither has a count.
        let countable = args.is_none_or(|a| {
            !a.has_error()
                && a.named_children(&mut a.walk())
                    .all(|c| !uncountable(c.kind()))
        });
        let arity = countable.then(|| {
            args.map_or(0, super::arg_count)
                + u32::from(node.child_by_field_name("closure").is_some())
        });
        self.push_call(node, callee, arity);
    }

    /// `println x`, `foo 'a', b: 1`, `obj.m arg`: a call without
    /// parentheses, its `receiver` the callee.
    fn record_command(&mut self, node: Node) {
        let Some(callee) = node.child_by_field_name("receiver") else {
            return;
        };
        let mut c = node.walk();
        let args: Vec<Node> = node.children_by_field_name("argument", &mut c).collect();
        let arity = (!node.has_error() && args.iter().all(|a| !uncountable(a.kind())))
            .then_some(args.len() as u32);
        self.push_call(node, callee, arity);
    }

    /// A call of `callee`, a name or a member access, at `node`.
    fn push_call(&mut self, node: Node, callee: Node, arity: Option<u32>) {
        // The site is the method's name: the links of a chain
        // (`a.b()\n  .c()\n  .c()`) all start where `a` does, and would
        // otherwise share one site.
        let (name, recv, site) = match callee.kind() {
            "identifier" => (self.text(callee).to_string(), None, node),
            k if MEMBER_ACCESSES.contains(&k) => {
                let Some(field) = callee
                    .child_by_field_name("field")
                    .or_else(|| callee.child_by_field_name("property"))
                else {
                    return;
                };
                (
                    member_name(field, self.source).0,
                    callee.child_by_field_name("object"),
                    field,
                )
            }
            _ => return,
        };
        if name.is_empty() {
            return;
        }
        // `(f).m()`, `((Foo) o).m()`, `new Foo().m()`: the value or the
        // type the receiver's shape stands for (`crate::receiver`).
        let mut typed_receiver = false;
        let receiver_hint = recv
            .map(|n| {
                let p = crate::receiver::peel("groovy", n, self.source);
                match p.type_name() {
                    Some(t) => {
                        typed_receiver = true;
                        t
                    }
                    None => self.text(p.node).to_string(),
                }
            })
            .unwrap_or_default();
        // `module('a') tries('1')`: Groovy calls `tries` on the result of
        // `module('a')`, a method call, never a bare one.
        let dot_call = recv.is_none() && continues_a_command_chain(node, self.source);
        self.facts.references.push(RefRecord {
            name,
            receiver_hint,
            site_line: (site.start_position().row as u32) + 1,
            site_byte: site.start_byte() as u32,
            typed_receiver,
            arity,
            dot_call,
            ..Default::default()
        });
    }

    /// `this(…)` calls one of the class's own constructors, named after
    /// the class as `new Box(…)` is; `super(…)` one of the superclass's,
    /// recorded as the `super` call it is so the base chain resolves it.
    /// Neither is ever the constructor it is written in. Only as a
    /// constructor's statement: elsewhere `this(…)` calls the object.
    fn constructor_delegation(
        &self,
        node: Node,
        kw: Node,
        args: Option<Node>,
    ) -> Option<RefRecord> {
        let stmt = node
            .parent()
            .filter(|p| p.kind() == "expression_statement")?;
        let ctor = stmt
            .parent()
            .filter(|b| b.kind() == "block")?
            .parent()
            .filter(|c| c.kind() == "constructor_declaration")?;
        let class = ctor.parent().and_then(|b| b.parent()).filter(|c| {
            matches!(
                c.kind(),
                "class_declaration" | "enum_declaration" | "record_declaration"
            )
        })?;
        let (name, receiver_hint) = match self.text(kw) {
            "this" => (
                self.text(class.child_by_field_name("name")?).to_string(),
                String::new(),
            ),
            _ => {
                let mut c = class.walk();
                let sup = class
                    .children(&mut c)
                    .find(|n| n.kind() == "superclass")
                    .and_then(|s| s.named_child(0))?;
                let name = crate::receiver::type_simple_name(self.text(sup)).to_string();
                (name, "super".to_string())
            }
        };
        if name.is_empty() {
            return None;
        }
        Some(RefRecord {
            name,
            receiver_hint,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            arity: args.map(super::arg_count),
            not_def: Some(ctor.start_byte() as u32),
            ..Default::default()
        })
    }

    /// `def t = make()` (a local of any type, or an untyped field): the
    /// local `t` typed as the result of the bare call `make()`. A
    /// declaration holding an ERROR is not read.
    fn record_call_result_locals(&mut self, node: Node) {
        if crate::locals::groovy_declaration_has_error(node)
            || (node.kind() == "field_declaration"
                && node.child_by_field_name("type").is_some())
        {
            return;
        }
        let mut pairs: Vec<(Node, Node)> = Vec::new();
        if let (Some(n), Some(v)) = (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        ) {
            pairs.push((n, v));
        }
        let mut c = node.walk();
        for d in node.named_children(&mut c) {
            if d.kind() == "variable_declarator"
                && let (Some(n), Some(v)) = (
                    d.child_by_field_name("name"),
                    d.child_by_field_name("value"),
                )
            {
                pairs.push((n, v));
            }
        }
        for (name, value) in pairs {
            if value.kind() == "method_invocation"
                && let Some(callee) = value.child_by_field_name("function")
                && callee.kind() == "identifier"
            {
                self.facts.local_types.push(cgg_core::LocalType {
                    var_name: self.text(name).to_string(),
                    type_name: format!("{}()", self.text(callee)),
                    scope_byte: node.start_byte() as u32,
                    type_module: String::new(),
                });
            }
        }
    }

    /// A method pointer: a value reference to `name` through `recv` —
    /// `this`, a type (`Util`), or a value bound only through its type.
    fn push_pointer(&mut self, recv: Node, name: Node) {
        let p = crate::receiver::peel("groovy", recv, self.source);
        let (receiver_hint, typed_receiver) = match p.type_name() {
            Some(t) => (t, true),
            None => (self.text(p.node).to_string(), false),
        };
        let (site_line, site_byte) = (
            (name.start_position().row as u32) + 1,
            name.start_byte() as u32,
        );
        let name = self.text(name).to_string();
        if name.is_empty() || receiver_hint.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint,
            site_line,
            site_byte,
            typed_receiver,
            value_ref: true,
            ..Default::default()
        });
    }
}

/// Arguments that make a call's count unknown: named arguments, collected
/// into one map parameter, and spreads.
fn uncountable(kind: &str) -> bool {
    matches!(
        kind,
        "named_argument" | "spread_arguments" | "spread_map_entry"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract_ctx(src: &str, ctx: &crate::ExtractCtx<'_>) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&language()).unwrap();
        let tree = p.parse(src, None).unwrap();
        GroovyPlugin.extract(
            ctx,
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.groovy"),
            &tree,
            src.as_bytes(),
        )
    }

    fn extract(src: &str) -> FileFacts {
        extract_ctx(src, &crate::ExtractCtx::plain())
    }

    fn extract_with_refs(src: &str) -> FileFacts {
        extract_ctx(src, &crate::ExtractCtx::plain().with_reference_sites(true))
    }

    /// WS26: `this.&m`, `obj.&m`, `Util.&m` (also inside arguments and
    /// command expressions) and `this::m` are method pointers; a bitwise
    /// `a & b` is not.
    #[test]
    fn method_pointers_are_value_references() {
        let f = extract_with_refs(
            "class A {\n\
             \x20 def m(x) { x }\n\
             \x20 def run(List xs, B obj, int a, int b) {\n\
             \x20   def f = this.&m\n\
             \x20   xs.each(this.&m)\n\
             \x20   def g = obj.&go\n\
             \x20   def h = Util.&helper\n\
             \x20   def k = this::m\n\
             \x20   def bits = a & b\n\
             \x20   def c = this.&curried.curry(1)\n\
             \x20   def fa = obj.field\n\
             \x20   return f\n\
             \x20 }\n\
             }\n",
        );
        let got: Vec<(&str, &str)> = f
            .references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("m", "this"),
                ("m", "this"),
                ("go", "obj"),
                ("helper", "Util"),
                ("m", "this"),
                ("curried", "this"),
            ]
        );
        let f = extract_with_refs(
            "class A {\n  def b() { owner.onFailure this.&onFailure }\n  def c() { foo Util.&bar, 1 }\n}\n",
        );
        let got: Vec<(&str, &str)> = f
            .references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert_eq!(got, vec![("onFailure", "this"), ("bar", "Util")]);
        let plain = extract("class A { def run() { def f = this.&m } }\n");
        assert!(plain.references.iter().all(|r| !r.value_ref));
    }

    /// WS30: a Spock feature method named by a string is a method under
    /// its string name, marked `string-named`, with the rest of the class
    /// and its calls intact.
    #[test]
    fn string_named_methods_are_methods() {
        let src = "package p\n\
                   import spock.lang.Specification\n\
                   class BuildSpec extends Specification {\n\
                   \x20 def setup() { prepare() }\n\
                   \x20 def \"stops when the build fails\"() {\n\
                   \x20   when:\n\
                   \x20   run()\n\
                   \x20   then:\n\
                   \x20   check()\n\
                   \x20 }\n\
                   \x20 void 'handles a single-quoted name'() { other() }\n\
                   \x20 def helper() { 1 }\n\
                   }\n";
        let f = extract(src);
        let got: Vec<(&str, bool)> = f
            .definitions
            .iter()
            .map(|d| {
                (
                    d.qualified_name.as_str(),
                    d.attributes.iter().any(|a| a == STRING_NAMED),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                ("p.BuildSpec.setup", false),
                ("p.BuildSpec.stops when the build fails", true),
                ("p.BuildSpec.handles a single-quoted name", true),
                ("p.BuildSpec.helper", false),
            ]
        );
        let calls: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        for c in ["prepare", "run", "check", "other"] {
            assert!(calls.contains(&c), "{c}: {calls:?}");
        }
        assert!(GroovyPlugin.parse_source(src.as_bytes()).is_none());
    }

    #[test]
    fn class_methods_with_package() {
        let src = "package com.example\nclass Service {\n  void run() {}\n  int process(String s) { return 0 }\n}\n";
        let f = extract(src);
        let qns: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.contains(&"com.example.Service.run"), "got: {qns:?}");
        assert!(qns.contains(&"com.example.Service.process"), "got: {qns:?}");
    }

    /// Calls with and without parentheses, through `.`, `?.`, `*.` and a
    /// string name, and their argument counts.
    #[test]
    fn calls_in_every_form() {
        let src = "class C {\n  void f(B obj, List xs) {\n    helper(); obj.run(1) { it }\n    \
                   println x\n    foo 'a', b: 1\n    obj?.safe()\n    xs*.each()\n    \
                   obj.\"quoted\"()\n    obj.cmd 1, 2\n    def r = script.sh script: 'x'\n  }\n}\n";
        let f = extract(src);
        let got: Vec<(&str, &str, Option<u32>)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str(), r.arity))
            .collect();
        assert_eq!(
            got,
            vec![
                ("helper", "", Some(0)),
                ("run", "obj", Some(2)),
                ("println", "", Some(1)),
                ("foo", "", None),
                ("safe", "obj", Some(0)),
                ("each", "xs", Some(0)),
                ("quoted", "obj", Some(0)),
                ("cmd", "obj", Some(2)),
                ("sh", "script", None),
            ]
        );
    }

    /// WS27: a chain continued on a new line is a member access; the call
    /// on it is a method call. A command chain the grammar splits into
    /// statements (`module('a') tries('1')`) calls the later names on the
    /// earlier results: method calls, not bare ones.
    #[test]
    fn continued_chains_are_method_calls() {
        let src = "class C {\n  def f(commandLine) {\n    def e = new Builder()\n      \
                   .commandLine(commandLine)\n      ?.run()\n    helper()\n    \
                   def r = 0..size()\n    module('a') tries('1')\n    a(); b()\n  }\n}\n";
        let f = extract(src);
        let r = |n: &str| f.references.iter().find(|r| r.name == n).unwrap();
        assert_eq!(r("commandLine").receiver_hint, "Builder");
        assert!(r("commandLine").typed_receiver);
        assert!(!r("helper").dot_call);
        assert!(!r("size").dot_call);
        assert!(!r("module").dot_call);
        assert!(r("tries").dot_call);
        assert!(!r("b").dot_call);
        // Each link of a chain is its own site, on its own line.
        let f = extract("def f(rule) {\n  rule.reset()\n    .with(1)\n    .with(2)\n}\n");
        let sites: Vec<u32> = f
            .references
            .iter()
            .filter(|r| r.name == "with")
            .map(|r| r.site_line)
            .collect();
        assert_eq!(sites, vec![4, 3]);
    }

    fn defs(f: &FileFacts) -> Vec<String> {
        f.definitions
            .iter()
            .map(|d| d.qualified_name.clone())
            .collect()
    }

    #[test]
    fn imports_plain_static_aliased_and_wildcard() {
        let f = extract(
            "import com.example.Service\nimport static java.lang.Math.max\n\
             import a.b.C as D\nimport static a.b.U.*\nimport x.y.*\nclass C { void f() {} }\n",
        );
        let got: Vec<(&str, &str, &str)> = f
            .imports
            .iter()
            .map(|i| (i.kind.as_str(), i.path.as_str(), i.alias.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("import", "com.example.Service", "Service"),
                ("from-import", "java.lang.Math.max", "max"),
                ("import", "a.b.C", "D"),
                ("from-import", "a.b.U", "*"),
                ("import", "x.y", "y"),
            ]
        );
    }

    #[test]
    fn a_closure_body_still_yields_its_calls() {
        // Gradle build scripts are mostly closures; missing calls inside
        // them would empty the graph for `.gradle` files.
        let f = extract(
            "class C {\n  void f() {\n    items.each { helper(it) }\n  }\n}\n\
             dependencies {\n  implementation 'a:b:1'\n}\n",
        );
        let names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        for n in ["each", "helper", "dependencies", "implementation"] {
            assert!(names.contains(&n), "{n}: {names:?}");
        }
    }

    #[test]
    fn script_functions_are_free_functions() {
        let f = extract("def helper(x) { return x }\nvoid other() { }\nhelper(1)\n");
        let got: Vec<(&str, DefVariant)> = f
            .definitions
            .iter()
            .map(|d| (d.qualified_name.as_str(), d.variant))
            .collect();
        assert_eq!(
            got,
            vec![
                ("helper", DefVariant::FreeFunction),
                ("other", DefVariant::FreeFunction),
            ]
        );
    }

    #[test]
    fn an_empty_file_yields_nothing_and_does_not_panic() {
        let f = extract("");
        assert!(f.definitions.is_empty());
        assert!(f.imports.is_empty());
    }

    #[test]
    fn malformed_source_does_not_panic() {
        let f = extract("class Broken {\n  void f( {\n");
        let _ = defs(&f);
    }

    /// The definitions named `qn` and whether each is tagged `abstract`.
    fn abstract_tags(f: &FileFacts, qn: &str) -> Vec<bool> {
        f.definitions
            .iter()
            .filter(|d| d.qualified_name == qn)
            .map(|d| d.attributes.iter().any(|a| a == "abstract"))
            .collect()
    }

    // WS22 gap 7: interface and `abstract` methods are declarations.
    #[test]
    fn bodiless_interface_and_abstract_methods_are_tagged_abstract() {
        let f = extract(
            "interface I { int a() }\nabstract class K { abstract int b()\n int c() { 1 }\n native int d() }\n",
        );
        assert_eq!(abstract_tags(&f, "I.a"), vec![true]);
        assert_eq!(abstract_tags(&f, "K.b"), vec![true]);
        assert_eq!(abstract_tags(&f, "K.c"), vec![false]);
        assert_eq!(abstract_tags(&f, "K.d"), vec![false]);
        let f = extract("class S { static int s() { 1 } }\n");
        let variants: Vec<DefVariant> = f.definitions.iter().map(|d| d.variant).collect();
        assert_eq!(variants, vec![DefVariant::StaticMethod]);
    }

    // WS22 (WS21 carry-over): constructors are callables named after
    // their class; `new` and `this(…)`/`super(…)` call them.
    #[test]
    fn constructors_and_constructions_are_recorded() {
        let f = extract(
            "class B { B(int n) { } }\nclass C extends B {\n  C(int x) { super(x) }\n  C() { this(1) }\n}\n\
             class P { P() { super() } }\nclass M { void run() { new C(1); new java.util.ArrayList<String>() } }\n",
        );
        let ctors: Vec<&str> = f
            .definitions
            .iter()
            .filter(|d| d.variant == DefVariant::Constructor)
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(ctors, vec!["B.B", "C.C", "C.C", "P.P"]);
        let refs: Vec<(&str, &str, Option<u32>, bool)> = f
            .references
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    r.receiver_hint.as_str(),
                    r.arity,
                    r.typed_receiver,
                )
            })
            .collect();
        assert!(refs.contains(&("B", "super", Some(1), false)), "{refs:?}");
        assert!(refs.contains(&("C", "", Some(1), false)), "{refs:?}");
        assert!(refs.contains(&("C", "C", Some(1), true)), "{refs:?}");
        assert!(
            refs.contains(&("ArrayList", "java.util.ArrayList", Some(0), true)),
            "{refs:?}"
        );
        // Near miss: `super()` in a class with no superclass calls nothing
        // in the tree, and is no call of a method named `super` either.
        assert!(
            !refs
                .iter()
                .any(|r| (r.1 == "super" && r.0 != "B") || r.0 == "super"),
            "{refs:?}"
        );
    }

    /// `def t = make()` and an untyped field record the bare call's
    /// result; a typed field and a call through a receiver do not.
    #[test]
    fn call_results_type_their_locals() {
        let f = extract(
            "class C {\n  def a = make()\n  private b = build()\n  Foo c = other()\n  \
             void f() { def t = tool(); String s = name(); def u = x.y() }\n}\n",
        );
        let got: Vec<(&str, &str)> = f
            .local_types
            .iter()
            .map(|l| (l.var_name.as_str(), l.type_name.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("a", "make()"),
                ("b", "build()"),
                ("t", "tool()"),
                ("s", "name()")
            ]
        );
    }
}
