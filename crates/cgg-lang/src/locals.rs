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
use std::collections::HashMap;
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
    /// The binder counts only when the node this many levels above it is
    /// a callable (`spec.functions`): a Go `parameter_declaration` names a
    /// parameter in a function's own list, never in a `func(x int)` type
    /// written inside one.
    owner_up: usize,
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
        owner_up: 0,
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
        owner_up: 0,
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
        owner_up: 0,
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

const fn owned(mut b: Binder, up: usize) -> Binder {
    b.owner_up = up;
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

/// Go (WS27): parameters, receivers and named results of a function,
/// method or function literal; `:=` (each name on the left), `var` and
/// `const` in a body; `range` and `for` clause variables; `if`/`switch`
/// initializers; the type-switch binding (after its value: `switch v :=
/// v.(type)`); `select` `case v := <-ch`. Package-level declarations are
/// outside every function and are not recorded.
const GO_SPEC: Spec = Spec {
    idents: &["identifier"],
    binders: &[
        // `parameter_declaration` → `parameter_list` → the callable.
        owned(b("parameter_declaration", &["name"], Scope::Param), 2),
        owned(
            b("variadic_parameter_declaration", &["name"], Scope::Param),
            2,
        ),
        b("short_var_declaration", &["left"], Scope::Decl),
        b("var_spec", &["name"], Scope::Decl),
        b("const_spec", &["name"], Scope::Decl),
        b("range_clause", &["left"], Scope::Decl),
        b("receive_statement", &["left"], Scope::Decl),
        after_value(
            b("type_switch_statement", &["alias"], Scope::Own),
            "value",
            &[],
        ),
    ],
    functions: &["function_declaration", "method_declaration", "func_literal"],
    blocks: &[
        "block",
        "statement_list",
        "if_statement",
        "for_statement",
        "expression_switch_statement",
        "type_switch_statement",
        "communication_case",
    ],
    patterns: &["expression_list"],
};

/// C# (WS27): parameters of methods, constructors, operators, indexers,
/// local functions, lambdas (`x => …` too) and anonymous methods,
/// `params` arrays included; locals (`var x`, typed, `using var`,
/// deconstruction); `foreach`, `for`, `using (…)`, `fixed` and `catch`
/// variables; pattern variables (`is T t`, `case T t`, property and
/// positional designations, `var (a, b)`) and `out var x`; query range
/// variables (`from`, `let`, `join`, `into`). A pattern or `out`
/// variable follows C#'s rule: in an `if` condition or an expression
/// statement it is in scope to the end of the enclosing block (`if (!(o
/// is Foo f)) return; f.M();`); in a loop, `using`, switch section or
/// arm, the statement. A local function's name is a binding of its
/// block. Fields and primary-constructor parameters are members, not
/// locals.
const CSHARP_SPEC: Spec = Spec {
    idents: &["identifier", "implicit_parameter"],
    binders: &[
        b("parameter", &["name"], Scope::Param),
        // A `params T[] rest` names its parameter in the list itself.
        b("parameter_list", &["name"], Scope::Param),
        b("lambda_expression", &["parameters"], Scope::Param),
        b("variable_declarator", &["name"], Scope::Decl),
        after(
            b("tuple_pattern", &["name"], Scope::Decl),
            &["variable_declarator"],
        ),
        after_value(b("foreach_statement", &["left"], Scope::Own), "right", &[]),
        b("catch_declaration", &["name"], Scope::Decl),
        b("declaration_pattern", &["name", ""], Scope::Decl),
        b("recursive_pattern", &["name"], Scope::Decl),
        b("declaration_expression", &["name"], Scope::Decl),
        b("from_clause", &["name"], Scope::Decl),
        b1("let_clause", Scope::Decl),
        b1("join_clause", Scope::Decl),
        b1("join_into_clause", Scope::Decl),
        // `… into grp select grp`: the continuation's name.
        b("query_expression", &[""], Scope::Own),
        b("local_function_statement", &["name"], Scope::Decl),
    ],
    functions: &[
        "method_declaration",
        "constructor_declaration",
        "destructor_declaration",
        "operator_declaration",
        "conversion_operator_declaration",
        "indexer_declaration",
        "accessor_declaration",
        "local_function_statement",
        "lambda_expression",
        "anonymous_method_expression",
    ],
    blocks: &[
        "block",
        "switch_section",
        "switch_expression_arm",
        "for_statement",
        "foreach_statement",
        "while_statement",
        "do_statement",
        "using_statement",
        "fixed_statement",
        "lock_statement",
        "catch_clause",
        "query_expression",
    ],
    patterns: &["parenthesized_variable_designation", "tuple_pattern"],
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
        "go" => &GO_SPEC,
        "csharp" => &CSHARP_SPEC,
        _ => return None,
    })
}

/// Every value binding of the file, in source order. Empty for a
/// language with no table.
pub fn collect(lang: &str, tree: &Tree, source: &[u8]) -> Vec<LocalBinding> {
    if lang == "c" || lang == "cpp" {
        return collect_c(tree, source, lang == "cpp");
    }
    if lang == "groovy" {
        return collect_groovy(tree, source);
    }
    if lang == "r" {
        return collect_r(tree, source);
    }
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
                if lang == "lua" {
                    lua_chunk_locals(tree, source, &mut out);
                }
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

/// What the C walk does at a node, by kind id.
#[derive(Clone, Copy, PartialEq)]
enum CRole {
    Other,
    Function,
    Declaration,
    Block,
    /// C++ `for (auto& x : xs)`: a block whose declarator binds after the
    /// range.
    ForRange,
    /// C++ `catch (const E& e)`: a block whose parameter binds over it.
    Catch,
}

/// C (WS27): a function's parameters (a K&R identifier when a K&R
/// declaration types it),
/// in scope over the whole definition; each declarator of a declaration
/// in a function body (arrays, pointers, function-pointer variables,
/// `static` locals, `for` init declarations), in scope from the end of
/// that declarator, its initializer included, to the end of its block or
/// `for`. The parameter names of a function-pointer declarator (`int
/// (*cb)(int y)`) and a block-scope prototype (`int proto(int);`) bind
/// nothing; file-scope declarations are not locals.
///
/// C's declarators do not fit the table-driven walk (a
/// `function_declarator` holds a variable only around a parenthesized
/// declarator), so this is its own single cursor walk, by kind id.
fn collect_c(tree: &Tree, src: &[u8], cpp: bool) -> Vec<LocalBinding> {
    let lang = tree.language();
    let roles: Vec<CRole> = (0..lang.node_kind_count())
        .map(|id| match lang.node_kind_for_id(id as u16).unwrap_or("") {
            "function_definition" | "lambda_expression" => CRole::Function,
            "declaration" | "condition_declaration" => CRole::Declaration,
            "compound_statement" | "for_statement" | "if_statement"
            | "switch_statement" | "while_statement" => CRole::Block,
            "for_range_loop" => CRole::ForRange,
            "catch_clause" => CRole::Catch,
            _ => CRole::Other,
        })
        .collect();
    let mut w = CWalk {
        roles,
        src,
        out: Vec::new(),
        blocks: Vec::new(),
        in_function: 0,
        cpp,
    };
    let mut cursor = tree.walk();
    w.visit(&mut cursor);
    let mut out = w.out;
    out.sort_by_key(|b| (b.start_byte, b.end_byte));
    out.dedup();
    out
}

struct CWalk<'s> {
    roles: Vec<CRole>,
    src: &'s [u8],
    out: Vec<LocalBinding>,
    /// End bytes of the blocks (and `for`s) of the current function
    /// enclosing the node being walked, innermost last.
    blocks: Vec<u32>,
    in_function: u32,
    /// C++ (`cgg_lang` calls this walk for C and C++).
    cpp: bool,
}

