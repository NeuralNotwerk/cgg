//! Function-defining C/C++ macros.
//!
//! `#define DEF_GETTER(n) int get_##n(void) { return n; }` invoked as
//! `DEF_GETTER(x)` defines `get_x`; without expansion that function does
//! not exist in the graph, every call to it is unresolved, and the macro
//! itself reads as dead. This is the C-family counterpart of
//! `rust_macro.rs`, and as conservative:
//!
//! * only invocations written where items go (file, namespace or class
//!   scope) are expanded — a macro used inside a function body cannot
//!   define a function;
//! * the macro is the last definition of that name earlier in the same
//!   file, or else the first one found in the file's `#include "…"`
//!   closure;
//! * one level only: parameters are substituted (with `#` stringizing and
//!   `##` pasting) but the result is not rescanned for further macros;
//! * an expansion is used only for the function definitions in it that
//!   parse without error. Anything else changes nothing.
//!
//! Plugins record candidates while extracting ([`record_fn_macro`],
//! [`scan_invocations`]); the definitions they generate are produced by
//! [`expand`], a pass over every C/C++ file once all are extracted —
//! the macro usually lives in a header, and the invocation in another
//! file. Generated definitions are tagged `macro:<name>` like Rust's: kept
//! out of by-name fan-out and reported dead at no more than medium.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use cgg_core::{DefRecord, FileFacts, FnMacro, MacroInvocation, NameMention, RefRecord};
use tree_sitter::Node;

/// An expansion larger than this is not parsed.
const MAX_EXPANSION_BYTES: usize = 64 * 1024;
/// How deep the `#include` closure is searched for a macro (as the
/// cross-file resolver's include closure).
const INCLUDE_DEPTH: u8 = 8;

/// A `preproc_function_def` as an [`FnMacro`]. The body is kept only when
/// it contains a `{`; a macro without one is still recorded (by name), so
/// an item-position invocation of it is credited to it.
pub(super) fn record_fn_macro(node: Node, src: &[u8]) -> Option<FnMacro> {
    let name = node
        .child_by_field_name("name")?
        .utf8_text(src)
        .ok()?
        .trim();
    if name.is_empty() {
        return None;
    }
    let params_text = node
        .child_by_field_name("parameters")
        .and_then(|p| p.utf8_text(src).ok())
        .unwrap_or("()");
    let inner = params_text
        .trim()
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or("");
    let mut params = Vec::new();
    let mut variadic = false;
    for p in inner.split(',') {
        let p = p.trim();
        if p.is_empty() {
            continue;
        }
        if p == "..." {
            variadic = true;
            params.push("__VA_ARGS__".to_string());
        } else if let Some(named) = p.strip_suffix("...") {
            variadic = true;
            params.push(named.trim().to_string());
        } else {
            params.push(p.to_string());
        }
    }
    let body = node
        .child_by_field_name("value")
        .and_then(|v| v.utf8_text(src).ok())
        .unwrap_or("");
    let body = if body.contains('{') {
        body.replace("\\\r\n", " ").replace("\\\n", " ")
    } else {
        String::new()
    };
    Some(FnMacro {
        name: name.to_string(),
        params,
        variadic,
        body,
        start_byte: node.start_byte() as u32,
    })
}

/// Node kinds whose contents are never at item position.
const NOT_ITEMS: &[&str] = &[
    "compound_statement",
    "preproc_def",
    "preproc_function_def",
    "preproc_call",
    "preproc_include",
    "argument_list",
    "parameter_list",
    "initializer_list",
    "field_initializer_list",
    "template_argument_list",
    "template_parameter_list",
    "string_literal",
    "raw_string_literal",
    "concatenated_string",
    "char_literal",
    "enumerator_list",
    "lambda_expression",
    "parenthesized_expression",
    "parenthesized_declarator",
    "attribute_specifier",
    "attribute_declaration",
    "comment",
];

/// Every call-shaped `NAME(…)` written where items go: the name starts a
/// declaration (it follows `;`, `{`, `}`, `)`, an access specifier, a
/// storage class, a preprocessor line, or nothing). The grammar parses
/// such an invocation in many shapes — an expression statement, a type,
/// a declarator, a field — so candidates are found by position, not by
/// node kind.
pub(super) fn scan_invocations(
    root: Node,
    src: &[u8],
    cpp: bool,
) -> Vec<MacroInvocation> {
    let mut out: Vec<MacroInvocation> = Vec::new();
    // A file the grammar could not read as a translation unit at all has
    // no scope to place anything in.
    if root.is_error() {
        return out;
    }
    let mut scope = Vec::new();
    scan(root, src, cpp, &mut scope, 0, &mut out);
    out.sort_by_key(|m| m.start_byte);
    // One invocation's arguments never hold another.
    let mut last_end = 0;
    out.retain(|m| {
        let keep = m.start_byte >= last_end;
        if keep {
            last_end = m.end_byte;
        }
        keep
    });
    out
}

