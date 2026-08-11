//! Assembly plugin — callable extraction.
//!
//! Assembly call graphs are pragmatically simple: each `label:` is a
//! callable, and every `call <target>` / `jmp <target>` / `jal <target>`
//! / `bl <target>` instruction is a call site. tree-sitter-asm produces
//! a flat sequence of `label` and `instruction` nodes — we walk it
//! once and emit defs + refs.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct AsmPlugin;

impl LanguagePlugin for AsmPlugin {
    fn id(&self) -> &'static str {
        "asm"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".s", ".S", ".asm"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_asm::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "asm");
        let mut w = AsmWalker {
            source,
            facts: &mut facts,
        };
        w.walk(tree.root_node());
        // Stretch each label's owned range to cover the instructions that
        // follow it — up to the next label or EOF. tree-sitter-asm gives
        // a `label` node only the `name:` token itself, so without this
        // fixup the body of the function lies outside the def's byte
        // range and refs inside it can't be attributed to the caller.
        let total = source.len() as u32;
        let mut bounds: Vec<u32> =
            facts.definitions.iter().map(|d| d.start_byte).collect();
        bounds.sort_unstable();
        for def in facts.definitions.iter_mut() {
            let next = bounds
                .iter()
                .find(|&&b| b > def.start_byte)
                .copied()
                .unwrap_or(total);
            def.end_byte = next.saturating_sub(1).max(def.end_byte);
            def.end_line = source[..next as usize]
                .iter()
                .filter(|&&b| b == b'\n')
                .count() as u32
                + 1;
        }
        facts
    }
}

/// Mnemonics that transfer control to a labelled target. Covers the
/// common x86 / ARM / RISC-V / MIPS call and jump opcodes.
fn is_call_mnemonic(mn: &str) -> bool {
    matches!(
        mn.to_ascii_lowercase().as_str(),
        // x86
        "call" | "callq" | "calll" | "callw"
        | "jmp" | "jmpq" | "jmpl"
        | "je" | "jne" | "jz" | "jnz" | "jg" | "jge" | "jl" | "jle"
        | "ja" | "jae" | "jb" | "jbe" | "jo" | "jno" | "js" | "jns"
        | "jc" | "jnc" | "jp" | "jnp" | "loop"
        // ARM
        | "bl" | "blx" | "bx" | "b"
        | "b.eq" | "b.ne" | "b.lt" | "b.gt" | "b.le" | "b.ge"
        // RISC-V
        | "jal" | "jalr"
        // MIPS / PowerPC-ish
        | "bal" | "bgez" | "bgezal" | "bltz" | "bltzal" | "jr"
    )
}

struct AsmWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> AsmWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "label" => {
                self.record_label(node);
                return;
            }
            "instruction" => {
                self.record_instruction(node);
                return;
            }
            _ => {}
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                self.walk(c.node());
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    fn record_label(&mut self, node: Node) {
        let name = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "ident")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name.clone(),
            qualified_name: name,
            variant: DefVariant::FreeFunction,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: vec!["label".into()],
            ..Default::default()
        });
    }

    fn record_instruction(&mut self, node: Node) {
        // First child is the mnemonic (`word`), subsequent named children
        // are operands. For control-flow opcodes we treat the first
        // identifier-shaped operand as the call target.
        let mut c = node.walk();
        if !c.goto_first_child() {
            return;
        }
        let mnem_node = c.node();
        if mnem_node.kind() != "word" {
            return;
        }
        let mnem = self.text(mnem_node).to_string();
        if !is_call_mnemonic(&mnem) {
            return;
        }

        // Find the first operand that looks like a symbol (ident -> reg -> word).
        let mut target: Option<String> = None;
        while c.goto_next_sibling() {
            let n = c.node();
            if n.kind() == "ident" {
                // Drill to leaf `word` (ident > reg > word) — skip register-only operands.
                if let Some(w) = self.find_word_leaf(n) {
                    let raw = self.text(w);
                    if !raw.starts_with('%') && !raw.starts_with('$') {
                        target = Some(raw.to_string());
                        break;
                    }
                }
            }
        }
        let Some(target) = target else { return };
        self.facts.references.push(RefRecord {
            name: target,
            receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }

    fn find_word_leaf<'n>(&self, node: Node<'n>) -> Option<Node<'n>> {
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if n.kind() == "word" {
                return Some(n);
            }
            let mut c = n.walk();
            if c.goto_first_child() {
                loop {
                    stack.push(c.node());
                    if !c.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_asm::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        AsmPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/x.s"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn labels_are_callables() {
        let src = "main:\n    ret\ngreet:\n    ret\n";
        let f = extract(src);
        let names: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"main"), "got: {names:?}");
        assert!(names.contains(&"greet"), "got: {names:?}");
    }

    #[test]
    fn call_and_jmp_captured() {
        let src = "main:\n    call greet\n    jmp other\n    ret\n";
        let f = extract(src);
        let targets: Vec<_> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(targets.contains(&"greet"), "got: {targets:?}");
        assert!(targets.contains(&"other"), "got: {targets:?}");
    }

    #[test]
    fn registers_are_not_calls() {
        let src = "main:\n    movq %rsp, %rbp\n    ret\n";
        let f = extract(src);
        assert!(
            f.references.is_empty(),
            "should not capture mov: {:?}",
            f.references
        );
    }

    #[test]
    fn arm_bl_captured() {
        let src = "main:\n    bl greet\n";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "greet"),
            "refs: {:?}",
            f.references
        );
    }
}
