//! `--externals` side-channel extraction shared by every language.
//!
//! One walk over the tree, run only when the run asks for externals
//! (`ExtractCtx::externals`). It finds calls of the process-spawning and
//! library-loading APIs in `cgg_core::stdlib::{PROCESS_SPAWN_APIS,
//! LIBRARY_LOAD_APIS}` and keeps the program or library their first
//! string literal argument names, plus shell-out literals that are not
//! calls at all (Ruby/Perl/PHP backticks). Like `plugins::dynuse`, it
//! keeps only the extracted strings, never nodes.
//!
//! The records land in `FileFacts::external_uses`, which no resolver
//! reads.

use cgg_core::externals::{ExternalUse, ExternalUseKind};
use cgg_core::stdlib::{LIBRARY_LOAD_APIS, PROCESS_SPAWN_APIS, externals_table};
use tree_sitter::{Node, Tree};

/// Whether `needle` occurs in `hay`.
fn memmem(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return false;
    }
    // `str::contains` is Two-Way; the byte-window fallback only runs on a
    // file that is not UTF-8.
    match (std::str::from_utf8(hay), std::str::from_utf8(needle)) {
        (Ok(h), Ok(n)) => h.contains(n),
        _ => hay.windows(needle.len()).any(|w| w == needle),
    }
}

/// Unify separators so `Command::new`, `os.system` and `os:cmd` compare
/// alike.
fn unify(s: &str) -> String {
    s.replace("::", ".")
        .replace("->", ".")
        .replace(['/', ':', '\\'], ".")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// Whether the callee text, as written, ends with the table entry.
#[cfg(test)]
fn callee_matches(entry: &str, callee: &str) -> bool {
    let e = unify(entry);
    let c = unify(callee);
    c == e || (c.ends_with(&e) && c[..c.len() - e.len()].ends_with('.'))
}

/// The callee of a call node, as written.
fn callee<'t>(n: Node<'t>) -> Option<Node<'t>> {
    for f in [
        "function",
        "name",
        "method",
        "macro",
        "callee",
        "constructor",
        "type",
        "target",
    ] {
        if let Some(c) = n.child_by_field_name(f) {
            return Some(c);
        }
    }
    n.named_child(0)
}

fn is_call(kind: &str) -> bool {
    kind.contains("call")
        || kind.contains("invocation")
        || matches!(
            kind,
            "new_expression"
                | "object_creation_expression"
                | "command"
                | "normal_command"
                | "apply"
                | "application"
                | "application_expression"
                | "macro_invocation"
                | "instance_creation_expression"
        )
}

/// Node kinds that run a shell command with no call around them.
const SHELL_LITERALS: &[&str] = &[
    "subshell",
    "shell_command_expression",
    "command_string",
    "backtick_string",
    "shell_command",
];

/// Strip quotes and string prefixes, and stop at an interpolation.
fn literal_text(raw: &str) -> String {
    let s = raw.trim();
    let s = s.trim_start_matches(['r', 'b', 'f', 'u', 'R', 'B', 'F', 'U', '@', '$', '#']);
    let s = s.trim_start_matches(['"', '\'', '`', '[', '(']);
    let s = s.trim_end_matches(['"', '\'', '`', ']', ')', '#']);
    let s = s.split("${").next().unwrap_or(s);
    let s = s.split("#{").next().unwrap_or(s);
    let s = s.split("\\(").next().unwrap_or(s);
    s.trim().to_string()
}