fn scan(
    node: Node,
    src: &[u8],
    cpp: bool,
    scope: &mut Vec<String>,
    classes: u32,
    out: &mut Vec<MacroInvocation>,
) {
    let mut c = node.walk();
    if !c.goto_first_child() {
        return;
    }
    loop {
        let child = c.node();
        // Inside an error holding a brace the grammar may have lost a
        // scope (a class or namespace opened through a macro): what an
        // invocation there generates could not be placed in it.
        let skip = NOT_ITEMS.contains(&child.kind())
            || (child.is_error() && src[child.byte_range()].contains(&b'{'))
            || (node.kind().starts_with("preproc_")
                && c.field_name() == Some("condition"));
        if !skip {
            match child.kind() {
                "namespace_definition" if cpp => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(src).ok())
                        .unwrap_or("")
                        .to_string();
                    scope.push(if name.is_empty() {
                        "(anonymous)".to_string()
                    } else {
                        name
                    });
                    scan(child, src, cpp, scope, classes, out);
                    scope.pop();
                }
                "class_specifier" | "struct_specifier" if cpp => {
                    let name = child
                        .child_by_field_name("name")
                        .and_then(|n| n.utf8_text(src).ok())
                        .unwrap_or("")
                        .to_string();
                    if name.is_empty() {
                        scan(child, src, cpp, scope, classes, out);
                    } else {
                        scope.push(name);
                        scan(child, src, cpp, scope, classes + 1, out);
                        scope.pop();
                    }
                }
                "identifier" | "type_identifier" | "field_identifier" => {
                    if let Some(m) = candidate(child, src, scope, classes) {
                        out.push(m);
                    }
                }
                _ => {
                    if child.child_count() > 0 {
                        scan(child, src, cpp, scope, classes, out);
                    }
                }
            }
        }
        if !c.goto_next_sibling() {
            break;
        }
    }
}

fn candidate(
    n: Node,
    src: &[u8],
    scope: &[String],
    classes: u32,
) -> Option<MacroInvocation> {
    let end = n.end_byte();
    let open = skip_ws(src, end);
    if src.get(open) != Some(&b'(') {
        return None;
    }
    if !starts_item(n) {
        return None;
    }
    let close = matching_paren(src, open)?;
    let name = n.utf8_text(src).ok()?.to_string();
    let args = std::str::from_utf8(&src[open + 1..close]).ok()?.to_string();
    Some(MacroInvocation {
        name,
        args,
        start_byte: n.start_byte() as u32,
        end_byte: (close + 1) as u32,
        line: n.start_position().row as u32 + 1,
        scope: scope.to_vec(),
        classes,
    })
}

/// Whether `n` is the first token of an item.
fn starts_item(n: Node) -> bool {
    let Some(l) = prev_leaf(n) else {
        return true;
    };
    if matches!(l.kind(), ";" | "{" | "}" | ")" | ":") {
        return true;
    }
    if l.parent()
        .is_some_and(|p| p.kind() == "storage_class_specifier")
    {
        return true;
    }
    // A preprocessor line before it: a directive that ended, or the head
    // of a conditional block the name sits in.
    let mut a = Some(l);
    while let Some(x) = a {
        if x.kind().starts_with("preproc_") {
            let contains =
                x.start_byte() <= n.start_byte() && n.end_byte() <= x.end_byte();
            return !contains || l.end_position().row < n.start_position().row;
        }
        a = x.parent();
    }
    false
}

/// The token before `n`, comments skipped.
fn prev_leaf(n: Node) -> Option<Node> {
    let mut cur = n;
    loop {
        if let Some(p) = cur.prev_sibling() {
            if p.kind() == "comment" {
                cur = p;
                continue;
            }
            let mut l = p;
            while l.child_count() > 0 {
                let last = l.child((l.child_count() - 1) as u32)?;
                if last.kind() == "comment" && l.child_count() > 1 {
                    l = l.child((l.child_count() - 2) as u32)?;
                } else {
                    l = last;
                }
            }
            return Some(l);
        }
        cur = cur.parent()?;
    }
}

