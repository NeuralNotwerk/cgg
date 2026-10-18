//! C and C++ function names used as values (WS26, gap 25).
//!
//! A function handed over without being called — a callback table
//! (`static ops_t ops = { .open = my_open, my_close }`), `p->cb = f`,
//! `qsort(xs, n, sz, cmp)`, `return f`, `&f`, C++ `&Class::m` — has no
//! incoming call, so it read as dead. This pass records each such name as
//! a [`RefRecord::value_ref`] site, only when the run records reference
//! sites, named the way a call of it would be: bare, or through the
//! scope a qualified name writes behind `&` (`&ns::f`, `&Class::m`). The
//! arguments of a macro-style call (`EXPECT_CALL(x, M)`) are not read.
//!
//! C and C++ have no scope table yet (`cgg_lang::locals`), so a bare name
//! any parameter or declarator of an enclosing function declares, or a
//! file-scope (or namespace-scope) variable of the file, is a variable and
//! is not recorded: a binding counts over the whole function, which can
//! only drop a reference, never invent one. Names no callable of the
//! language carries are dropped before resolution (`value_refs::prune`).
//! A registrar verb's arguments were captured by `registrar::capture`.

use std::collections::HashSet;

use cgg_core::RefRecord;
use tree_sitter::{Node, Tree};

/// Every value-position function name of the file, and the data member
/// names its structs and classes declare (see [`data_members`]).
///
/// One cursor walk, dispatching on kind ids: a second walk per function
/// for its bindings, a cursor per node or string kind comparisons cost
/// more than the rest of the C extraction on large generated sources.
/// Bindings are gathered on the way and candidates checked against them
/// when the walk is done, since a binding counts over its whole function.
pub(super) fn collect(
    ctx: &crate::ExtractCtx<'_>,
    tree: &Tree,
    src: &[u8],
) -> (Vec<RefRecord>, Vec<String>) {
    let _s = cgg_core::profile::span("extract::c-values");
    let root = tree.root_node();
    let mut w = Walk {
        ctx,
        src,
        kinds: Kinds::of(&tree.language()),
        frames: Vec::new(),
        frame_parent: Vec::new(),
        stack: Vec::new(),
        cands: Vec::new(),
        members: HashSet::new(),
        in_field: 0,
    };
    let mut cursor = root.walk();
    w.visit(&mut cursor, None);
    let file_vars = scope_variables(root, src);
    let Walk {
        frames,
        frame_parent,
        cands,
        members,
        ..
    } = w;
    let out = cands
        .into_iter()
        .filter(|(r, frame)| {
            let Some(mut f) = *frame else {
                return true;
            };
            if file_vars.contains(&r.name) {
                return false;
            }
            if f == usize::MAX {
                return true;
            }
            loop {
                if frames[f].contains(&r.name) {
                    return false;
                }
                match frame_parent[f] {
                    Some(p) => f = p,
                    None => return true,
                }
            }
        })
        .map(|(r, _)| r)
        .collect();
    let mut members: Vec<String> = members.into_iter().collect();
    members.sort_unstable();
    (out, members)
}

fn text<'s>(n: Node, src: &'s [u8]) -> &'s str {
    n.utf8_text(src).unwrap_or("")
}

/// What the walk does at a node, by kind id.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    None,
    Frame,
    Field,
    Args,
    Call,
    InitList,
    InitDecl,
    Assign,
    Return,
    Decl,
}

/// The [`Role`] of every kind id of a grammar (C and C++ name these kinds
/// alike; aliased ids of one name share its role).
struct Kinds {
    role: Vec<Role>,
}

impl Kinds {
    fn of(lang: &tree_sitter::Language) -> Self {
        let n = lang.node_kind_count();
        let role = (0..n)
            .map(|id| match lang.node_kind_for_id(id as u16).unwrap_or("") {
                "function_definition" | "lambda_expression" => Role::Frame,
                "field_declaration" => Role::Field,
                "argument_list" => Role::Args,
                "call_expression" => Role::Call,
                "initializer_list" => Role::InitList,
                "init_declarator" => Role::InitDecl,
                "assignment_expression" => Role::Assign,
                "return_statement" => Role::Return,
                "parameter_declaration"
                | "optional_parameter_declaration"
                | "variadic_parameter_declaration"
                | "declaration"
                | "for_range_loop"
                | "condition_declaration" => Role::Decl,
                _ => Role::None,
            })
            .collect();
        Kinds { role }
    }

