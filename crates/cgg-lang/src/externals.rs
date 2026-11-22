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
fn callee_matches(entry: &str, callee: &str) -> bool {
    let e = unify(entry);
    let c = unify(callee);
    c == e || (c.ends_with(&e) && c[..c.len() - e.len()].ends_with('.'))
}

/// The callee of a call node, as written.
fn callee<'t>(n: Node<'t>) -> Option<Node<'t>> {
    for f in ["function", "name", "method", "macro", "callee", "constructor", "type"] {
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
        if matches!(k, "argument" | "value_argument" | "arguments" | "argument_list" | "expression_statement") {
            n.named_child(0).unwrap_or(n)
        } else {
            n
        }
    }
    let first = unwrap(unwrap(first));
    let k = first.kind();
    let is_str = |k: &str| (k.contains("string") || k == "interpreted_string_literal") && !k.contains("content");
    if is_str(k) {
        return first_string(first, src, 0);
    }
    if k.contains("list") || k.contains("array") || k.contains("tuple") || k.contains("initializer") || k.contains("collection") {
        let mut c = first.walk();
        let el = first.named_children(&mut c).find(|n| !n.kind().contains("comment"))?;
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
        "struct_item" | "enum_item" | "union_item" | "type_item" | "trait_item"
            | "class_declaration" | "interface_declaration" | "enum_declaration"
            | "record_declaration" | "struct_declaration" | "protocol_declaration"
            | "object_declaration" | "class_definition" | "type_alias_declaration"
            | "type_definition" | "class_specifier" | "struct_specifier" | "enum_specifier"
            | "union_specifier" | "type_alias" | "typealias_declaration" | "class"
            | "module" | "trait_definition" | "object_definition" | "enum_definition"
    )
}

/// The first string literal inside `n`, depth-limited.
fn first_string(n: Node, src: &[u8], depth: usize) -> Option<String> {
    if depth > 5 {
        return None;
    }
    let k = n.kind();
    if (k.contains("string") || k.contains("char_literal") || k == "interpreted_string_literal")
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
    let mut words = t.split_whitespace();
    while let Some(w) = words.next() {
        if w == "COMMAND" {
            return words.next().map(|w| literal_text(w.trim_end_matches(')')));
        }
    }
    None
}

/// Extract process-spawn and library-load uses.
pub fn extract(language: &str, tree: &Tree, src: &[u8]) -> Vec<ExternalUse> {
    let spawn = externals_table(PROCESS_SPAWN_APIS, language);
    let load = externals_table(LIBRARY_LOAD_APIS, language);
    let shell_out = matches!(language, "ruby" | "perl" | "php" | "julia");
    let text = |n: Node| std::str::from_utf8(&src[n.byte_range()]).unwrap_or("");
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];
    while let Some(n) = stack.pop() {
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
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
        if declares_type(k)
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
        if !is_call(k) {
            continue;
        }
        let Some(f) = callee(n) else { continue };
        let ftext = text(f);
        if ftext.len() > 200 {
            continue;
        }
        let (kind, entry) = if let Some(e) = spawn.iter().find(|e| callee_matches(e, ftext)) {
            (ExternalUseKind::ProcessSpawn, *e)
        } else if let Some(e) = load.iter().find(|e| callee_matches(e, ftext)) {
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
            let rest: Vec<Node> = n.named_children(&mut c).filter(|ch| ch.id() != f.id()).collect();
            rest.first().and_then(|a| {
                let k = a.kind();
                ((k.contains("string") || k.contains("word")) && !k.contains("content"))
                    .then(|| first_string(*a, src, 0).unwrap_or_else(|| literal_text(text(*a))))
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
    out.sort_by_key(|u| (u.site_byte, u.kind as u8));
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

    #[test]
    fn literals_lose_quotes_and_interpolation() {
        assert_eq!(literal_text("\"git\""), "git");
        assert_eq!(literal_text("r\"libz.so\""), "libz.so");
        assert_eq!(literal_text("`ls ${dir}`"), "ls");
        assert_eq!(literal_text("@\"user32.dll\""), "user32.dll");
    }
}