impl CWalk<'_> {
    fn role(&self, n: Node<'_>) -> CRole {
        self.roles
            .get(n.kind_id() as usize)
            .copied()
            .unwrap_or(CRole::Other)
    }

    fn visit(&mut self, cursor: &mut tree_sitter::TreeCursor<'_>) {
        let node = cursor.node();
        let role = self.role(node);
        match role {
            CRole::Function => {
                self.parameters(node);
                self.in_function += 1;
            }
            CRole::Declaration if self.in_function > 0 => {
                if let Some(&end) = self.blocks.last() {
                    self.declaration(node, end);
                }
            }
            CRole::Block if self.in_function > 0 => {
                self.blocks.push(node.end_byte() as u32)
            }
            CRole::ForRange if self.in_function > 0 => {
                let end = node.end_byte() as u32;
                self.blocks.push(end);
                let from = node
                    .child_by_field_name("right")
                    .map_or(node.start_byte(), |r| r.end_byte())
                    as u32;
                if let Some(d) = node.child_by_field_name("declarator") {
                    for name in c_variables(d) {
                        self.push(name, from, end, node.start_byte() as u32, false);
                    }
                }
            }
            CRole::Catch if self.in_function > 0 => {
                let end = node.end_byte() as u32;
                self.blocks.push(end);
                if let Some(list) = node.child_by_field_name("parameters") {
                    let mut c = list.walk();
                    for p in list.named_children(&mut c) {
                        if let Some(name) =
                            p.child_by_field_name("declarator").and_then(c_variable)
                        {
                            self.push(
                                name,
                                list.start_byte() as u32,
                                end,
                                list.start_byte() as u32,
                                false,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
        if cursor.goto_first_child() {
            loop {
                self.visit(cursor);
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
        match role {
            CRole::Function => self.in_function -= 1,
            CRole::Block | CRole::ForRange | CRole::Catch if self.in_function > 0 => {
                self.blocks.pop();
            }
            _ => {}
        }
    }

    fn push(&mut self, name: Node<'_>, start: u32, end: u32, decl: u32, param: bool) {
        let name = name.utf8_text(self.src).unwrap_or("").trim();
        if name.is_empty() || start >= end {
            return;
        }
        self.out.push(LocalBinding {
            name: name.to_string(),
            start_byte: start,
            end_byte: end,
            decl_byte: decl.min(start),
            param,
        });
    }

    /// The parameters of a function definition: its own declarator
    /// chain's `function_declarator` (through pointers and attributes, as
    /// in `int *f(int x)`), each `parameter_declaration`'s variable or a
    /// K&R identifier.
    fn parameters(&mut self, def: Node<'_>) {
        let mut d = def.child_by_field_name("declarator");
        let mut list = None;
        // A C++ lambda: its parameters, and its init-captures (`[x =
        // std::move(y)]`), which bind over its body; a plain capture names
        // the enclosing variable.
        if def.kind() == "lambda_expression" {
            let (start, end) = (def.start_byte() as u32, def.end_byte() as u32);
            if let Some(caps) = def.child_by_field_name("captures") {
                let mut c = caps.walk();
                for cap in caps.named_children(&mut c) {
                    if cap.kind() == "lambda_capture_initializer"
                        && let Some(left) = cap.child_by_field_name("left")
                        && left.kind() == "identifier"
                    {
                        self.push(left, start, end, start, true);
                    }
                }
            }
        }
        for _ in 0..8 {
            let Some(n) = d else { break };
            match n.kind() {
                "function_declarator" | "abstract_function_declarator" => {
                    list = n.child_by_field_name("parameters");
                    break;
                }
                "pointer_declarator"
                | "reference_declarator"
                | "attributed_declarator"
                | "parenthesized_declarator" => {
                    d = n.child_by_field_name("declarator").or_else(|| {
                        n.named_child(n.named_child_count().saturating_sub(1) as u32)
                    });
                }
                _ => break,
            }
        }
        let Some(list) = list else { return };
        let (start, end) = (def.start_byte() as u32, def.end_byte() as u32);
        // A K&R identifier is a parameter only when a declaration between
        // the declarator and the body types it (`int kr(a) int a; {…}`):
        // a macro header misparsed as a definition (`STDMETHOD_(HRESULT,
        // CreateBitmap)(…)`, which swallows the methods after it as
        // "declarations") names no parameters.
        let mut kr_declared: Vec<&str> = Vec::new();
        {
            let mut c = def.walk();
            for n in def.named_children(&mut c) {
                if self.role(n) != CRole::Declaration {
                    continue;
                }
                let mut dc = n.walk();
                for d in n.children_by_field_name("declarator", &mut dc) {
                    if let Some(id) = c_variable(d) {
                        kr_declared.push(id.utf8_text(self.src).unwrap_or(""));
                    }
                }
            }
        }
        let mut c = list.walk();
        for p in list.named_children(&mut c) {
            let name = match p.kind() {
                "identifier"
                    if kr_declared.contains(&p.utf8_text(self.src).unwrap_or("")) =>
                {
                    Some(p)
                }
                // C++ `int n = 0` and `Args&&... args` too.
                "parameter_declaration"
                | "optional_parameter_declaration"
                | "variadic_parameter_declaration" => {
                    p.child_by_field_name("declarator").and_then(c_variable)
                }
                _ => None,
            };
            if let Some(name) = name {
                self.push(name, start, end, start, true);
            }
        }
    }

    /// Each variable a block-scope declaration declares, from the end of
    /// its declarator to `end`; none after a parse error in the declaration.
    fn declaration(&mut self, decl: Node<'_>, end: u32) {
        // A declarator after a parse error is not trusted: C++ read as C
        // (`STDMETHOD(F)(…) PURE; using Base::F;`) leaves `F` as one. An
        // error inside an initializer (`T *dep = va_arg(ap, struct T *);`)
        // comes after its declarator, which stands.
        let first_error = if decl.has_error() {
            let mut c = decl.walk();
            decl.children(&mut c)
                .find(|n| n.is_error() || n.is_missing())
                .map(|n| n.start_byte())
        } else {
            None
        };
        let at = decl.start_byte() as u32;
        let mut c = decl.walk();
        let declarators: Vec<Node<'_>> =
            decl.children_by_field_name("declarator", &mut c).collect();
        // C++: an object-like macro alone on the line before a statement
        // (`_LIBCPP_SUPPRESS_DEPRECATED_PUSH` then `install(new F(n));`)
        // parses as that statement's type, making the call a declaration
        // of a variable. An ALL-CAPS type name with the declarator on a
        // later line is that macro, not a declaration.
        if self.cpp
            && let (Some(ty), Some(first)) =
                (decl.child_by_field_name("type"), declarators.first())
            && ty.kind() == "type_identifier"
            && ty.end_position().row < first.start_position().row
            && ty.utf8_text(self.src).is_ok_and(|t| {
                t.chars()
                    .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
                    && t.chars().any(|ch| ch.is_ascii_uppercase())
            })
        {
            return;
        }
        for d in declarators {
            if first_error.is_some_and(|e| e < d.start_byte()) {
                continue;
            }
            let names = c_variables(d);
            // C++'s most vexing parse: `AXTree tree(state);` in a body reads
            // as a function declaration; a block-scope function declaration
            // is rare, a variable constructed with arguments is not.
            let names = if names.is_empty()
                && self.cpp
                && d.kind() == "function_declarator"
                && vexing_parse(decl, d)
            {
                d.child_by_field_name("declarator")
                    .filter(|n| n.kind() == "identifier")
                    .into_iter()
                    .collect()
            } else {
                names
            };
            for name in names {
                self.push(name, d.end_byte() as u32, end, at, false);
            }
        }
    }
}

/// The identifier a C declarator declares as a variable: through pointer,
/// array, init, parenthesized and attributed layers, and a function
/// declarator only around a parenthesized one (`(*cb)(int)` is a
/// variable, `proto(int)` a function).
fn c_variable(mut d: Node<'_>) -> Option<Node<'_>> {
    for _ in 0..16 {
        match d.kind() {
            "identifier" => return Some(d),
            // A pointer to a function: `(*cb)(int)`. A parenthesized name
            // alone (`(f)(int)`, `STDMETHOD(Name)(…)`) declares a function.
            "function_declarator" => {
                d = d
                    .child_by_field_name("declarator")
                    .filter(|inner| inner.kind() == "parenthesized_declarator")?;
                let mut c = d.walk();
                if !d
                    .named_children(&mut c)
                    .any(|n| n.kind() == "pointer_declarator")
                {
                    return None;
                }
            }
            "pointer_declarator"
            | "array_declarator"
            | "init_declarator"
            | "parenthesized_declarator"
            | "attributed_declarator"
            | "reference_declarator"
            | "variadic_declarator" => {
                d = d.child_by_field_name("declarator").or_else(|| {
                    d.named_child(d.named_child_count().checked_sub(1)? as u32)
                })?;
            }
            _ => return None,
        }
    }
    None
}

/// C++'s most vexing parse in a function body: `AXTree tree(state);`
/// read as a function declaration. Only that shape: no `auto` return or
/// storage class (`static auto map(T) -> int;` is a declaration), and
/// every "parameter" a bare type name with no declarator, as an argument
/// expression would parse.
fn vexing_parse(decl: Node<'_>, d: Node<'_>) -> bool {
    let mut c = decl.walk();
    let plain_type = decl.children(&mut c).all(|n| {
        !matches!(
            n.kind(),
            "storage_class_specifier" | "placeholder_type_specifier" | "auto"
        )
    });
    let Some(params) = d.child_by_field_name("parameters") else {
        return false;
    };
    let mut pc = params.walk();
    let args: Vec<Node<'_>> = params.named_children(&mut pc).collect();
    plain_type
        && !args.is_empty()
        && args.iter().all(|p| {
            p.kind() == "parameter_declaration"
                && p.child_by_field_name("declarator").is_none()
                && p.child_by_field_name("type").is_some_and(|t| {
                    matches!(t.kind(), "type_identifier" | "qualified_identifier")
                })
                && p.named_child_count() == 1
        })
        && d.child_by_field_name("declarator")
            .is_some_and(|n| n.kind() == "identifier")
        && d.next_named_sibling()
            .is_none_or(|n| n.kind() != "trailing_return_type")
}

/// [`c_variable`], and each name of a C++ structured binding (`auto [a,
/// b] = …`).
fn c_variables(d: Node<'_>) -> Vec<Node<'_>> {
    let inner = if d.kind() == "init_declarator" {
        d.child_by_field_name("declarator").unwrap_or(d)
    } else {
        d
    };
    let inner = if inner.kind() == "reference_declarator" {
        inner.named_child(0).unwrap_or(inner)
    } else {
        inner
    };
    if inner.kind() == "structured_binding_declarator" {
        let mut c = inner.walk();
        return inner
            .named_children(&mut c)
            .filter(|n| n.kind() == "identifier")
            .collect();
    }
    c_variable(d).into_iter().collect()
}

/// Groovy callables: a binding is a local only inside one (or a closure).
const GROOVY_CALLABLES: &[&str] = &[
    "method_declaration",
    "constructor_declaration",
    "function_definition",
    "static_initializer",
    "lambda_expression",
    "closure",
];

/// Groovy bodies whose declarations are members (fields), not locals.
const GROOVY_TYPE_BODIES: &[&str] = &[
    "class_body",
    "interface_body",
    "enum_body",
    "enum_body_declarations",
    "annotation_type_body",
];

/// Groovy nodes a declaration's scope ends with (callables and closures
/// too).
const GROOVY_BLOCKS: &[&str] = &[
    "block",
    "constructor_body",
    "switch_block_statement_group",
    "switch_rule",
    "for_statement",
    "enhanced_for_statement",
    "program",
];

/// Groovy declarations a script's own variables are not visible in: a
/// script's methods and types see only their own and their class's names.
const GROOVY_OPAQUE: &[&str] = &[
    "class_declaration",
    "interface_declaration",
    "enum_declaration",
    "annotation_type_declaration",
    "record_declaration",
    "method_declaration",
    "constructor_declaration",
    "function_definition",
];

/// Where a closure stands as a value (an argument, a trailing closure, an
/// initializer): there one with no parameter list binds `it`. A `{ … }`
/// the parser reads as a closure elsewhere is a block (a loop or `try`
/// body read as a statement).
const GROOVY_VALUE_PARENTS: &[&str] = &[
    "method_invocation",
    "argument_list",
    "variable_declarator",
    "assignment_expression",
    "map_item",
    "return_statement",
    "parenthesized_expression",
    "ternary_expression",
    "array_literal",
    "element_value_pair",
];

/// Groovy statement keywords the parser, recovering from an error, reads
/// as the name of a call with a trailing closure (`try { … }`): that
/// closure is a block, not a value.
const GROOVY_BLOCK_KEYWORDS: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "try",
    "catch",
    "finally",
    "switch",
    "synchronized",
];

/// Words that never name a Groovy parameter.
const GROOVY_NOT_NAMES: &[&str] = &[
    "def", "var", "final", "static", "int", "long", "short", "byte", "char", "boolean",
    "float", "double", "void", "in", "as", "new", "this", "super", "true", "false",
    "null", "return", "if", "else", "for", "while",
];

/// Groovy (WS27): parameters of methods, constructors and script
/// functions, in scope over the definition; closure parameters (`{ a, b
/// -> … }`, typed or not) and lambda parameters over the closure or
/// lambda, and the implicit `it` of a closure with no parameter list
/// that stands as a value; `def` and typed locals, from the end of their
/// declaration to the end of their block, closure or callable; classic
/// `for` initializers; `for (x in xs)` / `for (T x : xs)` after the
/// iterated value; `catch` parameters; try-with-resources; multiple
/// assignment `def (a, b) = …`. Fields and properties are members, not
/// locals.
///
/// tree-sitter-groovy reads much of Groovy as Java with errors: an
/// untyped parameter is a type with an empty name, a closure's parameter
/// list is a local declaration, a call or an `array_initializer` before a
/// lambda with no parameters, and `def (a, b) =` is an ERROR. So a
/// parameter list is read from its text between the delimiters the tree
/// places (`(`…`)`, `{`…`->`), each parameter kept only when it is
/// `[annotations] [modifiers] [Type] name [= default]`; nothing else
/// inside an ERROR node is read.
///
/// A script's own `def x = …` (outside every class and method) is in
/// scope in the script body and its closures, not in the script's
/// methods or classes. Only in a file that declares no type: where a
/// `class`, `interface` or `enum` appears, a top-level declaration may be
/// a member of a class the parser lost.
fn collect_groovy(tree: &Tree, src: &[u8]) -> Vec<LocalBinding> {
    let mut w = GroovyWalk {
        src,
        out: Vec::new(),
        arrows: HashMap::new(),
        script: Vec::new(),
        opaque: Vec::new(),
        declares_type: false,
    };
    let mut chain: Vec<Node<'_>> = Vec::new();
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        w.visit(node, &chain);
        if cursor.goto_first_child() {
            chain.push(node);
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                w.finish_script();
                let mut out = w.out;
                out.sort_by_key(|b| (b.start_byte, b.end_byte));
                out.dedup();
                return out;
            }
            chain.pop();
        }
    }
}

/// Where a Groovy declaration lives.
enum GroovyCtx {
    /// A member of a type: not a local.
    Field,
    /// Inside a callable or closure, in scope to this byte.
    Local(u32),
    /// A script's own variable, in scope to this byte outside the
    /// script's methods and types.
    Script(u32),
}

struct GroovyWalk<'s> {
    src: &'s [u8],
    out: Vec<LocalBinding>,
    /// Closures with a parameter list, by node id: where its `->` starts
    /// and the id of the lambda the parser hung the body on.
    arrows: HashMap<usize, (u32, usize)>,
    /// Script variables: name, declaration start, scope start and end.
    script: Vec<(String, u32, u32, u32)>,
    /// Outermost spans of the file's types and methods, in source order.
    opaque: Vec<(u32, u32)>,
    /// The file declares a type somewhere (or an ERROR holds the keyword
    /// of one).
    declares_type: bool,
}

impl GroovyWalk<'_> {
    fn text(&self, n: Node<'_>) -> &str {
        n.utf8_text(self.src).unwrap_or("")
    }

    fn push(&mut self, name: &str, start: usize, end: usize, decl: usize, param: bool) {
        if !groovy_ident(name) || start >= end {
            return;
        }
        self.out.push(LocalBinding {
            name: name.to_string(),
            start_byte: start as u32,
            end_byte: end as u32,
            decl_byte: decl.min(start) as u32,
            param,
        });
    }

    fn visit(&mut self, node: Node<'_>, chain: &[Node<'_>]) {
        let kind = node.kind();
        if GROOVY_OPAQUE.contains(&kind)
            && self
                .opaque
                .last()
                .is_none_or(|&(_, e)| (node.start_byte() as u32) >= e)
        {
            self.opaque
                .push((node.start_byte() as u32, node.end_byte() as u32));
        }
        if matches!(kind, "class" | "interface" | "enum")
            && chain.last().is_none_or(|p| p.kind() != "class_literal")
        {
            self.declares_type = true;
        }
        match kind {
            // A method's, constructor's or script function's own list.
            "formal_parameters" => {
                let Some(owner) = chain.last().copied() else {
                    return;
                };
                if !matches!(
                    owner.kind(),
                    "method_declaration"
                        | "constructor_declaration"
                        | "function_definition"
                ) || owner.child_by_field_name("parameters").map(|p| p.id())
                    != Some(node.id())
                {
                    return;
                }
                let inner = self.text(node);
                let inner = inner
                    .strip_prefix('(')
                    .and_then(|t| t.strip_suffix(')'))
                    .unwrap_or("");
                for name in groovy_param_names(inner) {
                    self.push(
                        &name,
                        owner.start_byte(),
                        owner.end_byte(),
                        owner.start_byte(),
                        true,
                    );
                }
            }
            "closure" | "array_initializer" => self.closure(node, chain),
            "lambda_expression" => {
                let handled = chain.iter().rev().take(2).any(|p| {
                    self.arrows
                        .get(&p.id())
                        .is_some_and(|&(_, lambda)| lambda == node.id())
                });
                if handled {
                    return;
                }
                let Some(params) = node.child_by_field_name("parameters") else {
                    return;
                };
                let names: Vec<String> = match params.kind() {
                    "identifier" => vec![self.text(params).to_string()],
                    "inferred_parameters" => {
                        let mut c = params.walk();
                        params
                            .named_children(&mut c)
                            .filter(|n| n.kind() == "identifier")
                            .map(|n| self.text(n).to_string())
                            .collect()
                    }
                    "formal_parameters" => {
                        let t = self.text(params);
                        let t = t
                            .strip_prefix('(')
                            .and_then(|t| t.strip_suffix(')'))
                            .unwrap_or("");
                        groovy_param_names(t)
                    }
                    _ => Vec::new(),
                };
                for name in names {
                    self.push(
                        &name,
                        node.start_byte(),
                        node.end_byte(),
                        node.start_byte(),
                        true,
                    );
                }
            }
            "local_variable_declaration" => {
                // A closure's parameter list the parser read as a
                // declaration (`{ a, b -> … }`).
                if let Some(p) = chain.last()
                    && let Some(&(arrow, _)) = self.arrows.get(&p.id())
                    && (node.start_byte() as u32) < arrow
                {
                    return;
                }
                if !groovy_declaration(node, self.src) {
                    return;
                }
                let mut names = Vec::new();
                let mut c = node.walk();
                for d in node.children_by_field_name("declarator", &mut c) {
                    if let Some(n) = d.child_by_field_name("name")
                        && n.kind() == "identifier"
                    {
                        names.push(self.text(n).to_string());
                    }
                }
                let start = groovy_statement_end(self.src, node.end_byte());
                self.declare(names, node.start_byte(), start, chain);
            }
            // `def (a, b) = …`: the parser stops at `=` and reads the
            // list as parameters; the value is the next statement.
            "ERROR" if node.child_count() == 3 => {
                let (Some(kw), Some(list), Some(eq)) =
                    (node.child(0), node.child(1), node.child(2))
                else {
                    return;
                };
                if kw.kind() != "def" || eq.kind() != "=" {
                    return;
                }
                let names: Vec<String> = match list.kind() {
                    "inferred_parameters" => {
                        let mut c = list.walk();
                        list.named_children(&mut c)
                            .filter(|n| n.kind() == "identifier")
                            .map(|n| self.text(n).to_string())
                            .collect()
                    }
                    "formal_parameters" => {
                        let t = self.text(list);
                        let t = t
                            .strip_prefix('(')
                            .and_then(|t| t.strip_suffix(')'))
                            .unwrap_or("");
                        groovy_param_names(t)
                    }
                    _ => return,
                };
                let start = node
                    .next_named_sibling()
                    .map_or(node.end_byte(), |v| v.end_byte());
                self.declare(names, node.start_byte(), start, chain);
            }
            // `for (x in xs)` / `for (T x : xs)`: after the iterated value.
            "enhanced_for_statement" => {
                if let Some(n) = node.child_by_field_name("name")
                    && n.kind() == "identifier"
                {
                    let from = node
                        .child_by_field_name("value")
                        .map_or(n.end_byte(), |v| v.end_byte());
                    let name = self.text(n).to_string();
                    self.push(&name, from, node.end_byte(), node.start_byte(), false);
                }
            }
            "catch_formal_parameter" => {
                if let (Some(n), Some(clause)) =
                    (node.child_by_field_name("name"), chain.last().copied())
                    && clause.kind() == "catch_clause"
                {
                    let name = self.text(n).to_string();
                    self.push(
                        &name,
                        n.end_byte(),
                        clause.end_byte(),
                        clause.start_byte(),
                        false,
                    );
                }
            }
            // `try (def r = open()) { … }`.
            "resource" => {
                if let Some(n) = node.child_by_field_name("name")
                    && let Some(t) = chain
                        .iter()
                        .rev()
                        .find(|p| p.kind() == "try_with_resources_statement")
                {
                    let name = self.text(n).to_string();
                    self.push(
                        &name,
                        node.end_byte(),
                        t.end_byte(),
                        node.start_byte(),
                        false,
                    );
                }
            }
            _ => {}
        }
    }

    /// A closure (or the `array_initializer` the parser reads one with a
    /// bare parameter list as): its parameters before `->`, else `it`.
    fn closure(&mut self, node: Node<'_>, chain: &[Node<'_>]) {
        let lambda = groovy_closure_lambda(node);
        if node.kind() == "array_initializer" && lambda.is_none() {
            return;
        }
        let Some(lambda) = lambda else {
            // `xs.each { it.go() }`.
            let Some(parent) = chain.last().copied() else {
                return;
            };
            let block_call = parent.kind() == "method_invocation"
                && parent
                    .child_by_field_name("name")
                    .is_some_and(|n| GROOVY_BLOCK_KEYWORDS.contains(&self.text(n)));
            if GROOVY_VALUE_PARENTS.contains(&parent.kind()) && !block_call {
                self.out.push(LocalBinding {
                    name: "it".to_string(),
                    start_byte: node.start_byte() as u32,
                    end_byte: node.end_byte() as u32,
                    decl_byte: node.start_byte() as u32,
                    param: true,
                });
            }
            return;
        };
        let mut c = lambda.walk();
        let Some(arrow) = lambda.children(&mut c).find(|n| n.kind() == "->") else {
            return;
        };
        self.arrows
            .insert(node.id(), (arrow.start_byte() as u32, lambda.id()));
        let text = self
            .src
            .get(node.start_byte()..arrow.start_byte())
            .and_then(|t| std::str::from_utf8(t).ok())
            .and_then(|t| t.strip_prefix('{'))
            .unwrap_or("");
        for name in groovy_param_names(text) {
            self.push(
                &name,
                node.start_byte(),
                node.end_byte(),
                node.start_byte(),
                true,
            );
        }
    }

    /// Record a declaration of `names` whose statement starts at `decl`
    /// and whose names are in scope from `start`.
    fn declare(
        &mut self,
        names: Vec<String>,
        decl: usize,
        start: usize,
        chain: &[Node<'_>],
    ) {
        match groovy_ctx(chain, &self.arrows) {
            GroovyCtx::Field => {}
            GroovyCtx::Local(end) => {
                for n in names {
                    self.push(&n, start, end as usize, decl, false);
                }
            }
            GroovyCtx::Script(end) => {
                for n in names {
                    self.script.push((n, decl as u32, start as u32, end));
                }
            }
        }
    }

    /// A script variable is in scope over the rest of its block, except
    /// inside the script's methods and types.
    fn finish_script(&mut self) {
        if self.declares_type {
            return;
        }
        let opaque = std::mem::take(&mut self.opaque);
        for (name, decl, start, end) in std::mem::take(&mut self.script) {
            let mut from = start;
            for &(s, e) in &opaque {
                if e <= from || s >= end {
                    continue;
                }
                if from < s {
                    self.push(&name, from as usize, s as usize, decl as usize, false);
                }
                from = from.max(e);
            }
            if from < end {
                self.push(&name, from as usize, end as usize, decl as usize, false);
            }
        }
    }
}

/// Whether a declaration under `chain` is a member, a local or a
/// script variable, and where its scope ends.
fn groovy_ctx(chain: &[Node<'_>], arrows: &HashMap<usize, (u32, usize)>) -> GroovyCtx {
    let mut block_end: Option<u32> = None;
    for n in chain.iter().rev() {
        let k = n.kind();
        let closure_like = k == "array_initializer" && arrows.contains_key(&n.id());
        if GROOVY_CALLABLES.contains(&k) || closure_like {
            return GroovyCtx::Local(block_end.unwrap_or(n.end_byte() as u32));
        }
        if GROOVY_TYPE_BODIES.contains(&k) {
            // A block directly in a class body is an initializer.
            return match block_end {
                Some(e) => GroovyCtx::Local(e),
                None => GroovyCtx::Field,
            };
        }
        if k == "program" {
            return GroovyCtx::Script(block_end.unwrap_or(n.end_byte() as u32));
        }
        if block_end.is_none() && GROOVY_BLOCKS.contains(&k) {
            block_end = Some(n.end_byte() as u32);
        }
    }
    GroovyCtx::Field
}

/// The lambda a closure's body hangs on when the closure has a parameter
/// list (`{ a -> … }`, `{ a, b -> … }`, `{ String s, int n -> … }`,
/// `{ -> … }`): its first named child, or the first after what the
/// parser made of the parameters, alone or as an expression statement.
fn groovy_closure_lambda(n: Node<'_>) -> Option<Node<'_>> {
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        match ch.kind() {
            "lambda_expression" => return Some(ch),
            "expression_statement" => {
                return ch
                    .named_child(0)
                    .filter(|l| l.kind() == "lambda_expression");
            }
            "local_variable_declaration" | "juxt_function_call" | "identifier" => {}
            _ => return None,
        }
    }
    None
}

/// Whether an ERROR sits directly in a Groovy declaration or one of its
/// declarators (`once per project`, a lost method header hung on a
/// field). An error inside a value (`new X([:])`) or the MISSING `;` the
/// grammar expects after every statement is not one.
pub(crate) fn groovy_declaration_has_error(node: Node<'_>) -> bool {
    let mut c = node.walk();
    node.children(&mut c).any(|k| {
        k.is_error()
            || (k.kind() == "variable_declarator" && {
                let mut dc = k.walk();
                k.children(&mut dc).any(|d| d.is_error())
            })
    })
}

/// Where the statement a declaration ending at `end` begins really ends:
/// past each following line that continues it (`? a()`, `: b()`,
/// `.c()`, `&& d`, `+ e`), which the parser reads as a statement of its
/// own. The initializer on those lines still sees the outer name
/// (`def runner = dir ? withDir(dir) : runner()`).
fn groovy_statement_end(src: &[u8], end: usize) -> usize {
    let mut pos = end.min(src.len());
    loop {
        let Some(off) = src[pos..].iter().position(|b| !b.is_ascii_whitespace()) else {
            return pos;
        };
        let next = pos + off;
        let crosses_line = src[pos..next].contains(&b'\n')
            || src.get(pos.wrapping_sub(1)) == Some(&b'\n');
        if !crosses_line || !matches!(src[next], b'.' | b'?' | b':' | b'&' | b'|' | b'+')
        {
            return pos;
        }
        pos = src[next..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(src.len(), |i| next + i + 1);
    }
}

/// Whether a `local_variable_declaration` is one. The parser also reads a
/// command expression (`succeeds tasks(':app')`, `a.extendsFrom
/// provider(b)`), a method header it lost (`private Task task(String
/// name) {`) and words inside a multi-line string as declarations. A
/// declaration ends its statement (a newline, `;`, `}` or the end of the
/// file follows it on its line), holds no ERROR of its own, and is typed by `def`,
/// `var` or modifiers alone, a primitive, or a type whose simple name is
/// capitalized, with a value; a command expression's verb is a
/// lowercase word, and a word or two alone on a line (text of a
/// multi-line string the parser read as code) has no type, or no value.
fn groovy_declaration(node: Node<'_>, src: &[u8]) -> bool {
    if groovy_declaration_has_error(node) {
        return false;
    }
    let text = src.get(node.start_byte()..node.end_byte()).unwrap_or(&[]);
    let body_end = text
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(0, |i| i + 1);
    let trailing = &text[body_end..];
    let terminated = text.get(body_end.wrapping_sub(1)) == Some(&b';')
        || trailing.contains(&b'\n')
        || src[node.end_byte()..]
            .iter()
            .find(|b| !matches!(b, b' ' | b'\t'))
            .is_none_or(|b| matches!(b, b'\n' | b'\r' | b';' | b'}' | b'/'));
    if !terminated {
        return false;
    }
    let Some(ty) = node.child_by_field_name("type") else {
        // `def x = …`, `final x = …`; a bare word on its own line (text
        // the parser took for code) declares nothing.
        let mut c = node.walk();
        return node
            .children(&mut c)
            .any(|k| matches!(k.kind(), "def" | "var" | "modifiers"));
    };
    let simple = match ty.kind() {
        "type_identifier" => Some(ty),
        "scoped_type_identifier" => {
            ty.named_child(ty.named_child_count().saturating_sub(1) as u32)
        }
        "generic_type" => ty.named_child(0).and_then(|t| match t.kind() {
            "scoped_type_identifier" => {
                t.named_child(t.named_child_count().saturating_sub(1) as u32)
            }
            _ => Some(t),
        }),
        "array_type" => ty.child_by_field_name("element").map(|e| {
            if e.kind() == "generic_type" {
                e.named_child(0).unwrap_or(e)
            } else {
                e
            }
        }),
        // `int`, `boolean`, `void`, …
        _ => return true,
    };
    // `String name` with no value reads the same as two words of a
    // multi-line string the parser took for code (`No dependencies`).
    let mut c = node.walk();
    if !node
        .children_by_field_name("declarator", &mut c)
        .any(|d| d.child_by_field_name("value").is_some())
    {
        return false;
    }
    let Some(simple) = simple else { return false };
    if simple.kind() != "type_identifier" {
        return true;
    }
    let name = simple.utf8_text(src).unwrap_or("");
    name == "def" || name == "var" || name.chars().next().is_some_and(char::is_uppercase)
}

fn groovy_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && cs.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
        && s != "_"
}

/// The names a Groovy parameter list's text declares (without its
/// delimiters): each comma-separated parameter that reads as
/// `[@Annotation(…)…] [modifiers] [Type] name [= default]`. A piece that
/// does not is skipped.
fn groovy_param_names(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut piece_start = 0usize;
    let mut chars = text.char_indices().peekable();
    let mut pieces: Vec<&str> = Vec::new();
    while let Some((i, ch)) = chars.next() {
        if let Some(q) = quote {
            if ch == '\\' {
                chars.next();
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth = (depth - 1).max(0),
            ',' if depth == 0 => {
                pieces.push(&text[piece_start..i]);
                piece_start = i + 1;
            }
            _ => {}
        }
    }
    pieces.push(&text[piece_start..]);
    for p in pieces {
        if let Some(n) = groovy_param_name(p) {
            out.push(n);
        }
    }
    out
}

fn groovy_param_name(piece: &str) -> Option<String> {
    // The default value.
    let mut p = piece;
    let mut depth = 0i32;
    for (i, ch) in piece.char_indices() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth = (depth - 1).max(0),
            '=' if depth == 0 => {
                p = &piece[..i];
                break;
            }
            _ => {}
        }
    }
    let mut p = p.trim();
    // Annotations: `@Name`, `@a.b.Name(…)`.
    while let Some(rest) = p.strip_prefix('@') {
        let end = rest
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '.'))
            .unwrap_or(rest.len());
        let mut rest = rest[end..].trim_start();
        if rest.starts_with('(') {
            let mut d = 0i32;
            let mut close = None;
            for (i, ch) in rest.char_indices() {
                match ch {
                    '(' => d += 1,
                    ')' => {
                        d -= 1;
                        if d == 0 {
                            close = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            rest = rest[close? + 1..].trim_start();
        }
        p = rest;
    }
    // Type arguments carry their own spaces and commas.
    let mut flat = String::with_capacity(p.len());
    let mut depth = 0i32;
    for ch in p.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = (depth - 1).max(0),
            _ if depth == 0 => flat.push(ch),
            _ => {}
        }
    }
    let tokens: Vec<&str> = flat.split_whitespace().collect();
    let (last, lead) = tokens.split_last()?;
    let type_like = |t: &str| {
        !t.is_empty()
            && t.chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '$' | '.' | '[' | ']'))
    };
    if !lead.iter().all(|t| type_like(t.trim_end_matches("..."))) {
        return None;
    }
    let name = last.trim_start_matches("...").trim_end_matches("[]");
    (groovy_ident(name) && !GROOVY_NOT_NAMES.contains(&name)).then(|| name.to_string())
}

/// Whether an R local assigned a literal (`f <- 1`, `f <- "x"`) is
/// recorded: a call `f()` looks only for a function binding, so a local
/// holding a number or a string does not hide the function `f`.
const R_LITERAL_LOCALS_SHADOW: bool = false;

/// R (WS27): a function's parameters (`function(x, y = 1, ...)`, `\(x)`),
/// in scope over the function; inside a function body, a variable
/// assigned with `<-`, `=` or `->` or by `assign("x", …)` with a literal
/// name and no environment argument, and a `for` variable (after its
/// sequence), each in scope to the end of the function: R scopes a local
/// by function, not by block, and a nested function sees it. `<<-` and
/// `->>` assign an enclosing or global binding, never a local; a
/// top-level assignment is the file's, not a local. A call `f()` looks
/// only for a function, so a binding known to hold something else does
/// not shadow one: a local assigned a literal number or string
/// (`R_LITERAL_LOCALS_SHADOW`), a parameter whose default is a number,
/// string or logical, and a binding of a call of its own name (`x <-
/// x()`, `tz = tz()`), which holds that function's result. One assigned
/// a function or any other value (whose kind is unknown) does.
fn collect_r(tree: &Tree, src: &[u8]) -> Vec<LocalBinding> {
    let mut out = Vec::new();
    // The enclosing function definitions of the current node.
    let mut funcs: Vec<Node<'_>> = Vec::new();
    let mut chain: Vec<Node<'_>> = Vec::new();
    let mut cursor = tree.walk();
    let text = |n: Node<'_>| n.utf8_text(src).unwrap_or("");
    let push = |out: &mut Vec<LocalBinding>,
                name: &str,
                start: usize,
                end: usize,
                decl: usize,
                param: bool| {
        let name = name.trim_matches('`');
        if name.is_empty() || name == "..." || start >= end {
            return;
        }
        out.push(LocalBinding {
            name: name.to_string(),
            start_byte: start as u32,
            end_byte: end as u32,
            decl_byte: decl.min(start) as u32,
            param,
        });
    };
    loop {
        let node = cursor.node();
        match node.kind() {
            "function_definition" => {
                if let Some(ps) = node.child_by_field_name("parameters") {
                    let mut c = ps.walk();
                    for p in ps.children_by_field_name("parameter", &mut c) {
                        if let Some(n) = p.child_by_field_name("name")
                            && n.kind() == "identifier"
                            && !p.child_by_field_name("default").is_some_and(|d| {
                                r_holds_no_function(d, text(n), src, true)
                            })
                        {
                            push(
                                &mut out,
                                text(n),
                                node.start_byte(),
                                node.end_byte(),
                                node.start_byte(),
                                true,
                            );
                        }
                    }
                }
            }
            "binary_operator" if !funcs.is_empty() => {
                let op = node.child_by_field_name("operator").map(text).unwrap_or("");
                let (target, value) = match op {
                    "<-" | "=" => (
                        node.child_by_field_name("lhs"),
                        node.child_by_field_name("rhs"),
                    ),
                    "->" => (
                        node.child_by_field_name("rhs"),
                        node.child_by_field_name("lhs"),
                    ),
                    _ => (None, None),
                };
                if let (Some(t), Some(v)) = (target, value)
                    && matches!(t.kind(), "identifier" | "string")
                    && !r_holds_no_function(v, r_target_name(t, src), src, false)
                {
                    let name = if t.kind() == "string" {
                        t.child_by_field_name("content").map(text).unwrap_or("")
                    } else {
                        text(t)
                    };
                    let f = funcs[funcs.len() - 1];
                    push(
                        &mut out,
                        name,
                        node.end_byte(),
                        f.end_byte(),
                        node.start_byte(),
                        false,
                    );
                }
            }
            "call" if !funcs.is_empty() => {
                if let Some(f) = node.child_by_field_name("function")
                    && f.kind() == "identifier"
                    && cgg_core::stdlib::R_ASSIGN_FUNCTIONS.contains(&text(f))
                    && let Some(args) = node.child_by_field_name("arguments")
                {
                    let mut c = args.walk();
                    let args: Vec<Node<'_>> =
                        args.children_by_field_name("argument", &mut c).collect();
                    let elsewhere = args.iter().any(|a| {
                        a.child_by_field_name("name").is_some_and(|n| {
                            cgg_core::stdlib::R_ASSIGN_ELSEWHERE_ARGS.contains(&text(n))
                        })
                    });
                    let positional: Vec<&Node<'_>> = args
                        .iter()
                        .filter(|a| a.child_by_field_name("name").is_none())
                        .collect();
                    if !elsewhere
                        && positional.len() == 2
                        && let Some(name) = positional[0].child_by_field_name("value")
                        && name.kind() == "string"
                        && let Some(value) = positional[1].child_by_field_name("value")
                        && !r_holds_no_function(
                            value,
                            name.child_by_field_name("content").map(text).unwrap_or(""),
                            src,
                            false,
                        )
                    {
                        let n =
                            name.child_by_field_name("content").map(text).unwrap_or("");
                        let f = funcs[funcs.len() - 1];
                        push(
                            &mut out,
                            n,
                            node.end_byte(),
                            f.end_byte(),
                            node.start_byte(),
                            false,
                        );
                    }
                }
            }
            "for_statement" if !funcs.is_empty() => {
                if let Some(v) = node.child_by_field_name("variable")
                    && v.kind() == "identifier"
                {
                    let from = node
                        .child_by_field_name("sequence")
                        .map_or(v.end_byte(), |s| s.end_byte());
                    let f = funcs[funcs.len() - 1];
                    push(
                        &mut out,
                        text(v),
                        from,
                        f.end_byte(),
                        node.start_byte(),
                        false,
                    );
                }
            }
            _ => {}
        }
        if cursor.goto_first_child() {
            if node.kind() == "function_definition" {
                funcs.push(node);
            }
            chain.push(node);
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                out.sort_by_key(|b| (b.start_byte, b.end_byte));
                out.dedup();
                return out;
            }
            if chain
                .pop()
                .is_some_and(|n| n.kind() == "function_definition")
            {
                funcs.pop();
            }
        }
    }
}