fn skip_ws(src: &[u8], mut i: usize) -> usize {
    while i < src.len() {
        match src[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'\\' if matches!(src.get(i + 1), Some(b'\n' | b'\r')) => i += 1,
            _ => break,
        }
    }
    i
}

/// The `)` matching the `(` at `open`, skipping strings, character
/// literals and comments.
fn matching_paren(src: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < src.len() {
        match src[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            q @ (b'"' | b'\'') => {
                i += 1;
                while i < src.len() && src[i] != q {
                    if src[i] == b'\\' {
                        i += 1;
                    } else if src[i] == b'\n' {
                        break;
                    }
                    i += 1;
                }
            }
            b'/' if src.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < src.len() && !(src[i] == b'*' && src[i + 1] == b'/') {
                    i += 1;
                }
                i += 1;
            }
            b'/' if src.get(i + 1) == Some(&b'/') => {
                while i < src.len() && src[i] != b'\n' {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
        if i - open > MAX_EXPANSION_BYTES {
            return None;
        }
    }
    None
}

// --- Substitution -----------------------------------------------------

#[derive(Debug, PartialEq)]
enum Tok<'a> {
    Ident(&'a str),
    Ws,
    Other(&'a str),
}

/// Lex a replacement list. Comments become whitespace.
fn lex(s: &str) -> Option<Vec<Tok<'_>>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(Tok::Ws);
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let end = s[i + 2..].find("*/").map(|p| i + 2 + p + 2)?;
            i = end;
            out.push(Tok::Ws);
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            out.push(Tok::Ws);
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let st = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Tok::Ident(&s[st..i]));
        } else if c.is_ascii_digit() {
            let st = i;
            while i < b.len()
                && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.')
            {
                i += 1;
            }
            out.push(Tok::Other(&s[st..i]));
        } else if c == b'"' || c == b'\'' {
            let st = i;
            i += 1;
            while i < b.len() && b[i] != c {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(b.len());
            out.push(Tok::Other(&s[st..i]));
        } else if c == b'#' && b.get(i + 1) == Some(&b'#') {
            out.push(Tok::Other("##"));
            i += 2;
        } else {
            let len = s[i..].chars().next()?.len_utf8();
            out.push(Tok::Other(&s[i..i + len]));
            i += len;
        }
    }
    Some(out)
}

/// Split an argument list at top-level commas (only parentheses nest, as
/// in the preprocessor).
fn split_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut esc = false;
    for ch in args.chars() {
        if let Some(q) = quote {
            cur.push(ch);
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => {
                quote = Some(ch);
                cur.push(ch);
            }
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth -= 1;
                cur.push(ch);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur).trim().to_string()),
            _ => cur.push(ch),
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn stringize(arg: &str) -> String {
    let collapsed = arg.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut s = String::from("\"");
    for ch in collapsed.chars() {
        if ch == '"' || ch == '\\' {
            s.push('\\');
        }
        s.push(ch);
    }
    s.push('"');
    s
}

