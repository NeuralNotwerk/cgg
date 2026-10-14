//! Receiver shapes (FINDINGS C2).
//!
//! A call's receiver is often not written as the bare value it is: it can
//! be parenthesized (`(f).m()`), awaited (`(await x).m()`), asserted
//! non-null (`f!.m()`), dereferenced (`(*p).m()`), cast (`((Foo) o).m()`,
//! `(o as Foo).M()`, `o.(Foo).M()`), or built in place (`new Foo().m()`,
//! `Foo().m()`, `Foo{A: 1}.M()`). Each plugin used to record such a
//! receiver as its raw text, which no later pass could read: the call was
//! left unresolved, or fanned out by name over every same-named method
//! (`new Foo()` starts lowercase, so it looked like a variable).
//!
//! [`peel`] reads the shape from the tree, by grammar node kind, for every
//! language at once: wrappers that do not change the value are looked
//! through, and a cast or a construction yields the type it names. A
//! receiver written in any of these forms is then recorded exactly as its
//! bare form would be — a variable for type propagation to type, or the
//! type itself.

use tree_sitter::Node;

/// A receiver expression with its non-semantic wrapping peeled away.
#[derive(Debug, Clone)]
pub struct Peeled<'t> {
    /// The value the call is made on, after looking through parentheses,
    /// `await`, non-null assertions and dereferences.
    pub node: Node<'t>,
    /// Set when the receiver was a cast or a construction: the type it is
    /// an instance of (`Foo`, `pkg.Foo`, `ns::Foo`), without type
    /// arguments, nullability or pointer marks. `node` is then the cast or
    /// construction itself.
    pub ty: Option<String>,
    /// Set when a dereference (`*p`) was looked through: `(*p).m()` is
    /// `p->m()`.
    pub deref: bool,
}

impl<'t> Peeled<'t> {
    /// The type as an owned string, when there is one.
    pub fn type_name(&self) -> Option<String> {
        self.ty.clone()
    }
}

/// Languages that construct without `new`: a call whose callee is a type
/// name (`Foo(…)`, capitalised) builds a value of that type.
fn constructs_by_call(lang: &str) -> bool {
    matches!(
        lang,
        "python" | "kotlin" | "swift" | "dart" | "scala" | "cpp"
    )
}

/// The receiver `node` is, read through its shape. See the module docs.
pub fn peel<'t>(lang: &str, node: Node<'t>, src: &'t [u8]) -> Peeled<'t> {
    let mut n = node;
    let mut deref = false;
    // Each step descends one level; real receivers nest a handful deep.
    for _ in 0..32 {
        let next = match n.kind() {
            "parenthesized_expression" => only_named_child(n),
            // Ruby `(x).m`.
            "parenthesized_statements" if lang == "ruby" => only_named_child(n),
            // Swift spells parentheses as a one-element tuple.
            "tuple_expression" if lang == "swift" => only_named_child(n),
            "await_expression" | "await" | "try_expression" => n
                .child_by_field_name("expr")
                .or_else(|| last_named_child(n)),
            // TS `f!`, C# `f!`: nullability only.
            "non_null_expression" => n.named_child(0),
            // Dart `f!`.
            "null_assertion_expression" => {
                n.child_by_field_name("value").or_else(|| n.named_child(0))
            }
            "postfix_unary_expression" if last_token_is(n, "!") => n.named_child(0),
            // Swift `f!`.
            "postfix_expression" if lang == "swift" => {
                let bang = n
                    .child_by_field_name("operation")
                    .is_some_and(|o| o.kind() == "bang");
                if bang {
                    n.child_by_field_name("target")
                } else {
                    None
                }
            }
            // Dart wraps `await x` in a unary expression; Go spells
            // address-of and dereference as unary operators.
            "unary_expression" => {
                let op = n
                    .child_by_field_name("operator")
                    .map(|o| o.utf8_text(src).unwrap_or(""));
                match op {
                    Some("&") => n.child_by_field_name("operand"),
                    Some("*") => {
                        deref = true;
                        n.child_by_field_name("operand")
                    }
                    Some(_) => None,
                    None => {
                        only_named_child(n).filter(|c| c.kind() == "await_expression")
                    }
                }
            }
            // C/C++ `*p` / `&x`.
            "pointer_expression" => {
                let op = n
                    .child_by_field_name("operator")
                    .map(|o| o.utf8_text(src).unwrap_or(""));
                match op {
                    Some("*") => {
                        deref = true;
                        n.child_by_field_name("argument")
                    }
                    Some("&") => n.child_by_field_name("argument"),
                    _ => None,
                }
            }
            _ => {
                let ty = shape_type(lang, n, src);
                if ty.is_none() && n.id() != node.id() && !is_value_like(n.kind()) {
                    return Peeled {
                        node,
                        ty: None,
                        deref: false,
                    };
                }
                return Peeled { node: n, ty, deref };
            }
        };
        match next {
            Some(c) => n = c,
            None => break,
        }
    }
    Peeled {
        node: n,
        ty: shape_type(lang, n, src),
        deref,
    }
}

