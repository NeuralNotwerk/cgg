//! R plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct RPlugin;

impl LanguagePlugin for RPlugin {
    fn id(&self) -> &'static str {
        "r"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".r", ".R", ".Rmd", ".rmd", ".qmd"]
    }
    /// An R Markdown or Quarto document is prose around ```` ```{r} ````
    /// chunks: the grammar reads only the chunks, every other line blanked
    /// to spaces so offsets and line numbers stay the file's own.
    fn parse_source(&self, source: &[u8]) -> Option<Vec<u8>> {
        r_chunks_only(source)
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["Rscript"]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_r::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "r");
        let mut w = RWalker {
            source,
            facts: &mut facts,
        };
        w.walk(tree.root_node());
        facts
    }
}

struct RWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> RWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "binary_operator" | "equals_assignment" => {
                // name <- function(...) { ... } OR name = function(...) { ... }
                self.try_record_function_assign(node);
                self.walk_children(node);
                return;
            }
            "call" => {
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

    fn try_record_function_assign(&mut self, node: Node) {
        // binary_operator: lhs <- rhs where rhs is function_definition
        // equals_assignment: lhs = rhs where rhs is function_definition
        let lhs = node.child_by_field_name("lhs").or_else(|| node.child(0));
        let rhs = node.child_by_field_name("rhs").or_else(|| node.child(2));
        let (Some(lhs), Some(rhs)) = (lhs, rhs) else {
            return;
        };
        if rhs.kind() != "function_definition" {
            return;
        }
        if lhs.kind() != "identifier" {
            return;
        }
        let name = self.text(lhs).to_string();
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
        let func = node
            .child_by_field_name("function")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if func.is_empty() {
            return;
        }
        // library() and source() are imports
        if func == "library" || func == "require" || func == "source" {
            let arg = node
                .child_by_field_name("arguments")
                .and_then(|a| a.children(&mut a.walk()).find(|c| c.kind() == "argument"))
                .and_then(|a| a.child_by_field_name("value"))
                .map(|n| {
                    self.text(n)
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string()
                })
                .unwrap_or_default();
            if !arg.is_empty() {
                self.facts.imports.push(ImportRecord {
                    kind: if func == "source" { "source" } else { "import" }.into(),
                    path: arg,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
            return;
        }
        // targets' `tar_source()` runs every R script under a directory
        // (`R/` by default): the includes are known only once every file
        // is read, so it is recorded as a `source-dir` import and expanded
        // by [`expand_source_dirs`].
        if func == "tar_source" || func == "targets::tar_source" {
            for dir in self.tar_source_dirs(node) {
                self.facts.imports.push(ImportRecord {
                    kind: SOURCE_DIR_KIND.into(),
                    path: dir,
                    alias: String::new(),
                    site_line: (node.start_position().row as u32) + 1,
                    site_byte: node.start_byte() as u32,
                });
            }
        }
        // Skip common builtins
        if matches!(
            func.as_str(),
            "print"
                | "cat"
                | "paste"
                | "paste0"
                | "c"
                | "list"
                | "data.frame"
                | "matrix"
                | "vector"
                | "length"
                | "nrow"
                | "ncol"
                | "names"
                | "class"
                | "is.null"
                | "is.na"
                | "return"
                | "stop"
                | "warning"
                | "message"
                | "if"
                | "for"
                | "while"
                | "repeat"
                | "next"
                | "break"
        ) {
            return;
        }
        let (name, recv) = if let Some(pos) = func.rfind('$') {
            (func[pos + 1..].to_string(), func[..pos].to_string())
        } else if let Some(pos) = func.rfind("::") {
            (func[pos + 2..].to_string(), func[..pos].to_string())
        } else {
            (func, String::new())
        };
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            ..Default::default()
        });
    }
}

/// Whether a line opens an R code chunk: ```` ```{r} ````, ```` ```{r label,
/// echo=FALSE} ````, ```` ```{R} ````, indented up to three spaces. Returns
/// the fence's backtick count.
fn r_chunk_open(line: &[u8]) -> Option<usize> {
    let indent = line.iter().take_while(|b| **b == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let ticks = rest.iter().take_while(|b| **b == b'`').count();
    if ticks < 3 {
        return None;
    }
    let info = rest[ticks..].trim_ascii_start();
    let info = info.strip_prefix(b"{")?;
    let lang = info.trim_ascii_start();
    let is_r = matches!(lang.first(), Some(b'r' | b'R'))
        && !lang
            .get(1)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
    is_r.then_some(ticks)
}

/// Whether a line closes a fence of `ticks` backticks.
fn closes_fence(line: &[u8], ticks: usize) -> bool {
    let t = line.trim_ascii();
    t.len() >= ticks && t.iter().all(|b| *b == b'`')
}

/// `source` with every byte outside R chunks (fences included) turned into
/// a space, newlines kept; `None` when the source holds no R chunk, so a
/// plain script parses as written.
fn r_chunks_only(source: &[u8]) -> Option<Vec<u8>> {
    let mut out = source.to_vec();
    let mut open: Option<usize> = None;
    let mut any = false;
    let mut start = 0;
    while start < source.len() {
        let end = source[start..]
            .iter()
            .position(|b| *b == b'\n')
            .map_or(source.len(), |i| start + i);
        let line = &source[start..end];
        let keep = match open {
            Some(ticks) if closes_fence(line, ticks) => {
                open = None;
                false
            }
            Some(_) => true,
            None => {
                open = r_chunk_open(line);
                any |= open.is_some();
                false
            }
        };
        if !keep {
            for b in &mut out[start..end] {
                if *b != b'\r' {
                    *b = b' ';
                }
            }
        }
        start = end + 1;
    }
    any.then_some(out)
}

/// Import kind of a `tar_source()` directory, before expansion.
pub const SOURCE_DIR_KIND: &str = "source-dir";

impl RWalker<'_> {
    /// The directories a `tar_source(…)` call sources: its `files`
    /// argument (first positional or named, a string or `c(…)` of
    /// strings), `R` when it has none.
    fn tar_source_dirs(&self, node: Node) -> Vec<String> {
        let Some(args) = node.child_by_field_name("arguments") else {
            return vec!["R".to_string()];
        };
        let mut c = args.walk();
        let list: Vec<Node> = args
            .children(&mut c)
            .filter(|n| n.kind() == "argument")
            .collect();
        let pick = list
            .iter()
            .find(|a| {
                a.child_by_field_name("name")
                    .is_some_and(|n| self.text(n) == "files")
            })
            .or_else(|| {
                list.iter()
                    .find(|a| a.child_by_field_name("name").is_none())
            });
        let Some(value) = pick.and_then(|a| a.child_by_field_name("value")) else {
            return vec!["R".to_string()];
        };
        let mut out = Vec::new();
        self.collect_strings(value, &mut out);
        out
    }

    fn collect_strings(&self, n: Node, out: &mut Vec<String>) {
        if n.kind() == "string" {
            let t = self.text(n).trim_matches(|c| c == '"' || c == '\'');
            if !t.is_empty() {
                out.push(t.to_string());
            }
            return;
        }
        let mut c = n.walk();
        for ch in n.named_children(&mut c) {
            self.collect_strings(ch, out);
        }
    }
}

/// Expand each `tar_source()` directory into one `source` import per R
/// script under it (recursively, as `tar_source` lists them), relative to
/// the calling file's directory, in path order. Run once every file is
/// read; the `source-dir` records are consumed.
pub fn expand_source_dirs(facts: &mut [&mut FileFacts]) {
    let scripts: Vec<std::path::PathBuf> = {
        let mut v: Vec<std::path::PathBuf> = facts
            .iter()
            .filter(|f| f.path.extension().is_some_and(|e| e == "R" || e == "r"))
            .map(|f| f.path.clone())
            .collect();
        v.sort();
        v
    };
    for f in facts.iter_mut() {
        if !f.imports.iter().any(|i| i.kind == SOURCE_DIR_KIND) {
            continue;
        }
        let base = f.path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let mut imports = Vec::with_capacity(f.imports.len());
        for imp in std::mem::take(&mut f.imports) {
            if imp.kind != SOURCE_DIR_KIND {
                imports.push(imp);
                continue;
            }
            let dir = base.join(imp.path.trim_start_matches("./"));
            for s in &scripts {
                if *s == f.path || !s.starts_with(&dir) {
                    continue;
                }
                let Ok(rel) = s.strip_prefix(&base) else {
                    continue;
                };
                imports.push(ImportRecord {
                    kind: "source".into(),
                    path: rel.to_string_lossy().replace('\\', "/"),
                    alias: String::new(),
                    site_line: imp.site_line,
                    site_byte: imp.site_byte,
                });
            }
        }
        f.imports = imports;
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
        RPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.R"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn function_definitions() {
        let f = extract(
            "greet <- function(name) { print(name) }\nprocess <- function(x) { x + 1 }\n",
        );
        assert!(f.definitions.iter().any(|d| d.simple_name == "greet"));
        assert!(f.definitions.iter().any(|d| d.simple_name == "process"));
    }

    #[test]
    fn library_is_import() {
        let f = extract("library(dplyr)\nsource(\"helpers.R\")\nf <- function() {}\n");
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "import" && i.path == "dplyr")
        );
        assert!(
            f.imports
                .iter()
                .any(|i| i.kind == "source" && i.path == "helpers.R")
        );
    }

    #[test]
    fn a_plain_script_parses_as_written() {
        assert!(r_chunks_only(b"f <- function(x) x\n").is_none());
    }

    #[test]
    fn an_rmd_keeps_only_its_r_chunks_at_their_offsets() {
        let src = "---\ntitle: x\n---\n\nSome prose f(x).\n\n```{r setup, echo=FALSE}\nlocal_fn <- function(x) summarise_data(x)\n```\n\n```{python}\ndef p(): pass\n```\n\n```{r}\nlocal_fn(1)\n```\n";
        let out = r_chunks_only(src.as_bytes()).unwrap();
        assert_eq!(out.len(), src.len());
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), src.lines().count());
        assert!(text.contains("local_fn <- function(x) summarise_data(x)"));
        assert!(text.contains("local_fn(1)"));
        assert!(
            !text.contains("prose") && !text.contains("def p") && !text.contains("```")
        );
        // Extraction over the rewritten parse finds the chunk's function
        // and its call.
        let mut p = Parser::new();
        p.set_language(&tree_sitter_r::LANGUAGE.into()).unwrap();
        let tree = p.parse(&text, None).unwrap();
        assert!(!tree.root_node().has_error(), "{text}");
        let f = RPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("report.Rmd"),
            &tree,
            src.as_bytes(),
        );
        assert!(f.definitions.iter().any(|d| d.simple_name == "local_fn"));
        assert!(f.references.iter().any(|r| r.name == "summarise_data"));
    }

    #[test]
    fn tar_source_names_its_directory() {
        let f = extract(
            "library(targets)\ntar_source()\ntar_source(files = c(\"lib\", \"x\"))\n",
        );
        let dirs: Vec<&str> = f
            .imports
            .iter()
            .filter(|i| i.kind == SOURCE_DIR_KIND)
            .map(|i| i.path.as_str())
            .collect();
        assert_eq!(dirs, vec!["R", "lib", "x"]);
    }

    #[test]
    fn a_source_dir_expands_to_the_scripts_under_it() {
        let mut main = FileFacts::new(FileId::new(0), PathBuf::from("p/_targets.R"), "r");
        main.imports.push(ImportRecord {
            kind: SOURCE_DIR_KIND.into(),
            path: "R".into(),
            alias: String::new(),
            site_line: 2,
            site_byte: 9,
        });
        let mut a = FileFacts::new(FileId::new(1), PathBuf::from("p/R/b.R"), "r");
        let mut b = FileFacts::new(FileId::new(2), PathBuf::from("p/R/sub/a.R"), "r");
        let mut c = FileFacts::new(FileId::new(3), PathBuf::from("p/other/c.R"), "r");
        expand_source_dirs(&mut [&mut main, &mut a, &mut b, &mut c]);
        let got: Vec<(&str, &str)> = main
            .imports
            .iter()
            .map(|i| (i.kind.as_str(), i.path.as_str()))
            .collect();
        assert_eq!(got, vec![("source", "R/b.R"), ("source", "R/sub/a.R")]);
    }

    #[test]
    fn calls_captured() {
        let f = extract("f <- function() { greet(\"x\"); dplyr::filter(df, x > 0) }\n");
        assert!(f.references.iter().any(|r| r.name == "greet"));
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "filter" && r.receiver_hint == "dplyr")
        );
    }
}
