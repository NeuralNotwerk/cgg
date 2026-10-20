//! Groovy plugin — callable extraction for Groovy and Gradle.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

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
        tree_sitter_groovy::LANGUAGE.into()
    }

    /// A method named by a string (Spock's `def "stops when …"() {}`) is
    /// beyond the grammar, which drops the method — and often the rest of
    /// the class — into an ERROR node. The grammar reads a copy with each
    /// such name an identifier of the same length (WS30); extraction reads
    /// the name as written.
    fn parse_source(&self, source: &[u8]) -> Option<Vec<u8>> {
        mask_string_method_names(source)
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

/// The name of a method declared with a string (`"stops when …"` /
/// `'…'`, as the source writes it once the grammar has read it as an
/// identifier): its content.
fn string_name(text: &str) -> Option<String> {
    let q = text.chars().next()?;
    (matches!(q, '"' | '\'') && text.len() >= 2 && text.ends_with(q))
        .then(|| text[1..text.len() - 1].to_string())
}

const METHOD_MODIFIERS: &[&str] = &[
    "def",
    "void",
    "public",
    "private",
    "protected",
    "static",
    "final",
    "synchronized",
    "abstract",
];

/// A copy of `source` in which each string method name — a line
/// `[modifiers] "name"(` or `'name'(` — is an identifier of the same byte
/// length; `None` when there is none.
fn mask_string_method_names(source: &[u8]) -> Option<Vec<u8>> {
    let mut out = source.to_vec();
    let mut masked = false;
    let mut start = 0usize;
    while start < source.len() {
        let end = source[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(source.len(), |i| start + i);
        let line = &source[start..end];
        if let Some((open, close)) = string_method_name(line) {
            let (a, b) = (start + open, start + close);
            if std::str::from_utf8(&source[a + 1..b]).is_ok() {
                masked = true;
                for byte in &mut out[a..=b] {
                    if !(byte.is_ascii_alphanumeric() || *byte == b'_') {
                        *byte = b'_';
                    }
                }
            }
        }
        start = end + 1;
    }
    masked.then_some(out)
}

/// In one line, the offsets of the opening and closing quote of a string
/// method name: one or more modifiers, then `"…"` or `'…'` (no escape, no
/// interpolation brace), then `(`.
fn string_method_name(line: &[u8]) -> Option<(usize, usize)> {
    let mut i = line.iter().position(|b| !b.is_ascii_whitespace())?;
    let mut modifiers = 0;
    loop {
        let word_end = line[i..]
            .iter()
            .position(|b| !(b.is_ascii_alphanumeric() || *b == b'_'))
            .map_or(line.len(), |p| i + p);
        let word = std::str::from_utf8(&line[i..word_end]).ok()?;
        if !METHOD_MODIFIERS.contains(&word) {
            break;
        }
        modifiers += 1;
        i = word_end;
        i += line[i..].iter().position(|b| !b.is_ascii_whitespace())?;
    }
    if modifiers == 0 {
        return None;
    }
    let quote = *line.get(i)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let close = i + 1 + line[i + 1..].iter().position(|&b| b == quote)?;
    let content = &line[i + 1..close];
    if content.is_empty() || content.contains(&b'\\') || content.contains(&b'$') {
        return None;
    }
    let after = close + 1;
    let paren = after
        + line[after..]
            .iter()
            .position(|b| !b.is_ascii_whitespace())?;
    (line[paren] == b'(').then_some((i, close))
}

struct GroovyWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Supertypes of the enclosing class, innermost last.
    ///
    /// Recorded from 0.6.8. Without it every `base_types` rule for this
    /// language was inert — the contract is identical to Java's and
    /// matched nothing.
    bases: Vec<Vec<String>>,
    /// Record method pointers as value references (WS26); see
    /// `crate::ExtractCtx`.
    reference_sites: bool,
}

impl<'a> GroovyWalker<'a> {
    fn text(&self, n: Node) -> &str {
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
                if let Some(name) = node.child_by_field_name("name").or_else(|| {
                    node.children(&mut node.walk()).find(|c| {
                        c.kind() == "scoped_identifier" || c.kind() == "identifier"
                    })
                }) {
                    self.scope
                        .push(self.text(name).replace(';', "").trim().to_string());
                }
                return;
            }
            "import_declaration" => {
                self.record_import(node);
                return;
            }
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                let class_bases = super::attrs::base_types(node, self.source);
                if !name.is_empty() {
                    self.scope.push(name);
                    self.bases.push(class_bases.clone());
                    self.walk_children(node);
                    self.bases.pop();
                    self.scope.pop();
                } else {
                    self.bases.push(class_bases.clone());
                    self.walk_children(node);
                    self.bases.pop();
                }
                return;
            }
            "method_declaration" => {
                self.record_method(node);
                self.walk_children(node);
                return;
            }
            // `Box(int x) { … }`: a constructor, named after its class as
            // Java's is, so `new Box(…)` and `this(…)` reach it.
            "constructor_declaration" => {
                if let Some(n) = node.child_by_field_name("name") {
                    let name = self.text(n).to_string();
                    if !name.is_empty() {
                        self.record_def(node, &name, DefVariant::Constructor);
                    }
                }
                self.walk_children(node);
                return;
            }
            // `this(…)` / `super(…)` as a constructor's first statement.
            "explicit_constructor_invocation" => {
                if let Some(r) = super::java::constructor_delegation(node, self.source) {
                    self.facts.references.push(r);
                }
                self.walk_children(node);
                return;
            }
            // `new Box(…)`: a call of `Box`'s constructor, looked up on the
            // type as a construction receiver is.
            "object_creation_expression" => {
                self.record_construction(node);
                self.walk_children(node);
                return;
            }
            "function_definition" => {
                let name_node = node.child_by_field_name("name");
                let string_named = name_node.and_then(|n| string_name(self.text(n)));
                let name = match &string_named {
                    Some(n) => n.clone(),
                    None => name_node
                        .map(|n| self.text(n).to_string())
                        .unwrap_or_default(),
                };
                if !name.is_empty() {
                    self.record_def(node, &name, DefVariant::FreeFunction);
                    if string_named.is_some()
                        && let Some(d) = self.facts.definitions.last_mut()
                    {
                        d.attributes.push(STRING_NAMED.to_string());
                    }
                }
                self.walk_children(node);
                return;
            }
            "method_invocation" => {
                self.record_call(node);
                self.walk_children(node);
                return;
            }
            "function_call" => {
                self.record_function_call(node);
                self.walk_children(node);
                return;
            }
            // `def t = make()`: the bare call's result, recorded as
            // `make()` for type propagation to read the callee's declared
            // return type (WS23).
            "local_variable_declaration" => {
                let mut c = node.walk();
                for d in node.children_by_field_name("declarator", &mut c) {
                    if let (Some(name), Some(value)) = (
                        d.child_by_field_name("name"),
                        d.child_by_field_name("value"),
                    ) && value.kind() == "method_invocation"
                        && value.child_by_field_name("object").is_none()
                        && let Some(callee) = value.child_by_field_name("name")
                    {
                        let var_name = self.text(name).to_string();
                        let type_name = format!("{}()", self.text(callee));
                        self.facts.local_types.push(cgg_core::LocalType {
                            var_name,
                            type_name,
                            scope_byte: node.start_byte() as u32,
                        });
                    }
                }
                self.walk_children(node);
                return;
            }
            // `this.&m`, `obj.&m`, `Util.&m`: a method pointer. The grammar
            // has no `.&` operator; it reads `a.&m` as `a & m` with the dot
            // in an ERROR node (`method_pointer`).
            "binary_expression" if self.reference_sites => {
                if let Some((recv, name)) = self.method_pointer(node, "left", "right") {
                    self.push_pointer(recv, name);
                }
            }
            // `this.&platform.curry(x)`: read as a field access whose dot is
            // followed by the ERROR `&`.
            "field_access" if self.reference_sites => {
                if let Some((recv, name)) = self.method_pointer(node, "object", "field") {
                    self.push_pointer(recv, name);
                }
            }
            // A method pointer the parser could not place (a command
            // expression, `owner.onFailure this.&onFailure`): an ERROR node
            // that is exactly `recv.&` before the method's name, or exactly
            // `.&name` after the receiver.
            "ERROR" if self.reference_sites => {
                self.method_pointer_in_error(node);
            }
            // Groovy 3 `this::m` / `Util::m`.
            "method_reference" if self.reference_sites => {
                let named: Vec<Node> = node.named_children(&mut node.walk()).collect();
                if let [recv, .., name] = named.as_slice()
                    && name.kind() == "identifier"
                    && recv.id() != name.id()
                {
                    self.push_pointer(*recv, *name);
                }
            }
            "closure_expression" => {
                // Closures are callable but typically anonymous; skip for now
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

    fn record_method(&mut self, node: Node) {
        let Some(name_node) = node.child_by_field_name("name") else {
            return;
        };
        let string_named = string_name(self.text(name_node));
        let simple = string_named
            .clone()
            .unwrap_or_else(|| self.text(name_node).to_string());
        if simple.is_empty() {
            return;
        }
        let is_static = node
            .children(&mut node.walk())
            .any(|c| c.kind() == "modifiers" && self.text(c).contains("static"));
        let variant = if is_static {
            DefVariant::StaticMethod
        } else {
            DefVariant::InherentMethod
        };
        let from = self.facts.definitions.len();
        self.record_def(node, &simple, variant);
        // `def "…"()`: a Spock feature method (`spock` rule).
        if string_named.is_some()
            && let Some(d) = self.facts.definitions.get_mut(from)
        {
            d.attributes.push(STRING_NAMED.to_string());
        }
        // An interface or `abstract` method: a declaration.
        let native = node
            .children(&mut node.walk())
            .filter(|c| c.kind() == "modifiers")
            .any(|m| m.children(&mut m.walk()).any(|t| t.kind() == "native"));
        super::attrs::mark_abstract_from(
            &mut self.facts.definitions,
            from,
            node.child_by_field_name("body").is_none() && !native,
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
            // Recorded from 0.6.8; before that no attribute rule could
            // match this language at all.
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
        let ident_node = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "scoped_identifier" || c.kind() == "identifier");
        let Some(ident_node) = ident_node else { return };
        let path = self.text(ident_node).to_string();
        if path.is_empty() {
            return;
        }

        let text = self.text(node);
        let is_static = text.contains("static ");

        if is_static {
            let alias = path.rsplit('.').next().unwrap_or("").to_string();
            self.facts.imports.push(ImportRecord {
                kind: "from-import".into(),
                path,
                alias,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        } else {
            let alias = path.rsplit('.').next().unwrap_or("").to_string();
            self.facts.imports.push(ImportRecord {
                kind: "import".into(),
                path,
                alias,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        let name = node
            .child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        // `(f).m()`, `((Foo) o).m()`, `new Foo().m()`: the value or the
        // type the receiver's shape stands for (`crate::receiver`).
        let mut typed_receiver = false;
        let recv = node
            .child_by_field_name("object")
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
        if name.is_empty() {
            return;
        }
        // The argument count, for overload narrowing (WS23): the listed
        // arguments plus a trailing closure. Named arguments (`f(a: 1)`)
        // are collected into one map parameter, and a spread passes an
        // unknown number, so neither has a count.
        let args = node.child_by_field_name("arguments");
        let countable = args.is_none_or(|a| {
            !a.has_error()
                && a.named_children(&mut a.walk())
                    .all(|c| !matches!(c.kind(), "map_item" | "spread_argument"))
        });
        let arity = countable.then(|| {
            args.map_or(0, super::arg_count)
                + u32::from(node.child_by_field_name("body").is_some())
        });
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver,
            arity,
            ..Default::default()
        });
    }

    /// `left.&right`: the receiver and method of a method pointer, read
    /// off `binary_expression(left, ERROR("."), "&", right)` or
    /// `field_access(object, ".", ERROR("&"), field)` — only when the source
    /// between them is exactly `.&`.
    fn method_pointer<'t>(
        &self,
        node: Node<'t>,
        recv_field: &str,
        name_field: &str,
    ) -> Option<(Node<'t>, Node<'t>)> {
        let left = node.child_by_field_name(recv_field)?;
        let right = node.child_by_field_name(name_field)?;
        if right.kind() != "identifier" {
            return None;
        }
        let between = self
            .source
            .get(left.end_byte()..right.start_byte())
            .and_then(|b| std::str::from_utf8(b).ok())?;
        let op: String = between.chars().filter(|c| !c.is_whitespace()).collect();
        (op == ".&").then_some((left, right))
    }

    fn method_pointer_in_error(&mut self, node: Node) {
        let text = self.text(node).trim().to_string();
        let ident = |t: &str| {
            !t.is_empty()
                && t.split('.').all(|seg| {
                    !seg.is_empty()
                        && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
                })
        };
        if let Some(recv) = text.strip_suffix(".&").map(str::trim_end)
            && ident(recv)
            && let Some(name) = node.next_named_sibling().and_then(|n| {
                // `this.&m` read as a declaration: `m` is its declarator.
                if n.kind() == "variable_declarator" {
                    n.child_by_field_name("name")
                } else {
                    Some(n)
                }
            })
            && name.kind() == "identifier"
        {
            let name_text = self.text(name).to_string();
            self.push_pointer_text(recv.to_string(), name_text, name);
            return;
        }
        if let Some(name) = text.strip_prefix(".&").map(str::trim_start)
            && ident(name)
            && !name.contains('.')
            && let Some(recv) = node.prev_named_sibling().and_then(|n| {
                if n.kind() == "variable_declarator" {
                    n.child_by_field_name("name")
                } else {
                    Some(n)
                }
            })
            && matches!(recv.kind(), "identifier" | "this" | "field_access")
        {
            let recv_text = self.text(recv).to_string();
            self.push_pointer_text(recv_text, name.to_string(), node);
        }
    }

    fn push_pointer_text(&mut self, receiver_hint: String, name: String, site: Node) {
        self.facts.references.push(RefRecord {
            name,
            receiver_hint,
            site_line: (site.start_position().row as u32) + 1,
            site_byte: site.start_byte() as u32,
            value_ref: true,
            ..Default::default()
        });
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

    fn record_function_call(&mut self, node: Node) {
        let name = node
            .child_by_field_name("function")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_groovy::LANGUAGE.into())
            .unwrap();
        let rewritten = GroovyPlugin.parse_source(src.as_bytes());
        let tree = p
            .parse(rewritten.as_deref().unwrap_or(src.as_bytes()), None)
            .unwrap();
        GroovyPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/X.groovy"),
            &tree,
            src.as_bytes(),
        )
    }

    fn extract_with_refs(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_groovy::LANGUAGE.into())
            .unwrap();
        let rewritten = GroovyPlugin.parse_source(src.as_bytes());
        let tree = p
            .parse(rewritten.as_deref().unwrap_or(src.as_bytes()), None)
            .unwrap();
        GroovyPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(true),
            FileId::new(0),
            &PathBuf::from("/tmp/x.groovy"),
            &tree,
            src.as_bytes(),
        )
    }

    /// WS26: `this.&m`, `obj.&m`, `Util.&m` (also inside arguments) and
    /// `this::m` are method pointers; a bitwise `a & b` is not.
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
        // The parser's ERROR shapes for a command expression.
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

    /// WS30: a Spock feature method named by a string, which the grammar
    /// cannot parse, is recovered under its string name, marked
    /// `string-named`, with the rest of the class and its calls intact.
    #[test]
    fn string_named_methods_are_recovered() {
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
        // A file with no string-named method is parsed as is; the copy
        // keeps every byte offset.
        assert!(
            GroovyPlugin
                .parse_source(b"class A { def f() { 1 } }\n")
                .is_none()
        );
        let copy = GroovyPlugin.parse_source(src.as_bytes()).unwrap();
        assert_eq!(copy.len(), src.len());
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

    #[test]
    fn method_invocation_captured() {
        let src = "class C { void f() { helper(); obj.run() } }\n";
        let f = extract(src);
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "helper" && r.receiver_hint.is_empty())
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "run" && r.receiver_hint == "obj")
        );
    }

    fn defs(f: &FileFacts) -> Vec<String> {
        f.definitions
            .iter()
            .map(|d| d.qualified_name.clone())
            .collect()
    }

    #[test]
    fn a_plain_import_is_recorded() {
        let f = extract("import com.example.Service\nclass C { void f() {} }\n");
        assert!(
            f.imports
                .iter()
                .any(|i| i.path.contains("com.example.Service")),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn a_static_import_is_a_from_import() {
        // The plugin distinguishes the two kinds; collapsing them would
        // lose the distinction the cross-file resolver keys on.
        let f = extract("import static java.lang.Math.max\nclass C { void f() {} }\n");
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "from-import" || i.path.contains("Math")),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn a_closure_body_still_yields_its_calls() {
        // Gradle build scripts are mostly closures; missing calls inside
        // them would empty the graph for `.gradle` files.
        let f =
            extract("class C {\n  void f() {\n    items.each { helper(it) }\n  }\n}\n");
        assert!(
            f.references.iter().any(|r| r.name == "helper"),
            "refs: {:?}",
            f.references.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_script_level_method_without_a_package_is_captured() {
        let f = extract("def helper(x) { return x }\n");
        assert!(
            f.definitions.iter().any(|d| d.simple_name == "helper"),
            "defs: {:?}",
            defs(&f)
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
            "interface I { int a() }\nabstract class K { abstract int b()\n int c() { 1 } }\n",
        );
        assert_eq!(abstract_tags(&f, "I.a"), vec![true]);
        assert_eq!(abstract_tags(&f, "K.b"), vec![true]);
        assert_eq!(abstract_tags(&f, "K.c"), vec![false]);
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
        // in the tree.
        assert!(
            !refs.iter().any(|r| r.1 == "super" && r.0 != "B"),
            "{refs:?}"
        );
    }
}
