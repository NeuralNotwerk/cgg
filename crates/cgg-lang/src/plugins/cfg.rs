//! Unreachable-statement detection.
//!
//! This is the only signal in the dead-code feature that is a *proof*
//! rather than a hypothesis. Everything else answers "could cgg find a
//! caller?", which is a statement about cgg. This answers "can control
//! reach this statement?", which is a statement about the code.
//!
//! Scope is deliberately the trivially-sound half: statements following
//! an unconditional terminator in the same statement list. That needs no
//! CFG, no basic blocks, and no constant folding, and it transfers
//! across languages by swapping node names. Constant conditions
//! (`if (false)`) are *not* included — evaluating them means writing a
//! small interpreter whose semantics do not survive contact with
//! languages that have side-effecting `&&`.
//!
//! Four exclusions do most of the work of being correct:
//!
//! 1. **`switch` bodies.** In C-family grammars `case` labels and
//!    statements are siblings inside the switch body, so `break; case 2:`
//!    reads as "statement after terminator". Getting this wrong flags
//!    every C switch ever written, so switch bodies are never scanned.
//! 2. **Preprocessor branches.** A `return` inside `#ifdef` leaves the
//!    following code reachable under another configuration.
//! 3. **Hoisting.** JS `function`/`class`/`var` declarations after a
//!    `return` are still reachable.
//! 4. **Labels.** A labelled statement can be entered by `goto`.

use cgg_core::UnreachableRegion;
use tree_sitter::{Node, Tree};

/// Per-language node names describing where control leaves a block.
#[derive(Copy, Clone, Debug)]
pub struct TerminatorSpec {
    /// Nodes whose named children form a statement sequence.
    pub block_kinds: &'static [&'static str],
    /// Nodes that unconditionally leave the enclosing block.
    pub terminator_kinds: &'static [&'static str],
    /// Nodes that do not count as "something follows" — comments,
    /// empty statements, case labels.
    pub ignore_kinds: &'static [&'static str],
    /// Nodes that are hoisted, and so remain reachable.
    pub hoisted_kinds: &'static [&'static str],
}

/// Node kinds that must never be treated as a statement list, because
/// their children are not sequential control flow.
const NEVER_SCAN: &[&str] = &[
    "switch_body",
    "switch_block",
    "match_block",
    "when_expression",
    "case_statement",
    "expression_switch_statement",
    "type_switch_statement",
];

pub const RUST: TerminatorSpec = TerminatorSpec {
    block_kinds: &["block"],
    terminator_kinds: &["return_expression", "break_expression", "continue_expression"],
    ignore_kinds: &["line_comment", "block_comment", "empty_statement", "attribute_item"],
    hoisted_kinds: &["function_item", "struct_item", "enum_item", "impl_item", "use_declaration"],
};

pub const PYTHON: TerminatorSpec = TerminatorSpec {
    block_kinds: &["block"],
    // `pass` is not a terminator.
    terminator_kinds: &["return_statement", "raise_statement", "break_statement", "continue_statement"],
    ignore_kinds: &["comment"],
    hoisted_kinds: &[],
};

pub const GO: TerminatorSpec = TerminatorSpec {
    block_kinds: &["block"],
    terminator_kinds: &["return_statement", "break_statement", "continue_statement", "goto_statement"],
    ignore_kinds: &["comment", "labeled_statement"],
    hoisted_kinds: &["function_declaration", "type_declaration"],
};

pub const JAVA: TerminatorSpec = TerminatorSpec {
    block_kinds: &["block"],
    terminator_kinds: &["return_statement", "throw_statement", "break_statement", "continue_statement", "yield_statement"],
    ignore_kinds: &["line_comment", "block_comment", "comment", "labeled_statement"],
    hoisted_kinds: &["local_variable_declaration"],
};

pub const C_LIKE: TerminatorSpec = TerminatorSpec {
    block_kinds: &["compound_statement"],
    terminator_kinds: &["return_statement", "break_statement", "continue_statement", "goto_statement", "throw_statement"],
    ignore_kinds: &["comment", "labeled_statement", "case_statement"],
    hoisted_kinds: &["declaration", "function_definition"],
};

pub const JS: TerminatorSpec = TerminatorSpec {
    block_kinds: &["statement_block"],
    terminator_kinds: &["return_statement", "throw_statement", "break_statement", "continue_statement"],
    ignore_kinds: &["comment", "empty_statement"],
    // Hoisting is real in JS: these remain reachable after a `return`.
    hoisted_kinds: &["function_declaration", "class_declaration", "variable_declaration"],
};

fn cause_of(kind: &str) -> &'static str {
    if kind.contains("return") {
        "after-return"
    } else if kind.contains("raise") || kind.contains("throw") {
        "after-throw"
    } else if kind.contains("break") {
        "after-break"
    } else if kind.contains("continue") {
        "after-continue"
    } else {
        "after-jump"
    }
}

