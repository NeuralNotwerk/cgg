//! Shell/Bash plugin — function definitions and command calls.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct BashPlugin;

impl LanguagePlugin for BashPlugin {
    fn id(&self) -> &'static str {
        "bash"
    }
    fn extensions(&self) -> &'static [&'static str] {
        // `.bats`: bats-core test files, Bash plus `@test` blocks.
        &[".sh", ".bash", ".bats"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["bash", "sh", "zsh", "dash", "ksh", "mksh", "ash", "bats"]
    }
    /// makepkg's build script has no extension or shebang.
    fn filenames(&self) -> &'static [&'static str] {
        &["PKGBUILD"]
    }
    /// bats-core's `@test "description" {` opens a test body the Bash
    /// grammar reads as a command named `@test` followed by loose
    /// statements. bats itself rewrites the line into a function
    /// definition before running it; this does the same, same length, so
    /// the body parses as one function the walker names
    /// `handler_at_<line>` (see [`bats_test_header`]).
    fn parse_source(&self, _path: &Path, source: &[u8]) -> Option<Vec<u8>> {
        if !source.windows(5).any(|w| w == b"@test") {
            return None;
        }
        let mut out = source.to_vec();
        let mut changed = false;
        let mut start = 0;
        while start < source.len() {
            let end = source[start..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(source.len(), |i| start + i);
            if let Some((at, brace, _)) = bats_test_header(&source[start..end]) {
                // `@test "d" {` -> `t______(){`: a name, `()`, the brace kept.
                let (from, to) = (start + at, start + brace);
                out[from] = b't';
                for b in &mut out[from + 1..to - 2] {
                    *b = b'_';
                }
                out[to - 2] = b'(';
                out[to - 1] = b')';
                changed = true;
            }
            start = end + 1;
        }
        changed.then_some(out)
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_bash::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "bash");
        let bats = path.extension().is_some_and(|e| e == "bats")
            || source.split(|&b| b == b'\n').next().is_some_and(|l| {
                l.starts_with(b"#!") && l.windows(4).any(|w| w == b"bats")
            });
        let mut w = BashWalker {
            source,
            facts: &mut facts,
            bats,
        };
        w.walk(tree.root_node());
        facts
    }
}

/// A bats-core test header line, `@test "description" {` (leading blanks
/// allowed, anything after the brace kept by bats as the body's first
/// line): the byte offsets of the `@`, of the opening `{`, and the
/// description unquoted. The rewrite needs at least three bytes before
/// the brace for `t()`, which `@test ` alone provides.
fn bats_test_header(line: &[u8]) -> Option<(usize, usize, String)> {
    let at = line.iter().position(|b| !b.is_ascii_whitespace())?;
    let rest = line.get(at..)?;
    if !rest.starts_with(b"@test")
        || !rest.get(5).is_some_and(|b| *b == b' ' || *b == b'\t')
    {
        return None;
    }
    // The last `{` preceded by a blank opens the body (bats' own rule).
    let brace = (at + 6..line.len())
        .rev()
        .find(|&i| line[i] == b'{' && (line[i - 1] == b' ' || line[i - 1] == b'\t'))?;
    let desc = std::str::from_utf8(&line[at + 5..brace]).ok()?.trim();
    let desc = super::registrar::unquote(desc);
    Some((at, brace, desc))
}

struct BashWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    /// A bats-core test file, where `run cmd …` calls `cmd`.
    bats: bool,
}

