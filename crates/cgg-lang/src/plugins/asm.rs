//! Assembly plugin — callable extraction.
//!
//! Assembly call graphs are pragmatically simple: each code `label:` is
//! a callable, and every call, jump or branch to a symbol (`call`, `jmp`,
//! `jcc`, `bl`, `b.cond`, `cbz`, RISC-V `j`/`tail`/`beq`…, `jal`) is a
//! call site. Local labels (`1:`, `.L*`) are branch targets inside a
//! function and data labels (in a data section, or in front of `.word`
//! and kin) are data, so neither is a callable. Under reference sites a
//! symbol whose address is taken (`la`/`lla`/`adr`/`lea`, `ldr =sym`,
//! AT&T `$sym`), laid down in a table (`.word`/`.quad`/`dd`…) or aliased
//! (`.set a, sym`) is a value reference. `.globl`/`.weak`/`global`/
//! `PUBLIC` set visibility. tree-sitter-asm produces a flat sequence of
//! `label`, `instruction` and `meta` nodes — we walk it once.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, RefRecord, Vis, ids::FileId};
use std::collections::HashMap;
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
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "asm");
        let mut w = AsmWalker {
            source,
            facts: &mut facts,
            reference_sites: ctx.reference_sites,
            bounds: Vec::new(),
            globals: HashMap::new(),
            data_section: false,
        };
        w.walk(tree.root_node());
        let AsmWalker {
            mut bounds,
            globals,
            ..
        } = w;
        // Stretch each label's owned range to cover the instructions that
        // follow it — up to the next code or data label, section switch or
        // EOF. tree-sitter-asm gives a `label` node only the `name:` token
        // itself, so without this fixup the body of the function lies
        // outside the def's byte range and refs inside it can't be
        // attributed to the caller. Local labels (`1:`, `.L1:`) are not
        // boundaries: they sit inside the function.
        let total = source.len() as u32;
        bounds.extend(facts.definitions.iter().map(|d| d.start_byte));
        bounds.sort_unstable();
        bounds.dedup();
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
        // `.globl`/`.global`/`.weak` (GAS), `global` (NASM) and `PUBLIC`
        // (MASM) export a symbol to the linker. In a file that declares
        // any, a label it does not name is local to the object file.
        if !globals.is_empty() {
            for def in facts.definitions.iter_mut() {
                match globals.get(def.simple_name.as_str()) {
                    Some(token) => {
                        def.visibility = token.clone();
                        def.vis = Vis::Public;
                    }
                    None => {
                        def.visibility = "local".into();
                        def.vis = Vis::Private;
                    }
                }
            }
        }
        facts
    }
}

/// Mnemonics that transfer control to a labelled target: calls, jumps
/// and conditional branches, across x86, ARM/AArch64, RISC-V and MIPS.
/// A jump or tail call to a label is recorded like a call — it is the
/// only way that label's code runs.
fn is_call_mnemonic(mn: &str) -> bool {
    let mn = mn.to_ascii_lowercase();
    // AArch64 conditional branches (`b.eq`, `b.hs`, ...).
    if let Some(cond) = mn.strip_prefix("b.") {
        return !cond.is_empty() && cond.chars().all(|c| c.is_ascii_alphabetic());
    }
    matches!(
        mn.as_str(),
        // x86
        "call" | "callq" | "calll" | "callw"
        | "jmp" | "jmpq" | "jmpl" | "jmpw"
        | "je" | "jne" | "jz" | "jnz" | "jg" | "jge" | "jl" | "jle"
        | "ja" | "jae" | "jb" | "jbe" | "jo" | "jno" | "js" | "jns"
        | "jc" | "jnc" | "jp" | "jnp" | "jpe" | "jpo" | "jna" | "jnae"
        | "jnb" | "jnbe" | "jng" | "jnge" | "jnl" | "jnle"
        | "jcxz" | "jecxz" | "jrcxz" | "loop" | "loope" | "loopne"
        | "loopz" | "loopnz"
        // ARM / AArch64
        | "bl" | "blx" | "bx" | "b"
        | "beq" | "bne" | "bcs" | "bcc" | "bhs" | "blo" | "bmi" | "bpl"
        | "bvs" | "bvc" | "bhi" | "bls" | "bge" | "blt" | "bgt" | "ble"
        | "cbz" | "cbnz" | "tbz" | "tbnz"
        // RISC-V (`call`/`tail`/`j` are assembler pseudo-instructions)
        | "jal" | "jalr" | "j" | "tail"
        | "bltu" | "bgeu" | "beqz" | "bnez" | "blez" | "bgez" | "bltz"
        | "bgtz" | "bgtu" | "bleu"
        // MIPS / PowerPC-ish
        | "bal" | "bgezal" | "bltzal" | "jr"
    )
}