/// The literal a call's first argument is: a string, or the first
/// element of a list/array/tuple when that is a string. Anything else (a
/// variable, a macro, a concatenation) is not a literal.
fn first_arg_literal(args: Node, src: &[u8]) -> Option<String> {
    let mut c = args.walk();
    let first = args
        .named_children(&mut c)
        .find(|n| !n.kind().contains("comment"))?;
    fn unwrap(n: Node<'_>) -> Node<'_> {
        let k = n.kind();
        if matches!(
            k,
            "argument"
                | "value_argument"
                | "arguments"
                | "argument_list"
                | "expression_statement"
        ) {
            n.named_child(0).unwrap_or(n)
        } else {
            n
        }
    }
    let first = unwrap(unwrap(first));
    let k = first.kind();
    let is_str = |k: &str| {
        (k.contains("string") || k == "interpreted_string_literal")
            && !k.contains("content")
    };
    if is_str(k) {
        return first_string(first, src, 0);
    }
    if k.contains("list")
        || k.contains("array")
        || k.contains("tuple")
        || k.contains("initializer")
        || k.contains("collection")
    {
        let mut c = first.walk();
        let el = first
            .named_children(&mut c)
            .find(|n| !n.kind().contains("comment"))?;
        let el = unwrap(el);
        if is_str(el.kind()) {
            return first_string(el, src, 0);
        }
    }
    None
}

/// Node kinds that declare a named type, across grammars.
fn declares_type(kind: &str) -> bool {
    matches!(
        kind,
        "struct_item"
            | "enum_item"
            | "union_item"
            | "type_item"
            | "trait_item"
            | "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "record_declaration"
            | "struct_declaration"
            | "protocol_declaration"
            | "object_declaration"
            | "class_definition"
            | "type_alias_declaration"
            | "type_definition"
            | "class_specifier"
            | "struct_specifier"
            | "enum_specifier"
            | "union_specifier"
            | "type_alias"
            | "typealias_declaration"
            | "class"
            | "module"
            | "trait_definition"
            | "object_definition"
            | "enum_definition"
    )
}

/// The first string literal inside `n`, depth-limited.
fn first_string(n: Node, src: &[u8], depth: usize) -> Option<String> {
    if depth > 5 {
        return None;
    }
    let k = n.kind();
    if (k.contains("string")
        || k.contains("char_literal")
        || k == "interpreted_string_literal")
        && !k.contains("content")
        || k == "atom" && depth > 0
    {
        let t = std::str::from_utf8(&src[n.byte_range()]).ok()?;
        let lit = literal_text(t);
        return (!lit.is_empty()).then_some(lit);
    }
    let mut c = n.walk();
    for ch in n.named_children(&mut c) {
        if let Some(s) = first_string(ch, src, depth + 1) {
            return Some(s);
        }
    }
    None
}

/// CMake writes the program after `COMMAND`, unquoted.
fn cmake_command(n: Node, src: &[u8]) -> Option<String> {
    let t = std::str::from_utf8(&src[n.byte_range()]).ok()?;
    let mut words = t.split(|c: char| c.is_whitespace() || c == '(');
    while let Some(w) = words.next() {
        if w == "COMMAND" {
            return words
                .find(|w| !w.is_empty())
                .map(|w| literal_text(w.trim_end_matches(')')));
        }
    }
    None
}

/// Extract process-spawn and library-load uses.
pub fn extract(language: &str, tree: &Tree, src: &[u8]) -> Vec<ExternalUse> {
    let spawn = externals_table(PROCESS_SPAWN_APIS, language);
    let load = externals_table(LIBRARY_LOAD_APIS, language);
    let shell_out = matches!(language, "ruby" | "perl" | "php" | "julia");
    // The last segment of every entry: a call whose callee does not end in
    // one is rejected on one set probe, before any string is built.
    let last_seg = |e: &str| -> String {
        unify(e)
            .rsplit('.')
            .next()
            .unwrap_or("")
            .trim_end_matches(')')
            .to_string()
    };
    let tails: std::collections::HashSet<String> = spawn
        .iter()
        .chain(load.iter())
        .map(|e| last_seg(e))
        .collect();
    // Rust is the one plugin that records no declaration for an `enum`
    // or `struct` without an `impl`; the others' qualified names carry
    // their types already.
    let want_declares = language == "rust";
    // A file that never spells an API's last identifier cannot call it.
    let mentions_api = tails
        .iter()
        .any(|t| !t.is_empty() && memmem(src, t.as_bytes()));
    let walk = want_declares || mentions_api || shell_out || language == "haskell";
    let spawn_u: Vec<(String, &str)> = spawn.iter().map(|e| (unify(e), *e)).collect();
    let load_u: Vec<(String, &str)> = load.iter().map(|e| (unify(e), *e)).collect();
    let text = |n: Node| std::str::from_utf8(&src[n.byte_range()]).unwrap_or("");
    let mut out = Vec::new();
    // Pre-order over one cursor: no per-node allocation. The cursor is
    // advanced before the node is looked at, so the body may `continue`.
    let mut cursor = tree.walk();
    let mut done = !walk;
    while !done {
        let n = cursor.node();
        if !cursor.goto_first_child() {
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    done = true;
                    break;
                }
            }
        }
        let k = n.kind();
        if shell_out && SHELL_LITERALS.contains(&k) {
            let lit = literal_text(text(n).trim_start_matches("%x"));
            out.push(ExternalUse {
                kind: ExternalUseKind::ProcessSpawn,
                module: lit,
                symbol: String::new(),
                alias: String::new(),
                form: "backticks".to_string(),
                site_line: n.start_position().row as u32 + 1,
                site_byte: n.start_byte() as u32,
            });
            continue;
        }
        if want_declares
            && declares_type(k)
            && let Some(name) = n.child_by_field_name("name")
        {
            let t = text(name);
            if !t.is_empty() && t.len() < 128 {
                out.push(ExternalUse {
                    kind: ExternalUseKind::Declares,
                    module: t.split(['<', '(', ' ']).next().unwrap_or(t).to_string(),
                    symbol: String::new(),
                    alias: String::new(),
                    form: "declares".to_string(),
                    site_line: n.start_position().row as u32 + 1,
                    site_byte: n.start_byte() as u32,
                });
            }
        }
        // Haskell: the plugin records no call for `T.pack x`, so the
        // qualified head of an application is kept here, side channel
        // only.
        if language == "haskell"
            && k == "qualified"
            && let Some(parent) = n.parent()
            && parent.kind() == "apply"
            && parent
                .child_by_field_name("function")
                .is_some_and(|f| f.id() == n.id())
            && let (Some(m), Some(id)) =
                (n.child_by_field_name("module"), n.child_by_field_name("id"))
        {
            out.push(ExternalUse {
                kind: ExternalUseKind::QualifiedCall,
                module: text(m).to_string(),
                symbol: text(id).to_string(),
                alias: String::new(),
                form: "qualified-apply".to_string(),
                site_line: parent.start_position().row as u32 + 1,
                site_byte: parent.start_byte() as u32,
            });
            continue;
        }
        if !mentions_api || !is_call(k) {
            continue;
        }
        let Some(f) = callee(n) else { continue };
        // Cheap reject: the callee's own last identifier.
        {
            let t = text(f);
            let tail = t
                .rsplit(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
                .find(|x| !x.is_empty())
                .unwrap_or("");
            if !tails.contains(tail) && !tails.contains(t.trim()) {
                continue;
            }
        }
        let args_node = n
            .child_by_field_name("arguments")
            .or_else(|| n.child_by_field_name("args"));
        // The callee as written: everything before the argument list, so a
        // method call reads `Runtime.getRuntime().exec`, not `exec`.
        // Erlang wraps `m:f(…)` as `remote(module, call(f, args))`.
        let start = match n.parent() {
            Some(p) if p.kind() == "remote" => p.start_byte(),
            _ => n.start_byte(),
        };
        let ftext = match args_node {
            Some(a) if a.start_byte() > start => {
                std::str::from_utf8(&src[start..a.start_byte()]).unwrap_or("")
            }
            _ => text(f),
        };
        if ftext.len() > 200 {
            continue;
        }
        let c = unify(ftext);
        let hit = |(e, _): &&(String, &str)| {
            c == *e || (c.ends_with(e.as_str()) && c[..c.len() - e.len()].ends_with('.'))
        };
        let (kind, entry) = if let Some((_, e)) = spawn_u.iter().find(hit) {
            (ExternalUseKind::ProcessSpawn, *e)
        } else if let Some((_, e)) = load_u.iter().find(hit) {
            (ExternalUseKind::LibraryLoad, *e)
        } else {
            continue;
        };
        let args = n
            .child_by_field_name("arguments")
            .or_else(|| n.child_by_field_name("args"))
            .or_else(|| n.child_by_field_name("argument"))
            .unwrap_or(n);
        let lit = if language == "cmake" {
            cmake_command(n, src)
        } else if args == n {
            // No arguments field (command-style calls): the first
            // argument is the first named child past the callee.
            let mut c = n.walk();
            let rest: Vec<Node> = n
                .named_children(&mut c)
                .filter(|ch| ch.id() != f.id())
                .collect();
            rest.first().and_then(|a| {
                let k = a.kind();
                if k == "arguments" || k == "argument_list" {
                    return first_arg_literal(*a, src);
                }
                ((k.contains("string") || k.contains("word")) && !k.contains("content"))
                    .then(|| {
                        first_string(*a, src, 0).unwrap_or_else(|| literal_text(text(*a)))
                    })
            })
        } else {
            first_arg_literal(args, src)
        };
        out.push(ExternalUse {
            kind,
            module: lit.unwrap_or_default(),
            symbol: ftext.split_whitespace().collect::<String>(),
            alias: String::new(),
            form: entry.to_string(),
            site_line: n.start_position().row as u32 + 1,
            site_byte: n.start_byte() as u32,
        });
    }
    if language == "rust" {
        rust_foreign_blocks(tree, src, &mut out);
    }
    out.extend(scan_lines(language, src));
    out.sort_by_key(|u| (u.site_byte, u.kind as u8));
    out
}

