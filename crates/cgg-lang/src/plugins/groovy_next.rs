//! The vendored dekobon/tree-sitter-groovy grammar (`vendor/groovy`) the
//! Groovy plugin parses with (issue #11). These tests pin what it was
//! vendored for: every Groovy shape the `tree-sitter-groovy` crate rejected,
//! and the gaps found in the dekobon grammar itself, parse without an
//! `ERROR`.

use super::groovy::language;
use tree_sitter::{Node, Parser};

fn first_error(node: Node<'_>) -> Option<(usize, usize, &'static str)> {
    if node.is_error() || node.is_missing() {
        let p = node.start_position();
        return Some((p.row, p.column, node.kind()));
    }
    let mut cursor = node.walk();
    node.children(&mut cursor).find_map(first_error)
}

fn parse_error(src: &str) -> Option<(usize, usize, &'static str)> {
    let mut parser = Parser::new();
    parser
        .set_language(&language())
        .expect("vendored grammar loads");
    let tree = parser.parse(src, None).expect("parse");
    first_error(tree.root_node())
}

/// Every shape issue #11 lists, and the gaps found in the dekobon grammar
/// itself while evaluating it (#112–#114 and #92 upstream, multi-line and
/// space-led slashy strings). Each is valid Groovy 4.
const KNOWN_GAPS: &[(&str, &str)] = &[
    ("untyped parameters", "def f(x, y) { x + y }\n"),
    (
        "leading-dot continuation",
        "def s = x\n    .m()\n    .n()\n",
    ),
    (
        "string as a DSL block",
        "dependencies {\n  'org:a:1.0' { transitive = false }\n}\n",
    ),
    ("string as a call", "'g:a:1'()\n"),
    (
        "nested quotes in an interpolation",
        "def s = \"${f(\"a\")}\"\n",
    ),
    ("multiple assignment", "def (a, b) = [1, 2]\n"),
    (
        "untyped modified uppercase field",
        "class C {\n  private static final ABSOLUTE = 1\n}\n",
    ),
    (
        "Java-style lambda argument",
        "list.forEach(x -> println(x))\n",
    ),
    ("chained command", "module('a') tries('1') alignsTo('2')\n"),
    (
        "command as an initializer",
        "def r = script.sh script: 'x'\n",
    ),
    (
        "closure after a literal argument",
        "run \"x\" {\n  println 1\n}\n",
    ),
    ("quoted member access", "obj.'NAME'\n"),
    ("multi-line slashy string", "def r = /a\nb/\n"),
    ("multi-line dollar-slashy string", "def r = $/a\nb/$\n"),
    (
        "slashy string led by a space",
        "s.replaceFirst(/ in [ \\dms]+/, '')\n",
    ),
    (
        "direct field access after ?. ??. *.",
        "def n = people*.@name\ndef m = a?.@b\ndef o = a??.@c\n",
    ),
    (
        "string key in a parenthesis-free call",
        "attributes 'Main-Class': 'a.B'\n",
    ),
    ("annotated untyped script variable", "@Field FOO = [a: 1]\n"),
    (
        "annotated untyped field",
        "class S {\n  @Subject svc = new Svc()\n}\n",
    ),
    (
        "annotations among modifiers",
        "class C {\n  private @Autowired Foo foo\n  public @Override String toString() { 'c' }\n  protected final transient @Delegate\n  Adapter a\n}\n",
    ),
    ("modifier before @interface", "public @interface X { }\n"),
    (
        "sealed before a modifier",
        "sealed abstract class W permits V { }\n",
    ),
    (
        "annotation after final in parameters",
        "def f(final @A x) { }\nxs.each { final @A y -> y }\n",
    ),
];

#[test]
fn vendored_groovy_grammar_parses_every_known_gap() {
    let failures: Vec<String> = KNOWN_GAPS
        .iter()
        .filter_map(|(name, src)| {
            parse_error(src)
                .map(|(row, col, kind)| format!("{name}: {kind} at {row}:{col}"))
        })
        .collect();
    assert!(failures.is_empty(), "shapes with an error: {failures:#?}");
}