/// Instructions that load a symbol's address — the function is handed
/// on as a value (`lla a4, _trap_handler` then `csrw mtvec, a4`).
fn is_address_mnemonic(mn: &str) -> bool {
    matches!(
        mn.to_ascii_lowercase().as_str(),
        "la" | "lla" | "adr" | "adrp" | "adrl" | "lea" | "leaq" | "leal" | "leaw"
    )
}

/// Data directives wide enough to hold an address: every symbol operand
/// is a value (vector, syscall and jump tables). GAS spells them with a
/// leading `.`, NASM/MASM without.
fn is_data_directive(d: &str) -> bool {
    matches!(
        d.to_ascii_lowercase().as_str(),
        ".word"
            | ".long"
            | ".quad"
            | ".dword"
            | ".xword"
            | ".int"
            | ".4byte"
            | ".8byte"
            | ".ptr"
            | ".dc.a"
            | ".dc.l"
            | "dd"
            | "dq"
    )
}

/// Directives that lay down data or reserve space: a label in front of
/// one is a datum, not code.
fn is_data_layout_directive(d: &str) -> bool {
    is_data_directive(d)
        || matches!(
            d.to_ascii_lowercase().as_str(),
            ".byte"
                | ".short"
                | ".hword"
                | ".2byte"
                | ".half"
                | ".ascii"
                | ".asciz"
                | ".string"
                | ".space"
                | ".skip"
                | ".zero"
                | ".fill"
                | ".float"
                | ".double"
                | ".octa"
                | ".incbin"
                | ".comm"
                | ".lcomm"
                | "db"
                | "dw"
                | "dt"
                | "resb"
                | "resw"
                | "resd"
                | "resq"
                | "times"
        )
}

/// Directives naming the symbols the object file exports, and the token
/// recorded as their visibility.
fn export_directive(d: &str) -> Option<&'static str> {
    match d.to_ascii_lowercase().as_str() {
        ".globl" | ".global" | "global" | "public" => Some("global"),
        ".weak" | "weak" => Some("weak"),
        _ => None,
    }
}

/// Symbol aliases: `.set a, b` makes `a` another name for `b`.
fn is_alias_directive(d: &str) -> bool {
    matches!(
        d.to_ascii_lowercase().as_str(),
        ".set" | ".equ" | ".equiv" | ".thumb_set" | ".weakref"
    )
}

/// Section switches. `Some(true)` enters a data section.
fn section_switch(d: &str, operands: &str) -> Option<bool> {
    let d = d.to_ascii_lowercase();
    let named = |name: &str| {
        let n = name.trim().trim_matches('"').to_ascii_lowercase();
        n.starts_with(".data")
            || n.starts_with(".bss")
            || n.starts_with(".rodata")
            || n.starts_with(".sdata")
            || n.starts_with(".sbss")
            || n.starts_with(".tdata")
            || n.starts_with(".tbss")
            || n == "data"
            || n == "bss"
            || n == "rodata"
    };
    match d.as_str() {
        ".text" => Some(false),
        ".data" | ".bss" | ".rodata" => Some(true),
        ".section" | "section" | "segment" => {
            let first = operands.split([',', ' ', '\t']).next().unwrap_or("");
            Some(named(first))
        }
        _ => None,
    }
}