fn use_at(
    kind: ExternalUseKind,
    module: &str,
    symbol: &str,
    form: &str,
    line: u32,
    byte: u32,
) -> ExternalUse {
    ExternalUse {
        kind,
        module: module.to_string(),
        symbol: symbol.to_string(),
        alias: String::new(),
        form: form.to_string(),
        site_line: line,
        site_byte: byte,
    }
}

/// Rust `extern "C" { fn f(); }` blocks: each item is a foreign
/// declaration of the library a preceding `#[link(name = "…")]` names.
fn rust_foreign_blocks(tree: &Tree, src: &[u8], out: &mut Vec<ExternalUse>) {
    let text = |n: Node| std::str::from_utf8(&src[n.byte_range()]).unwrap_or("");
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
        if n.kind() != "foreign_mod_item" {
            continue;
        }
        // `#[link(name = "z")]` is the block's preceding sibling.
        let mut lib = String::new();
        let mut prev = n.prev_named_sibling();
        while let Some(p) = prev {
            if p.kind() != "attribute_item" {
                break;
            }
            let t = text(p);
            if t.contains("link")
                && let Some(i) = t.find("name")
            {
                let rest = &t[i..];
                if let Some(q) = rest.find('"') {
                    let rest = &rest[q + 1..];
                    lib = rest.split('"').next().unwrap_or("").to_string();
                }
            }
            prev = p.prev_named_sibling();
        }
        let abi = text(n).split('{').next().unwrap_or("").trim().to_string();
        let lib = if lib.is_empty() {
            format!("<{abi}>")
        } else {
            lib
        };
        let mut c = n.walk();
        let Some(body) = n.child_by_field_name("body") else {
            continue;
        };
        for item in body.named_children(&mut c) {
            if item.kind() == "function_signature_item"
                && let Some(name) = item.child_by_field_name("name")
            {
                out.push(use_at(
                    ExternalUseKind::ForeignDecl,
                    &lib,
                    text(name),
                    "extern-block",
                    item.start_position().row as u32 + 1,
                    item.start_byte() as u32,
                ));
            }
        }
    }
}

