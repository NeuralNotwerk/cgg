//! Written types of closure parameters and typed patterns.
//!
//! A closure is not a callable cgg records, so its parameters never
//! reached the signature-driven parameter typing: `(Foo x) => x.run()`
//! typed `x` no better than `(x) => x.run()`, and the call fanned out by
//! name over every `run` in the tree. Where the type is written, it is
//! recorded here with the span the name is in scope over
//! ([`cgg_core::FileFacts::scoped_types`]); an untyped parameter records
//! nothing and keeps the reading it had.
//!
//! Shapes are grammar node kinds, per language:
//! * Dart `function_expression` — `(Foo x) => …`, `(Foo x) { … }`
//! * C# `lambda_expression` — `(Foo x) => …`, `(Foo? x) => { … }`
//! * Scala `lambda_expression` — `(x: Foo) => …`; `case_clause` —
//!   `case x: Foo => …`

use cgg_core::{FileFacts, ScopedType};
use tree_sitter::Node;

/// Record the typed parameters `node` introduces, if it is a closure or a
/// typed pattern clause of `lang`. Cheap for every other node: one kind
/// comparison.
pub fn record(lang: &str, node: Node, src: &[u8], facts: &mut FileFacts) {
    let params: Vec<(Node, Node)> = match (lang, node.kind()) {
        ("dart", "function_expression") => node
            .child_by_field_name("parameters")
            .map(|list| {
                formal_parameters(list)
                    .into_iter()
                    .filter_map(|p| {
                        let ty =
                            p.children(&mut p.walk()).find(|c| c.kind() == "type")?;
                        Some((p.child_by_field_name("name")?, ty))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ("csharp", "lambda_expression") => node
            .child_by_field_name("parameters")
            .filter(|l| l.kind() == "parameter_list")
            .map(|list| {
                list.named_children(&mut list.walk())
                    .filter(|p| p.kind() == "parameter")
                    .filter_map(|p| {
                        Some((
                            p.child_by_field_name("name")?,
                            p.child_by_field_name("type")?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ("scala", "lambda_expression") => node
            .child_by_field_name("parameters")
            .filter(|l| l.kind() == "bindings")
            .map(|list| {
                list.named_children(&mut list.walk())
                    .filter(|b| b.kind() == "binding")
                    .filter_map(|b| {
                        Some((
                            b.child_by_field_name("name")?,
                            b.child_by_field_name("type")?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default(),
        ("scala", "case_clause") => node
            .child_by_field_name("pattern")
            .filter(|p| p.kind() == "typed_pattern")
            .and_then(|p| {
                let name = p.child_by_field_name("pattern")?;
                (name.kind() == "identifier")
                    .then_some((name, p.child_by_field_name("type")?))
            })
            .into_iter()
            .collect(),
        _ => return,
    };
    if params.is_empty() {
        record_untyped_closure_arg(lang, node, src, facts);
        return;
    }
    for (name, ty) in params {
        let (Ok(name), Ok(ty)) = (name.utf8_text(src), ty.utf8_text(src)) else {
            continue;
        };
        let Some(ty) = nominal(lang, ty) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        facts.scoped_types.push(ScopedType {
            name: name.to_string(),
            type_name: ty.to_string(),
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
        });
    }
}

/// The parameter names of a closure none of whose parameters has a written
/// type; `None` for any other node, or a closure with a typed parameter.
fn untyped_params(lang: &str, node: Node, src: &[u8]) -> Option<Vec<String>> {
    let text = |n: Node| n.utf8_text(src).ok().map(str::to_string);
    let list = node.child_by_field_name("parameters")?;
    let mut out = Vec::new();
    match (lang, node.kind(), list.kind()) {
        ("dart", "function_expression", _) => {
            for p in formal_parameters(list) {
                if p.children(&mut p.walk()).any(|c| c.kind() == "type") {
                    return None;
                }
                let name = p.child_by_field_name("name").or_else(|| {
                    p.named_children(&mut p.walk())
                        .find(|c| c.kind() == "identifier")
                })?;
                out.push(text(name)?);
            }
        }
        ("csharp", "lambda_expression", "implicit_parameter") => out.push(text(list)?),
        ("csharp", "lambda_expression", "parameter_list") => {
            for p in list.named_children(&mut list.walk()) {
                if p.kind() != "parameter" || p.child_by_field_name("type").is_some() {
                    return None;
                }
                out.push(text(p.child_by_field_name("name")?)?);
            }
        }
        ("scala", "lambda_expression", "identifier") => out.push(text(list)?),
        ("scala", "lambda_expression", "bindings") => {
            for b in list.named_children(&mut list.walk()) {
                if b.kind() != "binding" || b.child_by_field_name("type").is_some() {
                    return None;
                }
                out.push(text(b.child_by_field_name("name")?)?);
            }
        }
        _ => return None,
    }
    (!out.is_empty()).then_some(out)
}

/// An untyped closure that is a positional argument of a bare call
/// (`each((x) => …)`, `Each(x => …)`, `withTempDir { dir => … }`) is
/// recorded with the callee and its position, so the resolver can type
/// its parameters from the callee's declared callback type
/// ([`cgg_core::FileFacts::closure_args`]).
fn record_untyped_closure_arg(lang: &str, node: Node, src: &[u8], facts: &mut FileFacts) {
    let Some(params) = untyped_params(lang, node, src) else {
        return;
    };
    let Some((callee, arg_index)) = positional_arg_of_bare_call(lang, node, src) else {
        return;
    };
    facts.closure_args.push(cgg_core::ClosureArg {
        callee,
        arg_index,
        params,
        start_byte: node.start_byte() as u32,
        end_byte: node.end_byte() as u32,
    });
}

/// `(callee, positional index)` when `closure` is a positional argument
/// of a call whose callee is a bare name.
fn positional_arg_of_bare_call(
    lang: &str,
    closure: Node,
    src: &[u8],
) -> Option<(String, u32)> {
    let mut n = closure;
    let (call, index) = loop {
        let p = n.parent()?;
        match p.kind() {
            "arguments" | "argument_list" => {
                let mut index = 0u32;
                for c in p.named_children(&mut p.walk()) {
                    if c.id() == n.id() {
                        break;
                    }
                    if c.kind() != "named_argument" {
                        index += 1;
                    }
                }
                break (p.parent()?, index);
            }
            // C# wraps each argument.
            "argument" if lang == "csharp" => {
                if p.child_by_field_name("name").is_some() || p.named_child_count() != 1 {
                    return None;
                }
                n = p;
            }
            // Scala `f { x => … }`: the block is the argument list.
            "block" if lang == "scala" => {
                if p.named_child_count() != 1 {
                    return None;
                }
                let call = p.parent()?;
                if call.kind() != "call_expression"
                    || call.child_by_field_name("arguments")?.id() != p.id()
                {
                    return None;
                }
                break (call, 0);
            }
            // Dart reads `(x) => x.m()` as `((x) => x).m()`: the argument
            // is the expression the closure starts.
            "member_expression" | "call_expression" | "selector"
                if lang == "dart" && p.start_byte() == closure.start_byte() =>
            {
                n = p;
            }
            _ => return None,
        }
    };
    if !matches!(call.kind(), "call_expression" | "invocation_expression") {
        return None;
    }
    let callee = call.child_by_field_name("function")?;
    if callee.kind() != "identifier" {
        return None;
    }
    Some((callee.utf8_text(src).ok()?.to_string(), index))
}

/// Dart `formal_parameter_list` → its `formal_parameter`s, including
/// those inside optional / named groups.
fn formal_parameters(list: Node) -> Vec<Node> {
    let mut out = Vec::new();
    let mut stack = vec![list];
    while let Some(n) = stack.pop() {
        for c in n.named_children(&mut n.walk()) {
            match c.kind() {
                "formal_parameter" => out.push(c),
                "optional_formal_parameters"
                | "optional_positional_formal_parameters"
                | "named_formal_parameters"
                | "default_formal_parameter"
                | "default_named_parameter" => stack.push(c),
                _ => {}
            }
        }
    }
    out.sort_by_key(|n| n.start_byte());
    out
}

/// The nominal class a written parameter type names: nullability and
/// maybe-wrappers removed ([`cgg_core::declared_type`]), type arguments
/// cut, and only a capitalised name (a type, not a primitive keyword).
fn nominal<'a>(lang: &str, ty: &'a str) -> Option<&'a str> {
    let ty = cgg_core::declared_type(lang, ty)?;
    let ty = ty.split(['<', '[', '(', ' ']).next()?.trim();
    let last = ty.rsplit('.').next().unwrap_or(ty);
    (cgg_core::looks_like_type_name(last)
        && ty
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.'))
    .then_some(ty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn scoped(
        lang: &str,
        l: tree_sitter::Language,
        src: &str,
    ) -> Vec<(String, String, String)> {
        let mut p = Parser::new();
        p.set_language(&l).unwrap();
        let tree = p.parse(src, None).unwrap();
        let mut facts = FileFacts::new(
            cgg_core::ids::FileId::new(0),
            std::path::PathBuf::from("x"),
            lang,
        );
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            record(lang, n, src.as_bytes(), &mut facts);
            stack.extend(n.children(&mut n.walk()));
        }
        let mut v: Vec<_> = facts
            .scoped_types
            .iter()
            .map(|s| {
                (
                    s.name.clone(),
                    s.type_name.clone(),
                    src[s.start_byte as usize..s.end_byte as usize].to_string(),
                )
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn dart_typed_closure_parameters_are_scoped_to_the_closure() {
        let v = scoped(
            "dart",
            tree_sitter_dart::LANGUAGE.into(),
            "void f(List<Foo> xs) {\n  xs.forEach((Foo x) { x.run(); });\n  xs.map((Bar? y) => y);\n  xs.forEach((z) => z.run());\n  xs.forEach((int i) {});\n}\n",
        );
        let names: Vec<(&str, &str)> =
            v.iter().map(|(n, t, _)| (n.as_str(), t.as_str())).collect();
        // Near misses: an untyped parameter and a primitive one.
        assert_eq!(names, vec![("x", "Foo"), ("y", "Bar")]);
        assert!(v[0].2.starts_with("(Foo x)") && v[0].2.ends_with('}'));
    }

    #[test]
    fn csharp_typed_lambda_parameters_are_scoped_to_the_lambda() {
        let v = scoped(
            "csharp",
            tree_sitter_c_sharp::LANGUAGE.into(),
            "class M { void F(List<Foo> xs) { xs.ForEach((Foo x) => x.Run()); xs.ForEach((Foo? y) => { y.Run(); }); xs.ForEach(z => z.Run()); xs.ForEach((int i) => i); } }",
        );
        let names: Vec<(&str, &str)> =
            v.iter().map(|(n, t, _)| (n.as_str(), t.as_str())).collect();
        assert_eq!(names, vec![("x", "Foo"), ("y", "Foo")]);
    }

    fn closure_args(
        lang: &str,
        l: tree_sitter::Language,
        src: &str,
    ) -> Vec<(String, u32, Vec<String>)> {
        let mut p = Parser::new();
        p.set_language(&l).unwrap();
        let tree = p.parse(src, None).unwrap();
        let mut facts = FileFacts::new(
            cgg_core::ids::FileId::new(0),
            std::path::PathBuf::from("x"),
            lang,
        );
        let mut stack = vec![tree.root_node()];
        while let Some(n) = stack.pop() {
            record(lang, n, src.as_bytes(), &mut facts);
            stack.extend(n.children(&mut n.walk()));
        }
        let mut v: Vec<_> = facts
            .closure_args
            .iter()
            .map(|c| (c.callee.clone(), c.arg_index, c.params.clone()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn untyped_closures_passed_to_a_bare_call_are_recorded_with_their_position() {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            closure_args(
                "dart",
                tree_sitter_dart::LANGUAGE.into(),
                "void f() {\n  each((x) => x.run());\n  pair(1, (a, b) { a.go(); });\n  xs.forEach((y) => y.run());\n  named(cb: (z) => z.run());\n}\n",
            ),
            // Near misses: a receiver call (`xs.forEach`) and a named
            // argument are not recorded.
            vec![
                ("each".into(), 0, s(&["x"])),
                ("pair".into(), 1, s(&["a", "b"]))
            ]
        );
        assert_eq!(
            closure_args(
                "csharp",
                tree_sitter_c_sharp::LANGUAGE.into(),
                "class M { void F() { Each(x => x.Run()); Two(1, (a, b) => a.Go()); xs.ForEach(y => y.Run()); } }",
            ),
            vec![
                ("Each".into(), 0, s(&["x"])),
                ("Two".into(), 1, s(&["a", "b"]))
            ]
        );
        assert_eq!(
            closure_args(
                "scala",
                tree_sitter_scala::LANGUAGE.into(),
                "object M {\n  def f(): Unit = {\n    withTempDir { dir => dir.list() }\n    each(x => x.run())\n    xs.foreach(y => y.run())\n  }\n}\n",
            ),
            vec![
                ("each".into(), 0, s(&["x"])),
                ("withTempDir".into(), 0, s(&["dir"]))
            ]
        );
    }

    #[test]
    fn scala_typed_lambdas_and_patterns_are_scoped() {
        let v = scoped(
            "scala",
            tree_sitter_scala::LANGUAGE.into(),
            "object M {\n  def f(xs: List[Foo]): Unit = {\n    xs.foreach((x: Foo) => x.run())\n    xs.foreach { case y: Bar => y.run() }\n    xs.foreach(z => z.run())\n    xs.foreach { case Some(w) => w }\n  }\n}\n",
        );
        let names: Vec<(&str, &str)> =
            v.iter().map(|(n, t, _)| (n.as_str(), t.as_str())).collect();
        assert_eq!(names, vec![("x", "Foo"), ("y", "Bar")]);
    }
}
