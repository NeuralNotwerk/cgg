//! Shared C-family extraction helpers for macros the tree does not define.
//!
//! * [`fn_macro_def`]: a function-defining macro from outside the tree
//!   (`PHP_METHOD(Redis, get) { … }`, `ZTEST(suite, case) { … }`), named
//!   by the table `cgg_core::stdlib::C_FN_DEFINING_MACROS`.
//! * [`parse_rewrite`]: a same-length copy of a C/C++/ObjC file for the
//!   grammar to parse, with the two macro tokens it cannot read blanked —
//!   an export macro between `class`/`struct` and the name
//!   (`class BOMBER_API UMapComp : public Base`, Qt `Q_*_EXPORT`,
//!   `DLLEXPORT`), which costs the class its name and base list, and a
//!   calling-convention macro between a return type and the function name
//!   (`AudioProcessor* JUCE_CALLTYPE createPluginFilter()`), which costs
//!   the file the function. Extraction still reads the original bytes.

use cgg_core::DefVariant;
use cgg_core::stdlib::{FnMacroNaming, fn_defining_macro};
use tree_sitter::Node;

/// Longest attribute text kept (as `attrs.rs`).
const MAX_ATTR: usize = 512;

/// The function a function-defining macro invocation defines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FnMacroDef {
    pub simple: String,
    /// The name as written relative to the enclosing scope
    /// (`Redis::get`, `suite::case`, `name`).
    pub qualified: String,
    pub variant: DefVariant,
    /// The invocation, whitespace collapsed (`ZTEST(suite, case)`).
    pub attribute: String,
}