/// The first `"…"` or `'…'` quoted string on a line.
fn quoted(line: &str) -> Option<&str> {
    let i = line.find(['"', '\''])?;
    let q = line[i..].chars().next()?;
    let rest = &line[i + 1..];
    rest.find(q).map(|j| &rest[..j])
}

/// Forms a line scan reads reliably and the grammars either do not
/// expose or expose only as comments: Kivy `#:import`, Verilog
/// `` `include ``, assembly `syscall`/`.extern`, Smithy `use`, Protobuf
/// `import`, OpenAPI/AsyncAPI external `$ref`, HCL `module` sources and
/// providers, Nix fetchers, and foreign declarations written as one line
/// (`foreign import`, OCaml `external`, Zig `extern "lib"`, Lean
/// `@[extern]`, Fortran `bind(c)`, C#/F# `DllImport`).
fn scan_lines(language: &str, src: &[u8]) -> Vec<ExternalUse> {
    let Ok(text) = std::str::from_utf8(src) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut byte = 0u32;
    let mut in_module_block = false;
    let mut in_providers = false;
    let mut fetcher: Option<(String, u32, u32)> = None;
    let mut owner = String::new();
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        let line_no = i as u32 + 1;
        let at = byte;
        byte += raw.len() as u32;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let push =
            |out: &mut Vec<ExternalUse>, kind, module: &str, symbol: &str, form: &str| {
                if !module.trim().is_empty() {
                    out.push(use_at(kind, module.trim(), symbol, form, line_no, at));
                }
            };
        match language {
            "kivy" => {
                if let Some(rest) = line.strip_prefix("#:import") {
                    let mut w = rest.split_whitespace();
                    let alias = w.next().unwrap_or("");
                    let module = w.next().unwrap_or(alias);
                    let mut u = use_at(
                        ExternalUseKind::Import,
                        module,
                        "",
                        "kv-import",
                        line_no,
                        at,
                    );
                    u.alias = alias.to_string();
                    out.push(u);
                } else if let Some(rest) = line.strip_prefix("#:include") {
                    push(
                        &mut out,
                        ExternalUseKind::Import,
                        rest.trim().trim_start_matches("force").trim(),
                        "",
                        "kv-include",
                    );
                }
            }
            "verilog" => {
                if line.starts_with("`include") {
                    if let Some(q) = quoted(line) {
                        push(&mut out, ExternalUseKind::Import, q, "", "include");
                    }
                } else if let Some(rest) = line.strip_prefix("import ") {
                    if rest.trim_start().starts_with("\"DPI") {
                        let name = rest
                            .split('(')
                            .next()
                            .unwrap_or("")
                            .split_whitespace()
                            .last()
                            .unwrap_or("");
                        push(
                            &mut out,
                            ExternalUseKind::ForeignDecl,
                            "DPI-C",
                            name,
                            "dpi-import",
                        );
                    } else {
                        let pkg = rest.split("::").next().unwrap_or("").trim();
                        push(
                            &mut out,
                            ExternalUseKind::Import,
                            pkg,
                            "",
                            "package-import",
                        );
                    }
                }
            }
            "asm" => {
                let l = line.split(['#', ';']).next().unwrap_or("").trim();
                let l = l
                    .strip_prefix(|c: char| c.is_ascii_digit())
                    .map_or(l, |r| r.trim_start());
                let mut w = l.split_whitespace();
                let first = w.next().unwrap_or("").to_ascii_lowercase();
                match first.as_str() {
                    ".extern" | "extern" | ".extrn" | "extrn" => {
                        for name in
                            l.split_whitespace().skip(1).flat_map(|x| x.split(','))
                        {
                            push(
                                &mut out,
                                ExternalUseKind::Import,
                                name.split(':').next().unwrap_or(name),
                                "",
                                "extern",
                            );
                        }
                    }
                    ".include" | "%include" | "include" => {
                        if let Some(q) = quoted(l) {
                            push(&mut out, ExternalUseKind::Import, q, "", "include");
                        }
                    }
                    m if cgg_core::stdlib::SYSCALL_MNEMONICS.contains(&m) => {
                        let operand = w.next().unwrap_or("");
                        // `int` only enters the kernel through its vector.
                        if m != "int"
                            || matches!(
                                operand,
                                "0x80"
                                    | "$0x80"
                                    | "0x2e"
                                    | "$0x2e"
                                    | "0x21"
                                    | "21h"
                                    | "80h"
                                    | "$128"
                            )
                        {
                            let sym = if operand.is_empty() {
                                m.to_string()
                            } else {
                                format!("{m} {operand}")
                            };
                            push(&mut out, ExternalUseKind::Syscall, "syscall", &sym, m);
                        }
                    }
                    _ => {}
                }
            }
            "smithy" => {
                if let Some(rest) = line.strip_prefix("use ") {
                    push(&mut out, ExternalUseKind::Import, rest, "", "use");
                }
            }
            "proto" => {
                if let Some(rest) = line.strip_prefix("import ") {
                    if let Some(q) = quoted(rest) {
                        push(&mut out, ExternalUseKind::Import, q, "", "import");
                    }
                }
            }
            "openapi" | "asyncapi" => {
                let l = line.trim_start_matches(['-', ' ', '{']);
                for key in ["$ref:", "\"$ref\":", "'$ref':"] {
                    if let Some(rest) = l.find(key).map(|i| &l[i + key.len()..]) {
                        let v = rest
                            .trim()
                            .trim_end_matches([',', '}'])
                            .trim()
                            .trim_matches(['"', '\'']);
                        if !v.starts_with('#') && !v.is_empty() {
                            let target = v.split('#').next().unwrap_or(v);
                            push(
                                &mut out,
                                ExternalUseKind::Import,
                                target,
                                v.split('#').nth(1).unwrap_or(""),
                                "$ref",
                            );
                        }
                    }
                }
            }
            "hcl" => {
                if line.starts_with("module ") && line.ends_with('{') {
                    in_module_block = true;
                } else if line.starts_with("required_providers") {
                    in_providers = true;
                } else if let Some(rest) = line.strip_prefix("provider ") {
                    if let Some(q) = quoted(rest) {
                        push(
                            &mut out,
                            ExternalUseKind::Provider,
                            q,
                            "provider",
                            "provider",
                        );
                    }
                } else if (in_module_block || in_providers) && line.starts_with("source")
                {
                    if let Some(q) = quoted(line) {
                        let form = if in_providers {
                            "required-provider"
                        } else {
                            "module-source"
                        };
                        push(
                            &mut out,
                            ExternalUseKind::Provider,
                            q,
                            if in_providers { "provider" } else { "module" },
                            form,
                        );
                    }
                }
                if line == "}" {
                    in_module_block = false;
                    if in_providers && raw.starts_with('}') {
                        in_providers = false;
                    }
                }
            }
            "nix" => {
                for f in [
                    "fetchFromGitHub",
                    "fetchFromGitLab",
                    "fetchgit",
                    "fetchGit",
                    "fetchurl",
                    "fetchTarball",
                    "fetchzip",
                    "fetchFromSourcehut",
                ] {
                    if line.contains(f)
                        && (line.contains(&format!("{f} {{"))
                            || line.contains(&format!("{f} \"")))
                    {
                        fetcher = Some((f.to_string(), line_no, at));
                        owner.clear();
                        let after = line.split(f).nth(1).unwrap_or("").trim_start();
                        if after.starts_with('"')
                            && let Some(q) = quoted(after)
                        {
                            push(&mut out, ExternalUseKind::Provider, q, f, f);
                            fetcher = None;
                        }
                    }
                }
                if let Some((f, l0, b0)) = fetcher.clone() {
                    let value = |part: &str, key: &str| -> Option<String> {
                        let (_, rest) = part.split_once(key)?;
                        let rest = rest.trim_start().strip_prefix('=')?;
                        Some(rest.trim().trim_matches(['"', '}', ' ']).to_string())
                    };
                    for part in line.split(';') {
                        if let Some(v) = value(part, "owner") {
                            owner = v;
                        }
                        if let Some(v) = value(part, "repo") {
                            let host = if f.contains("GitLab") {
                                "gitlab"
                            } else {
                                "github"
                            };
                            let m = format!("{host}:{owner}/{v}");
                            out.push(use_at(
                                ExternalUseKind::Provider,
                                &m,
                                &f,
                                &f,
                                l0,
                                b0,
                            ));
                            fetcher = None;
                        }
                        if let Some(v) = value(part, "url") {
                            out.push(use_at(
                                ExternalUseKind::Provider,
                                &v,
                                &f,
                                &f,
                                l0,
                                b0,
                            ));
                            fetcher = None;
                        }
                    }
                }
            }
            "haskell" => {
                if let Some(rest) = line.strip_prefix("foreign import") {
                    let lib = quoted(rest)
                        .map(|q| q.split_whitespace().next().unwrap_or(q).to_string())
                        .unwrap_or_else(|| "<ccall>".into());
                    let name = rest
                        .split("::")
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .last()
                        .unwrap_or("");
                    push(
                        &mut out,
                        ExternalUseKind::ForeignDecl,
                        &lib,
                        name,
                        "foreign-import",
                    );
                }
            }
            "ocaml" => {
                if let Some(rest) = line.strip_prefix("external ") {
                    let name = rest.split(':').next().unwrap_or("").trim();
                    let sym = rest.rsplit('=').next().and_then(quoted).unwrap_or(name);
                    push(
                        &mut out,
                        ExternalUseKind::ForeignDecl,
                        "<c-stubs>",
                        sym,
                        "external",
                    );
                }
            }
            "zig" => {
                if let Some(i) = line.find("extern \"") {
                    let rest = &line[i + 7..];
                    if let Some(lib) = quoted(rest) {
                        let name = rest
                            .split("fn ")
                            .nth(1)
                            .and_then(|r| r.split('(').next())
                            .unwrap_or("")
                            .trim();
                        if !name.is_empty() {
                            push(
                                &mut out,
                                ExternalUseKind::ForeignDecl,
                                lib,
                                name,
                                "extern",
                            );
                        }
                    }
                } else if line.contains("@cImport") || line.contains("@cInclude") {
                    if let Some(q) = line.split("@cInclude").nth(1).and_then(quoted) {
                        push(&mut out, ExternalUseKind::Import, q, "", "cInclude");
                    }
                }
            }
            "lean" => {
                if let Some(i) = line.find("@[extern") {
                    if let Some(q) = quoted(&line[i..]) {
                        let name = line
                            .split("def ")
                            .nth(1)
                            .or_else(|| line.split("opaque ").nth(1))
                            .and_then(|r| r.split_whitespace().next())
                            .unwrap_or("");
                        push(
                            &mut out,
                            ExternalUseKind::ForeignDecl,
                            "<c-ffi>",
                            if name.is_empty() { q } else { name },
                            "extern",
                        );
                    }
                }
            }
            "fortran" => {
                let l = line.to_ascii_lowercase();
                if let Some(i) = l.find("bind(c") {
                    let name =
                        quoted(&line[i..]).map(str::to_string).unwrap_or_else(|| {
                            l.split(['(', ' '])
                                .skip_while(|w| *w != "function" && *w != "subroutine")
                                .nth(1)
                                .unwrap_or("")
                                .to_string()
                        });
                    if !name.is_empty() && !name.starts_with("bind") {
                        push(
                            &mut out,
                            ExternalUseKind::ForeignDecl,
                            "<c-abi>",
                            &name,
                            "bind(c)",
                        );
                    }
                }
            }
            "csharp" | "fsharp" | "powershell" => {
                for attr in ["DllImport(", "LibraryImport("] {
                    if let Some(i) = line.find(attr)
                        && let Some(q) = quoted(&line[i..])
                    {
                        let sym = line
                            .split("static extern")
                            .nth(1)
                            .or_else(|| line.split("extern ").nth(1))
                            .and_then(|r| r.split('(').next())
                            .and_then(|r| r.split_whitespace().last())
                            .unwrap_or("");
                        let mut u = use_at(
                            ExternalUseKind::ForeignDecl,
                            q,
                            sym,
                            attr.trim_end_matches('('),
                            line_no,
                            at,
                        );
                        if sym.is_empty() {
                            // The declaration is on a following line.
                            u.symbol = String::new();
                        }
                        out.push(u);
                    }
                }
            }
            _ => {}
        }
    }
    // C#/F#: `[DllImport]` on its own line names the next `extern` method.
    if matches!(language, "csharp" | "fsharp" | "powershell") {
        let lines: Vec<&str> = text.lines().collect();
        for u in out
            .iter_mut()
            .filter(|u| u.kind == ExternalUseKind::ForeignDecl && u.symbol.is_empty())
        {
            let start = u.site_line as usize;
            if let Some(decl) = lines
                .iter()
                .skip(start)
                .take(4)
                .find(|l| l.contains("extern "))
            {
                u.symbol = decl
                    .split('(')
                    .next()
                    .and_then(|r| r.split_whitespace().last())
                    .unwrap_or("")
                    .to_string();
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callee_suffixes_match_on_segment_boundaries() {
        assert!(callee_matches("subprocess.run", "subprocess.run"));
        assert!(callee_matches("Command::new", "std::process::Command::new"));
        assert!(callee_matches("os:cmd", "os:cmd"));
        assert!(!callee_matches("exec", "myexec"));
        assert!(callee_matches("exec", "child_process.exec"));
    }

    fn uses(lang: &str, src: &str) -> Vec<ExternalUse> {
        let reg = crate::PluginRegistry::with_v1_plugins();
        let p = reg.all().iter().find(|p| p.id() == lang).unwrap();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&p.ts_language()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        extract(lang, &tree, src.as_bytes())
    }

    #[test]
    fn process_literals_are_read_in_every_call_shape() {
        let prog = |lang: &str, src: &str| -> Vec<String> {
            uses(lang, src)
                .into_iter()
                .filter(|u| u.kind == ExternalUseKind::ProcessSpawn)
                .map(|u| u.module)
                .collect()
        };
        assert_eq!(
            prog("erlang", "-module(a).\nrun() -> os:cmd(\"ls -la\").\n"),
            ["ls -la"]
        );
        assert_eq!(
            prog(
                "elixir",
                "defmodule A do\n def r, do: System.cmd(\"git\", [\"s\"])\nend\n"
            ),
            ["git"]
        );
        assert_eq!(
            prog(
                "java",
                "class A { void r() { Runtime.getRuntime().exec(\"git status\"); } }"
            ),
            ["git status"]
        );
        assert_eq!(
            prog("python", "import subprocess\nsubprocess.run([cmd, '-t'])\n"),
            [""]
        );
    }

    #[test]
    fn literals_lose_quotes_and_interpolation() {
        assert_eq!(literal_text("\"git\""), "git");
        assert_eq!(literal_text("r\"libz.so\""), "libz.so");
        assert_eq!(literal_text("`ls ${dir}`"), "ls");
        assert_eq!(literal_text("@\"user32.dll\""), "user32.dll");
    }
}
