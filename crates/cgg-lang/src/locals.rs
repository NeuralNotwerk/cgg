//! Value bindings: parameters and local variables, with the span they are
//! in scope over (FINDINGS A1/A4 generalisation, WS14).
//!
//! A bare call `cb()` where `cb` is a parameter, `debug("x")` after
//! `const debug = makeDebug()`, `f n` inside an OCaml function taking
//! `f`: the call goes through whatever value the name holds. Without a
//! record of the binding the cross-file resolver matched the name against
//! every same-named function in the tree. This pass records each binding
//! as [`LocalBinding`] so the resolver can refuse that match, and so a
//! receiver headed by a local (`c.json()` with a parameter `c`) is not
//! read as an import of the same name.
//!
//! One walk serves every language: a table per grammar names the node
//! kinds that bind names (and through which of their fields), and the
//! kinds that bound a binding's scope. Nothing here is a function name;
//! everything is a grammar node kind.

use cgg_core::LocalBinding;
use tree_sitter::{Node, Tree};

/// How a binder's scope is found.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// A parameter: the whole enclosing function (or closure).
    Param,
    /// A declaration: from the declaration to the end of the enclosing
    /// block or function.
    Decl,
    /// The binder node itself (a `for` loop, a `catch` clause, a match
    /// arm, a lambda whose parameters it holds directly).
    Own,
}

/// One binding form: a node kind, the fields of it a bound name may sit
/// under (`""` for an unnamed child), and how its scope is found.
struct Binder {
    kind: &'static str,
    fields: &'static [&'static str],
    scope: Scope,
    /// Only the binder's first identifier child names the binding (its
    /// other unnamed children are a type or a value).
    first_only: bool,
    /// When set, the binder counts only under a parent of this kind, and
    /// that parent is what the binding's scope is measured from (a Dart
    /// local function's signature names it in the enclosing block).
    parent: &'static str,
    /// A declaration whose binder is only its name list (`val x`, `local
    /// x`) under one of these parents is in scope from the end of that
    /// parent — past its initializer, which still sees the outer name
    /// (`val body = body("…")`).
    after: &'static [&'static str],
    /// An `Own`-scoped binder whose bound name is in scope only from its
    /// first child of this kind (Swift `if let x = x() {`: the body, not
    /// the condition).
    from_child: &'static str,
    /// An `Own`-scoped binder whose bound name is in scope only after its
    /// child under this field (Rust `if let Some(k) = k(..)`, `for x in
    /// x()`: the scrutinee still sees the outer name).
    after_field: &'static str,
    /// With `after_field`: the conditional the binder is the condition of
    /// (`if_expression`, `while_expression`), whose body (field
    /// `consequence` or `body`) the binding is in scope over. One wrapper
    /// between the two (a `let_chain`) is climbed through.
    extends_to: &'static [&'static str],
}

const fn b(kind: &'static str, fields: &'static [&'static str], scope: Scope) -> Binder {
    Binder {
        kind,
        fields,
        scope,
        first_only: false,
        parent: "",
        after: &[],
        from_child: "",
        after_field: "",
        extends_to: &[],
    }
}

const fn b1(kind: &'static str, scope: Scope) -> Binder {
    Binder {
        kind,
        fields: &[""],
        scope,
        first_only: true,
        parent: "",
        after: &[],
        from_child: "",
        after_field: "",
        extends_to: &[],
    }
}

const fn bp(
    kind: &'static str,
    field: &'static [&'static str],
    parent: &'static str,
) -> Binder {
    Binder {
        kind,
        fields: field,
        scope: Scope::Decl,
        first_only: false,
        parent,
        after: &[],
        from_child: "",
        after_field: "",
        extends_to: &[],
    }
}

const fn from_child(mut b: Binder, kind: &'static str) -> Binder {
    b.from_child = kind;
    b
}

const fn after_value(
    mut b: Binder,
    field: &'static str,
    extends_to: &'static [&'static str],
) -> Binder {
    b.after_field = field;
    b.extends_to = extends_to;
    b
}

const fn after(mut b: Binder, parents: &'static [&'static str]) -> Binder {
    b.after = parents;
    b
}