/// Whether a peeled receiver is a value a call can be typed or named
/// through: a name, a member chain, `this`, a call. Anything else that
/// sat in parentheses — `(a + b).m()`, `(x || y).m()`, `(c ? a : b).m()`
/// — is an expression whose value has no name, and keeps the reading
/// its written form always had rather than being matched by the text
/// of one operand.
fn is_value_like(kind: &str) -> bool {
    kind.contains("identifier")
        || kind.contains("member")
        || kind.contains("field")
        || kind.contains("attribute")
        || kind.contains("navigation")
        || kind.contains("selector")
        || kind.contains("call")
        || kind.contains("invocation")
        || kind.contains("this")
        || kind.contains("self")
        || kind.contains("super")
        || matches!(
            kind,
            "identifier"
                | "variable_name"
                | "constant"
                | "scope_resolution"
                | "simple_name"
        )
}

/// The type a cast or a construction names; `None` for any other node.
fn shape_type<'t>(lang: &str, n: Node<'t>, src: &'t [u8]) -> Option<String> {
    let ty: Node<'t> = match n.kind() {
        // Java / C# / C / C++ `(Foo) o`, Go `o.(Foo)`, C/C++ `Foo{…}`, Go
        // `Foo{…}`.
        "cast_expression"
        | "type_assertion_expression"
        | "compound_literal_expression"
        | "composite_literal" => n.child_by_field_name("type")?,
        // `o as Foo`: C# names the type `right`, Swift `name`; TypeScript
        // and Kotlin write it last.
        "as_expression" => n
            .child_by_field_name("right")
            .or_else(|| n.child_by_field_name("name"))
            .or_else(|| last_named_child(n))?,
        // Dart `o as Foo`: the type sits in a `type_cast` child.
        "type_cast_expression" => {
            let cast = named_children(n).find(|c| c.kind() == "type_cast")?;
            named_children(cast).find(|c| c.kind() == "type")?
        }
        // TypeScript `<Foo>o`.
        "type_assertion" => {
            let args = named_children(n).find(|c| c.kind() == "type_arguments")?;
            args.named_child(0)?
        }
        // Java / C# `new Foo(…)` (also with an initializer or an anonymous
        // body); PHP `new Foo()`, whose class is an unnamed child.
        "object_creation_expression" => n.child_by_field_name("type").or_else(|| {
            named_children(n).find(|c| matches!(c.kind(), "name" | "qualified_name"))
        })?,
        // JS/TS `new Foo(…)`; C++ `new Foo(…)`.
        "new_expression" => n
            .child_by_field_name("constructor")
            .or_else(|| n.child_by_field_name("type"))?,
        // Scala `new Foo(…)`.
        "instance_expression" => named_children(n).find(|c| {
            matches!(
                c.kind(),
                "type_identifier" | "generic_type" | "stable_type_identifier"
            )
        })?,
        // Scala `o.asInstanceOf[Foo]`.
        "generic_function" => {
            let f = n.child_by_field_name("function")?;
            let field = f.child_by_field_name("field")?;
            if !cgg_core::stdlib::is_cast_intrinsic(lang, field.utf8_text(src).ok()?) {
                return None;
            }
            named_children(n)
                .filter(|c| c.id() != f.id())
                .find(|c| c.kind() != "type_arguments" || c.named_child_count() > 0)
                .map(|c| {
                    if c.kind() == "type_arguments" {
                        c.named_child(0).unwrap_or(c)
                    } else {
                        c
                    }
                })?
        }
        // Ruby `Foo.new(…)`: a class's own constructing method.
        "call" if lang == "ruby" => {
            let method = n.child_by_field_name("method")?.utf8_text(src).ok()?;
            let recv = n.child_by_field_name("receiver")?;
            if !cgg_core::stdlib::is_constructing_method(lang, method)
                || !matches!(recv.kind(), "constant" | "scope_resolution")
            {
                return None;
            }
            recv
        }
        "call" | "call_expression" => return call_type(lang, n, src),
        _ => return None,
    };
    clean_type(lang, ty.utf8_text(src).ok()?)
}

