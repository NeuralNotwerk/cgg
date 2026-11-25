//! Fortran plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct FortranPlugin;

impl LanguagePlugin for FortranPlugin {
    fn id(&self) -> &'static str {
        "fortran"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[
            ".f90", ".f95", ".f03", ".f08", ".f", ".for", ".f77", ".ftn", ".fpp",
        ]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_fortran::LANGUAGE.into()
    }

    /// The grammar reads free-form source only, so a fixed-form file's
    /// column-1 `C`/`c`/`*` comment lines parse as statements and can
    /// sink the whole file into ERROR nodes. Turn them into `!` comments.
    fn parse_source(&self, path: &Path, source: &[u8]) -> Option<Vec<u8>> {
        fixed_form_comments_as_bang(path, source)
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "fortran");
        let mut w = FortranWalker {
            source,
            facts: &mut facts,
            module: String::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

/// Extensions whose source form is fixed by convention (compared
/// lowercased, so `.F` and `.FOR` count). `.fpp` is either form.
const FIXED_FORM_EXTENSIONS: &[&str] = &["f", "for", "f77", "ftn"];

/// A same-length copy of a fixed-form file with every column-1 comment
/// line (`C`, `c`, `*`, and the `D`/`d` debug lines compilers drop by
/// default) starting with `!`, or `None` when nothing changes.
///
/// The extension alone is not trusted: a `.f` file written in free form
/// would have `call x` or `do i = 1, n` at column 1 destroyed. A file is
/// read as free form, and left alone, when any line starts at column 1
/// with a letter that is not a comment marker, or has a letter as its
/// first character in columns 2 to 5 (only labels may sit there in fixed
/// form). A tab before the first character is the fixed-form tab layout
/// and says nothing.
pub(crate) fn fixed_form_comments_as_bang(path: &Path, source: &[u8]) -> Option<Vec<u8>> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if !FIXED_FORM_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }
    let is_comment_mark = |b: u8| matches!(b, b'C' | b'c' | b'*' | b'D' | b'd');
    let mut starts = Vec::new();
    for line in source.split(|&b| b == b'\n') {
        let Some(&first) = line.first() else { continue };
        if first.is_ascii_alphabetic() && !is_comment_mark(first) {
            return None;
        }
        if is_comment_mark(first) {
            let offset = line.as_ptr() as usize - source.as_ptr() as usize;
            starts.push(offset);
            continue;
        }
        for (col, &b) in line.iter().enumerate().take(5) {
            match b {
                b' ' => continue,
                b'\t' => break,
                _ if b.is_ascii_alphabetic() && col > 0 => return None,
                _ => break,
            }
        }
    }
    if starts.is_empty() {
        return None;
    }
    let mut out = source.to_vec();
    for i in starts {
        out[i] = b'!';
    }
    Some(out)
}

struct FortranWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    module: String,
}