/// A grammar's binding forms.
struct Spec {
    /// Leaf kinds that name a binding.
    idents: &'static [&'static str],
    binders: &'static [Binder],
    /// Node kinds that are callables: a binding is recorded only inside
    /// one, and a parameter's scope is the nearest one.
    functions: &'static [&'static str],
    /// Node kinds that bound a declaration's scope (besides functions).
    blocks: &'static [&'static str],
    /// Node kinds that are patterns: a name nested in one, under a
    /// binder's field, is bound too (`const [a, b] = …`).
    patterns: &'static [&'static str],
}

/// Fields of a pattern that never hold a bound name: a default value, a
/// type, an object key.
const STOP_FIELDS: &[&str] = &["right", "default", "default_value", "type", "key"];

const JS_SPEC: Spec = Spec {
    idents: &["identifier", "shorthand_property_identifier_pattern"],
    binders: &[
        b("formal_parameters", &[""], Scope::Param),
        b("required_parameter", &["pattern"], Scope::Param),
        b("optional_parameter", &["pattern"], Scope::Param),
        b("arrow_function", &["parameter"], Scope::Own),
        b("variable_declarator", &["name"], Scope::Decl),
        b("for_in_statement", &["left"], Scope::Own),
        b("catch_clause", &["parameter"], Scope::Own),
    ],
    functions: &[
        "function_declaration",
        "function_expression",
        "function",
        "arrow_function",
        "method_definition",
        "generator_function_declaration",
        "generator_function",
    ],
    blocks: &["statement_block", "for_statement", "for_in_statement"],
    patterns: &[
        "object_pattern",
        "array_pattern",
        "pair_pattern",
        "assignment_pattern",
        "rest_pattern",
        "object_assignment_pattern",
    ],
};

const PY_SPEC: Spec = Spec {
    idents: &["identifier"],
    binders: &[
        b("parameters", &[""], Scope::Param),
        b("lambda_parameters", &[""], Scope::Param),
        b("default_parameter", &["name"], Scope::Param),
        b("typed_parameter", &[""], Scope::Param),
        b("typed_default_parameter", &["name"], Scope::Param),
        b("assignment", &["left"], Scope::Decl),
        b("for_statement", &["left"], Scope::Own),
        b("as_pattern_target", &[""], Scope::Decl),
    ],
    functions: &["function_definition", "lambda"],
    blocks: &[],
    patterns: &[
        "pattern_list",
        "tuple_pattern",
        "list_pattern",
        "list_splat_pattern",
        "dictionary_splat_pattern",
    ],
};

const LUA_SPEC: Spec = Spec {
    idents: &["identifier"],
    binders: &[
        b("parameters", &["name"], Scope::Param),
        after(
            b("variable_list", &["name"], Scope::Decl),
            &["assignment_statement", "for_generic_clause"],
        ),
        b("for_numeric_clause", &["name"], Scope::Own),
    ],
    functions: &["function_declaration", "function_definition"],
    blocks: &["block", "for_statement"],
    patterns: &[],
};

const SCALA_SPEC: Spec = Spec {
    idents: &["identifier"],
    binders: &[
        b("parameter", &["name"], Scope::Param),
        b("class_parameter", &["name"], Scope::Param),
        b("lambda_expression", &["parameters"], Scope::Own),
        b("binding", &["name"], Scope::Param),
        b("val_definition", &["pattern"], Scope::Decl),
        b("var_definition", &["pattern"], Scope::Decl),
        b("case_clause", &["pattern"], Scope::Own),
    ],
    functions: &["function_definition", "lambda_expression"],
    blocks: &["block", "case_clause", "for_expression"],
    patterns: &[
        "tuple_pattern",
        "case_class_pattern",
        "typed_pattern",
        "infix_pattern",
        "capture_pattern",
        "alternative_pattern",
    ],
};

const KOTLIN_SPEC: Spec = Spec {
    idents: &["simple_identifier"],
    binders: &[
        b1("parameter", Scope::Param),
        after(
            b1("variable_declaration", Scope::Decl),
            &["property_declaration"],
        ),
        b("lambda_parameters", &[""], Scope::Param),
    ],
    functions: &[
        "function_declaration",
        "lambda_literal",
        "anonymous_function",
        "secondary_constructor",
    ],
    blocks: &["statements", "for_statement", "control_structure_body"],
    patterns: &["multi_variable_declaration"],
};