/// Substitute `args` into `m`'s body: `#p` stringizes, `a ## b` pastes,
/// every other parameter is replaced by its argument as written. `None`
/// when the argument count does not fit or the body uses `__VA_OPT__`.
pub(super) fn substitute(m: &FnMacro, args: &str) -> Option<String> {
    if m.body.is_empty() || m.body.contains("__VA_OPT__") {
        return None;
    }
    let mut vals = split_args(args);
    let fixed = if m.variadic {
        m.params.len() - 1
    } else {
        m.params.len()
    };
    if m.params.is_empty() {
        if !(vals.len() == 1 && vals[0].is_empty()) {
            return None;
        }
        vals.clear();
    } else if m.variadic {
        if vals.len() < fixed {
            return None;
        }
        let rest = vals.split_off(fixed).join(", ");
        vals.push(rest);
    } else if vals.len() != fixed {
        return None;
    }
    let lookup = |id: &str| {
        m.params
            .iter()
            .position(|p| p == id)
            .map(|i| vals[i].as_str())
    };
    let toks = lex(&m.body)?;
    let mut out = String::new();
    let mut i = 0;
    let mut glue = false;
    while i < toks.len() {
        let piece: String = match &toks[i] {
            Tok::Ws => {
                if !glue {
                    out.push(' ');
                }
                i += 1;
                continue;
            }
            Tok::Other("##") => {
                while out.ends_with(' ') {
                    out.pop();
                }
                glue = true;
                i += 1;
                // GNU `, ## __VA_ARGS__`: an empty variadic argument
                // swallows the comma.
                let next = toks[i..].iter().find(|t| **t != Tok::Ws);
                if m.variadic
                    && out.ends_with(',')
                    && let Some(Tok::Ident(id)) = next
                    && m.params.last().is_some_and(|p| p == id)
                    && vals.last().is_some_and(|v| v.is_empty())
                {
                    out.pop();
                }
                continue;
            }
            Tok::Other("#") => {
                let mut j = i + 1;
                while toks.get(j) == Some(&Tok::Ws) {
                    j += 1;
                }
                match toks.get(j) {
                    Some(Tok::Ident(id)) if lookup(id).is_some() => {
                        i = j + 1;
                        stringize(lookup(id).unwrap_or(""))
                    }
                    _ => {
                        i += 1;
                        "#".to_string()
                    }
                }
            }
            Tok::Ident(id) => {
                i += 1;
                lookup(id).unwrap_or(id).to_string()
            }
            Tok::Other(s) => {
                i += 1;
                (*s).to_string()
            }
        };
        out.push_str(&piece);
        glue = false;
    }
    let out = out.trim().to_string();
    (out.len() <= MAX_EXPANSION_BYTES).then_some(out)
}

// --- The pass ---------------------------------------------------------

/// Extract the definitions and calls of one expansion, as the plugin of
/// `lang` would, with `scope` (and its last `classes` entries as classes)
/// open around it.
fn extract_fragment(
    lang: &str,
    ctx: &crate::ExtractCtx<'_>,
    facts_like: &FileFacts,
    tree: &tree_sitter::Tree,
    text: &str,
    inv: &MacroInvocation,
) -> FileFacts {
    if lang == "cpp" {
        super::cpp::extract_fragment(
            ctx,
            facts_like.file,
            &facts_like.path,
            tree,
            text.as_bytes(),
            &inv.scope,
            inv.classes as usize,
        )
    } else {
        super::c::extract_fragment(
            ctx,
            facts_like.file,
            &facts_like.path,
            tree,
            text.as_bytes(),
        )
    }
}

fn normalize(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Where each file's quoted `#include`s land, by index into `files`.
struct Includes<'a> {
    paths: Vec<&'a Path>,
    includes: Vec<Vec<&'a str>>,
    by_exact: HashMap<PathBuf, usize>,
    by_last: HashMap<&'a std::ffi::OsStr, Vec<usize>>,
}

impl<'a> Includes<'a> {
    fn resolve(&self, from: usize, inc: &str) -> Option<usize> {
        let dir = self.paths[from].parent().unwrap_or(Path::new(""));
        if let Some(i) = self.by_exact.get(&normalize(&dir.join(inc))) {
            return Some(*i);
        }
        let last = Path::new(inc).file_name()?;
        let tail = inc.trim_start_matches("./").trim_start_matches("../");
        self.by_last
            .get(last)?
            .iter()
            .copied()
            .find(|&i| self.paths[i].ends_with(tail))
    }