impl<'a> FortranWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.module.is_empty() {
            simple.to_string()
        } else {
            format!("{}::{simple}", self.module)
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "module_statement" => {
                self.extract_module(node);
                self.walk_children(node);
            }
            "use_statement" => {
                self.record_import(node);
                self.walk_children(node);
            }
            // Match the enclosing form so the byte range covers the
            // body, not just the header line — needed for intra-file
            // edges to attribute call sites to their containing routine.
            "function" | "subroutine" | "program" => {
                let from = self.facts.definitions.len();
                self.record_callable(node);
                // A body inside an `interface` block declares a routine
                // defined elsewhere (or an abstract interface): a
                // declaration, not the routine.
                let in_interface = std::iter::successors(node.parent(), |n| n.parent())
                    .take_while(|n| {
                        !matches!(n.kind(), "function" | "subroutine" | "program")
                    })
                    .any(|n| n.kind() == "interface");
                super::attrs::mark_abstract_from(
                    &mut self.facts.definitions,
                    from,
                    in_interface,
                );
                self.walk_children(node);
            }
            "call_expression" | "subroutine_call" => {
                self.record_call(node);
                self.walk_children(node);
            }
            _ => self.walk_children(node),
        }
    }

    fn walk_children(&mut self, node: Node) {
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

    fn extract_module(&mut self, node: Node) {
        // module name ... end module
        // Find the name child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && child.kind() == "name"
            {
                self.module = self.text(child).to_string();
                break;
            }
        }
    }

    fn record_import(&mut self, node: Node) {
        // use module_name
        // Find the name child
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i as u32)
                && matches!(child.kind(), "name" | "module_name")
            {
                let module_name = self.text(child).to_string();
                self.facts.imports.push(ImportRecord {
                    kind: "use".to_string(),
                    path: module_name,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
                break;
            }
        }
    }

    fn record_callable(&mut self, node: Node) {
        // The header (`subroutine_statement` / `function_statement` /
        // `program_statement`) is a direct child of the enclosing form;
        // the routine's name lives inside that header.
        let header_kind = match node.kind() {
            "subroutine" => "subroutine_statement",
            "function" => "function_statement",
            "program" => "program_statement",
            _ => return,
        };
        let header = node
            .children(&mut node.walk())
            .find(|c| c.kind() == header_kind);
        let Some(header) = header else { return };
        let name = header
            .children(&mut header.walk())
            .find(|c| c.kind() == "name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() {
            return;
        }

        let qn = self.qn(&name);
        let variant = DefVariant::FreeFunction;
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        // tree-sitter-fortran doesn't expose field names on call_expression /
        // subroutine_call. The callee is the first named `identifier` child.
        let mut c = node.walk();
        if !c.goto_first_child() {
            return;
        }
        loop {
            let n = c.node();
            if n.kind() == "identifier" {
                let name = self.text(n).to_string();
                if !name.is_empty() {
                    self.facts.references.push(RefRecord {
                        name,
                        receiver_hint: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                        ..Default::default()
                    });
                }
                return;
            }
            if !c.goto_next_sibling() {
                return;
            }
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
        p.set_language(&tree_sitter_fortran::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        FortranPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.f90"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn module_and_functions() {
        let src = "module math_utils\ncontains\nfunction add(a, b) result(c)\nreal :: a, b, c\nc = a + b\nend function add\nend module math_utils\n";
        let f = extract(src);
        eprintln!("Definitions found: {}", f.definitions.len());
        for def in &f.definitions {
            eprintln!("  - {}: {}", def.simple_name, def.qualified_name);
        }
        assert!(!f.definitions.is_empty(), "Expected definitions, got none");
        assert_eq!(f.definitions[0].simple_name, "add");
    }

    /// The definitions named `qn` and whether each is tagged `abstract`.
    fn abstract_tags(f: &FileFacts, qn: &str) -> Vec<bool> {
        f.definitions
            .iter()
            .filter(|d| d.qualified_name == qn)
            .map(|d| d.attributes.iter().any(|a| a == "abstract"))
            .collect()
    }

    // WS22 gap 7: a routine body in an `interface` block declares a
    // routine; the module procedure is the routine.
    #[test]
    fn interface_bodies_are_tagged_abstract() {
        let f = extract(
            "module m\n  interface\n    function g(x) result(r)\n      real :: x, r\n    end function g\n  end interface\ncontains\n  function g(x) result(r)\n    real :: x, r\n    r = x\n  end function g\nend module m\n",
        );
        assert_eq!(abstract_tags(&f, "m::g"), vec![true, false]);
    }

    /// Parse `src` as the pipeline does for a file at `path`: through
    /// `parse_source` when it rewrites, extracting from the original.
    fn extract_at(path: &str, src: &str) -> FileFacts {
        let rewritten = FortranPlugin.parse_source(Path::new(path), src.as_bytes());
        let mut p = Parser::new();
        p.set_language(&tree_sitter_fortran::LANGUAGE.into())
            .unwrap();
        let tree = p
            .parse(rewritten.as_deref().unwrap_or(src.as_bytes()), None)
            .unwrap();
        FortranPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from(path),
            &tree,
            src.as_bytes(),
        )
    }

    const FIXED_FORM: &str = "C*****************************\n\
        C   HEADER\n\
        *   another comment\n\
        \x20     SUBROUTINE UMAT(STRESS,\n\
        \x20    1 NTENS)\n\
        C   body\n\
        \x20     CALL HELPER(STRESS)\n\
        \x20     RETURN\n\
        \x20     END\n\
        c lower-case comment\n\
        \x20     SUBROUTINE HELPER(S)\n\
        \x20     RETURN\n\
        \x20     END\n";

    // P1 fixed-form rewrite: column-1 `C` comments sank every routine of
    // a fixed-form Abaqus UMAT file into ERROR nodes (0 callables).
    #[test]
    fn fixed_form_column_one_comments_do_not_hide_routines() {
        let f = extract_at("/t/umat.for", FIXED_FORM);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("umat")),
            "{names:?}"
        );
        assert!(
            names.iter().any(|n| n.eq_ignore_ascii_case("helper")),
            "{names:?}"
        );
        // Same length, offsets unchanged, only column-1 marks touched.
        let r = FortranPlugin
            .parse_source(Path::new("/t/UMAT.F"), FIXED_FORM.as_bytes())
            .unwrap();
        assert_eq!(r.len(), FIXED_FORM.len());
        let text = String::from_utf8(r).unwrap();
        assert!(text.starts_with("!*****"), "{text}");
        assert!(text.contains("\n!   HEADER\n!   another"), "{text}");
        assert!(text.contains("\n! lower-case"), "{text}");
        assert!(text.contains("      CALL HELPER"), "{text}");
    }

    #[test]
    fn free_form_sources_are_never_rewritten() {
        // A free-form extension, whatever its content.
        assert!(
            FortranPlugin
                .parse_source(Path::new("/t/m.f90"), FIXED_FORM.as_bytes())
                .is_none()
        );
        // A `.f` file written in free form: `call` at column 1 is code.
        let free = "subroutine s()\ncall t()\nend subroutine s\n";
        assert!(
            FortranPlugin
                .parse_source(Path::new("/t/m.f"), free.as_bytes())
                .is_none()
        );
        // Indented free form: a letter in columns 2-5 is not fixed form.
        let indented =
            "module m\ncontains\n  subroutine s()\n  end subroutine s\nend module m\n";
        assert!(
            FortranPlugin
                .parse_source(Path::new("/t/m.f"), indented.as_bytes())
                .is_none()
        );
    }
}