/// A call that is a cast (`static_cast<Foo*>(o)`) or, in a language that
/// constructs by calling the type, a construction (`Foo(…)`).
fn call_type<'t>(lang: &str, n: Node<'t>, src: &'t [u8]) -> Option<String> {
    let callee = n.child_by_field_name("function").or_else(|| n.child(0))?;
    if callee.kind() == "template_function" {
        let name = callee.child_by_field_name("name")?.utf8_text(src).ok()?;
        if !cgg_core::stdlib::is_cast_intrinsic(lang, name) {
            return None;
        }
        let args = callee.child_by_field_name("arguments")?;
        return clean_type(lang, args.named_child(0)?.utf8_text(src).ok()?);
    }
    if !constructs_by_call(lang) {
        return None;
    }
    // A path written across lines (`Headers\n  .Builder()`) is the same
    // path.
    let path: String = callee
        .utf8_text(src)
        .ok()?
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let plain_ident = matches!(
        callee.kind(),
        "identifier" | "simple_identifier" | "type_identifier"
    );
    // A dotted type path (`pkg.Foo(…)`), but never a C++ scope path:
    // `Foo::Create()` is a static factory far more often than a type.
    let dotted = !plain_ident
        && lang != "cpp"
        && matches!(
            callee.kind(),
            "attribute"
                | "navigation_expression"
                | "field_expression"
                | "member_expression"
        );
    if !(plain_ident || dotted) {
        return None;
    }
    let segs_plain = path
        .split('.')
        .all(|s| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_'));
    let last = path.rsplit('.').next()?;
    // A PEP 8 private class (`_Builder()`) is a class all the same.
    let last = if lang == "python" {
        last.trim_start_matches('_')
    } else {
        last
    };
    let typed = segs_plain && last.starts_with(char::is_uppercase);
    typed.then_some(path)
}

/// `Foo<T>` / `Foo?` / `*pkg.Foo` / `const Foo*` / `\App\Foo` → the
/// nominal type, or `None` when what is left is not a name path.
fn clean_type(lang: &str, raw: &str) -> Option<String> {
    // A type written across lines or with spaces inside a path
    // (`a .b .Foo`) is the same type; a space after a keyword is kept
    // until the keyword is dropped.
    let joined: String = {
        let mut out = String::with_capacity(raw.len());
        let mut prev_ws = false;
        for c in raw.trim().chars() {
            if c.is_whitespace() {
                prev_ws = true;
                continue;
            }
            if prev_ws
                && (c.is_alphanumeric() || c == '_')
                && out.ends_with(|p: char| p.is_alphanumeric() || p == '_')
            {
                out.push(' ');
            }
            prev_ws = false;
            out.push(c);
        }
        out
    };
    let mut t = joined.as_str();
    loop {
        let before = t;
        for kw in ["const ", "struct ", "class ", "volatile "] {
            if let Some(rest) = t.strip_prefix(kw) {
                t = rest.trim_start();
            }
        }
        t = t.trim_start_matches(['&', '*', '\\', ' ']);
        if t == before {
            break;
        }
    }
    // Type arguments: `Foo<T>`, Scala/Go `Foo[T]`.
    if let Some(i) = t.find(['<', '[', '(', '{']) {
        t = &t[..i];
    }
    t = t.trim_end_matches(['?', '!', '*', '&', ' ']);
    if lang == "php" {
        t = t.rsplit('\\').next().unwrap_or(t);
    }
    let ok = t
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && t.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | ':' | '$'));
    ok.then(|| t.to_string())
}