const SWIFT_SPEC: Spec = Spec {
    idents: &["simple_identifier"],
    binders: &[
        b("parameter", &["name"], Scope::Param),
        b("lambda_parameter", &["name"], Scope::Param),
        after(
            b("pattern", &["bound_identifier", ""], Scope::Decl),
            &["property_declaration"],
        ),
        from_child(b("if_statement", &["bound_identifier"], Scope::Own), "{"),
        b("guard_statement", &["bound_identifier"], Scope::Decl),
    ],
    functions: &[
        "function_declaration",
        "lambda_literal",
        "init_declaration",
        "computed_property",
    ],
    blocks: &["statements", "for_statement"],
    patterns: &["pattern"],
};

const RUST_SPEC: Spec = Spec {
    idents: &["identifier"],
    binders: &[
        b("parameter", &["pattern"], Scope::Param),
        b("closure_parameters", &[""], Scope::Param),
        b("let_declaration", &["pattern"], Scope::Decl),
        after_value(b("for_expression", &["pattern"], Scope::Own), "value", &[]),
        after_value(
            b("let_condition", &["pattern"], Scope::Own),
            "value",
            &["if_expression", "while_expression"],
        ),
        b("match_arm", &["pattern"], Scope::Own),
    ],
    functions: &["function_item", "closure_expression"],
    blocks: &["block", "if_expression", "match_arm", "for_expression"],
    patterns: &[
        "tuple_pattern",
        "tuple_struct_pattern",
        "struct_pattern",
        "field_pattern",
        "slice_pattern",
        "ref_pattern",
        "mut_pattern",
        "reference_pattern",
        "captured_pattern",
        "or_pattern",
        "match_pattern",
    ],
};

const DART_SPEC: Spec = Spec {
    idents: &["identifier"],
    binders: &[
        b("formal_parameter", &["name"], Scope::Param),
        b1("formal_parameter", Scope::Param),
        b("initialized_variable_definition", &["name"], Scope::Decl),
        b("initialized_identifier", &["name"], Scope::Decl),
        b("for_loop_parts", &["name"], Scope::Own),
        b("catch_parameters", &[""], Scope::Own),
        // A local function names a value in its enclosing block (the
        // plugin records no callable for it).
        bp(
            "function_signature",
            &["name"],
            "local_function_declaration",
        ),
    ],
    functions: &[
        "function_declaration",
        "method_declaration",
        "local_function_declaration",
        "function_expression",
    ],
    blocks: &["block", "for_statement"],
    patterns: &[],
};

const OCAML_SPEC: Spec = Spec {
    idents: &["value_pattern"],
    binders: &[
        b("parameter", &["pattern"], Scope::Param),
        b("match_case", &["pattern"], Scope::Own),
    ],
    functions: &["let_binding", "fun_expression", "function_expression"],
    blocks: &["match_case"],
    patterns: &[
        "tuple_pattern",
        "parenthesized_pattern",
        "constructor_pattern",
        "typed_pattern",
        "alias_pattern",
        "or_pattern",
        "cons_pattern",
        "list_pattern",
        "record_pattern",
        "field_pattern",
        "labeled_argument",
        "label",
    ],
};

const ZIG_SPEC: Spec = Spec {
    idents: &["identifier"],
    binders: &[
        b("parameter", &["name"], Scope::Param),
        b1("variable_declaration", Scope::Decl),
        b("payload", &[""], Scope::Decl),
    ],
    functions: &["function_declaration"],
    blocks: &["block", "for_statement", "while_statement", "if_statement"],
    patterns: &[],
};

fn spec(lang: &str) -> Option<&'static Spec> {
    Some(match lang {
        "javascript" | "typescript" => &JS_SPEC,
        "python" => &PY_SPEC,
        "lua" => &LUA_SPEC,
        "scala" => &SCALA_SPEC,
        "kotlin" => &KOTLIN_SPEC,
        "swift" => &SWIFT_SPEC,
        "rust" => &RUST_SPEC,
        "dart" => &DART_SPEC,
        "ocaml" => &OCAML_SPEC,
        "zig" => &ZIG_SPEC,
        _ => return None,
    })
}