/// The name an R assignment target binds (`x`, `` `x` ``, `"x"`).
fn r_target_name<'s>(t: Node<'_>, src: &'s [u8]) -> &'s str {
    let n = if t.kind() == "string" {
        t.child_by_field_name("content")
    } else {
        Some(t)
    };
    n.and_then(|n| n.utf8_text(src).ok())
        .unwrap_or("")
        .trim_matches('`')
}

/// Whether the value an R binding of `name` holds is known not to be a
/// function, so a call `name()` (which R resolves to a function binding
/// only, skipping any other) passes it by: a literal (for a local, under
/// `R_LITERAL_LOCALS_SHADOW`; for a parameter's default, a number,
/// string or logical, since `NULL` is how an optional function is
/// written), or a call of the function of that very name (`x <- x()`,
/// `tz = tz()`), whose result the binding holds.
fn r_holds_no_function(v: Node<'_>, name: &str, src: &[u8], param: bool) -> bool {
    let own_call = v.kind() == "call"
        && v.child_by_field_name("function").is_some_and(|f| {
            f.kind() == "identifier"
                && f.utf8_text(src).unwrap_or("").trim_matches('`') == name
        });
    let literal = if param {
        r_literal(v) && !matches!(v.kind(), "null" | "na")
    } else {
        !R_LITERAL_LOCALS_SHADOW && r_literal(v)
    };
    own_call || literal
}