    fn role(&self, n: Node) -> Role {
        self.role
            .get(n.kind_id() as usize)
            .copied()
            .unwrap_or(Role::None)
    }
}

struct Walk<'c, 's> {
    ctx: &'c crate::ExtractCtx<'c>,
    src: &'s [u8],
    kinds: Kinds,
    /// Every name each function or lambda binds anywhere in it, by frame.
    frames: Vec<HashSet<String>>,
    frame_parent: Vec<Option<usize>>,
    /// The frames enclosing the node being walked, innermost last.
    stack: Vec<usize>,
    /// Value sites with the innermost frame a bare name is checked from:
    /// `None` for a name written through a scope (never a variable),
    /// `usize::MAX` for a bare name outside any function (only file
    /// variables can hide it).
    cands: Vec<(RefRecord, Option<usize>)>,
    members: HashSet<String>,
    /// How many field declarations enclose the node being walked.
    in_field: u32,
}

impl Walk<'_, '_> {
    fn visit(&mut self, cursor: &mut tree_sitter::TreeCursor, parent: Option<Node>) {
        let node = cursor.node();
        let role = self.kinds.role(node);
        if role == Role::Frame {
            self.frames.push(HashSet::new());
            self.frame_parent.push(self.stack.last().copied());
            self.stack.push(self.frames.len() - 1);
        }
        match role {
            // The outermost declaration only, as a member's own struct type
            // (`struct { int a; } inner;`) declares fields of that type.
            Role::Field if self.in_field == 0 => {
                field_names(node, self.src, &mut self.members)
            }
            Role::Args => {
                if let Some(call) = parent.filter(|p| self.kinds.role(*p) == Role::Call) {
                    let callee = call
                        .child_by_field_name("function")
                        .map(|f| text(f, self.src))
                        .unwrap_or("");
                    if !crate::plugins::registrar::admits(self.ctx, callee)
                        && !macro_style(callee)
                    {
                        self.values_of_children(node, false);
                    }
                }
            }
            Role::InitList => self.values_of_children(node, true),
            Role::InitDecl => {
                self.bind_declared(node);
                if let Some(v) = node.child_by_field_name("value") {
                    self.value_ref_in(v, 0, false);
                }
            }
            Role::Assign => {
                // A name assigned to is a variable, however its declaration
                // parsed (`g_autoptr(T) err = NULL;` reads as a call).
                if let Some(left) = node
                    .child_by_field_name("left")
                    .filter(|l| l.kind() == "identifier")
                {
                    let name = text(left, self.src).to_string();
                    self.bind(name);
                }
                if let Some(v) = node.child_by_field_name("right") {
                    self.value_ref_in(v, 0, false);
                }
            }
            Role::Return => self.values_of_children(node, false),
            Role::Decl => self.bind_declared(node),
            _ => {}
        }
        if role == Role::Field {
            self.in_field += 1;
        }
        if cursor.goto_first_child() {
            loop {
                self.visit(cursor, Some(node));
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
        if role == Role::Field {
            self.in_field -= 1;
        }
        if role == Role::Frame {
            self.stack.pop();
        }
    }

    /// The variables a declaration declares bind in every enclosing frame
    /// (a binding counts over the whole of each function around it).
    fn bind_declared(&mut self, decl: Node) {
        if self.stack.is_empty() {
            return;
        }
        let mut names = HashSet::new();
        declared_names(decl, self.src, &mut names);
        for name in names {
            self.bind(name);
        }
    }

    fn bind(&mut self, name: String) {
        for &f in &self.stack {
            self.frames[f].insert(name.clone());
        }
    }

    /// Each named child as a value; an initializer list's `.f = v` pairs
    /// by their value.
    fn values_of_children(&mut self, node: Node, pairs: bool) {
        let mut c = node.walk();
        if !c.goto_first_child() {
            return;
        }
        loop {
            let e = c.node();
            if e.is_named() {
                let v = if pairs && e.kind() == "initializer_pair" {
                    e.child_by_field_name("value")
                } else {
                    Some(e)
                };
                if let Some(v) = v {
                    self.value_ref_in(v, 0, false);
                }
            }
            if !c.goto_next_sibling() {
                break;
            }
        }
    }

    /// The names one value expression stands for: a name, `&name`, a cast
    /// of one, the branches of `c ? f : g`, through parentheses.
    /// `addressed` says it is the operand of `&`, the only place a
    /// qualified name is read as a function: a bare `Kind::Value` in value
    /// position is an enumerator or a static datum far more often.
    fn value_ref_in(&mut self, v: Node, depth: u8, addressed: bool) {
        if depth > 4 {
            return;
        }
        match v.kind() {
            "identifier" => {
                let frame = self.stack.last().copied();
                if let Some(r) = record(v, String::new(), false, self.src) {
                    self.cands.push((r, Some(frame.unwrap_or(usize::MAX))));
                }
            }
            "qualified_identifier" if addressed => {
                if let Some(name_node) = qualified_name(v) {
                    let scope = text_between(v, name_node, self.src);
                    let path = !scope.is_empty();
                    if let Some(r) = record(name_node, scope, path, self.src) {
                        self.cands.push((r, None));
                    }
                }
            }
            "pointer_expression" => {
                let is_address = {
                    let mut c = v.walk();
                    v.children(&mut c).any(|t| t.kind() == "&")
                };
                if let Some(arg) = v.child_by_field_name("argument")
                    && is_address
                {
                    self.value_ref_in(arg, depth + 1, true);
                }
            }
            "parenthesized_expression" => {
                if let Some(n) = v.named_child(0) {
                    self.value_ref_in(n, depth + 1, addressed);
                }
            }
            "cast_expression" => {
                if let Some(n) = v.child_by_field_name("value") {
                    self.value_ref_in(n, depth + 1, addressed);
                }
            }
            "conditional_expression" => {
                for field in ["consequence", "alternative"] {
                    if let Some(b) = v.child_by_field_name(field) {
                        self.value_ref_in(b, depth + 1, addressed);
                    }
                }
            }
            _ => {}
        }
    }
}

/// A callee spelled like a macro (`EXPECT_CALL`, `DEFINE_HANDLER`): no
/// lowercase letter. Its arguments are tokens the expansion pastes — a
/// field or a method name as often as a function — so they are not read.
/// A reserved identifier (`__x(…)`) is not excluded: libraries name their
/// internal registration functions so (`__archive_read_register_format(a,
/// …, bid, …)`), and skipping them lost those callbacks' references.
fn macro_style(callee: &str) -> bool {
    callee.len() > 1
        && callee.chars().any(|c| c.is_ascii_uppercase())
        && !callee.chars().any(|c| c.is_ascii_lowercase())
}

/// The final name node of a qualified identifier, when it is a plain
/// identifier (`ns::f`, `Class::m`).
fn qualified_name(n: Node) -> Option<Node> {
    let mut last = n;
    loop {
        match last.child_by_field_name("name") {
            Some(next) if next.kind() == "qualified_identifier" => last = next,
            Some(name) if name.kind() == "identifier" => return Some(name),
            _ => return None,
        }
    }
}

/// The scope a qualified name writes before its final name, without the
/// trailing `::` (empty for `::f`).
fn text_between(outer: Node, name: Node, src: &[u8]) -> String {
    let scope = src
        .get(outer.start_byte()..name.start_byte())
        .map(String::from_utf8_lossy)
        .unwrap_or_default();
    let scope = scope.trim_end();
    scope.strip_suffix("::").unwrap_or(scope).trim().to_string()
}

fn record(name_node: Node, recv: String, path: bool, src: &[u8]) -> Option<RefRecord> {
    let name = text(name_node, src);
    if name.is_empty() {
        return None;
    }
    Some(RefRecord {
        name: name.to_string(),
        receiver_hint: recv,
        site_line: (name_node.start_position().row as u32) + 1,
        site_byte: name_node.start_byte() as u32,
        path_receiver: path,
        value_ref: true,
        ..Default::default()
    })
}

/// The identifier a declarator declares, unless it declares a function
/// (`int f(int);`): through pointer, array, reference, parenthesis,
/// attribute and initializer layers. A function pointer (`int (*cb)(int)`,
/// a parameter or a local) is a variable: its function declarator wraps
/// a parenthesized declarator, not the name itself.
fn declared_variable<'t>(mut d: Node<'t>) -> Option<Node<'t>> {
    for _ in 0..16 {
        match d.kind() {
            "identifier" | "field_identifier" => return Some(d),
            "function_declarator" => {
                d = d
                    .child_by_field_name("declarator")
                    .filter(|inner| inner.kind() == "parenthesized_declarator")?;
            }
            "pointer_declarator"
            | "array_declarator"
            | "reference_declarator"
            | "init_declarator"
            | "parenthesized_declarator"
            | "attributed_declarator" => {
                d = d
                    .child_by_field_name("declarator")
                    .or_else(|| d.named_children(&mut d.walk()).last())?;
            }
            _ => return None,
        }
    }
    None
}