fn named_children<'t>(n: Node<'t>) -> impl Iterator<Item = Node<'t>> {
    (0..n.named_child_count() as u32).filter_map(move |i| n.named_child(i))
}

/// The one named child of `n` that is not a comment.
fn only_named_child(n: Node<'_>) -> Option<Node<'_>> {
    let mut it = named_children(n).filter(|c| !c.kind().contains("comment"));
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

fn last_named_child(n: Node<'_>) -> Option<Node<'_>> {
    named_children(n)
        .filter(|c| !c.kind().contains("comment"))
        .last()
}

fn last_token_is(n: Node<'_>, tok: &str) -> bool {
    n.child((n.child_count() as u32).saturating_sub(1))
        .is_some_and(|c| c.kind() == tok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    /// The receiver of every member call in `src`, peeled, as
    /// `(type or peeled text, deref)`, in source order.
    fn receivers(
        lang: &str,
        ts: tree_sitter::Language,
        src: &str,
    ) -> Vec<(String, bool)> {
        let mut p = Parser::new();
        p.set_language(&ts).unwrap();
        let tree = p.parse(src, None).unwrap();
        fn object(n: Node<'_>) -> Option<Node<'_>> {
            [
                "object",
                "expression",
                "operand",
                "argument",
                "value",
                "target",
            ]
            .iter()
            .find_map(|f| n.child_by_field_name(f))
            .or_else(|| {
                (n.kind() == "navigation_expression")
                    .then(|| n.named_child(0))
                    .flatten()
            })
        }
        let mut out = Vec::new();
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            let recv = match n.kind() {
                "method_invocation"
                | "member_call_expression"
                | "nullsafe_member_call_expression" => n.child_by_field_name("object"),
                "call" | "call_expression" | "invocation_expression" => n
                    .child_by_field_name("function")
                    .or_else(|| n.child(0))
                    .filter(|f| {
                        f.kind() != "identifier" && f.kind() != "simple_identifier"
                    })
                    .and_then(object),
                _ => None,
            };
            if let Some(r) = recv {
                let pe = peel(lang, r, src.as_bytes());
                let s = match pe.ty {
                    Some(t) => format!("T:{t}"),
                    None => pe.node.utf8_text(src.as_bytes()).unwrap().to_string(),
                };
                out.push((s, pe.deref));
            }
            for i in (0..n.child_count() as u32).rev() {
                stack.push(n.child(i).unwrap());
            }
        }
        out
    }

    fn names(v: Vec<(String, bool)>) -> Vec<String> {
        v.into_iter().map(|(s, _)| s).collect()
    }

    #[test]
    fn java_parens_casts_and_constructions() {
        let src = "class M { void r(Object o, Foo f) { f.m(); (f).m(); ((Foo) o).m(); \
                   new Foo().m(); (new a.b.Foo<X>()).m(); g().m(); } }";
        assert_eq!(
            names(receivers("java", tree_sitter_java::LANGUAGE.into(), src)),
            vec!["f", "f", "T:Foo", "T:Foo", "T:a.b.Foo", "g()"]
        );
    }

    #[test]
    fn csharp_as_await_and_initializers() {
        let src = "class M { async void R(object o, Task<Foo> t) { (o as Foo).M(); \
                   ((Foo)o).M(); (await t).M(); new Foo { X = 1 }.M(); (f!).M(); } }";
        assert_eq!(
            names(receivers(
                "csharp",
                tree_sitter_c_sharp::LANGUAGE.into(),
                src
            )),
            vec!["T:Foo", "T:Foo", "t", "T:Foo", "f"]
        );
    }

    #[test]
    fn typescript_as_assertion_and_new() {
        let src = "async function r(o: any, p) { (o as Foo).m(); (<Foo>o).m(); \
                   new Foo().m(); (await p).m(); f!.m(); }";
        assert_eq!(
            names(receivers(
                "typescript",
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                src
            )),
            vec!["T:Foo", "T:Foo", "T:Foo", "p", "f"]
        );
    }

    #[test]
    fn go_literals_assertions_and_derefs() {
        let src = "package p\nfunc r(o any, p *Foo) {\n(f).M()\nFoo{A: 1}.M()\n(&Foo{}).M()\n\
                   o.(*pkg.Foo).M()\n(*p).M()\n}\n";
        let got = receivers("go", tree_sitter_go::LANGUAGE.into(), src);
        assert_eq!(
            got,
            vec![
                ("f".to_string(), false),
                ("T:Foo".to_string(), false),
                ("T:Foo".to_string(), false),
                ("T:pkg.Foo".to_string(), false),
                ("p".to_string(), true),
            ]
        );
    }

    #[test]
    fn cpp_constructions_and_casts() {
        let src = "void r(void* o, Foo* p) { (f).m(); Foo().m(); Foo{}.m(); ((Foo*)o)->m(); \
                   static_cast<Foo*>(o)->m(); (*p).m(); Make().m(); Foo::Create().m(); }";
        assert_eq!(
            names(receivers("cpp", tree_sitter_cpp::LANGUAGE.into(), src)),
            vec![
                "f",
                "T:Foo",
                "T:Foo",
                "T:Foo",
                "T:Foo",
                "p",
                "T:Make",
                "Foo::Create()"
            ]
        );
    }

    #[test]
    fn python_constructions_need_a_capitalised_callee() {
        let src = "async def r(p):\n    (f).m()\n    Foo().m()\n    pkg.Foo(1).m()\n    \
                   make().m()\n    (await p).m()\n";
        assert_eq!(
            names(receivers(
                "python",
                tree_sitter_python::LANGUAGE.into(),
                src
            )),
            vec!["f", "T:Foo", "T:pkg.Foo", "pkg", "make()", "p"]
        );
    }

    #[test]
    fn ruby_new_and_parentheses() {
        let src = "def r(f)\n  Foo.new.m\n  (A::Foo.new(1)).m\n  (f).m\n  make.m\nend\n";
        let mut p = Parser::new();
        p.set_language(&tree_sitter_ruby::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        let mut got = Vec::new();
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            if n.kind() == "call"
                && n.child_by_field_name("method")
                    .is_some_and(|m| m.utf8_text(src.as_bytes()).unwrap() == "m")
                && let Some(r) = n.child_by_field_name("receiver")
            {
                let pe = peel("ruby", r, src.as_bytes());
                got.push(match pe.ty {
                    Some(t) => format!("T:{t}"),
                    None => pe.node.utf8_text(src.as_bytes()).unwrap().to_string(),
                });
            }
            for i in (0..n.child_count() as u32).rev() {
                stack.push(n.child(i).unwrap());
            }
        }
        assert_eq!(got, vec!["T:Foo", "T:A::Foo", "f", "make"]);
    }

    #[test]
    fn kotlin_constructions_across_lines_and_casts() {
        let src = "fun r(o: Any) {\n  val a = Headers\n    .Builder()\n    .add(1)\n  \
                   val b = (o as? Foo)?.m()\n  val c = make().m()\n}\n";
        let got = names(receivers(
            "kotlin",
            tree_sitter_kotlin_sg::LANGUAGE.into(),
            src,
        ));
        assert!(got.contains(&"T:Headers.Builder".to_string()), "{got:?}");
        assert!(got.contains(&"T:Foo".to_string()), "{got:?}");
        assert!(got.contains(&"make()".to_string()), "{got:?}");
    }

    #[test]
    fn a_parenthesized_expression_that_is_not_a_value_keeps_its_text() {
        let src = "def r(a, b, c):\n    (a + b).m()\n    (a or b).m()\n    (a if c else b).m()\n    (a.b).m()\n";
        assert_eq!(
            names(receivers(
                "python",
                tree_sitter_python::LANGUAGE.into(),
                src
            )),
            vec!["(a + b)", "(a or b)", "(a if c else b)", "a.b"]
        );
    }

    #[test]
    fn php_parenthesized_new() {
        let src =
            "<?php\nfunction r($f) { ($f)->m(); (new \\App\\Foo())->m(); $f?->m(); }\n";
        assert_eq!(
            names(receivers("php", tree_sitter_php::LANGUAGE_PHP.into(), src)),
            vec!["$f", "T:Foo", "$f"]
        );
    }
}
