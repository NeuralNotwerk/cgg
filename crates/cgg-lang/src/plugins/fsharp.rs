//! F# plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct FsharpPlugin;

impl LanguagePlugin for FsharpPlugin {
    fn id(&self) -> &'static str {
        "fsharp"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".fs", ".fsi", ".fsx"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_fsharp::LANGUAGE_FSHARP.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "fsharp");
        let mut w = FsharpWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
            reference_sites: ctx.reference_sites,
            modules: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct FsharpWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// The type an enclosing `inherit Base(…)` names, innermost last.
    bases: Vec<Vec<String>>,
    /// Record functions used as values (`value_ref`):
    /// `crate::ExtractCtx::reference_sites`.
    reference_sites: bool,
    /// The enclosing namespace and modules, outermost first: `A.B` for
    /// `namespace A.B`, `X.Mod` for `module X.Mod`, then each nested
    /// `module Inner =` (WS34).
    modules: Vec<String>,
}

impl<'a> FsharpWalker<'a> {
    /// A module-level function's qualified name: its module path, then
    /// its name (`A.B.Mod.f`), so `Mod.f` written elsewhere can find it.
    fn module_qn(&self, simple: &str) -> String {
        if !self.scope.is_empty() || self.modules.is_empty() {
            return self.qn(simple);
        }
        format!("{}.{simple}", self.modules.join("."))
    }
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.into()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
    }
    fn child_kind<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                if c.node().kind() == kind {
                    return Some(c.node());
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }

    fn walk(&mut self, node: Node) {
        if self.reference_sites {
            self.capture_value_positions(node);
        }
        match node.kind() {
            "anon_type_defn" => {
                let name = self
                    .child_kind(node, "type_name")
                    .and_then(|n| self.child_kind(n, "identifier"))
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    let inherits = self.child_kind(node, "class_inherits_decl");
                    let base = inherits
                        .and_then(|i| self.child_kind(i, "simple_type"))
                        .map(|t| self.text(t).trim().to_string())
                        .filter(|t| !t.is_empty());
                    self.scope.push(name);
                    self.bases.push(base.into_iter().collect());
                    self.record_primary_constructor(node, inherits);
                    self.walk_children(node);
                    self.bases.pop();
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "function_or_value_defn" => {
                self.record_let(node);
                self.walk_children(node);
                return;
            }
            "member_defn" => {
                self.record_member(node);
                self.walk_children(node);
                return;
            }
            // `namespace A.B` / `module A.B.Mod` (the file's) and a nested
            // `module Inner =`: the path a module-qualified call names.
            "namespace" | "named_module" | "module_defn" => {
                let name = if node.kind() == "module_defn" {
                    self.child_kind(node, "identifier")
                } else {
                    node.child_by_field_name("name")
                }
                .map(|n| self.text(n).trim().to_string())
                .filter(|n| !n.is_empty() && n != "global");
                match name {
                    Some(n) => {
                        if node.kind() != "module_defn"
                            && !self.facts.packages.contains(&n)
                        {
                            self.facts.packages.push(n.clone());
                        }
                        self.modules.push(n);
                        self.walk_children(node);
                        self.modules.pop();
                    }
                    None => self.walk_children(node),
                }
                return;
            }
            "import_decl" => {
                if let Some(li) = self.child_kind(node, "long_identifier") {
                    let path = self.text(li).to_string();
                    if !path.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "open".into(),
                            path,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                return;
            }
            "application_expression" => {
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

    fn record_let(&mut self, node: Node) {
        // Every function binding is a callable: `let f x = …`, the
        // `let f (x: T) : R = …` form the grammar reads as a value
        // pattern, and each binding of `let rec f … and g …`.
        for (i, b) in crate::locals::fsharp_let_bindings(node, self.source)
            .into_iter()
            .enumerate()
        {
            let Some(ident) = b.name else {
                continue;
            };
            let name = self.text(ident).to_string();
            if name.is_empty() {
                continue;
            }
            // The first binding starts at the `let` (and its attributes);
            // a later one at its `and`.
            let text = self.source.get(b.start..b.end).unwrap_or_default();
            let qn = self.module_qn(&name);
            self.facts.definitions.push(DefRecord {
                simple_name: name,
                qualified_name: qn,
                variant: if self.scope.is_empty() {
                    DefVariant::FreeFunction
                } else {
                    DefVariant::InherentMethod
                },
                start_line: (b.start_row as u32) + 1,
                end_line: (b.end_row as u32) + 1,
                start_byte: b.start as u32,
                end_byte: b.end as u32,
                signature_hint: super::extract_signature(
                    std::str::from_utf8(text).unwrap_or(""),
                ),
                visibility: String::new(),
                // F# attributes are `[<FunctionName("X")>]`, which the
                // shared collector reads like any other. Recorded from
                // 0.6.8; before that F# could not match an attribute rule
                // at all, so Azure Functions — a first-party .NET runtime —
                // had nothing to key on.
                attributes: if i == 0 {
                    super::attrs::collect_with_preceding(node, self.source)
                } else {
                    Vec::new()
                },
                base_types: self.bases.last().cloned().unwrap_or_default(),
                ..Default::default()
            });
        }
    }

    fn record_member(&mut self, node: Node) {
        // `new(…) = Box(…)`: an additional constructor, named after its
        // type as the primary one is.
        if self.child_kind(node, "additional_constr_defn").is_some() {
            if let Some(ty) = self.scope.last().cloned() {
                self.push_constructor(
                    &ty,
                    node,
                    node.end_byte(),
                    node.end_position().row,
                );
            }
            return;
        }
        let Some(method) = self.child_kind(node, "method_or_prop_defn") else {
            return;
        };
        let Some(prop) = self.child_kind(method, "property_or_ident") else {
            return;
        };
        // property_or_ident contains identifier(s); we want the last one (after `this.`)
        let mut last_ident: Option<Node> = None;
        let mut c = prop.walk();
        if c.goto_first_child() {
            loop {
                if c.node().kind() == "identifier" {
                    last_ident = Some(c.node());
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        let Some(name_node) = last_ident else { return };
        let name = self.text(name_node).to_string();
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
            variant: DefVariant::InherentMethod,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // F# attributes are `[<FunctionName("X")>]`, which the
            // shared collector reads like any other. Recorded from
            // 0.6.8; before that F# could not match an attribute rule
            // at all, so Azure Functions — a first-party .NET runtime —
            // had nothing to key on.
            attributes: super::attrs::collect_with_preceding(node, self.source),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    /// `type Box(x: int) = inherit Base(x) …`: the primary constructor,
    /// named after the type and spanning its header — the parameters and
    /// the `inherit` clause, whose `Base(x)` is the `super` call it makes
    /// — but not the members, which are callables of their own. A type
    /// with no parameter list (a record, a union, a module-like type) has
    /// none.
    fn record_primary_constructor(&mut self, node: Node, inherits: Option<Node>) {
        let Some(args) = self.child_kind(node, "primary_constr_args") else {
            return;
        };
        let Some(ty) = self.scope.last().cloned() else {
            return;
        };
        let last = inherits.unwrap_or(args);
        self.push_constructor(&ty, node, last.end_byte(), last.end_position().row);
        let Some(inherits) = inherits else { return };
        let Some(base) = self.child_kind(inherits, "simple_type") else {
            return;
        };
        // `inherit Base` with no argument list calls no constructor here.
        if base.next_named_sibling().is_none() {
            return;
        }
        let name = crate::receiver::type_simple_name(self.text(base))
            .trim()
            .to_string();
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: "super".to_string(),
            site_line: (base.start_position().row as u32) + 1,
            site_byte: base.start_byte() as u32,
            not_def: Some(node.start_byte() as u32),
            ..Default::default()
        });
    }

    /// A constructor of type `ty` spanning `node`'s start to `end`.
    fn push_constructor(&mut self, ty: &str, node: Node, end: usize, end_row: usize) {
        let text =
            std::str::from_utf8(&self.source[node.start_byte()..end]).unwrap_or("");
        self.facts.definitions.push(DefRecord {
            simple_name: ty.to_string(),
            qualified_name: self.qn(ty),
            variant: DefVariant::Constructor,
            start_line: (node.start_position().row as u32) + 1,
            end_line: (end_row as u32) + 1,
            start_byte: node.start_byte() as u32,
            end_byte: end as u32,
            signature_hint: super::extract_signature(text),
            visibility: String::new(),
            attributes: super::attrs::collect_with_preceding(node, self.source),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        // application_expression: first child is the callee. May be nested
        // application_expression (curried application) — descend leftward.
        let mut cur = node;
        // Where the site is: the application, or the callee itself when
        // the grammar folded it into an operator expression.
        let mut site = node;
        loop {
            if let Some(mut first) = cur.child(0) {
                if first.kind() == "application_expression" {
                    cur = first;
                    continue;
                }
                // `xs |> f a`, `x + f y`: application binds tighter than
                // any operator, but the grammar reads `(xs |> f) a`. The
                // callee is the operator's right operand.
                if first.kind() == "infix_expression" {
                    // Unless what it is "applied" to is a block the
                    // grammar lost track of (`while a < b.Length do …`).
                    if applies_to_block(cur) {
                        break;
                    }
                    match last_named(first) {
                        Some(r) if r.kind() == "application_expression" => {
                            cur = r;
                            site = r;
                            continue;
                        }
                        Some(r) => {
                            first = r;
                            site = r;
                        }
                        None => break,
                    }
                }
                // `f<int> x`: a type application (`typed_expression`)
                // calls what it applies type arguments to.
                if first.kind() == "typed_expression"
                    && let Some(applied) = first.named_child(0)
                {
                    first = applied;
                }
                let inner = if first.kind() == "long_identifier_or_op" {
                    first.child(0).unwrap_or(first)
                } else {
                    first
                };
                match inner.kind() {
                    // `A.B.f x` the grammar splits as `A.B` dot `f` (inside an
                    // operator expression, WS34): the same dotted call.
                    "dot_expression" => {
                        let (Some(base), Some(field)) = (
                            inner.child_by_field_name("base"),
                            inner.child_by_field_name("field"),
                        ) else {
                            break;
                        };
                        // `N.S` dot `Mod.h`: the whole path is `N.S.Mod.h`.
                        let full: String =
                            format!("{}.{}", self.text(base), self.text(field))
                                .split_whitespace()
                                .collect();
                        let path_like = full.split('.').all(|seg| {
                            !seg.is_empty()
                                && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
                        });
                        let (recv, name) = full
                            .rsplit_once('.')
                            .map(|(a, b)| (a.to_string(), b.to_string()))
                            .unwrap_or_default();
                        if path_like && !recv.is_empty() && !name.is_empty() {
                            self.facts.references.push(RefRecord {
                                name,
                                receiver_hint: recv,
                                site_line: (site.start_position().row as u32) + 1,
                                site_byte: site.start_byte() as u32,
                                arity: None,
                                ..Default::default()
                            });
                        }
                    }
                    "long_identifier" => {
                        // Dotted: take last identifier as name, prefix as receiver.
                        let mut idents: Vec<String> = Vec::new();
                        let mut c = inner.walk();
                        if c.goto_first_child() {
                            loop {
                                if c.node().kind() == "identifier" {
                                    idents.push(self.text(c.node()).to_string());
                                }
                                if !c.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                        if let Some(name) = idents.pop() {
                            let receiver = idents.join(".");
                            self.facts.references.push(RefRecord {
                                name,
                                receiver_hint: receiver,
                                site_line: (site.start_position().row as u32) + 1,
                                site_byte: site.start_byte() as u32,
                                ..Default::default()
                            });
                        }
                    }
                    "identifier" => {
                        let name = self.text(inner).to_string();
                        // `new() = Box(1)` delegates to another constructor
                        // of `Box`, never to itself.
                        let not_def = (self.scope.last() == Some(&name))
                            .then(|| {
                                std::iter::successors(node.parent(), |n| n.parent())
                                    .find(|n| n.kind() == "additional_constr_defn")
                                    .and_then(|a| a.parent())
                                    .map(|m| m.start_byte() as u32)
                            })
                            .flatten();
                        self.facts.references.push(RefRecord {
                            name,
                            receiver_hint: String::new(),
                            site_line: (site.start_position().row as u32) + 1,
                            site_byte: site.start_byte() as u32,
                            not_def,
                            ..Default::default()
                        });
                    }
                    _ => {}
                }
            }
            break;
        }
    }
}

impl FsharpWalker<'_> {
    /// Functions used as values, when the run records reference sites: an
    /// application's arguments (`List.map f xs`), an operator's operands
    /// (`xs |> f`, `f >> g`), a record or object-expression field value,
    /// list, array and tuple elements, and the value a `let` or member
    /// returns (`let handler = f`). Recorded the way the call would be —
    /// `f` bare, `M.f` through a module or type — marked `value_ref`.
    /// A capitalized bare name is a union case or type, and a member read
    /// through a variable (`x.Length`) a property read, so neither is
    /// recorded. Names a parameter or local binds are dropped through the
    /// F# locals (`value_refs::prune`).
    fn capture_value_positions(&mut self, node: Node) {
        let mut values: Vec<Node> = Vec::new();
        match node.kind() {
            "application_expression" => {
                let mut c = node.walk();
                values.extend(node.named_children(&mut c).skip(1));
            }
            "infix_expression" => {
                // Only an operator that takes a function: `=` is also a
                // named argument (`Foo(key = v)`), and arithmetic and
                // comparison operands are data.
                let applies = node
                    .named_children(&mut node.walk())
                    .find(|k| k.kind() == "infix_op")
                    .is_some_and(|op| FUNCTION_OPERATORS.contains(&self.text(op)));
                if !applies {
                    return;
                }
                let mut c = node.walk();
                let mut operands: Vec<Node> = node
                    .named_children(&mut c)
                    .filter(|k| k.kind() != "infix_op")
                    .collect();
                // `(xs |> f) a` as the grammar reads `xs |> f a`: the right
                // operand is the callee (`record_call`).
                let callee = node.parent().is_some_and(|p| {
                    p.kind() == "application_expression"
                        && p.child(0).is_some_and(|f| f.id() == node.id())
                        && !applies_to_block(p)
                });
                if callee {
                    operands.pop();
                }
                values.extend(operands);
            }
            "field_initializer" => values.extend(node.child_by_field_name("value")),
            "list_expression" | "array_expression" | "tuple_expression" => {
                let mut c = node.walk();
                values.extend(node.named_children(&mut c));
            }
            "function_or_value_defn" => {
                let mut c = node.walk();
                values.extend(node.children_by_field_name("body", &mut c));
            }
            "method_or_prop_defn" => {
                // The body: the last child, unless that is the name or the
                // parameters.
                let mut c = node.walk();
                let mut last = None;
                if c.goto_first_child() {
                    loop {
                        if c.node().is_named() {
                            last = Some((c.node(), c.field_name().is_some()));
                        }
                        if !c.goto_next_sibling() {
                            break;
                        }
                    }
                }
                values.extend(last.filter(|(_, field)| !field).map(|(n, _)| n));
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
            "long_identifier_or_op" => {
                if let Some(inner) = v.named_child(0) {
                    self.value_ref_in(inner, depth + 1);
                }
                return;
            }
            "paren_expression" => {
                if let Some(inner) = last_named(v) {
                    self.value_ref_in(inner, depth + 1);
                }
                return;
            }
            "identifier" | "long_identifier" => {}
            _ => return,
        }
        let mut idents: Vec<Node> = Vec::new();
        if v.kind() == "identifier" {
            idents.push(v);
        } else {
            let mut c = v.walk();
            idents.extend(
                v.named_children(&mut c)
                    .filter(|k| k.kind() == "identifier"),
            );
        }
        let Some(name_node) = idents.pop() else {
            return;
        };
        let name = self.text(name_node).to_string();
        let lower = |t: &str| t.starts_with(|c: char| c.is_lowercase() || c == '_');
        let receiver: Vec<&str> = idents.iter().map(|i| self.text(*i)).collect();
        match receiver.first() {
            None if !lower(&name) => return,
            Some(head) if lower(head) => return,
            _ => {}
        }
        if name.is_empty() || name == "_" {
            return;
        }
        let receiver_hint = receiver.join(".");
        self.facts.references.push(RefRecord {
            name,
            receiver_hint,
            site_line: (name_node.start_position().row as u32) + 1,
            site_byte: name_node.start_byte() as u32,
            value_ref: true,
            ..Default::default()
        });
    }
}

/// The operators whose operands are functions: pipelines, composition
/// and the common monadic and applicative combinators.
const FUNCTION_OPERATORS: &[&str] = &[
    "|>", "||>", "|||>", "<|", "<||", "<|||", ">>", "<<", ">>=", "=<<", ">=>", "<=<",
    "<!>", "<*>", "<&>", "<^>",
];

/// Whether an application's first argument is a block (`do …`, `if …`,
/// a `let … in …`): the grammar folding a statement into an application
/// (`while i < s.Length do …`), not a call.
fn applies_to_block(app: Node) -> bool {
    app.child(1).is_some_and(|a| {
        matches!(
            a.kind(),
            "do_expression"
                | "if_expression"
                | "match_expression"
                | "while_expression"
                | "for_expression"
                | "try_expression"
                | "declaration_expression"
                | "sequential_expression"
        )
    })
}

/// The last named child of `n`.
fn last_named(n: Node) -> Option<Node> {
    n.named_child(n.named_child_count().checked_sub(1)? as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        FsharpPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/x.fs"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn let_binding_with_args() {
        let src = "module M\nlet greet name = printfn \"Hi\"\nlet add a b = a + b\n";
        let f = extract(src);
        let names: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(names.contains(&"add"), "got: {names:?}");
    }

    #[test]
    fn type_members() {
        let src = "type Service(name: string) =\n    member this.Run() = printfn \"x\"\n    member this.Stop() = ()\n";
        let f = extract(src);
        let qns: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert!(qns.iter().any(|q| q.ends_with("Run")), "got: {qns:?}");
        assert!(qns.iter().any(|q| q.ends_with("Stop")), "got: {qns:?}");
    }

    #[test]
    fn open_is_import() {
        let src = "open System\nopen System.IO\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.path == "System"));
        assert!(f.imports.iter().any(|i| i.path == "System.IO"));
    }

    #[test]
    fn application_captured() {
        let src = "let go () = greet \"x\"\nlet h () = s.Run()\n";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "greet"),
            "refs: {:?}",
            f.references
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "Run" && r.receiver_hint == "s"),
            "refs: {:?}",
            f.references
        );
    }

    /// C1: `f<int> 1` / `M.f<int> 1` are calls of `f`, never dropped.
    #[test]
    fn type_application_calls_are_recorded() {
        let f = extract(
            "module M\nlet g () =\n    f<int> 1\n    N.h<string> \"a\"\n    f 2\n",
        );
        let calls: Vec<(&str, &str)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str()))
            .collect();
        assert_eq!(
            calls.iter().filter(|c| **c == ("f", "")).count(),
            2,
            "{calls:?}"
        );
        assert!(calls.contains(&("h", "N")), "{calls:?}");
    }

    // WS22 (WS21 carry-over): the primary and additional constructors are
    // callables named after their type; `inherit Base(x)` calls the base's.
    #[test]
    fn constructors_are_recorded() {
        let f = extract(
            "module M\ntype B(n: int) =\n  member this.N = n\ntype C(x: int) =\n  inherit B(x)\n  new() = C(1)\n  member this.X = x\ntype R = { A: int }\n",
        );
        let ctors: Vec<(&str, u32)> = f
            .definitions
            .iter()
            .filter(|d| d.variant == DefVariant::Constructor)
            .map(|d| (d.qualified_name.as_str(), d.start_line))
            .collect();
        assert_eq!(ctors, vec![("B.B", 2), ("C.C", 4), ("C.C", 6)]);
        let x = f
            .definitions
            .iter()
            .find(|d| d.qualified_name == "C.X")
            .unwrap();
        assert_eq!(x.base_types, vec!["B"]);
        let refs: Vec<(&str, &str, bool)> = f
            .references
            .iter()
            .map(|r| {
                (
                    r.name.as_str(),
                    r.receiver_hint.as_str(),
                    r.not_def.is_some(),
                )
            })
            .collect();
        assert!(refs.contains(&("B", "super", true)), "{refs:?}");
        assert!(refs.contains(&("C", "", true)), "{refs:?}");
        // Near miss: a record type has no constructor node.
        assert!(!f.definitions.iter().any(|d| d.qualified_name == "R.R"));
    }

    #[test]
    fn every_function_binding_is_a_callable() {
        let f = extract(
            "module M\nlet rec ra x = rb x\nand rb (y: int) : int = ra y\nand [<A>] rc z = z\nlet getCol (s: int) : int = s\nlet private pf (x: int) : int = x\nlet value = 1\nlet lam = fun x -> x\nlet Wrap w = w\nlet (a, b) = (1, 2)\n",
        );
        let defs: Vec<(&str, u32, u32)> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.as_str(), d.start_line, d.end_line))
            .collect();
        for (name, line) in [("ra", 2), ("rb", 3), ("rc", 4), ("getCol", 5), ("pf", 6)] {
            assert!(
                defs.iter()
                    .any(|(n, s, e)| *n == name && *s == line && *e == line),
                "{name} at {line}: {defs:?}"
            );
        }
        // Near misses: values, a lambda value and a destructuring stay values.
        for name in ["value", "lam", "a", "b"] {
            assert!(!defs.iter().any(|(n, ..)| *n == name), "{name}: {defs:?}");
        }
        // Each `and` binding spans only itself.
        let rb = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "rb")
            .unwrap();
        let ra = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "ra")
            .unwrap();
        assert!(ra.end_byte <= rb.start_byte, "{ra:?} {rb:?}");
    }

    fn extract_with_refs(src: &str, refs: bool) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        FsharpPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(refs),
            FileId::new(0),
            &PathBuf::from("/tmp/x.fs"),
            &tree,
            src.as_bytes(),
        )
    }

    /// WS26b: a function used as a value is a reference site only when the
    /// run asks for them; an application's head is a call, never a value.
    #[test]
    fn function_values_are_reference_sites() {
        let src = "module V\nlet h = List.map g1 [1]\nlet k = [1] |> List.map g2 |> M.g3\nlet c = g4 >> g5\nlet r = { Handler = g6; Other = M.G7 }\nlet l = [ g8; None ]\nlet a = [| g9 |]\nlet v = g10\nlet run () = head 1 + s.Length\n";
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
            ("M", "g3"),
            ("", "g4"),
            ("", "g5"),
            ("", "g6"),
            ("M", "G7"),
            ("", "g8"),
            ("", "g9"),
            ("", "g10"),
        ] {
            assert!(
                got.contains(&(recv.to_string(), name.to_string())),
                "missing {recv}.{name} in {got:?}"
            );
        }
        let names: Vec<&str> = got.iter().map(|(_, n)| n.as_str()).collect();
        for n in ["head", "map", "None", "Length", "run"] {
            assert!(!names.contains(&n), "{n} recorded: {got:?}");
        }
        assert!(
            !extract_with_refs(src, false)
                .references
                .iter()
                .any(|r| r.value_ref)
        );
    }

    /// WS26b: the grammar reads `xs |> f a` as `(xs |> f) a` and
    /// `x + g y` as `(x + g) y`; the callee is the operator's right operand.
    #[test]
    fn a_call_folded_into_an_operator_is_recorded() {
        let f = extract("let run xs =\n    xs |> step 1 |> ignore\n    1 + other 2\n");
        let calls: Vec<&str> = f
            .references
            .iter()
            .filter(|r| !r.value_ref)
            .map(|r| r.name.as_str())
            .collect();
        assert!(calls.contains(&"step"), "{calls:?}");
        assert!(calls.contains(&"other"), "{calls:?}");
        // Near miss: the left operand is not the callee.
        assert!(!calls.contains(&"xs"), "{calls:?}");
    }

    /// WS26b near misses: a named argument or comparison (`=`) and an
    /// arithmetic operand are data, and a `while … do` block the grammar
    /// folds into an application is no call.
    #[test]
    fn data_operands_and_folded_blocks_are_not_functions() {
        let f = extract_with_refs(
            "let f () =\n    Foo(key = keyFn) |> ignore\n    let n = a + sizeFn\n    while i < target.Length do\n        i <- i + 1\n",
            true,
        );
        let names: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        for n in ["keyFn", "sizeFn", "Length"] {
            assert!(!names.contains(&n), "{n}: {names:?}");
        }
    }
}
