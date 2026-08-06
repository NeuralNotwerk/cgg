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
        &["forName", "getMethod", "getDeclaredMethod", "getField"],
    ),
    (
        "csharp",
        &["GetMethod", "GetType", "CreateInstance", "GetProperty"],
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

        if !n.kind().contains("call") {
            continue;
        }
        let Some(func) = n.child_by_field_name("function") else {
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
}