/// An R literal number, string or constant, negated or not: a value no
/// call can reach.
fn r_literal(v: Node<'_>) -> bool {
    match v.kind() {
        "float" | "integer" | "complex" | "string" | "na" | "true" | "false" | "null"
        | "inf" | "nan" => true,
        "unary_operator" => v.child_by_field_name("rhs").is_some_and(r_literal),
        "parenthesized_expression" => {
            v.child_by_field_name("body").is_some_and(r_literal)
        }
        _ => false,
    }
}

/// Lua `local` declarations at file scope (WS19): in scope from the end of
/// the statement to the end of the file. `local log = ngx.log` is a value
/// the file holds, so a bare `log(…)` is never some required module's
/// `log`. A local that reads the same-named member of a call's result
/// (`local f = require(…).f`) or of an earlier file-scope local holding
/// one (`local t = require(…)` then `local eq = t.eq`) is another name for
/// that member, and is not recorded: the module reading of `eq(…)`
/// stands. Neither is a local declared without a value.
fn lua_chunk_locals(tree: &Tree, source: &[u8], out: &mut Vec<LocalBinding>) {
    let root = tree.root_node();
    let end = root.end_byte() as u32;
    // Names of earlier file-scope locals whose value is a call.
    let mut call_held: Vec<String> = Vec::new();
    let text = |n: Node<'_>| n.utf8_text(source).unwrap_or("").trim().to_string();
    let mut c = root.walk();
    for stmt in root.named_children(&mut c) {
        if stmt.kind() != "variable_declaration" {
            continue;
        }
        let mut sc = stmt.walk();
        let inner = stmt.named_children(&mut sc).next();
        let (names, values) = match inner {
            Some(a) if a.kind() == "assignment_statement" => {
                let mut ac = a.walk();
                let kids: Vec<Node<'_>> = a.named_children(&mut ac).collect();
                let names = kids.iter().find(|k| k.kind() == "variable_list").copied();
                let values = kids.iter().find(|k| k.kind() == "expression_list").copied();
                (names, values)
            }
            Some(l) if l.kind() == "variable_list" => (Some(l), None),
            _ => continue,
        };
        let Some(names) = names else { continue };
        let mut nc = names.walk();
        let names: Vec<Node<'_>> = names
            .named_children(&mut nc)
            .filter(|n| n.kind() == "identifier")
            .collect();
        let values: Vec<Node<'_>> = values
            .map(|v| {
                let mut vc = v.walk();
                v.named_children(&mut vc).collect()
            })
            .unwrap_or_default();
        for (i, name) in names.iter().enumerate() {
            let name = text(*name);
            if name.is_empty() || name == "_" {
                continue;
            }
            // `local x` alone, assigned later (often inside a function):
            // what it will hold is not known here.
            let Some(value) = values.get(i).copied() else {
                continue;
            };
            let value = Some(value);
            if let Some(v) = value
                && matches!(v.kind(), "dot_index_expression" | "method_index_expression")
            {
                let field = v
                    .child_by_field_name("field")
                    .or_else(|| v.child_by_field_name("method"))
                    .map(text);
                let table = v.child_by_field_name("table");
                let held = table.is_some_and(|t| {
                    t.kind() == "function_call" || call_held.contains(&text(t))
                });
                if field.as_deref() == Some(name.as_str()) && held {
                    continue;
                }
            }
            if value.is_some_and(|v| v.kind() == "function_call") {
                call_held.push(name.clone());
            }
            out.push(LocalBinding {
                name,
                start_byte: stmt.end_byte() as u32,
                end_byte: end,
                decl_byte: stmt.start_byte() as u32,
                param: false,
            });
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
                && (b.owner_up == 0
                    || level
                        .checked_sub(1 + b.owner_up)
                        .and_then(|i| chain.get(i))
                        .is_some_and(|(o, _)| spec.functions.contains(&o.kind())))
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
                // A parameter list, a lambda that holds its parameters
                // directly (JS `x => …`, a Scala lambda), or a declaration
                // inside a parameter list (a Kotlin lambda's `{ x: T -> }`
                // is a `variable_declaration` under `lambda_parameters`):
                // a parameter, whose scope is that callable.
                param: binder.scope == Scope::Param
                    || spec.functions.contains(&binder.kind)
                    || grand.is_some_and(|g| {
                        spec.binders
                            .iter()
                            .any(|b| b.kind == g.kind() && b.scope == Scope::Param)
                    }),
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

    /// WS19 gap 3: a lambda's or closure's parameter is marked a
    /// parameter, like a named function's; a local bound by assignment
    /// is not.
    #[test]
    fn closure_parameters_are_marked_parameters() {
        let cases: Vec<(&str, tree_sitter::Language, &str)> = vec![
            (
                "javascript",
                tree_sitter_javascript::LANGUAGE.into(),
                "function f(p) { const loc = 1; xs.map((go) => go()); xs.map(z => z()); }",
            ),
            (
                "python",
                tree_sitter_python::LANGUAGE.into(),
                "def f(p):\n    loc = 1\n    g = lambda go: go()\n",
            ),
            (
                "rust",
                tree_sitter_rust::LANGUAGE.into(),
                "fn f(p: u8) { let loc = 1; let g = |go: fn()| go(); }",
            ),
            (
                "ocaml",
                tree_sitter_ocaml::LANGUAGE_OCAML.into(),
                "let f p = let loc = 1 in let g = fun go -> go () in g",
            ),
            (
                "kotlin",
                tree_sitter_kotlin_sg::LANGUAGE.into(),
                "fun f(p: Int) { val loc = 1; val g = { go: () -> Unit -> go() } }",
            ),
            (
                "scala",
                tree_sitter_scala::LANGUAGE.into(),
                "object O { def f(p: Int): Unit = { val loc = 1; val g = (go: () => Unit) => go() } }",
            ),
        ];
        for (lang, ts, src) in cases {
            let mut p = tree_sitter::Parser::new();
            p.set_language(&ts).unwrap();
            let tree = p.parse(src, None).unwrap();
            let got = collect(lang, &tree, src.as_bytes());
            let param = |n: &str| {
                got.iter()
                    .filter(|b| b.name == n)
                    .map(|b| b.param)
                    .collect::<Vec<_>>()
            };
            assert_eq!(param("p"), vec![true], "{lang}: {got:?}");
            assert_eq!(param("go"), vec![true], "{lang}: {got:?}");
            if lang != "ocaml" {
                assert_eq!(param("loc"), vec![false], "{lang}: {got:?}");
            }
            if lang == "javascript" {
                assert_eq!(param("z"), vec![true], "{lang}: {got:?}");
            }
        }
    }

    /// WS19: Lua file-scope `local`s are bindings from the end of their
    /// statement to the end of the file; a same-named member read of an
    /// earlier local holding a call's result is an alias, not a binding.
    #[test]
    fn lua_file_scope_locals() {
        let src = "local t = require('m')\nlocal log = ngx.log\nlocal eq = t.eq\nlocal g = require('m').g\nlocal n = 1\nlocal later\nfunction f() log(1) end\n";
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_lua::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        let got = collect("lua", &tree, src.as_bytes());
        let names: Vec<&str> = got.iter().map(|b| b.name.as_str()).collect();
        for want in ["t", "log", "n"] {
            assert!(names.contains(&want), "{want} in {got:?}");
        }
        for not in ["eq", "g", "later"] {
            assert!(!names.contains(&not), "{not} in {got:?}");
        }
        let log = got.iter().find(|b| b.name == "log").unwrap();
        assert_eq!(log.start_byte as usize, src.find("\nlocal eq").unwrap());
        assert_eq!(log.end_byte as usize, src.len());
        assert!(!log.param);
    }

    fn go(src: &str) -> Vec<LocalBinding> {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_go::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        collect("go", &tree, src.as_bytes())
    }

    /// The binding of `name` whose scope holds the `nth` occurrence of
    /// `at` in `src`.
    fn go_bound_at(
        got: &[LocalBinding],
        src: &str,
        name: &str,
        at: &str,
        nth: usize,
    ) -> bool {
        let site = src.match_indices(at).nth(nth).expect(at).0 as u32;
        got.iter()
            .any(|b| b.name == name && b.start_byte <= site && site < b.end_byte)
    }

    /// WS27: every Go binding form inside a function is a local; the
    /// package's own `var`/`const`, function names and a `func(x T)`
    /// type's parameter names are not.
    #[test]
    fn go_binding_forms() {
        let src = "package p\n\
                   var top = 1\n\
                   const k0 = 2\n\
                   func (r *R) m(a, b int, rest ...string) (n int, err error) {\n\
                     var cb func(typeParam int)\n\
                     s := 1\n\
                     x, y := f()\n\
                     var v1, v2 = 1, 2\n\
                     const c1 = 3\n\
                     for i := 0; i < 3; i++ {}\n\
                     for key, val := range xs {}\n\
                     if e := g(); e != nil {}\n\
                     switch w := h(); w {}\n\
                     switch t := q.(type) { case int: _ = t }\n\
                     select { case got := <-ch: _ = got }\n\
                     fn := func(lp int) {}\n\
                   }\n\
                   func run(fn2 func(ctx int) error) {}\n";
        let got = go(src);
        let names: Vec<&str> = got.iter().map(|b| b.name.as_str()).collect();
        for want in [
            "r", "a", "b", "rest", "n", "err", "cb", "s", "x", "y", "v1", "v2", "c1",
            "i", "key", "val", "e", "w", "t", "got", "fn", "lp", "fn2",
        ] {
            assert!(names.contains(&want), "{want} in {got:?}");
        }
        for not in [
            "top",
            "k0",
            "m",
            "run",
            "typeParam",
            "ctx",
            "f",
            "g",
            "xs",
            "q",
            "ch",
        ] {
            assert!(!names.contains(&not), "{not} in {got:?}");
        }
        let param = |n: &str| got.iter().find(|b| b.name == n).unwrap().param;
        for p in ["r", "a", "rest", "n", "lp", "fn2"] {
            assert!(param(p), "{p} is a parameter");
        }
        for l in ["s", "x", "key", "e", "t", "got", "fn"] {
            assert!(!param(l), "{l} is a local");
        }
    }

    /// A Go declaration is in scope after its own statement and to the
    /// end of its block, clause or `if`/`for`/`switch`: `x := x()` calls
    /// the outer `x`, a type switch's value still sees the outer name, and
    /// a sibling block or a later function does not see the local.
    #[test]
    fn go_scopes() {
        let src = "package p\n\
                   func f() {\n\
                     now := now()\n\
                     now()\n\
                     { inner := 1; _ = inner }\n\
                     inner()\n\
                     for v := range v() { v() }\n\
                     v()\n\
                     switch q := q.(type) { case int: q() }\n\
                     if e := e(); e != nil { e() } else { e() }\n\
                     e()\n\
                     select { case r := <-ch: r() }\n\
                     r()\n\
                   }\n\
                   func later() { now() }\n";
        let got = go(src);
        assert!(!go_bound_at(&got, src, "now", "now(", 0), "{got:?}");
        assert!(go_bound_at(&got, src, "now", "now(", 1));
        assert!(
            !go_bound_at(&got, src, "now", "now(", 2),
            "later() sees no local"
        );
        assert!(!go_bound_at(&got, src, "inner", "inner()", 0));
        assert!(
            !go_bound_at(&got, src, "v", "v()", 0),
            "the range expression"
        );
        assert!(go_bound_at(&got, src, "v", "v()", 1));
        assert!(!go_bound_at(&got, src, "v", "v()", 2));
        assert!(!go_bound_at(&got, src, "q", "q.(type)", 0));
        assert!(go_bound_at(&got, src, "q", "q()", 0));
        assert!(!go_bound_at(&got, src, "e", "e()", 0), "the initializer");
        assert!(go_bound_at(&got, src, "e", "e()", 1));
        assert!(go_bound_at(&got, src, "e", "e()", 2), "else sees it in Go");
        assert!(!go_bound_at(&got, src, "e", "e()", 3));
        assert!(go_bound_at(&got, src, "r", ": r()", 0));
        assert!(!go_bound_at(&got, src, "r", "\nr()", 0));
    }

    fn cs(src: &str) -> Vec<LocalBinding> {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        collect("csharp", &tree, src.as_bytes())
    }

    /// WS27: every C# binding form inside a member is a local; fields,
    /// the method's own name, types and a discard are not.
    #[test]
    fn csharp_binding_forms() {
        let src = "class C { int field; C(int ctor) { }
          void M(int a, ref int b, out int c, in int d, params int[] rest, int _) {
            var x = 1; int y = 2, z; using var u = Open();
            using (var w = Open()) { }
            foreach (var it in xs) { } foreach (var (k, v) in dict) { }
            for (int i = 0; i < 3; i++) { }
            try { } catch (Exception e) { }
            if (o is Foo foo) { } if (o is Foo { P: var pp } rec) { }
            if (o is (int q1, var (pa, pb))) { }
            switch (o) { case Bar bar: break; }
            var r = o switch { Baz baz => 1, _ => 0 };
            Parse(s, out var parsed);
            var (t1, t2) = Tup();
            var q = from a1 in xs let l1 = a1 join j1 in ys on a1 equals j1 into g1 select l1;
            int Local(int lp) => lp;
            Func<int, int> lam = n => n; Func<int, int, int> lam2 = (m1, m2) => m1;
            Action del = delegate (int dp) { };
          } }";
        let got = cs(src);
        let names: Vec<&str> = got.iter().map(|b| b.name.as_str()).collect();
        for want in [
            "ctor", "a", "b", "c", "d", "rest", "x", "y", "z", "u", "w", "it", "k", "v",
            "i", "e", "foo", "pp", "rec", "q1", "pa", "pb", "bar", "baz", "parsed", "t1",
            "t2", "a1", "l1", "j1", "g1", "Local", "lp", "n", "m1", "m2", "dp", "lam",
            "del",
        ] {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
        for not in [
            "field", "M", "C", "Foo", "Bar", "Open", "xs", "Tup", "P", "_", "o",
        ] {
            assert!(!names.contains(&not), "{not} in {names:?}");
        }
        let param = |n: &str| got.iter().find(|b| b.name == n).unwrap().param;
        for p in ["a", "rest", "lp", "n", "m1", "dp", "ctor"] {
            assert!(param(p), "{p} is a parameter");
        }
        for l in ["x", "it", "e", "foo", "parsed", "a1"] {
            assert!(!param(l), "{l} is a local");
        }
    }

    /// C# scopes: a declaration is in scope after its declarator (`var
    /// now = now();` calls the method); a pattern variable in an `if`
    /// condition is in scope in the enclosing block after the `if`, one
    /// in a `while` condition only in the loop; a `foreach` variable not
    /// in its collection expression; a `catch` variable in its clause.
    #[test]
    fn csharp_scopes() {
        let src = "class C { void M() {
            var now = now(); now();
            if (!(o is Foo f)) return; f();
            while (o is Foo w) { w(); } w();
            foreach (var it in it()) { it(); } it();
            try { } catch (Exception e) { e(); } e();
          }
          void Other() { now(); } }";
        let got = cs(src);
        let at = |name: &str, needle: &str, nth: usize| {
            let site = src.match_indices(needle).nth(nth).expect(needle).0 as u32;
            got.iter()
                .any(|b| b.name == name && b.start_byte <= site && site < b.end_byte)
        };
        assert!(!at("now", "now()", 0), "initializer: {got:?}");
        assert!(at("now", "now()", 1));
        assert!(!at("now", "now()", 2), "another method");
        assert!(at("f", "f()", 0), "after the if, in the block");
        assert!(at("w", "{ w()", 0));
        assert!(!at("w", " w();\n", 0), "after the loop");
        assert!(!at("it", "it()", 0), "the collection");
        assert!(at("it", "it()", 1));
        assert!(!at("it", "it()", 2));
        assert!(at("e", "e()", 0));
        assert!(!at("e", "e()", 1));
    }

    fn c_locals(src: &str) -> Vec<LocalBinding> {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        collect("c", &tree, src.as_bytes())
    }

    /// WS27: C parameters (K&R too) and block-scope declarators bind;
    /// file-scope variables, a function-pointer declarator's parameter
    /// names and a block-scope prototype do not.
    #[test]
    fn c_binding_forms() {
        let src = "int g;\nint kr(a, b) int a; char *b; { return a; }\n\
                   int *f(int x, int (*cb)(int y), int arr[], ...) {\n\
                   int i = 0, *p, a2[3]; static int s; void (*fp)(int z) = 0;\n\
                   for (int k = 0; k < 3; k++) { }\n\
                   int proto(int q);\n return 0; }\n";
        let got = c_locals(src);
        let names: Vec<&str> = got.iter().map(|b| b.name.as_str()).collect();
        for want in ["a", "b", "x", "cb", "arr", "i", "p", "a2", "s", "fp", "k"] {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
        for not in ["g", "kr", "f", "y", "z", "proto", "q"] {
            assert!(!names.contains(&not), "{not} in {names:?}");
        }
        // A parenthesized name with no `*` declares a function.
        let paren = c_locals("void f(void) { int (g)(int); STDMETHOD(Name)(int a); }\n");
        assert!(paren.is_empty(), "{paren:?}");
        let broken =
            c_locals("void f(void) { STDMETHOD(N)(int a) PURE; using B::N; N(1); }\n");
        assert!(!broken.iter().any(|b| b.name == "N"), "{broken:?}");
        let va = c_locals(
            "void f(void) { struct T *dep = va_arg(ap, struct T *); g(dep); }\n",
        );
        assert!(va.iter().any(|b| b.name == "dep"), "{va:?}");
        // A macro header misparsed as a K&R definition names nothing.
        let misparsed = c_locals(
            "STDMETHOD_(HRESULT, CreateBitmap)(int w) { return CreateBitmap(w); }\n",
        );
        assert!(
            !misparsed.iter().any(|b| b.name == "CreateBitmap"),
            "{misparsed:?}"
        );
        let param = |n: &str| got.iter().find(|b| b.name == n).unwrap().param;
        assert!(param("a") && param("cb") && param("x"));
        assert!(!param("i") && !param("fp") && !param("k"));
    }

    /// A C local is in scope after its declarator, to the end of its
    /// block or `for`; a parameter over its function only.
    #[test]
    fn c_scopes() {
        let src = "void f(void (*cb)(void)) {\n cb();\n { int now = now(); now(); }\n now();\n\
                   for (int k = k(); ; ) { k(); }\n k();\n}\nvoid g(void) { cb(); }\n";
        let got = c_locals(src);
        let at = |name: &str, needle: &str, nth: usize| {
            let site = src.match_indices(needle).nth(nth).expect(needle).0 as u32;
            got.iter()
                .any(|b| b.name == name && b.start_byte <= site && site < b.end_byte)
        };
        assert!(at("cb", "cb()", 0));
        assert!(!at("cb", "cb()", 1), "another function");
        assert!(!at("now", "now()", 0), "the initializer");
        assert!(at("now", "now()", 1));
        assert!(!at("now", "now()", 2), "after the block");
        assert!(!at("k", "k()", 0), "the for initializer");
        assert!(at("k", "k()", 1));
        assert!(!at("k", "k()", 2), "after the for");
    }

    fn cpp_locals(src: &str) -> Vec<LocalBinding> {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_cpp::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        collect("cpp", &tree, src.as_bytes())
    }

    /// WS27: C++ parameters (references, lambdas, init-captures),
    /// structured bindings, range-for, `if`/`switch` initializers and
    /// `catch` bind; template parameters, a `std::function<void(int y)>`
    /// type's names, plain captures and class fields do not.
    #[test]
    fn cpp_binding_forms() {
        let src = "template <typename T, int N>\n\
                   int A::f(const T& a, std::function<void(int y)> cb, int* p) {\n\
                   auto [k, v] = pair();\n for (auto& it : xs) { }\n\
                   if (auto r = g(); r) { }\n switch (int s = h(); s) { }\n\
                   try { } catch (const std::exception& e) { }\n\
                   auto lam = [x = std::move(z), &w, n = 1](int lp) { int inner = lp; return inner; };\n\
                   static int st; int arr[3], *q; int& ref = st;\n}\n\
                   struct S { int field; void m(int mp) { int ml = mp; } };\n";
        let got = cpp_locals(src);
        let names: Vec<&str> = got.iter().map(|b| b.name.as_str()).collect();
        for want in [
            "a", "cb", "p", "k", "v", "it", "r", "s", "e", "lam", "x", "n", "lp",
            "inner", "st", "arr", "q", "ref", "mp", "ml",
        ] {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
        let more = cpp_locals(
            "void g(int n = 0, Args&&... rest) { AXTree tree(state); use(tree, n);\n\
             static auto map(T) -> int; int proto(int); auto make(T) -> int; }\n",
        );
        for want in ["n", "rest", "tree"] {
            assert!(more.iter().any(|b| b.name == want), "{want}: {more:?}");
        }
        for not in ["map", "proto", "make"] {
            assert!(!more.iter().any(|b| b.name == not), "{not}: {more:?}");
        }
        // An object-like macro alone on the line before a call statement.
        let macro_line = cpp_locals(
            "void g() {\n  _LIBCPP_SUPPRESS_DEPRECATED_PUSH\n  install(new F(name_));\n\
             Widget w(1); HANDLE h = open();\n}\n",
        );
        assert!(
            !macro_line.iter().any(|b| b.name == "install"),
            "{macro_line:?}"
        );
        for want in ["w", "h"] {
            assert!(
                macro_line.iter().any(|b| b.name == want),
                "{want}: {macro_line:?}"
            );
        }
        for not in ["T", "N", "y", "w", "z", "field", "f", "pair", "xs"] {
            assert!(!names.contains(&not), "{not} in {names:?}");
        }
        let param = |n: &str| got.iter().find(|b| b.name == n).unwrap().param;
        for p in ["a", "cb", "x", "lp", "mp"] {
            assert!(param(p), "{p} is a parameter");
        }
        for l in ["k", "it", "r", "e", "inner"] {
            assert!(!param(l), "{l} is a local");
        }
    }

    /// C++ scopes: a range-for variable is not in scope in its range; an
    /// `if` initializer is in scope over the whole `if`; a `catch`
    /// variable over its clause; a lambda parameter over its lambda only.
    #[test]
    fn cpp_scopes() {
        let src = "void f() {\n for (auto it : it()) { it(); }\n it();\n\
                   if (auto r = r(); r) { r(); } else { r(); }\n r();\n\
                   try { } catch (E e) { e(); }\n e();\n\
                   auto l = [](int cb) { cb(); };\n cb();\n}\n";
        let got = cpp_locals(src);
        let at = |name: &str, needle: &str, nth: usize| {
            let site = src.match_indices(needle).nth(nth).expect(needle).0 as u32;
            got.iter()
                .any(|b| b.name == name && b.start_byte <= site && site < b.end_byte)
        };
        assert!(!at("it", "it()", 0), "the range");
        assert!(at("it", "it()", 1));
        assert!(!at("it", "it()", 2));
        assert!(!at("r", "r()", 0), "the initializer");
        assert!(at("r", "r()", 1));
        assert!(at("r", "r()", 2), "else sees it");
        assert!(!at("r", "r()", 3));
        assert!(at("e", "e()", 0));
        assert!(!at("e", "e()", 1));
        assert!(at("cb", "cb()", 0));
        assert!(!at("cb", "cb()", 1));
    }

    fn groovy_locals(src: &str) -> Vec<LocalBinding> {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_groovy::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        collect("groovy", &tree, src.as_bytes())
    }

    /// Whether `name` is bound at the `nth` occurrence of `needle`.
    fn bound_at(
        got: &[LocalBinding],
        src: &str,
        name: &str,
        needle: &str,
        nth: usize,
    ) -> bool {
        let site = src.match_indices(needle).nth(nth).expect(needle).0 as u32;
        got.iter()
            .any(|b| b.name == name && b.start_byte <= site && site < b.end_byte)
    }

    /// Groovy (WS27): method, constructor and script-function parameters
    /// (untyped ones too, which the parser reads as a type with no name),
    /// closure parameters in every shape the parser gives them, Groovy 3
    /// lambdas, `def` and typed locals, `for` variables, `catch` and
    /// multiple assignment; never a field or a property.
    #[test]
    fn groovy_binding_forms() {
        let src = "class Svc {\n\
                   def field = 1\n\
                   String prop\n\
                   Svc(Closure init) { init() }\n\
                   def run(cb, x = 1, def y, Object... rest) {\n\
                   def c1 = { String a, int b -> a(b) }\n\
                   def c2 = { k1, k2 -> k1(k2) }\n\
                   items.each { Map<String, Integer> m, n2 ->\n m()\n }\n\
                   items.each { item -> item() }\n\
                   String typed = 't'\n\
                   def (p, q) = pair()\n\
                   for (e1 in xs) { e1() }\n\
                   for (int i = 0; i < 3; i++) { }\n\
                   for (String e2 : xs) { e2() }\n\
                   try { go() } catch (IOException ex) { ex() }\n\
                   def lam = (l1, l2) -> l1(l2)\n\
                   }\n\
                   }\n\
                   def top(fn, int n) { fn(n) }\n";
        let got = groovy_locals(src);
        let names: Vec<&str> = got.iter().map(|b| b.name.as_str()).collect();
        for want in [
            "init", "cb", "x", "y", "rest", "a", "b", "c1", "k1", "k2", "c2", "m", "n2",
            "item", "typed", "p", "q", "e1", "i", "e2", "ex", "lam", "l1", "l2", "fn",
            "n",
        ] {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
        for not in [
            "field", "prop", "Svc", "run", "top", "String", "int", "Map", "xs",
        ] {
            assert!(!names.contains(&not), "{not} in {names:?}");
        }
        let param = |n: &str| got.iter().find(|b| b.name == n).unwrap().param;
        for p in [
            "init", "cb", "x", "rest", "a", "k1", "m", "item", "l1", "fn",
        ] {
            assert!(param(p), "{p} is a parameter");
        }
        for l in ["c1", "typed", "p", "e1", "i", "ex", "lam"] {
            assert!(!param(l), "{l} is a local");
        }
    }

    /// Groovy scopes: a closure parameter only inside its closure; `it`
    /// only in a closure with no parameter list that stands as a value;
    /// a local after its declaration (not in its own initializer) to the
    /// end of its block; a `for` variable after its iterated value.
    #[test]
    fn groovy_scopes() {
        let src = "class C {\n\
                   def run() {\n\
                   xs.each { it() }\n\
                   xs.each { -> it() }\n\
                   xs.each { v -> v() }\n\
                   v()\n\
                   def h = h()\n\
                   h()\n\
                   if (ok) { def blk = 1; blk() }\n\
                   blk()\n\
                   for (z in z()) { z() }\n\
                   def runner = ok\n  ? other()\n  : runner()\n\
                   runner()\n\
                   }\n\
                   }\n";
        let got = groovy_locals(src);
        assert!(
            !bound_at(&got, src, "runner", "runner()", 0),
            "a continued initializer"
        );
        assert!(bound_at(&got, src, "runner", "runner()", 1));
        assert!(bound_at(&got, src, "it", "it()", 0), "{got:?}");
        assert!(
            !bound_at(&got, src, "it", "it()", 1),
            "an explicit empty list"
        );
        assert!(bound_at(&got, src, "v", "v()", 0));
        assert!(!bound_at(&got, src, "v", "v()", 1));
        assert!(!bound_at(&got, src, "h", "h()", 0), "its own initializer");
        assert!(bound_at(&got, src, "h", "h()", 1));
        assert!(bound_at(&got, src, "blk", "blk()", 0));
        assert!(!bound_at(&got, src, "blk", "blk()", 1));
        assert!(!bound_at(&got, src, "z", "z()", 0), "the iterated value");
        assert!(bound_at(&got, src, "z", "z()", 1));
    }

    /// A script's own variables are in scope in the script body and its
    /// closures, never in the script's methods or classes; a closure the
    /// parser misreads as a loop or `try` body binds no `it`.
    #[test]
    fn groovy_script_variables() {
        let src = "def run = { -> 1 }\n\
                   run()\n\
                   task t { doLast { run() } }\n\
                   def helper() { run() }\n\
                   println(String.class)\n\
                   run()\n";
        let got = groovy_locals(src);
        assert!(bound_at(&got, src, "run", "run()", 0));
        assert!(bound_at(&got, src, "run", "run()", 1), "a closure");
        assert!(!bound_at(&got, src, "run", "run()", 2), "a script method");
        assert!(bound_at(&got, src, "run", "run()", 3), "after it");
        // A file that declares a class: top-level declarations may be
        // members of a class the parser lost.
        let with_class = format!("{src}class K {{ def m() {{ run() }} }}\n");
        assert!(
            !groovy_locals(&with_class).iter().any(|b| b.name == "run"),
            "{with_class}"
        );
        let blocks = groovy_locals("class C { def f() { try { it() } finally { } } }\n");
        assert!(!blocks.iter().any(|b| b.name == "it"), "{blocks:?}");
    }

    /// What the parser reads as a declaration but is not one binds
    /// nothing: a command expression (`succeeds tasks(':app')`, `a.extendsFrom
    /// provider(b)`, `noop thing`), a method header it lost, words inside a
    /// multi-line string; a field is a member. Real declarations beside
    /// them still bind.
    #[test]
    fn groovy_misread_declarations_bind_nothing() {
        let src = "class C {\n\
                   def f() {\n\
                   succeeds tasks(':app').release.assemble\n\
                   master.extendsFrom provider(one), provider(two)\n\
                   noop thing\n\
                   bare\n\
                   def kept = 1\n\
                   List<String> names = []\n\
                   int count = 0\n\
                   No value\n\
                   def later\n\
                   }\n\
                   }\n\
                   protected abstract C lost(Collection<String> values)\n";
        let got = groovy_locals(src);
        let names: Vec<&str> = got.iter().map(|b| b.name.as_str()).collect();
        for not in ["tasks", "provider", "thing", "lost", "bare", "value"] {
            assert!(!names.contains(&not), "{not} in {names:?}");
        }
        for want in ["kept", "names", "count", "later"] {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
    }

    /// A Groovy parameter list's text: defaults, annotations, generics,
    /// arrays and varargs; a piece that is not a parameter is skipped.
    #[test]
    fn groovy_parameter_text() {
        assert_eq!(
            groovy_param_names(
                " @Named('a, b') final Map<String, List<Integer>> m, int[] xs = [1, 2], \
                 String... rest, untyped, f = { a, b -> a }, foo(), 'str' "
            ),
            vec!["m", "xs", "rest", "untyped", "f"]
        );
        assert!(groovy_param_names(" ").is_empty());
        assert!(groovy_param_names("int").is_empty());
    }

    fn r_locals(src: &str) -> Vec<LocalBinding> {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_r::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        collect("r", &tree, src.as_bytes())
    }

    /// R (WS27): parameters (`\(x)` lambdas too), `<-`, `=`, `->` and
    /// `assign("x", …)` locals and `for` variables inside a function;
    /// never `<<-`/`->>`, an `assign` into another environment, a
    /// top-level assignment, a replacement (`names(x) <- …`), or a local
    /// holding a literal number or string.
    #[test]
    fn r_binding_forms() {
        let src = "top <- function() 1\n\
                   glob <- 2\n\
                   g <- function(p1, p2 = 1, ...) {\n\
                   fun <- function(a) a\n\
                   un = p1()\n\
                   (function(x) x) -> arrow\n\
                   assign(\"named\", p1)\n\
                   assign(\"away\", p1, envir = e)\n\
                   for (i in p1) i\n\
                   l <- \\(k) k\n\
                   up <<- function() 1\n\
                   p1 ->> up2\n\
                   names(p1) <- 'n'\n\
                   lit <- 1; lit2 <- \"s\"; lit3 <- -2L; lit4 <- NULL\n\
                   }\n";
        let got = r_locals(src);
        let names: Vec<&str> = got.iter().map(|b| b.name.as_str()).collect();
        for want in ["p1", "fun", "a", "un", "arrow", "named", "i", "l", "k"] {
            assert!(names.contains(&want), "{want} in {names:?}");
        }
        for not in [
            "top", "glob", "g", "p2", "away", "up", "up2", "names", "lit", "lit2",
            "lit3", "lit4", "...",
        ] {
            assert!(!names.contains(&not), "{not} in {names:?}");
        }
        let param = |n: &str| got.iter().find(|b| b.name == n).unwrap().param;
        for p in ["p1", "a", "k"] {
            assert!(param(p), "{p} is a parameter");
        }
        for l in ["fun", "un", "i"] {
            assert!(!param(l), "{l} is a local");
        }
    }

    /// R resolves a call to a function binding only: a parameter whose
    /// default is a number, string or logical, or a binding of a call of
    /// its own name, holds no function and shadows nothing; a `NULL`
    /// default (an optional function) or a call of another name does.
    #[test]
    fn r_bindings_that_hold_no_function() {
        let src = "g <- function(flag = TRUE, n = 3, s = 'x', tz = tz(), fun = NULL, h = make()) {\n\
                   own <- own(1)\n\
                   other <- make(1)\n\
                   }\n";
        let names: Vec<String> = r_locals(src).into_iter().map(|b| b.name).collect();
        for not in ["flag", "n", "s", "tz", "own"] {
            assert!(!names.iter().any(|n| n == not), "{not} in {names:?}");
        }
        for want in ["fun", "h", "other"] {
            assert!(names.iter().any(|n| n == want), "{want} in {names:?}");
        }
    }

    /// R scopes a local by function: in scope after its assignment (not
    /// in its own value), across blocks to the end of the function, and
    /// in nested functions; never in a sibling function.
    #[test]
    fn r_scopes() {
        let src = "f <- function() {\n\
                   h()\n\
                   h <- k(h())\n\
                   if (TRUE) { b <- g() }\n\
                   h(); b()\n\
                   inner <- function() h()\n\
                   for (v in v()) v()\n\
                   }\n\
                   other <- function() { h(); b(); v() }\n";
        let got = r_locals(src);
        let at = |name: &str, needle: &str, nth: usize| {
            let site = src.match_indices(needle).nth(nth).expect(needle).0 as u32;
            got.iter()
                .any(|b| b.name == name && b.start_byte <= site && site < b.end_byte)
        };
        assert!(!at("h", "h()", 0), "before the assignment");
        assert!(!at("h", "h()", 1), "its own value");
        assert!(at("h", "h()", 2));
        assert!(at("b", "b()", 0), "a block's local is the function's");
        assert!(at("h", "h()", 3), "a nested function");
        assert!(!at("v", "v()", 0), "the sequence");
        assert!(at("v", "v()", 1));
        assert!(!at("h", "h()", 4), "a sibling function");
        assert!(!at("b", "b()", 1));
        assert!(!at("v", "v()", 2));
    }
}