    /// The files `from` includes, transitively, breadth-first.
    fn closure(&self, from: usize) -> Vec<usize> {
        let mut seen = HashSet::from([from]);
        let mut order = Vec::new();
        let mut frontier = vec![from];
        for _ in 0..INCLUDE_DEPTH {
            let mut next = Vec::new();
            for f in frontier {
                for inc in &self.includes[f] {
                    if let Some(t) = self.resolve(f, inc)
                        && seen.insert(t)
                    {
                        order.push(t);
                        next.push(t);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            frontier = next;
        }
        order
    }
}

/// Expand every item-position invocation of a function-defining macro in
/// the C and C++ files of a run, and clear the candidates. See the module
/// documentation.
pub fn expand(files: &mut [&mut FileFacts], ctx: &crate::ExtractCtx<'_>) {
    let macros: Vec<Vec<FnMacro>> = files
        .iter_mut()
        .map(|f| std::mem::take(&mut f.fn_macros))
        .collect();
    let invocations: Vec<Vec<MacroInvocation>> = files
        .iter_mut()
        .map(|f| std::mem::take(&mut f.macro_invocations))
        .collect();
    let names: HashSet<&str> = macros.iter().flatten().map(|m| m.name.as_str()).collect();
    if names.is_empty() || invocations.iter().all(Vec::is_empty) {
        return;
    }
    let wanted: Vec<bool> = invocations
        .iter()
        .map(|v| v.iter().any(|m| names.contains(m.name.as_str())))
        .collect();
    if !wanted.iter().any(|w| *w) {
        return;
    }
    let paths: Vec<PathBuf> = files.iter().map(|f| f.path.clone()).collect();
    let includes: Vec<Vec<String>> = files
        .iter()
        .map(|f| {
            f.imports
                .iter()
                .filter(|i| i.kind == "include")
                .map(|i| i.path.clone())
                .collect()
        })
        .collect();
    let mut by_exact = HashMap::new();
    let mut by_last: HashMap<&std::ffi::OsStr, Vec<usize>> = HashMap::new();
    for (i, p) in paths.iter().enumerate() {
        by_exact.entry(normalize(p)).or_insert(i);
        if let Some(last) = p.file_name() {
            by_last.entry(last).or_default().push(i);
        }
    }
    let inc = Includes {
        paths: paths.iter().map(PathBuf::as_path).collect(),
        includes: includes
            .iter()
            .map(|v| v.iter().map(String::as_str).collect())
            .collect(),
        by_exact,
        by_last,
    };
    let mut parsers: HashMap<&'static str, tree_sitter::Parser> = HashMap::new();
    for (fi, invs) in invocations.iter().enumerate() {
        if !wanted[fi] {
            continue;
        }
        let lang: &'static str = if files[fi].language == "cpp" {
            "cpp"
        } else {
            "c"
        };
        let mut closure: Option<Vec<usize>> = None;
        for inv in invs {
            if !names.contains(inv.name.as_str()) {
                continue;
            }
            // The definition in effect: the last one earlier in this
            // file, else the first file of the include closure with one.
            let local = macros[fi]
                .iter()
                .filter(|m| m.name == inv.name && m.start_byte < inv.start_byte)
                .next_back();
            let found = local.or_else(|| {
                let cl = closure.get_or_insert_with(|| inc.closure(fi));
                cl.iter().find_map(|&t| {
                    macros[t].iter().filter(|m| m.name == inv.name).next_back()
                })
            });
            let Some(m) = found else {
                continue;
            };
            let parser = parsers.entry(lang).or_insert_with(|| {
                let mut p = tree_sitter::Parser::new();
                let l: tree_sitter::Language = if lang == "cpp" {
                    tree_sitter_cpp::LANGUAGE.into()
                } else {
                    tree_sitter_c::LANGUAGE.into()
                };
                let _ = p.set_language(&l);
                p
            });
            let ctx = ctx.for_language(lang);
            let expanded = apply(files[fi], m, inv, lang, parser, &ctx);
            credit(files[fi], m, inv, expanded, &names);
        }
    }
}

/// Fold the expansion of `inv` into `facts`. Returns whether anything was
/// generated.
fn apply(
    facts: &mut FileFacts,
    m: &FnMacro,
    inv: &MacroInvocation,
    lang: &str,
    parser: &mut tree_sitter::Parser,
    ctx: &crate::ExtractCtx<'_>,
) -> bool {
    let Some(text) = substitute(m, &inv.args) else {
        return false;
    };
    let Some(tree) = parser.parse(&text, None) else {
        return false;
    };
    // The function definitions that parsed cleanly.
    let mut clean: Vec<(u32, u32)> = Vec::new();
    collect_clean_functions(tree.root_node(), &mut clean);
    if clean.is_empty() {
        return false;
    }
    let (s, e) = (inv.start_byte, inv.end_byte);
    // What the grammar made of the invocation itself (a prototype or
    // field named after the macro) goes; anything else overlapping it
    // means the invocation is not where items go after all.
    // (A function body named after the macro that runs past the
    // invocation — `TEST(a, b) { … }`, a macro used as a function header —
    // is not a misparse: it overlaps, so the invocation is left alone.)
    let misparse = |d: &DefRecord| {
        !d.attributes.iter().any(|a| a == "macro")
            && ((s <= d.start_byte && d.end_byte <= e)
                || (d.simple_name == inv.name
                    && s <= d.start_byte
                    && d.start_byte < e
                    && (!d.has_body || d.end_byte <= e + 1)))
    };
    if facts
        .definitions
        .iter()
        .any(|d| !misparse(d) && d.start_byte < e && s < d.end_byte)
    {
        return false;
    }
    let mut sub = extract_fragment(lang, ctx, facts, &tree, &text, inv);
    sub.definitions
        .retain(|d| clean.contains(&(d.start_byte, d.end_byte)));
    let n = sub.definitions.len() as u32;
    if n == 0 {
        return false;
    }
    // Slot 0 (the invocation's first byte) belongs to no generated
    // definition: the credit reference to the macro sits there.
    let k = (e - s).saturating_sub(1) / n;
    if k == 0 {
        return false;
    }
    let slot = |i: u32| {
        let a = s + 1 + i * k;
        (a, if i + 1 == n { e } else { a + k })
    };
    let orig: Vec<(u32, u32)> = sub
        .definitions
        .iter()
        .map(|d| (d.start_byte, d.end_byte))
        .collect();
    let owner_of = |b: u32| {
        orig.iter()
            .enumerate()
            .filter(|(_, (a, z))| *a <= b && b < *z)
            .min_by_key(|(_, (a, z))| z - a)
            .map(|(i, _)| i as u32)
    };
    let locals = crate::locals::collect(lang, &tree, text.as_bytes());

    facts.definitions.retain(|d| !misparse(d));
    // Calls written in the arguments are recorded again, inside the
    // generated function that holds them.
    facts
        .references
        .retain(|r| !(s < r.site_byte && r.site_byte < e));
    for (i, mut d) in std::mem::take(&mut sub.definitions).into_iter().enumerate() {
        let (a, z) = slot(i as u32);
        d.start_byte = a;
        d.end_byte = z;
        d.start_line = inv.line;
        d.end_line = inv.line;
        d.attributes.push(format!("macro:{}", inv.name));
        facts.definitions.push(d);
    }
    for mut r in std::mem::take(&mut sub.references) {
        let Some(i) = owner_of(r.site_byte) else {
            continue;
        };
        r.site_byte = slot(i).0;
        r.site_line = inv.line;
        facts.references.push(r);
    }
    for mut lt in std::mem::take(&mut sub.local_types) {
        let Some(i) = owner_of(lt.scope_byte) else {
            continue;
        };
        lt.scope_byte = slot(i).0;
        facts.local_types.push(lt);
    }
    for mut l in locals {
        let Some(i) = owner_of(l.start_byte) else {
            continue;
        };
        let (a, z) = slot(i);
        l.start_byte = a;
        l.end_byte = z;
        l.decl_byte = a;
        facts.locals.push(l);
    }
    facts.field_types.append(&mut sub.field_types);
    true
}

/// Byte spans of the `function_definition`s in an expansion that contain
/// no parse error, outside any function body.
fn collect_clean_functions(node: Node, out: &mut Vec<(u32, u32)>) {
    let mut c = node.walk();
    for ch in node.children(&mut c) {
        match ch.kind() {
            "function_definition" => {
                if !ch.has_error() {
                    out.push((ch.start_byte() as u32, ch.end_byte() as u32));
                }
            }
            "compound_statement" => {}
            _ => collect_clean_functions(ch, out),
        }
    }
}

/// Credit the macro with the invocation: a reference to it at the
/// invocation's first byte, which no definition encloses — the same
/// top-level invocation a call-shaped one already is. An invocation that
/// generated nothing also names its identifier arguments, which the
/// unexpanded code may call (`name_mentions`), and credits an argument
/// that is itself a function-like macro (`LIST(DECLARE)`).
fn credit(
    facts: &mut FileFacts,
    m: &FnMacro,
    inv: &MacroInvocation,
    expanded: bool,
    names: &HashSet<&str>,
) {
    let enclosed = |b: u32| {
        facts.definitions.iter().any(|d| {
            d.start_byte <= b
                && b < d.end_byte
                && !d.attributes.iter().any(|a| a.starts_with("macro:"))
        })
    };
    if enclosed(inv.start_byte) {
        return;
    }
    let has_ref = |facts: &FileFacts, name: &str, b: u32| {
        facts
            .references
            .iter()
            .any(|r| r.site_byte == b && r.name == name)
    };
    if !has_ref(facts, &m.name, inv.start_byte) {
        facts.references.push(RefRecord {
            name: m.name.clone(),
            site_line: inv.line,
            site_byte: inv.start_byte,
            macro_call: true,
            ..Default::default()
        });
    }
    if expanded {
        return;
    }
    for arg in split_args(&inv.args) {
        let is_ident = !arg.is_empty()
            && arg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !arg.starts_with(|c: char| c.is_ascii_digit());
        if !is_ident {
            continue;
        }
        if names.contains(arg.as_str()) {
            let b = inv.start_byte;
            if !has_ref(facts, &arg, b) {
                facts.references.push(RefRecord {
                    name: arg.clone(),
                    site_line: inv.line,
                    site_byte: b,
                    macro_call: true,
                    ..Default::default()
                });
            }
        } else {
            facts.name_mentions.push(NameMention {
                name: arg,
                via: "macro-arg".into(),
                site_line: inv.line,
                site_byte: inv.start_byte,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(name: &str, params: &[&str], variadic: bool, body: &str) -> FnMacro {
        FnMacro {
            name: name.into(),
            params: params.iter().map(|s| s.to_string()).collect(),
            variadic,
            body: body.into(),
            start_byte: 0,
        }
    }

    #[test]
    fn pastes_and_stringizes() {
        let m = mac(
            "DEF",
            &["n"],
            false,
            "int get_##n(void) { return lookup(#n); }",
        );
        assert_eq!(
            substitute(&m, "x").as_deref(),
            Some("int get_x(void) { return lookup(\"x\"); }")
        );
        // Wrong argument count: nothing.
        assert_eq!(substitute(&m, "x, y"), None);
    }

    #[test]
    fn variadic_arguments_and_gnu_comma() {
        let m = mac(
            "V",
            &["n", "__VA_ARGS__"],
            true,
            "void n##_f(void) { g(1, ## __VA_ARGS__); }",
        );
        assert_eq!(
            substitute(&m, "a").as_deref(),
            Some("void a_f(void) { g(1); }")
        );
        assert_eq!(
            substitute(&m, "a, 2, 3").as_deref(),
            Some("void a_f(void) { g(1,2, 3); }")
        );
    }

    fn extract(path: &str, src: &str) -> FileFacts {
        use crate::LanguagePlugin;
        let cpp = path.ends_with(".cpp") || path.ends_with(".hpp");
        let mut p = tree_sitter::Parser::new();
        let lang: tree_sitter::Language = if cpp {
            tree_sitter_cpp::LANGUAGE.into()
        } else {
            tree_sitter_c::LANGUAGE.into()
        };
        p.set_language(&lang).unwrap();
        let tree = p.parse(src, None).unwrap();
        let ctx = crate::ExtractCtx::plain();
        let path = std::path::PathBuf::from(path);
        let id = cgg_core::ids::FileId::new(0);
        if cpp {
            super::super::cpp::CppPlugin.extract(&ctx, id, &path, &tree, src.as_bytes())
        } else {
            super::super::c::CPlugin.extract(&ctx, id, &path, &tree, src.as_bytes())
        }
    }

    fn run(mut files: Vec<FileFacts>) -> Vec<FileFacts> {
        let mut refs: Vec<&mut FileFacts> = files.iter_mut().collect();
        expand(&mut refs, &crate::ExtractCtx::plain());
        files
    }

    fn names(f: &FileFacts) -> Vec<(String, Vec<String>)> {
        f.definitions
            .iter()
            .map(|d| (d.qualified_name.clone(), d.attributes.clone()))
            .collect()
    }

    /// C4: a macro defined in a header and invoked at file scope in the
    /// including file generates its functions there — with or without a
    /// trailing `;`, ahead of a real function — tagged `macro:<name>`,
    /// spans disjoint, the calls in them attributed to them, and the
    /// invocation credited to the macro.
    #[test]
    fn header_macro_generates_functions_in_the_including_file() {
        let h = extract(
            "/t/gen.h",
            "#define DEF_PAIR(n) \\\n  int get_##n(void) { return lookup(#n); } \\\n  void set_##n(int v) { store(v); }\n",
        );
        let c = extract(
            "/t/gen.c",
            "#include \"gen.h\"\nDEF_PAIR(x)\nDEF_PAIR(y);\nint real(void) { return get_x(); }\nint f(void) {\n  DEF_PAIR(inner)\n  return 0;\n}\n",
        );
        let out = run(vec![h, c]);
        let c = &out[1];
        let got = names(c);
        for want in ["get_x", "set_x", "get_y", "set_y"] {
            assert!(
                got.iter()
                    .any(|(q, a)| q == want && a == &vec!["macro:DEF_PAIR".to_string()]),
                "{want} missing from {got:?}"
            );
        }
        // Not inside a function body.
        assert!(!got.iter().any(|(q, _)| q.contains("inner")), "{got:?}");
        // `real` keeps its own span, after the invocations.
        let real = c
            .definitions
            .iter()
            .find(|d| d.simple_name == "real")
            .unwrap();
        assert_eq!(real.start_line, 4);
        let gen_spans: Vec<(u32, u32)> = c
            .definitions
            .iter()
            .filter(|d| d.attributes.iter().any(|a| a.starts_with("macro:")))
            .map(|d| (d.start_byte, d.end_byte))
            .collect();
        for (i, a) in gen_spans.iter().enumerate() {
            for b in &gen_spans[i + 1..] {
                assert!(a.1 <= b.0 || b.1 <= a.0, "{gen_spans:?}");
            }
        }
        // `lookup` sits inside generated `get_x`.
        let get_x = c
            .definitions
            .iter()
            .find(|d| d.simple_name == "get_x")
            .unwrap();
        assert!(c.references.iter().any(|r| r.name == "lookup"
            && get_x.start_byte <= r.site_byte
            && r.site_byte < get_x.end_byte));
        // The invocations credit the macro from outside every definition.
        let credits: Vec<u32> = c
            .references
            .iter()
            .filter(|r| r.name == "DEF_PAIR")
            .map(|r| r.site_byte)
            .collect();
        assert!(credits.len() >= 2, "{credits:?}");
        for b in credits.iter().take(2) {
            assert!(
                !c.definitions.iter().any(|d| d.start_byte <= *b
                    && *b < d.end_byte
                    && !d.attributes.contains(&"macro".to_string())),
                "credit at {b} is inside a definition"
            );
        }
        // The candidate lists are consumed.
        assert!(
            out.iter()
                .all(|f| f.fn_macros.is_empty() && f.macro_invocations.is_empty())
        );
    }

    /// Near misses: a function-like macro with no function in its body
    /// generates nothing (but is credited); an X-macro list invoked with a
    /// macro's name credits that macro; a macro the file cannot see (not
    /// included) is not expanded.
    #[test]
    fn macros_that_define_no_function_or_are_not_visible_generate_nothing() {
        let h = extract(
            "/t/m.h",
            "#define LOCK(n) static struct lock n = { 0, { 1 } }\n#define LIST(X) X(a) X(b)\n#define DECL(n) void h_##n(void) { }\n",
        );
        let c = extract(
            "/t/a.c",
            "#include \"m.h\"\nLOCK(big);\nLIST(DECL)\nint g(void) { return 0; }\n",
        );
        let other = extract("/t/b.c", "DECL(z)\nint k(void) { return 0; }\n");
        let out = run(vec![h, c, other]);
        let a = &out[1];
        assert!(
            !a.definitions
                .iter()
                .any(|d| d.attributes.iter().any(|x| x.starts_with("macro:"))),
            "{:?}",
            names(a)
        );
        for m in ["LOCK", "LIST", "DECL"] {
            assert!(a.references.iter().any(|r| r.name == m), "{m} not credited");
        }
        let b = &out[2];
        assert!(
            !b.definitions.iter().any(|d| d.simple_name == "h_z"),
            "{:?}",
            names(b)
        );
    }

    /// C++: a macro invoked in a class body generates methods of that
    /// class, in its namespace; the prototype the grammar made of the
    /// invocation is dropped.
    #[test]
    fn cpp_class_body_macro_generates_methods() {
        let f = extract(
            "/t/r.cpp",
            "#define ACCESSOR(n) int n() const { return n##_; } void set_##n(int v) { n##_ = v; touch(); }\nnamespace geo {\nclass Rect {\n public:\n  ACCESSOR(w)\n  ACCESSOR(h);\n  void touch() {}\n};\n}\n",
        );
        let out = run(vec![f]);
        let got: Vec<String> = out[0]
            .definitions
            .iter()
            .map(|d| d.qualified_name.clone())
            .collect();
        for want in [
            "geo::Rect::w",
            "geo::Rect::set_w",
            "geo::Rect::h",
            "geo::Rect::set_h",
        ] {
            assert!(got.iter().any(|q| q == want), "{want} missing from {got:?}");
        }
        assert!(!got.iter().any(|q| q.ends_with("::ACCESSOR")), "{got:?}");
    }

    #[test]
    fn arguments_split_on_parenthesis_depth_only() {
        assert_eq!(
            split_args("a, f(b, c), \"x,y\""),
            vec!["a", "f(b, c)", "\"x,y\""]
        );
    }
}