/// Every value binding of the file, in source order. Empty for a
/// language with no table.
pub fn collect(lang: &str, tree: &Tree, source: &[u8]) -> Vec<LocalBinding> {
    let Some(spec) = spec(lang) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    // Depth-first with a cursor, keeping the ancestor chain (each node
    // with the field it hangs from its parent by): `Node::parent` and a
    // per-leaf search of the parent's children are both linear, and a
    // minified bundle has nodes with tens of thousands of children.
    let mut chain: Vec<(Node<'_>, &'static str)> = Vec::new();
    let mut functions = 0usize;
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        let is_ident = spec.idents.contains(&node.kind());
        // Only an identifier and a node the walk descends into use their
        // field. `TreeCursor::field_name` is slow on the children of a
        // node with a huge child count, and asking it of every leaf of a
        // generated byte-string literal cost 2.3 s on one 6.5 MB Qt
        // resource file (0.2 s without).
        let field = if is_ident || node.child_count() > 0 {
            cursor.field_name().unwrap_or("")
        } else {
            ""
        };
        if is_ident
            && functions > 0
            && let Some(b) = binding(spec, node, field, &chain, source, &out)
        {
            out.push(b);
        }
        // Descend (not into an identifier: OCaml's `value_pattern` has a
        // child of its own).
        if !is_ident && cursor.goto_first_child() {
            if spec.functions.contains(&node.kind()) {
                functions += 1;
            }
            chain.push((node, field));
            continue;
        }
        // Next sibling, else climb until one has a next sibling.
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                out.sort_by_key(|b| (b.start_byte, b.end_byte));
                out.dedup();
                return out;
            }
            if let Some((n, _)) = chain.pop()
                && spec.functions.contains(&n.kind())
            {
                functions -= 1;
            }
        }
    }
}

/// `leaf` names a binding when, climbing through pattern nodes, it
/// reaches a binder by one of the binder's name fields. `chain` holds the
/// leaf's ancestors, root first, each with the field it hangs from its
/// own parent by; `leaf_field` is the leaf's.
fn binding(
    spec: &Spec,
    leaf: Node<'_>,
    leaf_field: &'static str,
    chain: &[(Node<'_>, &'static str)],
    source: &[u8],
    prior: &[LocalBinding],
) -> Option<LocalBinding> {
    let name = leaf.utf8_text(source).ok()?.trim();
    if name.is_empty() || name == "_" {
        return None;
    }
    let mut cur = leaf;
    let mut field = leaf_field;
    let mut level = chain.len();
    for _ in 0..8 {
        let (parent, parent_field) = *chain.get(level.checked_sub(1)?)?;
        let grand = level
            .checked_sub(2)
            .and_then(|i| chain.get(i))
            .map(|(g, _)| *g);
        let binder = spec.binders.iter().find(|b| {
            b.kind == parent.kind()
                && b.fields.contains(&field)
                && (!b.first_only || first_ident_child(spec, parent) == Some(cur.id()))
                && (b.parent.is_empty() || grand.is_some_and(|g| g.kind() == b.parent))
        });
        if let Some(binder) = binder {
            // `var pick = H.pick`, `const Range = require("./range").Range`,
            // `x = obj.x`: the local is another name for the member it
            // reads, which carries the same name; it shadows nothing.
            if aliases_same_member(parent, name, source, prior) {
                return None;
            }
            // The binder's own ancestors (and itself) for the scope walk.
            let ancestors = if binder.parent.is_empty() {
                &chain[..level]
            } else {
                &chain[..level.checked_sub(1)?]
            };
            let (start, end) = scope_of(spec, ancestors, binder)?;
            // The declaration statement around the binder (`const` before
            // a JS declarator), unless the binder sits directly in a
            // block or function.
            let decl = match ancestors.len() {
                0 => start,
                n => {
                    let binder_node = ancestors[n - 1].0;
                    match n.checked_sub(2).map(|i| ancestors[i].0) {
                        Some(p)
                            if !spec.blocks.contains(&p.kind())
                                && !spec.functions.contains(&p.kind()) =>
                        {
                            p.start_byte().min(binder_node.start_byte()) as u32
                        }
                        _ => binder_node.start_byte() as u32,
                    }
                }
            };
            return Some(LocalBinding {
                name: name.to_string(),
                start_byte: start,
                end_byte: end,
                decl_byte: decl.min(start),
            });
        }
        if spec.binders.iter().any(|b| b.kind == parent.kind())
            || STOP_FIELDS.contains(&field)
            || !spec.patterns.contains(&parent.kind())
        {
            return None;
        }
        cur = parent;
        field = parent_field;
        level -= 1;
    }
    None
}

