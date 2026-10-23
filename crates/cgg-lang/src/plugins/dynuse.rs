//! Reflection / dynamic-use capture.
//!
//! Captures identifier-shaped string and symbol literals that appear in
//! a small allowlist of reflective call positions — `getattr(o, "m")`,
//! `send(:sym)`, `Class.forName("...")`.
//!
//! This is a **suppression-only** signal and never becomes an edge.
//! Turning a string into a call edge would put a guess into the call
//! graph, and cgg's whole value is that its graph contains only what it
//! could resolve. What this enables instead is an honest caveat: "a
//! literal somewhere names this symbol, so cgg may simply be unable to
//! see the call".
//!
//! Two gates keep it cheap and quiet: the literal must be
//! identifier-shaped, and it must sit in an allowlisted call position.
//! Capturing every string in a codebase would cost memory for nothing —
//! vulture measured 120 MiB to 580 MiB on tensorflow when it retained
//! AST nodes, and dropped the feature over it. Only the extracted
//! strings are kept here, never nodes.

use cgg_core::DynUse;
use tree_sitter::{Node, Tree};

/// Reflective call positions, per language: the callee name whose
/// string argument names another callable.
const REFLECTIVE: &[(&str, &[&str])] = &[
    (
        "python",
        &["getattr", "setattr", "hasattr", "delattr", "import_module"],
    ),
    (
        "ruby",
        &[
            "send",
            "public_send",
            "method",
            "respond_to?",
            "instance_variable_get",
        ],
    ),
    (
        "java",
        &[
            "forName",
            "getMethod",
            "getDeclaredMethod",
            "getField",
            "getDeclaredField",
            "loadClass",
            // `MethodHandles.lookup().findVirtual(Cls.class, "m", type)`:
            // the name is the second argument; every argument is scanned.
            "findVirtual",
            "findStatic",
            "findSpecial",
            "findGetter",
            "findSetter",
            "findStaticGetter",
            "findStaticSetter",
            "findVarHandle",
            "findStaticVarHandle",
        ],
    ),
    (
        "csharp",
        &[
            "GetMethod",
            "GetType",
            "CreateInstance",
            "GetProperty",
            "GetField",
            "GetMember",
            "GetEvent",
            "InvokeMember",
            // `Delegate.CreateDelegate(typeof(D), target, "Method")`.
            "CreateDelegate",
        ],
    ),
    ("javascript", &["require", "importScripts"]),
    ("typescript", &["require"]),
    (
        "php",
        &["call_user_func", "call_user_func_array", "method_exists"],
    ),
    ("go", &["MethodByName", "FieldByName"]),
];

/// A literal is a candidate only if it could name a definition.
fn is_identifier_shaped(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

fn strip_quotes(s: &str) -> &str {
    s.trim()
        .trim_start_matches(['"', '\'', ':'])
        .trim_end_matches(['"', '\''])
}

/// Extract dynamic-use hints from a parsed file.
pub fn extract(tree: &Tree, source: &[u8], language: &str) -> Vec<DynUse> {
    let Some((_, callees)) = REFLECTIVE.iter().find(|(l, _)| *l == language) else {
        return Vec::new();
    };
    let text =
        |n: Node| -> &str { std::str::from_utf8(&source[n.byte_range()]).unwrap_or("") };

    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let mut c = n.walk();
        stack.extend(n.children(&mut c));

        if let Some((ty, at)) = member_enumeration(n, language, source) {
            out.push(DynUse {
                name: ty,
                via: cgg_core::DYN_MEMBERS_OF.to_string(),
                site_line: at.start_position().row as u32 + 1,
                site_byte: at.start_byte() as u32,
            });
            continue;
        }

        // Java's `method_invocation` and C#'s `invocation_expression`
        // are calls too; neither has "call" in its kind.
        if !(n.kind().contains("call")
            || matches!(n.kind(), "method_invocation" | "invocation_expression"))
        {
            continue;
        }
        // The callee: `function` (Python, C#, PHP, JS), `name` (Java's
        // `o.getMethod(…)` keeps the receiver in `object`), `method`
        // (Ruby's `o.send(:m)`).
        let Some(func) = ["function", "name", "method"]
            .iter()
            .find_map(|f| n.child_by_field_name(f))
        else {
            continue;
        };
        let fname = text(func).rsplit(['.', ':']).next().unwrap_or("").trim();
        if !callees.contains(&fname) {
            continue;
        }
        let Some(args) = n.child_by_field_name("arguments") else {
            continue;
        };
        let mut ac = args.walk();
        for a in args.children(&mut ac) {
            // C# and PHP wrap each argument in an `argument` node.
            let a = if a.kind() == "argument" {
                match a.named_child(0) {
                    Some(inner) => inner,
                    None => continue,
                }
            } else {
                a
            };
            let k = a.kind();
            if !(k.contains("string") || k == "symbol" || k == "simple_symbol") {
                continue;
            }
            let lit = strip_quotes(text(a));
            if !is_identifier_shaped(lit) {
                continue;
            }
            out.push(DynUse {
                name: lit.rsplit('.').next().unwrap_or(lit).to_string(),
                via: fname.to_string(),
                site_line: a.start_position().row as u32 + 1,
                site_byte: a.start_byte() as u32,
            });
        }
    }
    out.sort_by(|a, b| a.site_byte.cmp(&b.site_byte).then(a.name.cmp(&b.name)));
    out.dedup_by(|a, b| a.site_byte == b.site_byte && a.name == b.name);
    out
}