/// The variable names `decl` (a declaration or parameter) declares.
fn declared_names(decl: Node, src: &[u8], out: &mut HashSet<String>) {
    let mut c = decl.walk();
    for d in decl.children_by_field_name("declarator", &mut c) {
        if d.kind() == "structured_binding_declarator" {
            let mut cc = d.walk();
            for id in d.named_children(&mut cc) {
                out.insert(text(id, src).to_string());
            }
            continue;
        }
        if let Some(id) = declared_variable(d) {
            out.insert(text(id, src).to_string());
        }
    }
}

/// The data member names the file's structs, unions and classes declare
/// (`struct ops { int (*open)(void); int count; }` gives `open`, `count`),
/// sorted. A function pointer member is data too: `{ .open = my_open }`
/// names the function through the field, and a bare `open` inside a member
/// function is the field.
#[cfg(test)]
fn data_members(tree: &Tree, src: &[u8]) -> Vec<String> {
    let ctx = crate::ExtractCtx::plain().with_reference_sites(true);
    collect(&ctx, tree, src).1
}

/// The names one `field_declaration` declares (not a member function).
fn field_names(n: Node, src: &[u8], out: &mut HashSet<String>) {
    let mut c = n.walk();
    for d in n.children_by_field_name("declarator", &mut c) {
        if let Some(id) = declared_variable(d) {
            out.insert(text(id, src).to_string());
        }
    }
}