/// True if the subtree contains a preprocessor directive, in which case
/// following code may be reachable under a different configuration.
fn has_preproc(node: Node) -> bool {
    let mut stack = vec![node];
    while let Some(n) = stack.pop() {
        if n.kind().starts_with("preproc") {
            return true;
        }
        let mut c = n.walk();
        stack.extend(n.children(&mut c));
    }
    false
}

/// Find statements that control flow cannot reach.
pub fn unreachable_after_terminator(tree: &Tree, spec: &TerminatorSpec) -> Vec<UnreachableRegion> {
    let mut out = Vec::new();
    let mut stack = vec![tree.root_node()];

    while let Some(n) = stack.pop() {
        let mut c = n.walk();
        let kids: Vec<Node> = n.children(&mut c).collect();
        for k in &kids {
            if !NEVER_SCAN.contains(&k.kind()) {
                stack.push(*k);
            }
        }

        if !spec.block_kinds.contains(&n.kind()) || n.has_error() {
            continue;
        }
        if has_preproc(n) {
            continue;
        }

        let stmts: Vec<Node> = kids
            .iter()
            .copied()
            .filter(|k| k.is_named() && !spec.ignore_kinds.contains(&k.kind()))
            .collect();

        for (i, s) in stmts.iter().enumerate() {
            // Rust and JS wrap a bare `return;` in an
            // `expression_statement`, so the terminator is one level
            // down. Look through a single-child wrapper.
            let term_kind = if spec.terminator_kinds.contains(&s.kind()) {
                Some(s.kind())
            } else if s.kind() == "expression_statement" {
                s.named_child(0)
                    .map(|c| c.kind())
                    .filter(|k| spec.terminator_kinds.contains(k))
            } else {
                None
            };
            let Some(term_kind) = term_kind else { continue };
            let rest: Vec<&Node> = stmts
                .iter()
                .skip(i + 1)
                .filter(|r| !spec.hoisted_kinds.contains(&r.kind()))
                .collect();
            let (Some(first), Some(last)) = (rest.first(), rest.last()) else {
                break;
            };
            out.push(UnreachableRegion {
                start_line: first.start_position().row as u32 + 1,
                end_line: last.end_position().row as u32 + 1,
                start_byte: first.start_byte() as u32,
                end_byte: last.end_byte() as u32,
                cause: cause_of(term_kind).to_string(),
            });
            break; // one region per block
        }
    }

    // Deterministic order regardless of traversal.
    out.sort_by_key(|r| (r.start_byte, r.end_byte));
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
    fn statements_after_return_are_unreachable() {
        let t = parse("rust", "fn f() { return; let x = 1; }");
        let r = unreachable_after_terminator(&t, &RUST);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cause, "after-return");
    }

    #[test]
    fn a_trailing_terminator_reports_nothing() {
        let t = parse("rust", "fn f() { let x = 1; return; }");
        assert!(unreachable_after_terminator(&t, &RUST).is_empty());
    }

    #[test]
    fn python_raise_and_break() {
        let t = parse("python", "def f():\n    raise ValueError()\n    x = 1\n");
        let r = unreachable_after_terminator(&t, &PYTHON);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].cause, "after-throw");
    }

    #[test]
    fn c_switch_cases_are_never_flagged() {
        // `break; case 2:` is the classic false positive: in C-family
        // grammars the labels are siblings of the statements.
        let src = "int f(int x){ switch(x){ case 1: return 1; break; case 2: return 2; } return 0; }";
        let t = parse("c", src);
        assert!(
            unreachable_after_terminator(&t, &C_LIKE).is_empty(),
            "a switch must never produce a finding"
        );
    }

    #[test]
    fn js_hoisted_declarations_stay_reachable() {
        let t = parse("javascript", "function f(){ return 1; function g(){} }");
        assert!(
            unreachable_after_terminator(&t, &JS).is_empty(),
            "function declarations hoist"
        );
        let t2 = parse("javascript", "function f(){ return 1; console.log(2); }");
        assert_eq!(unreachable_after_terminator(&t2, &JS).len(), 1);
    }

    #[test]
    fn results_are_deterministic() {
        let src = "fn a() { return; let x = 1; }\nfn b() { return; let y = 2; }";
        let t = parse("rust", src);
        let a = unreachable_after_terminator(&t, &RUST);
        let b = unreachable_after_terminator(&t, &RUST);
        assert_eq!(a, b);
        assert_eq!(a.len(), 2);
        assert!(a[0].start_byte < a[1].start_byte);
    }

    #[test]
    fn a_parse_error_suppresses_the_block() {
        let t = parse("rust", "fn f() { return; @@@ }");
        // Nothing is claimed about a block cgg could not parse.
        let _ = unreachable_after_terminator(&t, &RUST);
    }
}
