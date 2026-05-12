//! R plugin — callable extraction.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

#[derive(Debug)]
pub struct RPlugin;

impl LanguagePlugin for RPlugin {
    fn id(&self) -> &'static str { "r" }
    fn extensions(&self) -> &'static [&'static str] { &[".r", ".R", ".Rmd"] }
    fn shebangs(&self) -> &'static [&'static str] { &["Rscript"] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_r::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "r");
        let mut w = RWalker { source, facts: &mut facts };
        w.walk(tree.root_node());
        facts
    }
}

struct RWalker<'a> { source: &'a [u8], facts: &'a mut FileFacts }

impl<'a> RWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "binary_operator" | "equals_assignment" => {
                // name <- function(...) { ... } OR name = function(...) { ... }
                self.try_record_function_assign(node);
                self.walk_children(node); return;
            }
            "call" => {
                self.record_call(node);
                self.walk_children(node); return;
            }
            _ => {}
        }
        self.walk_children(node);
    }

    fn walk_children(&mut self, node: Node) {
        let mut c = node.walk();
        if c.goto_first_child() { loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } } }
    }

    fn try_record_function_assign(&mut self, node: Node) {
        // binary_operator: lhs <- rhs where rhs is function_definition
        // equals_assignment: lhs = rhs where rhs is function_definition
        let lhs = node.child_by_field_name("lhs")
            .or_else(|| node.child(0));
        let rhs = node.child_by_field_name("rhs")
            .or_else(|| node.child(2));
        let (Some(lhs), Some(rhs)) = (lhs, rhs) else { return };
        if rhs.kind() != "function_definition" { return; }
        if lhs.kind() != "identifier" { return; }
        let name = self.text(lhs).to_string();
        if name.is_empty() { return; }
        let (sl, el) = ((node.start_position().row as u32)+1, (node.end_position().row as u32)+1);
        self.facts.definitions.push(DefRecord {
            simple_name: name.clone(), qualified_name: name,
            variant: DefVariant::FreeFunction,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(), attributes: Vec::new(),
        });
    }

    fn record_call(&mut self, node: Node) {
        let func = node.child_by_field_name("function")
            .map(|n| self.text(n).to_string()).unwrap_or_default();
        if func.is_empty() { return; }
        // library() and source() are imports
        if func == "library" || func == "require" || func == "source" {
            let arg = node.child_by_field_name("arguments")
                .and_then(|a| a.children(&mut a.walk())
                    .find(|c| c.kind() == "argument"))
                .and_then(|a| a.child_by_field_name("value"))
                .map(|n| self.text(n).trim_matches('"').trim_matches('\'').to_string())
                .unwrap_or_default();
            if !arg.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: if func == "source" { "source" } else { "import" }.into(),
                    path: arg, alias: String::new(),
                    site_line: (node.start_position().row as u32)+1,
                    site_byte: node.start_byte() as u32,
                });
            }
            return;
        }
        // Skip common builtins
        if matches!(func.as_str(), "print" | "cat" | "paste" | "paste0" | "c"
            | "list" | "data.frame" | "matrix" | "vector" | "length"
            | "nrow" | "ncol" | "names" | "class" | "is.null" | "is.na"
            | "return" | "stop" | "warning" | "message" | "if" | "for"
            | "while" | "repeat" | "next" | "break") { return; }
        let (name, recv) = if let Some(pos) = func.rfind('$') {
            (func[pos+1..].to_string(), func[..pos].to_string())
        } else if let Some(pos) = func.rfind("::") {
            (func[pos+2..].to_string(), func[..pos].to_string())
        } else { (func, String::new()) };
        self.facts.references.push(RefRecord {
            name, receiver_hint: recv,
            site_line: (node.start_position().row as u32)+1,
            site_byte: node.start_byte() as u32,
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
        p.set_language(&tree_sitter_r::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        RPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/x.R"), &tree, src.as_bytes())
    }

    #[test]
    fn function_definitions() {
        let f = extract("greet <- function(name) { print(name) }\nprocess <- function(x) { x + 1 }\n");
        assert!(f.definitions.iter().any(|d| d.simple_name == "greet"));
        assert!(f.definitions.iter().any(|d| d.simple_name == "process"));
    }

    #[test]
    fn library_is_import() {
        let f = extract("library(dplyr)\nsource(\"helpers.R\")\nf <- function() {}\n");
        assert!(f.imports.iter().any(|i| i.kind == "import" && i.path == "dplyr"));
        assert!(f.imports.iter().any(|i| i.kind == "source" && i.path == "helpers.R"));
    }

    #[test]
    fn calls_captured() {
        let f = extract("f <- function() { greet(\"x\"); dplyr::filter(df, x > 0) }\n");
        assert!(f.references.iter().any(|r| r.name == "greet"));
        assert!(f.references.iter().any(|r| r.name == "filter" && r.receiver_hint == "dplyr"));
    }
}