/// `macro_name` invoked with the parenthesised `args` (the text from `(`
/// to `)`, both included) at `line`: the function it defines, when
/// `macro_name` is a known function-defining macro and the arguments name
/// one. `cpp` limits the table to the entries C++ reads the same way.
pub(crate) fn fn_macro_def(
    macro_name: &str,
    args: &str,
    line: u32,
    cpp: bool,
) -> Option<FnMacroDef> {
    let naming = fn_defining_macro(macro_name, cpp)?;
    let inner = args.trim().strip_prefix('(')?.strip_suffix(')')?;
    let parts = split_args(inner);
    let ident = |i: usize| {
        parts
            .get(i)
            .map(|s| s.trim())
            .filter(|s| is_ident(s))
            .map(str::to_string)
    };
    let (simple, qualified, variant) = match naming {
        FnMacroNaming::First => {
            let n = ident(0)?;
            (n.clone(), n, DefVariant::FreeFunction)
        }
        FnMacroNaming::OwnerMethod => {
            let (o, m) = (ident(0)?, ident(1)?);
            (m.clone(), format!("{o}::{m}"), DefVariant::InherentMethod)
        }
        FnMacroNaming::SuiteCase => {
            let (s, c) = (ident(0)?, ident(1)?);
            (c.clone(), format!("{s}::{c}"), DefVariant::FreeFunction)
        }
        FnMacroNaming::Described => {
            if !parts.iter().any(|p| p.trim_start().starts_with('"')) {
                return None;
            }
            let n = format!("{macro_name}_at_{line}");
            (n.clone(), n, DefVariant::FreeFunction)
        }
    };
    let mut attribute = format!(
        "{macro_name}{}",
        args.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    if attribute.len() > MAX_ATTR {
        let mut cut = MAX_ATTR;
        while !attribute.is_char_boundary(cut) {
            cut -= 1;
        }
        attribute.truncate(cut);
    }
    Some(FnMacroDef {
        simple,
        qualified,
        variant,
        attribute,
    })
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Top-level comma split, honouring brackets and string/char literals.
fn split_args(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let (mut depth, mut start, mut i) = (0i32, 0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if start < s.len() || !out.is_empty() {
        out.push(&s[start.min(s.len())..]);
    }
    out
}

/// Whether `s` is written like a macro: upper-case letters, digits and
/// underscores, starting with a letter, with at least two letters.
pub(crate) fn is_macro_case(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_uppercase()
        && b.iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
        && b.iter().filter(|c| c.is_ascii_uppercase()).count() >= 2
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    Ident,
    Punct(u8),
}

/// A lexed token: kind and byte span.
#[derive(Debug, Clone, Copy)]
struct T {
    kind: Tok,
    start: usize,
    end: usize,
}

/// Tokens of the source outside comments, string and char literals and
/// preprocessor lines (a directive's text is never a declaration head).
fn lex(src: &[u8]) -> Vec<T> {
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line_start = true;
    while i < src.len() {
        let c = src[i];
        if c == b'\n' {
            line_start = true;
            i += 1;
            continue;
        }
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if line_start && c == b'#' {
            // Skip the directive, continuation lines included.
            while i < src.len() && src[i] != b'\n' {
                if src[i] == b'\\' && src.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                i += 1;
            }
            continue;
        }
        line_start = false;
        if c == b'/' && src.get(i + 1) == Some(&b'/') {
            while i < src.len() && src[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && src.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < src.len() && !(src[i] == b'*' && src[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        if c == b'"' || c == b'\'' {
            i += 1;
            while i < src.len() && src[i] != c && src[i] != b'\n' {
                if src[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < src.len() && (src[i].is_ascii_alphanumeric() || src[i] == b'_') {
                i += 1;
            }
            // A raw string (`R"(…)"`) or prefixed literal: skip it whole.
            if src.get(i) == Some(&b'"') {
                let prefix = &src[s..i];
                if prefix.ends_with(b"R") {
                    if let Some(open) = src[i..].iter().position(|&b| b == b'(') {
                        let delim = &src[i + 1..i + open];
                        let mut close = Vec::with_capacity(delim.len() + 2);
                        close.push(b')');
                        close.extend_from_slice(delim);
                        close.push(b'"');
                        let body = i + open + 1;
                        i = src[body..]
                            .windows(close.len())
                            .position(|w| w == close.as_slice())
                            .map_or(src.len(), |p| body + p + close.len());
                        continue;
                    }
                }
                continue; // the literal is skipped on the next turn
            }
            out.push(T {
                kind: Tok::Ident,
                start: s,
                end: i,
            });
            continue;
        }
        if c.is_ascii_digit() {
            while i < src.len()
                && (src[i].is_ascii_alphanumeric() || matches!(src[i], b'.' | b'\''))
            {
                i += 1;
            }
            out.push(T {
                kind: Tok::Punct(b'0'),
                start: i - 1,
                end: i,
            });
            continue;
        }
        out.push(T {
            kind: Tok::Punct(c),
            start: i,
            end: i + 1,
        });
        i += 1;
    }
    out
}

/// Keywords after which a macro-cased token is the type, not a calling
/// convention (`static LRESULT f(`, `unsigned LONG g(`), or after which
/// no declaration head follows at all (`return X f(` is not code).
const NOT_A_TYPE_BEFORE: &[&str] = &[
    "static",
    "inline",
    "extern",
    "virtual",
    "explicit",
    "const",
    "volatile",
    "constexpr",
    "consteval",
    "constinit",
    "friend",
    "typedef",
    "register",
    "unsigned",
    "signed",
    "struct",
    "class",
    "enum",
    "union",
    "typename",
    "return",
    "else",
    "case",
    "goto",
    "throw",
    "new",
    "delete",
    "sizeof",
    "co_return",
    "co_yield",
    "co_await",
    "using",
    "namespace",
    "template",
    "operator",
    "public",
    "private",
    "protected",
    "do",
    "if",
    "while",
    "switch",
    "for",
    "_Noreturn",
    "__inline",
    "__inline__",
    "thread_local",
    "mutable",
    "auto",
];

/// The same-length rewrite `parse_source` hands the grammar, or `None`
/// when nothing needs blanking.
pub(crate) fn parse_rewrite(src: &[u8], cpp: bool) -> Option<Vec<u8>> {
    // Cheap pre-check: nothing macro-cased at all means nothing to do.
    if !src
        .windows(2)
        .any(|w| w[0].is_ascii_uppercase() && w[1].is_ascii_uppercase())
    {
        return None;
    }
    let toks = lex(src);
    let text = |t: &T| std::str::from_utf8(&src[t.start..t.end]).unwrap_or("");
    let is_ident = |i: usize| toks.get(i).is_some_and(|t| t.kind == Tok::Ident);
    let punct = |i: usize, p: u8| toks.get(i).is_some_and(|t| t.kind == Tok::Punct(p));
    let mut blank: Vec<(usize, usize)> = Vec::new();
    for i in 0..toks.len() {
        if !is_ident(i) || !is_macro_case(text(&toks[i])) {
            continue;
        }
        // `class X_API Name :` / `class X_API Name {` / `class X_API Name final`.
        if cpp
            && i > 0
            && is_ident(i - 1)
            && matches!(text(&toks[i - 1]), "class" | "struct")
            && is_ident(i + 1)
            && text(&toks[i + 1]) != "final"
            && (punct(i + 2, b':') && !punct(i + 3, b':')
                || punct(i + 2, b'{')
                || is_ident(i + 2) && text(&toks[i + 2]) == "final")
        {
            blank.push((toks[i].start, toks[i].end));
            continue;
        }
        // `T* MACRO name(` / `T MACRO ns::name(`: a calling convention
        // between the return type and the name.
        if i == 0 {
            continue;
        }
        let prev = &toks[i - 1];
        let after_type = match prev.kind {
            Tok::Punct(b'*') | Tok::Punct(b'&') | Tok::Punct(b'>') => true,
            Tok::Ident => !NOT_A_TYPE_BEFORE.contains(&text(prev)),
            _ => false,
        };
        if !after_type {
            continue;
        }
        // The name: an identifier, possibly `A::B::name`, then `(`.
        let mut j = i + 1;
        if !is_ident(j) {
            continue;
        }
        while punct(j + 1, b':') && punct(j + 2, b':') && is_ident(j + 3) {
            j += 3;
        }
        if !punct(j + 1, b'(') {
            continue;
        }
        // The name itself must not be macro-cased: `int FOO BAR(x)` is
        // more likely a macro applied to a macro than a function.
        if is_macro_case(text(&toks[j])) {
            continue;
        }
        // The token before must itself follow a declaration start: the
        // type is the first token of the item, or follows a qualifier.
        // Inside an expression (`a * FOO f(`) the C is not valid anyway.
        blank.push((toks[i].start, toks[i].end));
    }
    let semis = if cpp {
        class_body_macro_lines(src, &toks, &mut blank)
    } else {
        Vec::new()
    };
    if blank.is_empty() && semis.is_empty() {
        return None;
    }
    let mut out = src.to_vec();
    for (s, e) in blank {
        for b in &mut out[s..e] {
            *b = b' ';
        }
    }
    for at in semis {
        out[at] = b';';
    }
    Some(out)
}

/// Macro invocations on lines of their own in a C++ class body, which the
/// grammar runs into the next member when they have no `;` (`Q_OBJECT`,
/// `GENERATED_BODY()`, `UFUNCTION(BlueprintCallable)`): the member after
/// one is lost or misread. A bare one (`Q_OBJECT`) is blanked; one with
/// arguments gets a `;` in the whitespace after it (so `c_macro` still
/// sees an in-tree macro's invocation), when there is whitespace to use.
/// Returns the byte offsets to turn into `;`.
fn class_body_macro_lines(
    src: &[u8],
    toks: &[T],
    blank: &mut Vec<(usize, usize)>,
) -> Vec<usize> {
    let text = |t: &T| std::str::from_utf8(&src[t.start..t.end]).unwrap_or("");
    let nl = |a: usize, b: usize| src[a..b].contains(&b'\n');
    let mut semis = Vec::new();
    // One flag per open brace: whether it opened a class body.
    let mut stack: Vec<bool> = Vec::new();
    let mut head = 0usize;
    let mut i = 0usize;
    while i < toks.len() {
        let t = toks[i];
        match t.kind {
            Tok::Punct(b'{') => {
                // The head from its last `class`/`struct`/`union`: an
                // annotation macro before it (`UCLASS(…)`) is not part of it.
                let h = &toks[head..i];
                let kw = h.iter().rposition(|x| {
                    x.kind == Tok::Ident
                        && matches!(text(x), "class" | "struct" | "union")
                });
                let class = kw.is_some_and(|k| {
                    let after_enum =
                        k > 0 && h[k - 1].kind == Tok::Ident && text(&h[k - 1]) == "enum";
                    let paren_or_init = h[k..]
                        .iter()
                        .any(|x| matches!(x.kind, Tok::Punct(b'(') | Tok::Punct(b'=')));
                    !after_enum && !paren_or_init
                });
                stack.push(class);
                head = i + 1;
            }
            Tok::Punct(b'}') => {
                stack.pop();
                head = i + 1;
            }
            Tok::Punct(b';') => head = i + 1,
            Tok::Ident
                if stack.last() == Some(&true)
                    && is_macro_case(text(&t))
                    && (i == 0 || nl(toks[i - 1].end, t.start)) =>
            {
                let Some(next) = toks.get(i + 1) else { break };
                if next.kind == Tok::Punct(b'(') && !nl(t.end, next.start) {
                    // Find the matching `)`.
                    let mut depth = 0i32;
                    let mut k = i + 1;
                    while k < toks.len() {
                        match toks[k].kind {
                            Tok::Punct(b'(') => depth += 1,
                            Tok::Punct(b')') => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Tok::Punct(b';') | Tok::Punct(b'{') | Tok::Punct(b'}') => {
                                break;
                            }
                            _ => {}
                        }
                        k += 1;
                    }
                    if toks.get(k).is_some_and(|c| c.kind == Tok::Punct(b')'))
                        && let Some(after) = toks.get(k + 1)
                        && nl(toks[k].end, after.start)
                        && !matches!(
                            after.kind,
                            Tok::Punct(b';')
                                | Tok::Punct(b'{')
                                | Tok::Punct(b':')
                                | Tok::Punct(b',')
                        )
                    {
                        match src[toks[k].end..after.start]
                            .iter()
                            .position(|b| *b == b' ' || *b == b'\t')
                        {
                            Some(off) => semis.push(toks[k].end + off),
                            // No whitespace to hold a `;` (`GENERATED_BODY()`,
                            // a blank line, `public:` at column 0): blank
                            // the invocation instead.
                            None => blank.push((t.start, toks[k].end)),
                        }
                        head = k + 1;
                        i = k + 1;
                        continue;
                    }
                } else if nl(t.end, next.start)
                    && !matches!(
                        next.kind,
                        Tok::Punct(b'(')
                            | Tok::Punct(b';')
                            | Tok::Punct(b':')
                            | Tok::Punct(b',')
                            | Tok::Punct(b'=')
                            | Tok::Punct(b'{')
                    )
                {
                    blank.push((t.start, t.end));
                }
            }
            _ => {}
        }
        i += 1;
    }
    semis
}

/// The parameter list of the function declarator a definition's
/// declarator wraps (`int *MACRO(a, b)`), when the declared name is a
/// plain identifier.
pub(crate) fn macro_params(decl: Node<'_>) -> Option<Node<'_>> {
    let mut n = decl;
    loop {
        match n.kind() {
            "function_declarator" => {
                let d = n.child_by_field_name("declarator")?;
                if d.kind() != "identifier" {
                    return None;
                }
                return n.child_by_field_name("parameters");
            }
            "pointer_declarator" | "attributed_declarator" | "reference_declarator" => {
                n = n.child_by_field_name("declarator")?;
            }
            _ => return None,
        }
    }
}

/// `MACRO(args)` with no `;`, at item position (no enclosing function
/// body), followed directly by a block: the function `MACRO` defines when
/// it is a function-defining macro from outside the tree
/// (`cgg_core::stdlib::C_FN_DEFINING_MACROS`).
pub(crate) fn fn_macro_block(node: Node, src: &[u8], cpp: bool) -> Option<FnMacroDef> {
    if node.kind() != "expression_statement" {
        return None;
    }
    let semi = node.child(node.child_count().checked_sub(1)? as u32)?;
    if !(semi.is_missing() || semi.kind() != ";") {
        return None;
    }
    let body = node.next_sibling()?;
    if body.kind() != "compound_statement" {
        return None;
    }
    let call = node.named_child(0)?;
    if call.kind() != "call_expression" {
        return None;
    }
    let func = call.child_by_field_name("function")?;
    if func.kind() != "identifier" {
        return None;
    }
    let mut a = node.parent();
    while let Some(p) = a {
        if matches!(
            p.kind(),
            "compound_statement" | "function_definition" | "lambda_expression"
        ) {
            return None;
        }
        a = p.parent();
    }
    let args = call.child_by_field_name("arguments")?;
    fn_macro_def(
        func.utf8_text(src).ok()?,
        args.utf8_text(src).ok()?,
        node.start_position().row as u32 + 1,
        cpp,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rw(s: &str, cpp: bool) -> String {
        parse_rewrite(s.as_bytes(), cpp)
            .map(|v| String::from_utf8(v).unwrap())
            .unwrap_or_else(|| s.to_string())
    }

    #[test]
    fn export_macros_in_a_class_head_are_blanked() {
        assert_eq!(
            rw("class BOMBER_API UMapComp final : public A {};", true),
            "class            UMapComp final : public A {};"
        );
        assert_eq!(
            rw("class Q_CORE_EXPORT QFoo : public QObject {", true),
            "class               QFoo : public QObject {"
        );
        assert_eq!(rw("struct DLLEXPORT S {", true), "struct           S {");
        // A class whose own name is macro-cased keeps it.
        assert_eq!(rw("class CPU : public X {", true), "class CPU : public X {");
        assert_eq!(rw("class FOO final {", true), "class FOO final {");
        // C: `struct POINT pt;` declares a variable of a macro-cased tag.
        assert_eq!(rw("struct POINT pt;", false), "struct POINT pt;");
        assert_eq!(rw("struct POINT pt;", true), "struct POINT pt;");
        // `class A::B` is a qualified name, not a macro.
        assert_eq!(rw("class NS_X Foo::Bar {", true), "class NS_X Foo::Bar {");
    }

    #[test]
    fn class_body_macro_lines_are_terminated() {
        let src = "class T : public QObject {\n  Q_OBJECT\npublic:\n  void a() {}\n  UFUNCTION(BlueprintCallable)\n  void b();\n  GENERATED_BODY()\n  void c();\n  TF_DISALLOW(T);\n};\n";
        let want = "class T : public QObject {\n          \npublic:\n  void a() {}\n  UFUNCTION(BlueprintCallable)\n; void b();\n  GENERATED_BODY()\n; void c();\n  TF_DISALLOW(T);\n};\n";
        assert_eq!(rw(src, true), want);
        // An annotation macro before the class head does not hide the body.
        assert_eq!(
            rw("UCLASS(BlueprintType)\nclass U : A {\n  Q_OBJECT\n};", true),
            "UCLASS(BlueprintType)\nclass U : A {\n          \n};"
        );
        // No whitespace after it to hold a `;`: blanked.
        assert_eq!(
            rw("class U : A {\n\tGENERATED_BODY()\n\npublic:\n};", true),
            "class U : A {\n\t                \n\npublic:\n};"
        );
        // Not in a function body, an enum, or C.
        let f =
            "void f() {\n  FOO\n  BAR(x)\n  g();\n}\nenum class E {\n  A_B\n  , C\n};\n";
        assert_eq!(rw(f, true), f);
        assert_eq!(rw(src, false), src);
    }

    #[test]
    fn calling_convention_macros_are_blanked() {
        assert_eq!(
            rw("AudioProcessor* JUCE_CALLTYPE createPluginFilter() {", true),
            "AudioProcessor*               createPluginFilter() {"
        );
        assert_eq!(
            rw("LRESULT CALLBACK WndProc(HWND h) {", false),
            "LRESULT          WndProc(HWND h) {"
        );
        assert_eq!(
            rw("int WINAPI ns::f(int) {", true),
            "int        ns::f(int) {"
        );
        // A macro-cased return type after a storage class stays.
        assert_eq!(
            rw("static LRESULT WndProc(HWND h) {", false),
            "static LRESULT WndProc(HWND h) {"
        );
        assert_eq!(
            rw("LRESULT WndProc(HWND h);", false),
            "LRESULT WndProc(HWND h);"
        );
        // Comments, strings and directives are not read.
        assert_eq!(
            rw(
                "// int FOO f(\n#define X int FOO f(\nchar*s=\"int FOO f(\";",
                false
            ),
            "// int FOO f(\n#define X int FOO f(\nchar*s=\"int FOO f(\";"
        );
        assert_eq!(rw("return FOO f(x);", false), "return FOO f(x);");
    }

    #[test]
    fn function_defining_macros_name_their_function() {
        let d = fn_macro_def("PHP_METHOD", "(Redis, get)", 3, false).unwrap();
        assert_eq!(
            (d.simple.as_str(), d.qualified.as_str()),
            ("get", "Redis::get")
        );
        assert_eq!(d.variant, DefVariant::InherentMethod);
        assert_eq!(d.attribute, "PHP_METHOD(Redis, get)");
        let d = fn_macro_def("BPF_KPROBE", "(do_unlinkat, int dfd)", 1, false).unwrap();
        assert_eq!(d.qualified, "do_unlinkat");
        let d = fn_macro_def("ZTEST", "(suite_a, test_one)", 1, true).unwrap();
        assert_eq!(
            (d.simple.as_str(), d.qualified.as_str()),
            ("test_one", "suite_a::test_one")
        );
        let d = fn_macro_def("TEST_CASE", "(\"desc\", \"[tag]\")", 7, false).unwrap();
        assert_eq!(d.qualified, "TEST_CASE_at_7");
        assert!(fn_macro_def("TEST_CASE", "(x)", 7, false).is_none());
        // gtest's TEST is C++'s, named by the gtest rule.
        assert!(fn_macro_def("TEST", "(A, B)", 1, true).is_none());
        assert!(fn_macro_def("TEST", "(A, B)", 1, false).is_some());
        assert!(fn_macro_def("NOT_KNOWN", "(A, B)", 1, false).is_none());
    }
}