/// A label that is local to the assembler: GAS numeric labels (`1:`,
/// referenced as `1f`/`1b`) and `.L` labels, NASM `.`-prefixed labels
/// local to the previous label. They are branch targets inside a
/// function, never functions.
fn is_local_label(name: &str) -> bool {
    name.starts_with('.') || name.starts_with(|c: char| c.is_ascii_digit())
}

/// Register names written without a sigil (RISC-V, ARM/AArch64, Intel
/// syntax x86), which share the operand position with symbols.
fn is_register(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    let numbered = |prefix: &str, max: u32| {
        n.strip_prefix(prefix)
            .and_then(|d| d.parse::<u32>().ok())
            .is_some_and(|v| v <= max)
    };
    if matches!(
        n.as_str(),
        "zero"
            | "ra"
            | "sp"
            | "gp"
            | "tp"
            | "fp"
            | "lr"
            | "pc"
            | "ip"
            | "sb"
            | "sl"
            | "xzr"
            | "wzr"
            | "rip"
            | "eip"
            | "rax"
            | "rbx"
            | "rcx"
            | "rdx"
            | "rsi"
            | "rdi"
            | "rbp"
            | "rsp"
            | "eax"
            | "ebx"
            | "ecx"
            | "edx"
            | "esi"
            | "edi"
            | "ebp"
            | "esp"
            | "ax"
            | "bx"
            | "cx"
            | "dx"
            | "si"
            | "di"
            | "bp"
            | "al"
            | "ah"
            | "bl"
            | "bh"
            | "cl"
            | "ch"
            | "dl"
            | "dh"
            | "sil"
            | "dil"
            | "bpl"
            | "spl"
            | "cs"
            | "ds"
            | "es"
            | "fs"
            | "gs"
            | "ss"
    ) {
        return true;
    }
    numbered("x", 31)
        || numbered("w", 30)
        || numbered("r", 15)
        || numbered("a", 7)
        || numbered("t", 6)
        || numbered("s", 11)
        || numbered("f", 31)
        || numbered("v", 31)
        || numbered("q", 31)
        || numbered("d", 31)
        || numbered("fa", 7)
        || numbered("ft", 11)
        || numbered("fs", 11)
        || numbered("xmm", 31)
        || numbered("ymm", 31)
        || numbered("zmm", 31)
        || numbered("cr", 15)
        || numbered("dr", 7)
        || n.strip_prefix('r')
            .and_then(|r| r.strip_suffix(['d', 'w', 'b']))
            .and_then(|d| d.parse::<u32>().ok())
            .is_some_and(|v| (8..=15).contains(&v))
}

/// The symbol an operand names, if any: `foo`, `foo@PLT`, `=foo` (ARM
/// literal pool), `$foo` (AT&T immediate, only when `immediate`),
/// `foo(%rip)`, `[foo]`, `[rel foo]`, `%lo(foo)`. Registers, numbers,
/// numeric local-label references (`1f`), `.L` labels and `.` are not.
fn operand_symbol(op: &str, immediate: bool) -> Option<&str> {
    let mut s = op.trim();
    s = s.strip_prefix('=').unwrap_or(s).trim_start();
    if let Some(rest) = s.strip_prefix('$') {
        if !immediate {
            return None;
        }
        s = rest;
    }
    // Intel size and distance keywords: `call qword [rel f]`, `jmp near f`.
    loop {
        let lower = s.to_ascii_lowercase();
        let Some(kw) = [
            "qword ", "dword ", "word ", "byte ", "tword ", "ptr ", "near ", "far ",
            "short ",
        ]
        .into_iter()
        .find(|kw| lower.starts_with(kw)) else {
            break;
        };
        s = s[kw.len()..].trim_start();
    }
    if let Some(rest) = s.strip_prefix('[') {
        s = rest.trim_start();
        s = s.strip_prefix("rel ").unwrap_or(s).trim_start();
    }
    // RISC-V relocation operators: `%pcrel_hi(sym)`, `%lo(sym)`.
    if s.starts_with('%') {
        let open = s.find('(')?;
        if !s[1..open]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return None;
        }
        s = &s[open + 1..];
    }
    let end = s
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '$'))
        .unwrap_or(s.len());
    let sym = &s[..end];
    if sym.is_empty()
        || sym == "."
        || sym.starts_with(|c: char| c.is_ascii_digit())
        || sym.starts_with(".L")
        || is_register(sym)
    {
        return None;
    }
    Some(sym)
}