/// Variables declared at file or namespace scope (`static int count;`,
/// `ops_t ops = {…}`), not functions.
fn scope_variables(root: Node, src: &[u8]) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        match n.kind() {
            "declaration" => declared_names(n, src, &mut out),
            "translation_unit"
            | "namespace_definition"
            | "declaration_list"
            | "linkage_specification"
            | "preproc_if"
            | "preproc_ifdef"
            | "preproc_else"
            | "preproc_elif" => {
                let mut c = n.walk();
                stack.extend(n.named_children(&mut c));
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(lang: tree_sitter::Language, src: &str) -> Vec<(String, String)> {
        let mut p = tree_sitter::Parser::new();
        p.set_language(&lang).unwrap();
        let tree = p.parse(src, None).unwrap();
        let ctx = crate::ExtractCtx::plain().with_reference_sites(true);
        collect(&ctx, &tree, src.as_bytes())
            .0
            .into_iter()
            .map(|r| (r.receiver_hint, r.name))
            .collect()
    }

    fn has(got: &[(String, String)], recv: &str, name: &str) -> bool {
        got.contains(&(recv.to_string(), name.to_string()))
    }

    /// WS26 (gap 25): a C function named as a value — in a callback
    /// table, an array, an assignment, an argument, a return, behind `&`
    /// or a cast — is a reference site.
    #[test]
    fn c_function_values_are_reference_sites() {
        let src = "static const struct ops t = { .open = my_open, my_close };\n\
                   static int (*arr[])(void) = { in_array };\n\
                   static int (*pick(void))(void) { return returned; }\n\
                   int main(void) {\n\
                     o.cb = assigned;\n\
                     sort_with(xs, 2, 4, cmp);\n\
                     pass(&addr_of);\n\
                     void *p = (void *)casted;\n\
                     pass(c ? yes : no);\n\
                   }\n";
        let got = refs(tree_sitter_c::LANGUAGE.into(), src);
        for n in [
            "my_open", "my_close", "in_array", "returned", "assigned", "cmp", "addr_of",
            "casted", "yes", "no",
        ] {
            assert!(has(&got, "", n), "missing {n}: {got:?}");
        }
    }

    /// The near misses: a parameter (a function pointer too), a local, a
    /// file-scope variable, a string, a field designator and a member
    /// read are not references.
    #[test]
    fn c_variables_strings_and_designators_are_not_reference_sites() {
        let src = "static int limit = 3;\n\
                   static const struct ops t = { .open = 0 };\n\
                   void take(int (*cb)(void), int n) {\n\
                     int local = 1;\n\
                     int (*fp)(void) = 0;\n\
                     pass(cb); pass(n); pass(local); pass(fp); pass(limit);\n\
                     pass(\"in_string\"); pass(o.field); pass(p->field);\n\
                     EXPECT_CALL(mock, Method);\n\
                     g_autoptr(GError) err = NULL; pass(err);\n\
                   }\n";
        let got = refs(tree_sitter_c::LANGUAGE.into(), src);
        let names: Vec<&str> = got.iter().map(|(_, n)| n.as_str()).collect();
        for n in [
            "cb",
            "n",
            "local",
            "fp",
            "limit",
            "in_string",
            "open",
            "field",
            "mock",
            "Method",
            "err",
        ] {
            assert!(!names.contains(&n), "{n} recorded: {got:?}");
        }
    }

    /// C++: `&Class::m` and `ns::f` carry the scope they write.
    #[test]
    fn cpp_member_pointers_and_qualified_names_carry_their_scope() {
        let src = "void W::wire() { connect(&ui::W::onClick); }\n\
                   int main() { std::sort(b, e, &ui::byName); std::sort(b, e, Kind::Value); auto f = [](int x) { return pass(x); }; }\n";
        let got = refs(tree_sitter_cpp::LANGUAGE.into(), src);
        assert!(has(&got, "ui::W", "onClick"), "{got:?}");
        assert!(has(&got, "ui", "byName"), "{got:?}");
        assert!(!got.iter().any(|(_, n)| n == "x"), "{got:?}");
        // An unaddressed qualified name is an enumerator far more often.
        assert!(!got.iter().any(|(_, n)| n == "Value"), "{got:?}");
    }

    /// Data members, function pointers among them, are recorded by name.
    #[test]
    fn struct_and_class_data_members_are_recorded() {
        let src = "struct ops { int (*open)(void); int count, *list; };\n\
                   class Img { public: double height; int width() const; };\n";
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_cpp::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        assert_eq!(
            data_members(&tree, src.as_bytes()),
            vec!["count", "height", "list", "open"]
        );
    }

    /// An ordinary run records nothing: the plugins call [`collect`] only
    /// when the run asks for reference sites.
    #[test]
    fn plugins_record_value_sites_only_when_asked() {
        use crate::LanguagePlugin;
        let src = "int main(void) { int xs[2]; sort_with(xs, 2, 4, cmp); }\n";
        let mut p = tree_sitter::Parser::new();
        p.set_language(&tree_sitter_c::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        let run = |on: bool| {
            crate::plugins::c::CPlugin
                .extract(
                    &crate::ExtractCtx::plain().with_reference_sites(on),
                    cgg_core::ids::FileId::new(0),
                    std::path::Path::new("/tmp/__cgg_test__/x.c"),
                    &tree,
                    src.as_bytes(),
                )
                .references
                .iter()
                .filter(|r| r.value_ref)
                .count()
        };
        assert_eq!(run(false), 0);
        assert_eq!(run(true), 1);
    }
}
