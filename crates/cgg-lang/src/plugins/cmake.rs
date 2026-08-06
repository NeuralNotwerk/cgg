//! CMake plugin — callable extraction.
//!
//! CMake has user-defined `function(name ...)` / `macro(name ...)`
//! blocks, and every other call is a `normal_command` invocation of
//! either a built-in (`add_library`, `target_link_libraries`, …) or a
//! user-defined function. `include(path)` and `add_subdirectory(dir)`
//! are recorded as imports.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct CmakePlugin;

impl LanguagePlugin for CmakePlugin {
    fn id(&self) -> &'static str {
        "cmake"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".cmake"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_cmake::LANGUAGE.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "cmake");
        let mut w = CmakeWalker {
            source,
            facts: &mut facts,
        };
        w.walk(tree.root_node());
        facts
    }
}

struct CmakeWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> CmakeWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "function_def" => {
                self.record_block(node, "function_command");
                self.walk_children(node);
                return;
            }
            "macro_def" => {
                self.record_block(node, "macro_command");
                self.walk_children(node);
                return;
            }
            "normal_command" => {
                self.record_call(node);
                self.walk_children(node);
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

    fn record_block(&mut self, node: Node, header_kind: &str) {
        let header = node
            .children(&mut node.walk())
            .find(|c| c.kind() == header_kind);
        let Some(header) = header else { return };
        let args = header
            .children(&mut header.walk())
            .find(|c| c.kind() == "argument_list");
        let Some(args) = args else { return };
        let name = args
            .children(&mut args.walk())
            .find(|c| c.kind() == "argument")
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

    fn record_call(&mut self, node: Node) {
        let Some(ident) = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "identifier")
        else {
            return;
        };
        let name_raw = self.text(ident);
        let name = name_raw.to_string();
        let lower = name_raw.to_ascii_lowercase();
        if name.is_empty() {
            return;
        }

        if matches!(
            lower.as_str(),
            "include" | "add_subdirectory" | "find_package"
        )
            && let Some(args) = node
                .children(&mut node.walk())
                .find(|c| c.kind() == "argument_list")
                && let Some(first) = args
                    .children(&mut args.walk())
                    .find(|c| c.kind() == "argument")
                {
                    let path = self
                        .text(first)
                        .trim_matches(|c: char| c == '"' || c == '\'')
                        .to_string();
                    if !path.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: lower,
                            path,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                        return;
                    }
                }

        self.facts.references.push(RefRecord {
            name,
            receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
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
        p.set_language(&tree_sitter_cmake::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        CmakePlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/x.cmake"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn function_macro_and_call() {
        let src = "function(my_helper target)\n  add_library(${target} STATIC src.c)\nendfunction()\nmacro(noisy what)\n  message(STATUS ${what})\nendmacro()\nmy_helper(mylib)\n";
        let f = extract(src);
        let names: Vec<_> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(names.contains(&"my_helper"), "got: {names:?}");
        assert!(names.contains(&"noisy"), "got: {names:?}");
        assert!(f.references.iter().any(|r| r.name == "add_library"));
        assert!(f.references.iter().any(|r| r.name == "my_helper"));
    }

    #[test]
    fn include_is_import() {
        let src = "include(FetchContent)\nadd_subdirectory(deps/foo)\n";
        let f = extract(src);
        assert!(
            f.imports
                .iter()
                .any(|i| i.path == "FetchContent" && i.kind == "include")
        );
        assert!(
            f.imports
                .iter()
                .any(|i| i.path == "deps/foo" && i.kind == "add_subdirectory")
        );
    }
}