/// Split an operand list on top-level commas (`foo(%rip), %rax`).
fn split_operands(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0i32, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

struct AsmWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    reference_sites: bool,
    /// Byte offsets that end the previous label's range without being a
    /// callable: data labels and section switches.
    bounds: Vec<u32>,
    /// Exported symbol → its directive token (`global`, `weak`).
    globals: HashMap<String, String>,
    /// Inside `.data`/`.bss`/`.rodata` (or a `.section` of those).
    data_section: bool,
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
            "instruction" | "meta" => {
                self.record_statement(node);
                return;
            }
            // The grammar cannot read an AArch64 `b.ne target`: `b.ne` is
            // an ERROR and `target` an instruction of its own, on the
            // same line.
            "ERROR" if self.record_dotted_branch(node) => return,
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

    /// `b.ne target` read as an ERROR (`b.ne`) and a one-word
    /// instruction (`target`) on the same line: record the branch.
    fn record_dotted_branch(&mut self, node: Node) -> bool {
        let text = self.text(node);
        if !(text.starts_with("b.") && is_call_mnemonic(text)) {
            return false;
        }
        let Some(next) = node.next_sibling() else {
            return false;
        };
        if next.kind() != "instruction"
            || next.start_position().row != node.end_position().row
            || next.named_child_count() != 1
        {
            return false;
        }
        let Some(sym) = operand_symbol(self.text(next), false).map(str::to_string) else {
            return false;
        };
        self.push_ref(node, &sym, false);
        true
    }

    /// The statement after `node`, skipping comments and further labels.
    fn next_statement<'n>(&self, node: Node<'n>) -> Option<Node<'n>> {
        let mut n = node.next_named_sibling();
        while let Some(s) = n {
            if s.kind() == "instruction" || s.kind() == "meta" {
                return Some(s);
            }
            n = s.next_named_sibling();
        }
        None
    }

    fn record_label(&mut self, node: Node) {
        let name = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "ident")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() || is_local_label(&name) {
            return;
        }
        // A label in a data section, or one laying down data, names a
        // datum: not a callable, but it ends the code label before it.
        let is_data = self.data_section
            || self
                .next_statement(node)
                .and_then(|s| s.child(0))
                .is_some_and(|k| is_data_layout_directive(self.text(k)));
        if is_data {
            self.bounds.push(node.start_byte() as u32);
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

    /// The mnemonic or directive of an `instruction`/`meta` node and the
    /// source text of its operands (comments excluded).
    fn split_statement<'n>(&self, node: Node<'n>) -> Option<(String, String)> {
        let head = node.child(0)?;
        if !matches!(head.kind(), "word" | "meta_ident") {
            return None;
        }
        let mut c = node.walk();
        let ops: Vec<Node> = node
            .children(&mut c)
            .skip(1)
            .filter(|n| !n.kind().ends_with("comment"))
            .collect();
        let text = match (ops.first(), ops.last()) {
            (Some(a), Some(b)) => self
                .source
                .get(a.start_byte()..b.end_byte())
                .and_then(|b| std::str::from_utf8(b).ok())
                .unwrap_or("")
                .to_string(),
            _ => String::new(),
        };
        Some((self.text(head).to_string(), text))
    }

    fn push_ref(&mut self, node: Node, name: &str, value: bool) {
        self.facts.references.push(RefRecord {
            name: name.to_string(),
            receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            value_ref: value,
            ..Default::default()
        });
    }

    fn record_statement(&mut self, node: Node) {
        let Some((head, ops)) = self.split_statement(node) else {
            return;
        };
        if let Some(data) = section_switch(&head, &ops) {
            self.data_section = data;
            self.bounds.push(node.start_byte() as u32);
            return;
        }
        if let Some(token) = export_directive(&head) {
            for op in split_operands(&ops) {
                // NASM `global f:function`.
                let op = op.split(':').next().unwrap_or(op);
                if let Some(sym) = operand_symbol(op, false) {
                    self.globals
                        .entry(sym.to_string())
                        .or_insert_with(|| token.to_string());
                }
            }
            return;
        }
        if is_call_mnemonic(&head) {
            // The target is the last symbol operand: RISC-V and ARM write
            // the compared registers first (`beq a0, a1, target`,
            // `jal ra, target`).
            let target = split_operands(&ops)
                .into_iter()
                .rev()
                .find_map(|op| {
                    let op = op.trim();
                    // `call *%rax` / `jmp *table(,%rax,8)` is indirect.
                    if op.starts_with('*') || op.starts_with('%') {
                        return None;
                    }
                    operand_symbol(op.split('@').next().unwrap_or(op), false)
                })
                .map(str::to_string);
            if let Some(t) = target {
                self.push_ref(node, &t, false);
            }
            return;
        }
        if !self.reference_sites {
            return;
        }
        let operands = split_operands(&ops);
        let mut values: Vec<String> = Vec::new();
        if is_address_mnemonic(&head) || is_data_directive(&head) {
            for op in &operands {
                if let Some(sym) =
                    operand_symbol(op.split('@').next().unwrap_or(op), false)
                {
                    values.push(sym.to_string());
                }
            }
        } else if is_alias_directive(&head) {
            if let Some(sym) = operands.get(1).and_then(|op| operand_symbol(op, false)) {
                values.push(sym.to_string());
            }
        } else if node.kind() == "instruction" {
            // `ldr x0, =sym` (literal pool) and AT&T `mov $sym, %rax`.
            for op in &operands {
                let op = op.trim();
                if op.starts_with('=') || op.starts_with('$') {
                    if let Some(sym) = operand_symbol(op, true) {
                        values.push(sym.to_string());
                    }
                }
            }
        }
        for v in values {
            self.push_ref(node, &v, true);
        }
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

    fn extract_refs(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_asm::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        AsmPlugin.extract(
            &crate::ExtractCtx::plain().with_reference_sites(true),
            FileId::new(0),
            &PathBuf::from("/tmp/x.S"),
            &tree,
            src.as_bytes(),
        )
    }

    fn names(f: &FileFacts) -> Vec<&str> {
        f.definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect()
    }

    fn calls(f: &FileFacts) -> Vec<&str> {
        f.references
            .iter()
            .filter(|r| !r.value_ref)
            .map(|r| r.name.as_str())
            .collect()
    }

    fn values(f: &FileFacts) -> Vec<&str> {
        f.references
            .iter()
            .filter(|r| r.value_ref)
            .map(|r| r.name.as_str())
            .collect()
    }

    // P1 assembly: RISC-V `j`/`tail`/branches were not control transfers,
    // and the branch's target is its last operand, after the registers.
    #[test]
    fn riscv_jumps_and_branches_reach_their_target() {
        let src = "_start:\n\tj _start_real\n\ttail tail_fn\n\tbeq a0, a1, _try\n\
                   \tbnez t0, 1f\n\tjal ra, linked\n\tjalr ra\n\tjr t0\n\tcall callee\n\
                   1:\n\tret\n";
        let f = extract(src);
        assert_eq!(
            calls(&f),
            vec!["_start_real", "tail_fn", "_try", "linked", "callee"]
        );
    }

    #[test]
    fn arm_and_x86_branch_forms() {
        let src = "f:\n\tcbz x0, armtgt\n\tb.ne cond\n\tbx lr\n\tb .\n\
                   \tjmp *%rax\n\tcall foo@PLT\n\tjne near_tgt\n\tcall qword [rel slot]\n";
        let f = extract(src);
        assert_eq!(calls(&f), vec!["armtgt", "cond", "foo", "near_tgt", "slot"]);
    }

    #[test]
    fn local_and_data_labels_are_not_callables() {
        let src = "f:\n\tbnez a0, 1f\n1:\n\tret\n.Lloc:\n\tret\n\
                   _hdr:\n\t.word 0x1234\n\
                   g:\n\tret\n\
                   .data\nbuf:\n\tnop\n\
                   .text\nh:\n\tret\n";
        let f = extract(src);
        assert_eq!(names(&f), vec!["f", "g", "h"]);
        // `f` owns its local labels; the data label ends it.
        let f_def = &f.definitions[0];
        let hdr = src.find("_hdr").unwrap() as u32;
        assert!(f_def.end_byte < hdr, "{f_def:?}");
        assert!(f_def.end_byte > src.find(".Lloc").unwrap() as u32);
        // `g` ends at the section switch.
        assert!(f.definitions[1].end_byte < src.find(".data").unwrap() as u32);
    }

    #[test]
    fn address_taking_tables_and_aliases_are_value_references() {
        let src = "f:\n\tlla a4, _trap_handler\n\tla t0, %lo(lo_sym)\n\
                   \tadr x0, armsym\n\tldr x2, =pool_sym\n\
                   \tlea rax, [nasm_sym]\n\tleaq rip_sym(%rip), %rax\n\
                   \tmovq $imm_sym, %rax\n\tmovq $0x10, %rax\n\
                   \t.set alias, set_target\n\
                   table:\n\t.word vec_a\n\t.quad vec_b, vec_c\n\t.4byte 7\n";
        let f = extract_refs(src);
        assert_eq!(
            values(&f),
            vec![
                "_trap_handler",
                "lo_sym",
                "armsym",
                "pool_sym",
                "nasm_sym",
                "rip_sym",
                "imm_sym",
                "set_target",
                "vec_a",
                "vec_b",
                "vec_c",
            ]
        );
        // Without reference sites, none.
        assert!(values(&extract(src)).is_empty());
    }

    #[test]
    fn export_directives_set_visibility() {
        let src =
            "\t.globl a, b\n\t.weak c\na:\n\tret\nb:\n\tret\nc:\n\tret\nd:\n\tret\n";
        let f = extract(src);
        let vis: Vec<(&str, &str, Vis)> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.as_str(), d.visibility.as_str(), d.vis))
            .collect();
        assert_eq!(
            vis,
            vec![
                ("a", "global", Vis::Public),
                ("b", "global", Vis::Public),
                ("c", "weak", Vis::Public),
                ("d", "local", Vis::Private),
            ]
        );
        // NASM `global`, and a file declaring no export leaves all unknown.
        let nasm = extract(
            "global _start\nsection .text\n_start:\n\tcall kmain\nisr0:\n\tiret\nidt_table:\n\tdd isr0\n",
        );
        assert_eq!(names(&nasm), vec!["_start", "isr0"]);
        assert_eq!(nasm.definitions[0].vis, Vis::Public);
        assert_eq!(nasm.definitions[1].vis, Vis::Private);
        let plain = extract("f:\n\tret\n");
        assert_eq!(plain.definitions[0].vis, Vis::Unknown);
    }
}