/// A reflective enumeration of a named type's members at `n`
/// ([`cgg_core::stdlib::MEMBER_ENUMERATION_APIS`]): the type's simple
/// name, and the node to anchor the record at. Only when the type is
/// written: `typeof(T)`, `T.class`, `T::class` or `'T'`, a constant
/// receiver, a class name argument.
fn member_enumeration<'n>(
    n: Node<'n>,
    language: &str,
    src: &[u8],
) -> Option<(String, Node<'n>)> {
    let apis = cgg_core::stdlib::MEMBER_ENUMERATION_APIS
        .iter()
        .find(|(l, _)| *l == language)
        .map(|(_, a)| *a)?;
    let text = |x: Node| std::str::from_utf8(&src[x.byte_range()]).unwrap_or("");
    let simple = |t: &str| -> Option<String> {
        let t = t.trim().trim_matches(['"', '\'']);
        let t = t.split('<').next().unwrap_or(t);
        let t = t.rsplit(['.', ':', '\\']).next().unwrap_or(t).trim();
        (!t.is_empty()
            && t.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_'))
        .then(|| t.to_string())
    };
    let named = |x: Node| apis.contains(&text(x).trim());
    match language {
        // `typeof(T).GetMethods()`, `typeof(T).GetTypeInfo().DeclaredMethods`.
        "csharp" => {
            if n.kind() != "member_access_expression"
                || !n.child_by_field_name("name").is_some_and(named)
            {
                return None;
            }
            let mut recv = n.child_by_field_name("expression")?;
            // Through `.GetTypeInfo()`.
            if recv.kind() == "invocation_expression"
                && let Some(f) = recv.child_by_field_name("function")
                && f.kind() == "member_access_expression"
                && f.child_by_field_name("name")
                    .is_some_and(|x| text(x) == "GetTypeInfo")
            {
                recv = f.child_by_field_name("expression")?;
            }
            (recv.kind() == "typeof_expression")
                .then(|| recv.child_by_field_name("type"))
                .flatten()
                .and_then(|t| simple(text(t)))
                .map(|t| (t, n))
        }
        // `T.class.getDeclaredMethods()`.
        "java" => {
            if n.kind() != "method_invocation"
                || !n.child_by_field_name("name").is_some_and(named)
            {
                return None;
            }
            let obj = n.child_by_field_name("object")?;
            (obj.kind() == "class_literal")
                .then(|| obj.named_child(0))
                .flatten()
                .and_then(|t| simple(text(t)))
                .map(|t| (t, n))
        }
        // `get_class_methods(T::class)` / `get_class_methods('T')`;
        // `(new ReflectionClass(T::class))->getMethods()`.
        "php" => {
            let class_arg = |args: Node<'n>| -> Option<String> {
                let a = args.named_child(0)?;
                let a = if a.kind() == "argument" {
                    a.named_child(0)?
                } else {
                    a
                };
                match a.kind() {
                    "class_constant_access_expression" => {
                        let last = a
                            .named_child(a.named_child_count().saturating_sub(1) as u32)?;
                        if text(last) != "class" {
                            return None;
                        }
                        simple(text(a.named_child(0)?))
                    }
                    k if k.contains("string") => simple(text(a)),
                    _ => None,
                }
            };
            match n.kind() {
                "function_call_expression" => {
                    let f = n.child_by_field_name("function")?;
                    if text(f) != "get_class_methods" {
                        return None;
                    }
                    class_arg(n.child_by_field_name("arguments")?).map(|t| (t, n))
                }
                "member_call_expression" => {
                    if n.child_by_field_name("name")
                        .is_none_or(|x| text(x) != "getMethods")
                    {
                        return None;
                    }
                    let mut o = n.child_by_field_name("object")?;
                    if o.kind() == "parenthesized_expression" {
                        o = o.named_child(0)?;
                    }
                    if o.kind() != "object_creation_expression" {
                        return None;
                    }
                    let mut oc = o.walk();
                    let kids: Vec<Node> = o.named_children(&mut oc).collect();
                    let cls = kids.iter().find(|k| k.kind() == "name")?;
                    if text(*cls) != "ReflectionClass" {
                        return None;
                    }
                    let args = kids.iter().find(|k| k.kind() == "arguments")?;
                    class_arg(*args).map(|t| (t, n))
                }
                _ => None,
            }
        }
        // `T.instance_methods`, `A::T.instance_methods(false)`.
        "ruby" => {
            if n.kind() != "call" || !n.child_by_field_name("method").is_some_and(named) {
                return None;
            }
            let r = n.child_by_field_name("receiver")?;
            matches!(r.kind(), "constant" | "scope_resolution")
                .then(|| simple(text(r)))
                .flatten()
                .map(|t| (t, n))
        }
        // `inspect.getmembers(T)` with a capitalized name.
        "python" => {
            if n.kind() != "call" {
                return None;
            }
            let f = n.child_by_field_name("function")?;
            let fname = text(f).rsplit('.').next().unwrap_or("");
            if !apis.contains(&fname) {
                return None;
            }
            let a = n.child_by_field_name("arguments")?.named_child(0)?;
            (matches!(a.kind(), "identifier" | "attribute")
                && simple(text(a))
                    .is_some_and(|t| t.starts_with(|c: char| c.is_uppercase())))
            .then(|| simple(text(a)))
            .flatten()
            .map(|t| (t, n))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginRegistry;

    fn parse(lang: &str, src: &str) -> Tree {
        let reg = PluginRegistry::with_v1_plugins();
        let p = reg.all().iter().find(|p| p.id() == lang).expect("plugin");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&p.ts_language()).unwrap();
        parser.parse(src, None).expect("parse")
    }

    #[test]
    fn python_getattr_names_a_callable() {
        let src = "def f(o):\n    return getattr(o, \"handler\")\n";
        let t = parse("python", src);
        let d = extract(&t, src.as_bytes(), "python");
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].name, "handler");
        assert_eq!(d[0].via, "getattr");
    }

    #[test]
    fn non_identifier_literals_are_ignored() {
        let src = "def f(o):\n    return getattr(o, \"not a name!\")\n";
        let t = parse("python", src);
        assert!(extract(&t, src.as_bytes(), "python").is_empty());
    }

    #[test]
    fn ordinary_calls_are_not_reflective() {
        let src = "def f(o):\n    return helper(o, \"handler\")\n";
        let t = parse("python", src);
        assert!(extract(&t, src.as_bytes(), "python").is_empty());
    }

    #[test]
    fn java_get_declared_method_names_a_method() {
        let src = "class A { void f() throws Exception { A.class.getDeclaredMethod(\"m\").invoke(this); helper(\"n\"); } }";
        let t = parse("java", src);
        let d = extract(&t, src.as_bytes(), "java");
        assert_eq!(
            d.iter()
                .map(|x| (x.name.as_str(), x.via.as_str()))
                .collect::<Vec<_>>(),
            vec![("m", "getDeclaredMethod")]
        );
    }

    #[test]
    fn csharp_get_method_names_a_method() {
        let src = "class A { void F() { typeof(A).GetMethod(\"M\", 0); Log(\"N\"); } }";
        let t = parse("csharp", src);
        let d = extract(&t, src.as_bytes(), "csharp");
        assert_eq!(
            d.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            vec!["M"]
        );
    }

    #[test]
    fn ruby_send_names_a_method() {
        let src = "class A\n  def f\n    send(:m)\n    puts(:n)\n  end\nend\n";
        let t = parse("ruby", src);
        let d = extract(&t, src.as_bytes(), "ruby");
        assert_eq!(
            d.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            vec!["m"]
        );
    }

    #[test]
    fn php_call_user_func_names_a_function() {
        let src = "<?php\nfunction f() { call_user_func('m'); strlen('n'); }\n";
        let t = parse("php", src);
        let d = extract(&t, src.as_bytes(), "php");
        assert_eq!(
            d.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(),
            vec!["m"]
        );
    }

    #[test]
    fn a_language_with_no_allowlist_yields_nothing() {
        let src = "fn f() {}";
        let t = parse("rust", src);
        assert!(extract(&t, src.as_bytes(), "rust").is_empty());
    }

    #[test]
    fn identifier_shape_gate() {
        assert!(is_identifier_shaped("handler"));
        assert!(is_identifier_shaped("_private"));
        assert!(is_identifier_shaped("mod.func"));
        assert!(!is_identifier_shaped(""));
        assert!(!is_identifier_shaped("has space"));
        assert!(!is_identifier_shaped("1leading"));
        assert!(!is_identifier_shaped(&"x".repeat(200)));
    }

    #[test]
    fn output_is_deterministic() {
        let src = "def f(o):\n    getattr(o, \"a\")\n    getattr(o, \"b\")\n";
        let t = parse("python", src);
        let a = extract(&t, src.as_bytes(), "python");
        assert_eq!(a, extract(&t, src.as_bytes(), "python"));
        assert_eq!(
            a.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );
    }

    #[test]
    fn java_field_and_method_handle_lookups_name_members() {
        let src = "class A { void f() throws Throwable {\n  A.class.getDeclaredField(\"byField\");\n  MethodHandles.lookup().findVirtual(A.class, \"byFindVirtual\", mt);\n  MethodHandles.lookup().findStatic(A.class, \"byFindStatic\", mt);\n  getClass().getClassLoader().loadClass(\"p.Loaded\");\n  A.class.getDeclaredConstructor();\n  helper(A.class, \"notReflective\");\n} }";
        let t = parse("java", src);
        let mut got: Vec<(String, String)> = extract(&t, src.as_bytes(), "java")
            .into_iter()
            .map(|d| (d.name, d.via))
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                ("Loaded".into(), "loadClass".into()),
                ("byField".into(), "getDeclaredField".into()),
                ("byFindStatic".into(), "findStatic".into()),
                ("byFindVirtual".into(), "findVirtual".into()),
            ]
        );
    }

    #[test]
    fn csharp_invoke_member_field_and_delegate_name_members() {
        let src = "class A { void F() {\n  typeof(A).InvokeMember(\"ByInvokeMember\", BindingFlags.InvokeMethod, null, this, null);\n  typeof(A).GetField(\"ByGetField\");\n  typeof(A).GetMember(\"ByGetMember\");\n  typeof(A).GetEvent(\"ByGetEvent\");\n  Delegate.CreateDelegate(typeof(Action), this, \"ByCreateDelegate\");\n  Log(\"NotReflective\");\n} }";
        let t = parse("csharp", src);
        let mut got: Vec<String> = extract(&t, src.as_bytes(), "csharp")
            .into_iter()
            .map(|d| d.name)
            .collect();
        got.sort();
        assert_eq!(
            got,
            vec![
                "ByCreateDelegate",
                "ByGetEvent",
                "ByGetField",
                "ByGetMember",
                "ByInvokeMember"
            ]
        );
    }

    /// WS28b: a reflective enumeration of a type the code names records
    /// the type; one on a value (`o.GetType()`, `getClass()`) does not.
    #[test]
    fn member_enumerations_name_the_type() {
        let cases: &[(&str, &str, &[&str])] = &[
            (
                "csharp",
                "class A { void F(object o) { var a = typeof(S.Sur).GetTypeInfo().DeclaredMethods; var b = typeof(Gen<int>).GetMethods(); var c = o.GetType().GetMethods(); } }",
                &["Gen", "Sur"],
            ),
            (
                "java",
                "class A { void f() { A.class.getDeclaredMethods(); getClass().getMethods(); } }",
                &["A"],
            ),
            (
                "php",
                "<?php\n$a = get_class_methods(Foo::class);\n$b = get_class_methods('Bar');\n$c = (new ReflectionClass(Baz::class))->getMethods();\n$d = get_class_methods($x);\n",
                &["Bar", "Baz", "Foo"],
            ),
            (
                "ruby",
                "x = Foo.instance_methods(false)\ny = obj.instance_methods\n",
                &["Foo"],
            ),
            (
                "python",
                "import inspect\nm = inspect.getmembers(Handler)\nn = inspect.getmembers(obj)\n",
                &["Handler"],
            ),
        ];
        for (lang, src, want) in cases {
            let t = parse(lang, src);
            let mut got: Vec<String> = extract(&t, src.as_bytes(), lang)
                .into_iter()
                .filter(|d| d.via == cgg_core::DYN_MEMBERS_OF)
                .map(|d| d.name)
                .collect();
            got.sort();
            assert_eq!(
                got,
                want.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                "{lang}"
            );
        }
    }
}
