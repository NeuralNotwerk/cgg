//! Shell/Bash plugin — function definitions and command calls.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::LanguagePlugin;

#[derive(Debug)]
pub struct BashPlugin;

impl LanguagePlugin for BashPlugin {
    fn id(&self) -> &'static str { "bash" }
    fn extensions(&self) -> &'static [&'static str] { &[".sh", ".bash"] }
    fn shebangs(&self) -> &'static [&'static str] { &["bash", "sh", "zsh"] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_bash::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "bash");
        let mut w = BashWalker { source, facts: &mut facts };
        w.walk(tree.root_node());
        facts
    }
}

struct BashWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> BashWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }

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
                if !c.goto_next_sibling() { break; }
            }
        }
    }

    fn record_function(&mut self, node: Node) {
        let name = node.child_by_field_name("name")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if name.is_empty() { return; }
        let (sl, el) = ((node.start_position().row as u32) + 1, (node.end_position().row as u32) + 1);
        self.facts.definitions.push(DefRecord {
            simple_name: name.clone(),
            qualified_name: name,
            variant: DefVariant::FreeFunction,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_command(&mut self, node: Node) {
        let name_node = node.child_by_field_name("name");
        let Some(name_node) = name_node else { return };
        // command_name -> word
        let name = if name_node.kind() == "command_name" {
            name_node.child(0).map(|n| self.text(n).to_string()).unwrap_or_default()
        } else {
            self.text(name_node).to_string()
        };
        if name.is_empty() { return; }

        // `source` and `.` are import-like
        if name == "source" || name == "." {
            // Get the first argument as the sourced file
            let arg = node.child_by_field_name("argument")
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
                .map(|n| self.text(n).trim_matches('"').trim_matches('\'').to_string())
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
        if matches!(name.as_str(), "echo" | "printf" | "cd" | "exit" | "return"
            | "export" | "local" | "readonly" | "unset" | "set" | "shift"
            | "test" | "[" | "[[" | "true" | "false" | ":" | "eval"
            | "exec" | "trap" | "wait" | "kill" | "read" | "declare"
            | "typeset" | "let" | "pushd" | "popd" | "dirs") {
            return;
        }

        self.facts.references.push(RefRecord {
            name,
            receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
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
        BashPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.sh"), &tree, src.as_bytes())
    }

    #[test]
    fn function_definitions() {
        let src = "greet() { echo \"hi\"; }\nprocess() { greet; }\n";
        let f = extract(src);
        let names: Vec<&str> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(names.contains(&"process"), "got: {names:?}");
    }

    #[test]
    fn function_calls_captured() {
        let src = "greet() { echo \"hi\"; }\nmain() { greet; helper_fn \"arg\"; }\n";
        let f = extract(src);
        assert!(f.references.iter().any(|r| r.name == "greet"), "refs: {:?}", f.references);
        assert!(f.references.iter().any(|r| r.name == "helper_fn"), "refs: {:?}", f.references);
    }

    #[test]
    fn source_directive_is_import() {
        let src = "source ./lib.sh\n. ./utils.sh\ngreet() { echo hi; }\n";
        let f = extract(src);
        assert!(f.imports.iter().any(|i| i.kind == "source" && i.path == "./lib.sh"), "imports: {:?}", f.imports);
        assert!(f.imports.iter().any(|i| i.kind == "source" && i.path == "./utils.sh"), "imports: {:?}", f.imports);
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