/// Whether a declaration's value is a member access ending in the very
/// name it declares (`pick = H.pick`).
fn aliases_same_member(
    binder: Node<'_>,
    name: &str,
    source: &[u8],
    prior: &[LocalBinding],
) -> bool {
    // Zig's `const f = std.process.f;` names its value by no field: the
    // declaration's last named child.
    let Some(value) = binder
        .child_by_field_name("value")
        .or_else(|| binder.child_by_field_name("right"))
        .or_else(|| {
            (binder.kind() == "variable_declaration")
                .then(|| {
                    binder.named_child(binder.named_child_count().checked_sub(1)? as u32)
                })
                .flatten()
        })
    else {
        return false;
    };
    if !matches!(
        value.kind(),
        "member_expression" | "attribute" | "field_expression" | "navigation_expression"
    ) {
        return false;
    }
    let last = value
        .child_by_field_name("property")
        .or_else(|| value.child_by_field_name("attribute"))
        .or_else(|| value.child_by_field_name("field"))
        .or_else(|| {
            value.named_child(value.named_child_count().saturating_sub(1) as u32)
        });
    // Zig (WS14b): a read of a local value's member (`const surface =
    // core.rt_surface.surface;` with a local `core`) is that value, not
    // another name for a declaration of a module.
    let zig = binder.kind() == "variable_declaration";
    let head = value
        .utf8_text(source)
        .ok()
        .and_then(|t| t.split(['.', '[', '(']).next())
        .map(str::trim)
        .unwrap_or("");
    let at = value.start_byte() as u32;
    if zig
        && prior
            .iter()
            .any(|b| b.name == head && b.start_byte <= at && at < b.end_byte)
    {
        return false;
    }
    last.and_then(|n| n.utf8_text(source).ok())
        .is_some_and(|t| t.trim().trim_start_matches('.') == name)
}

/// The first child of `n` that is an identifier leaf.
fn first_ident_child(spec: &Spec, n: Node<'_>) -> Option<usize> {
    let mut c = n.walk();
    n.children(&mut c)
        .find(|ch| spec.idents.contains(&ch.kind()))
        .map(|ch| ch.id())
}