impl<'a> BashWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_definition" => {
                self.record_function(node);
                self.walk_children(node);
                return;
            }
            "command" => {
                self.record_command(node);
                // Don't recurse into command arguments
                return;
            }
            "declaration_command" => {
                // `source ./file.sh` or `. ./file.sh`
                self.check_source(node);
                return;
            }
            _ => {}
        }
        self.walk_children(node);
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

    fn record_function(&mut self, node: Node) {
        if self.record_bats_test(node) {
            return;
        }
        let name = node
            .child_by_field_name("name")
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
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    /// A bats `@test` block (parsed as a function through
    /// [`BashPlugin::parse_source`]): minted as `handler_at_<line>`, with a
    /// reference carrying `@test` as its registering context and the
    /// description as its route, the shape every harness rule reads.
    fn record_bats_test(&mut self, node: Node) -> bool {
        let line_start = self.source[..node.start_byte()]
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(0, |i| i + 1);
        let line_end = self.source[node.start_byte()..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(self.source.len(), |i| node.start_byte() + i);
        let Some((at, _, desc)) = bats_test_header(&self.source[line_start..line_end])
        else {
            return false;
        };
        if line_start + at != node.start_byte() {
            return false;
        }
        let line = (node.start_position().row as u32) + 1;
        let simple = format!("handler_at_{line}");
        self.facts.definitions.push(DefRecord {
            simple_name: simple.clone(),
            qualified_name: simple.clone(),
            variant: DefVariant::NamedClosure,
            start_line: line,
            end_line: (node.end_position().row as u32) + 1,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: vec!["synthetic".to_string()],
            ..Default::default()
        });
        self.facts.references.push(RefRecord {
            name: simple,
            receiver_hint: cgg_core::VALUE_REF_HINT.to_string(),
            site_line: line,
            site_byte: node.start_byte() as u32,
            context: "@test".to_string(),
            route: desc,
            ..Default::default()
        });
        true
    }

    fn record_command(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        let Some(name_node) = name_node else { return };
        // command_name -> word
        let name = if name_node.kind() == "command_name" {
            name_node
                .child(0)
                .map(|n| self.text(n).to_string())
                .unwrap_or_default()
        } else {
            self.text(name_node).to_string()
        };
        if name.is_empty() {
            return;
        }

        // `source` and `.` are import-like
        if name == "source" || name == "." {
            // Get the first argument as the sourced file
            let arg = node
                .child_by_field_name("argument")
                .or_else(|| {
                    let count = node.child_count();
                    for i in 0..count {
                        let c = node.child(i as u32).unwrap();
                        if c.kind() == "word" || c.kind() == "string" {
                            return Some(c);
                        }
                    }
                    None
                })
                .map(|n| {
                    self.text(n)
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string()
                })
                .unwrap_or_default();
            if !arg.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: "source".into(),
                    path: arg,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
            return;
        }

        // Skip common builtins that aren't user functions
        if matches!(
            name.as_str(),
            "echo"
                | "printf"
                | "cd"
                | "exit"
                | "return"
                | "export"
                | "local"
                | "readonly"
                | "unset"
                | "set"
                | "shift"
                | "test"
                | "["
                | "[["
                | "true"
                | "false"
                | ":"
                | "eval"
                | "exec"
                | "trap"
                | "wait"
                | "kill"
                | "read"
                | "declare"
                | "typeset"
                | "let"
                | "pushd"
                | "popd"
                | "dirs"
        ) {
            return;
        }

        // bats' `run [-N] [--separate-stderr] [!] cmd args…` calls `cmd`.
        let run_target = (self.bats && name == "run")
            .then(|| {
                let mut c = node.walk();
                node.children_by_field_name("argument", &mut c)
                    .find(|a| {
                        let t = self.text(*a);
                        a.kind() == "word" && !t.starts_with('-') && t != "!"
                    })
                    .map(|a| (self.text(a).to_string(), a))
            })
            .flatten();
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
        if let Some((target, a)) = run_target {
            self.facts.references.push(RefRecord {
                name: target,
                receiver_hint: String::new(),
                site_line: (a.start_position().row as u32) + 1,
                site_byte: a.start_byte() as u32,
                ..Default::default()
            });
        }
    }

    fn check_source(&mut self, _node: Node) {
        // declaration_command handles `local`, `declare`, etc. — not imports
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
        p.set_language(&tree_sitter_bash::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        BashPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.sh"),
            &tree,
            src.as_bytes(),
        )
    }

    fn extract_rewritten(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_bash::LANGUAGE.into()).unwrap();
        let parsed = BashPlugin
            .parse_source(Path::new("t.bats"), src.as_bytes())
            .unwrap_or_else(|| src.as_bytes().to_vec());
        assert_eq!(parsed.len(), src.len(), "the rewrite keeps every offset");
        let tree = p.parse(&parsed, None).unwrap();
        BashPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.bats"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn bats_test_blocks_are_minted_as_handlers_with_their_description() {
        let src = "#!/usr/bin/env bats\nload test_helper\nsetup() {\n  x=1\n}\n\
                   @test \"adds two\" {\n  run add 1 2\n}\n\
                   \t@test 'other — é' {\n  helper\n}\n";
        let f = extract_rewritten(src);
        let defs: Vec<(&str, u32, u32)> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.as_str(), d.start_line, d.end_line))
            .collect();
        assert_eq!(
            defs,
            vec![
                ("setup", 3, 5),
                ("handler_at_6", 6, 8),
                ("handler_at_9", 9, 11)
            ]
        );
        let routes: Vec<(&str, &str)> = f
            .references
            .iter()
            .filter(|r| r.context == "@test")
            .map(|r| (r.name.as_str(), r.route.as_str()))
            .collect();
        assert_eq!(
            routes,
            vec![("handler_at_6", "adds two"), ("handler_at_9", "other — é")]
        );
        // The body's commands are calls made from the test.
        assert!(f.references.iter().any(|r| r.name == "add"));
        assert!(f.references.iter().any(|r| r.name == "helper"));
        // `@test` itself and its closing brace are not calls.
        assert!(
            !f.references
                .iter()
                .any(|r| r.name == "@test" || r.name == "}")
        );
    }

    #[test]
    fn a_file_without_bats_tests_is_parsed_as_written() {
        assert!(
            BashPlugin
                .parse_source(Path::new("t.bats"), b"echo '@test'\n")
                .is_none()
        );
        assert!(
            BashPlugin
                .parse_source(Path::new("t.bats"), b"f() { :; }\n")
                .is_none()
        );
    }

    #[test]
    fn function_definitions() {
        let src = "greet() { echo \"hi\"; }\nprocess() { greet; }\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(names.contains(&"process"), "got: {names:?}");
    }

    #[test]
    fn function_calls_captured() {
        let src = "greet() { echo \"hi\"; }\nmain() { greet; helper_fn \"arg\"; }\n";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "greet"),
            "refs: {:?}",
            f.references
        );
        assert!(
            f.references.iter().any(|r| r.name == "helper_fn"),
            "refs: {:?}",
            f.references
        );
    }

    #[test]
    fn source_directive_is_import() {
        let src = "source ./lib.sh\n. ./utils.sh\ngreet() { echo hi; }\n";
        let f = extract(src);
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "source" && i.path == "./lib.sh"),
            "imports: {:?}",
            f.imports
        );
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "source" && i.path == "./utils.sh"),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn builtins_filtered_out() {
        let src = "f() { echo hi; cd /tmp; my_func; }\n";
        let f = extract(src);
        assert!(!f.references.iter().any(|r| r.name == "echo"));
        assert!(!f.references.iter().any(|r| r.name == "cd"));
        assert!(f.references.iter().any(|r| r.name == "my_func"));
    }
}