/// The span a binding is in scope over. `chain` ends with the binder
/// itself, preceded by its ancestors (root first).
fn scope_of(
    spec: &Spec,
    chain: &[(Node<'_>, &'static str)],
    form: &Binder,
) -> Option<(u32, u32)> {
    let scope = form.scope;
    let (binder, _) = *chain.last()?;
    // Only inside a callable: a module-level binding is the module's. A
    // binder that is itself a callable (a Dart local function) is in
    // scope in the block around it, so its search starts above it.
    let is_fn = |n: &Node<'_>| spec.functions.contains(&n.kind());
    let above = &chain[..chain.len() - 1];
    let func_at = if is_fn(&binder) {
        above.iter().rposition(|(n, _)| is_fn(n))?
    } else {
        chain.iter().rposition(|(n, _)| is_fn(n))?
    };
    let func = chain[func_at].0;
    let span = |n: Node<'_>| (n.start_byte() as u32, n.end_byte() as u32);
    match scope {
        Scope::Param => Some(span(func)),
        Scope::Own if !form.from_child.is_empty() => {
            let mut c = binder.walk();
            let from = binder
                .children(&mut c)
                .find(|ch| ch.kind() == form.from_child)?;
            Some((from.start_byte() as u32, binder.end_byte() as u32))
        }
        Scope::Own if !form.after_field.is_empty() => {
            let from = binder
                .child_by_field_name(form.after_field)
                .map_or(binder.start_byte(), |v| v.end_byte());
            // The conditional this binder is the condition of (directly,
            // or through one wrapper such as a `let_chain`): in scope over
            // its body, not its `else`.
            let cond = above
                .iter()
                .rev()
                .take(2)
                .map(|(n, _)| *n)
                .find(|n| form.extends_to.contains(&n.kind()));
            let end = cond
                .and_then(|c| {
                    c.child_by_field_name("consequence")
                        .or_else(|| c.child_by_field_name("body"))
                })
                .map_or(binder.end_byte(), |b| b.end_byte().max(binder.end_byte()));
            Some((from as u32, end as u32))
        }
        Scope::Own => Some(span(binder)),
        Scope::Decl => {
            let block = above
                .iter()
                .rev()
                .map(|(n, _)| *n)
                .find(|n| spec.blocks.contains(&n.kind()) || is_fn(n))
                .unwrap_or(func);
            // In scope after the declaration, not in its own initializer
            // (`let now = now();` calls the outer `now`); a local function
            // from its own start (it may recurse).
            let start = if is_fn(&binder) {
                binder.start_byte()
            } else {
                match above.last() {
                    Some((p, _)) if form.after.contains(&p.kind()) => p.end_byte(),
                    _ => binder.end_byte(),
                }
            };
            Some((start as u32, block.end_byte() as u32))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(lang: &str, ts: tree_sitter::Language, src: &str) -> Vec<String> {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&ts).unwrap();
        let tree = p.parse(src, None).unwrap();
        collect(lang, &tree, src.as_bytes())
            .into_iter()
            .map(|b| b.name)
            .collect()
    }

    #[test]
    fn javascript_params_locals_and_closure_params() {
        let got = names(
            "javascript",
            tree_sitter_javascript::LANGUAGE.into(),
            "const top = 1;\nfunction f(a, {b, c: d}, ...rest) { const k = 1, [m, n] = x; \
             for (const it of xs) {} xs.map((e) => e); xs.map(z => z); try {} catch (err) {} }",
        );
        for want in ["a", "b", "d", "rest", "k", "m", "n", "it", "e", "z", "err"] {
            assert!(got.contains(&want.to_string()), "{want} in {got:?}");
        }
        // Module level, a value, an object key: not bindings here.
        for not in ["top", "x", "xs", "c", "f"] {
            assert!(!got.contains(&not.to_string()), "{not} in {got:?}");
        }
    }

    /// `let now = now();`: the initializer still sees the outer `now`.
    #[test]
    fn a_declaration_is_not_in_scope_in_its_own_initializer() {
        let cases: Vec<(&str, tree_sitter::Language, &str)> = vec![
            (
                "rust",
                tree_sitter_rust::LANGUAGE.into(),
                "fn f() { let now = now(); now; }",
            ),
            (
                "rust",
                tree_sitter_rust::LANGUAGE.into(),
                "fn f() { if let Some(now) = now(1) { now; } }",
            ),
            (
                "rust",
                tree_sitter_rust::LANGUAGE.into(),
                "fn f() { while let Some(now) = now(1) { now; } }",
            ),
            (
                "rust",
                tree_sitter_rust::LANGUAGE.into(),
                "fn f() { if a && let Some(now) = now(1) { now; } }",
            ),
            (
                "rust",
                tree_sitter_rust::LANGUAGE.into(),
                "fn f() { for now in now() { now; } }",
            ),
            (
                "javascript",
                tree_sitter_javascript::LANGUAGE.into(),
                "function f() { const now = now(); now; }",
            ),
            (
                "python",
                tree_sitter_python::LANGUAGE.into(),
                "def f():\n    now = now()\n    now\n",
            ),
            (
                "kotlin",
                tree_sitter_kotlin_sg::LANGUAGE.into(),
                "fun f() { val now = now(); now }",
            ),
            (
                "lua",
                tree_sitter_lua::LANGUAGE.into(),
                "local function f() local now = now(); return now end",
            ),
        ];
        for (lang, ts, src) in cases {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&ts).unwrap();
            let tree = p.parse(src, None).unwrap();
            let got = collect(lang, &tree, src.as_bytes());
            let b = got
                .iter()
                .find(|b| b.name == "now")
                .unwrap_or_else(|| panic!("{lang}"));
            let call = src.find("now(").unwrap() as u32;
            assert!(b.start_byte > call, "{lang}: {b:?} covers the initializer");
            let after = src.rfind("now").unwrap() as u32;
            assert!(b.start_byte <= after && after < b.end_byte, "{lang}: {b:?}");
        }
    }

    /// `if let Some(k) = k(..) { .. } else { k() }`: the `else` branch
    /// does not see the pattern's binding.
    #[test]
    fn an_if_let_binding_is_not_in_scope_in_its_else() {
        let src = "fn f() { if let Some(k) = k(1) { k; } else { k(2); } }";
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_rust::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        let got = collect("rust", &tree, src.as_bytes());
        let b = got.iter().find(|b| b.name == "k").expect("k bound");
        let body = src.find("k;").unwrap() as u32;
        let other = src.find("k(2)").unwrap() as u32;
        assert!(b.start_byte <= body && body < b.end_byte, "{b:?}");
        assert!(other >= b.end_byte, "{b:?} covers the else branch");
    }

    /// Zig `const fatal = std.process.fatal;` in a function names the
    /// same function: no shadow; `const op = pick();` is a value.
    #[test]
    fn a_zig_same_named_member_alias_is_not_a_binding() {
        let src = "fn f(core: *C) void { const fatal = std.process.fatal; const op = pick(); \
                   const surface = core.rt.surface; }";
        let got = names("zig", tree_sitter_zig::LANGUAGE.into(), src);
        assert!(!got.contains(&"fatal".to_string()), "{got:?}");
        assert!(got.contains(&"op".to_string()), "{got:?}");
        // A member of a local value is a value.
        assert!(got.contains(&"surface".to_string()), "{got:?}");
    }

    /// `var pick = H.pick` names the same function under its own name:
    /// no shadow. `var pick = choose()` is a value: a shadow.
    #[test]
    fn a_same_named_member_alias_is_not_a_binding() {
        let src = "function f(H) { var pick = H.pick; var other = H.thing; }";
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        let got: Vec<String> = collect("javascript", &tree, src.as_bytes())
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert!(!got.contains(&"pick".to_string()), "{got:?}");
        assert!(got.contains(&"other".to_string()), "{got:?}");
    }

    #[test]
    fn scope_spans_are_the_block_or_function() {
        let src = "function f(a) { { const k = 1; } k(); }";
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_javascript::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        let got = collect("javascript", &tree, src.as_bytes());
        let k = got.iter().find(|b| b.name == "k").unwrap();
        let call = src.find("k()").unwrap() as u32;
        assert!(k.end_byte <= call, "the inner block ends before the call");
        let a = got.iter().find(|b| b.name == "a").unwrap();
        assert!(a.start_byte <= call && call < a.end_byte);
    }

    #[test]
    fn dart_ocaml_lua_python_rust_kotlin_swift_scala_zig() {
        type Case<'a> = (
            &'a str,
            tree_sitter::Language,
            &'a str,
            &'a [&'a str],
            &'a [&'a str],
        );
        let cases: Vec<Case<'_>> = vec![
            (
                "dart",
                tree_sitter_dart::LANGUAGE.into(),
                "void main(List<String> args) { final cb = make(); void inner() {} \
                 test('x', (tester) async { tester.pump(); }); }",
                &["args", "cb", "tester", "inner"],
                &["main", "make", "test"],
            ),
            (
                "ocaml",
                tree_sitter_ocaml::LANGUAGE_OCAML.into(),
                "let iter f s = List.iter (fun e -> f e) s",
                &["f", "s", "e"],
                &["iter"],
            ),
            (
                "lua",
                tree_sitter_lua::LANGUAGE.into(),
                "local function f(a, b) local k = g(); for i, v in ipairs(t) do end end",
                &["a", "b", "k", "i", "v"],
                &["f", "g", "t"],
            ),
            (
                "python",
                tree_sitter_python::LANGUAGE.into(),
                "def f(a, b=1, *args, c: int = 2, **kw):\n    k = 1\n    p, q = t\n",
                &["a", "b", "args", "c", "kw", "k", "p", "q"],
                &["f", "t", "int"],
            ),
            (
                "rust",
                tree_sitter_rust::LANGUAGE.into(),
                "fn f(a: i32) { let cb = |x: i32| x; let (p, q) = t; }",
                &["a", "cb", "x", "p", "q"],
                &["f", "t"],
            ),
            (
                "kotlin",
                tree_sitter_kotlin_sg::LANGUAGE.into(),
                "fun f(a: Int) { val k = 1; xs.map { e -> e } }",
                &["a", "k", "e"],
                &["f", "xs"],
            ),
            (
                "swift",
                tree_sitter_swift::LANGUAGE.into(),
                "func f(_ a: Int) { let k = 1; for it in xs {} }",
                &["a", "k", "it"],
                &["f", "xs"],
            ),
            (
                "scala",
                tree_sitter_scala::LANGUAGE.into(),
                "object O { def f(a: Int) = { val k = 1; xs.map(e => e) } }",
                &["a", "k", "e"],
                &["f", "xs", "O"],
            ),
            (
                "zig",
                tree_sitter_zig::LANGUAGE.into(),
                "fn f(a: u32) void { const k = 1; for (xs) |it| {} }",
                &["a", "k", "it"],
                &["f", "xs"],
            ),
        ];
        for (lang, ts, src, want, not) in cases {
            let got = names(lang, ts, src);
            for w in want {
                assert!(got.contains(&w.to_string()), "{lang}: {w} in {got:?}");
            }
            for n in not {
                assert!(!got.contains(&n.to_string()), "{lang}: {n} not in {got:?}");
            }
        }
    }
}
