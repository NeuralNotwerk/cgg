//! Thread-local parser pool.
//!
//! `tree_sitter::Parser` instances are expensive to allocate but cheap
//! to re-parameterize with a new language. The pool amortizes that
//! cost by keeping one parser per (thread, language) pair in a
//! `thread_local!` cell.
//!
//! The pool exposes a single operation — [`ParserPool::parse`] — that
//! parses a byte slice with the configured language and returns the
//! tree plus wall-clock timing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

use anyhow::{Result, anyhow};
use tree_sitter::{Language, Parser, Tree};

use crate::{LanguagePlugin, PluginRegistry};

thread_local! {
    static PARSERS: RefCell<HashMap<&'static str, Parser>> = RefCell::new(HashMap::new());
}

/// Output of [`ParserPool::parse`].
#[derive(Debug)]
pub struct ParseOutcome {
    pub tree: Tree,
    pub parse_ms: f64,
}

/// Stateless facade around the thread-local parser cache.
///
/// The pool holds a borrow of the `PluginRegistry` so it can look up
/// `tree_sitter::Language` objects by plugin id.
#[derive(Debug, Clone, Copy)]
pub struct ParserPool<'r> {
    registry: &'r PluginRegistry,
}

impl<'r> ParserPool<'r> {
    pub fn new(registry: &'r PluginRegistry) -> Self {
        Self { registry }
    }

    /// Parse `source` using the plugin identified by `plugin_id`.
    ///
    /// Returns an error if the plugin is unknown or if tree-sitter
    /// rejects the language (should never happen for a compiled-in
    /// grammar).
    pub fn parse(&self, plugin_id: &'static str, source: &[u8]) -> Result<ParseOutcome> {
        let plugin = self
            .registry
            .by_id(plugin_id)
            .ok_or_else(|| anyhow!("unknown plugin id: {plugin_id}"))?;
        let lang = plugin.ts_language();
        let start = Instant::now();
        let tree = PARSERS.with(|cell| -> Result<Tree> {
            let mut map = cell.borrow_mut();
            let parser = map.entry(plugin_id).or_insert_with(Parser::new);
            set_language(parser, &lang)?;
            parser
                .parse(source, None)
                .ok_or_else(|| anyhow!("tree-sitter returned no tree for {plugin_id}"))
        })?;
        Ok(ParseOutcome {
            tree,
            parse_ms: start.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Look up a plugin by id. Convenience for Task 4+ call sites.
    pub fn plugin(&self, plugin_id: &str) -> Option<&'r dyn LanguagePlugin> {
        self.registry.by_id(plugin_id)
    }
}

#[inline]
fn set_language(parser: &mut Parser, lang: &Language) -> Result<()> {
    parser
        .set_language(lang)
        .map_err(|e| anyhow!("set_language failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_v1_languages() {
        let reg = PluginRegistry::with_v1_plugins();
        let pool = ParserPool::new(&reg);

        // Minimal viable fragments; each should parse cleanly.
        let cases: &[(&str, &[u8])] = &[
            ("rust", b"fn main() {}"),
            ("python", b"def f():\n    pass\n"),
            ("javascript", b"function f() { return 1; }"),
            ("typescript", b"function f(): number { return 1; }"),
            ("go", b"package main\nfunc main() {}\n"),
            ("java", b"class A { void m() {} }"),
            ("c", b"int main(void) { return 0; }"),
            ("cpp", b"int main() { return 0; }"),
            ("csharp", b"class A { void M() {} }"),
        ];
        for (id, src) in cases {
            let out = pool.parse(id, src).expect("parse");
            let root = out.tree.root_node();
            assert!(!root.has_error(), "parse error in {id}: {:?}", root);
            assert!(out.parse_ms >= 0.0);
        }
    }

    #[test]
    fn amortizes_across_calls() {
        let reg = PluginRegistry::with_v1_plugins();
        let pool = ParserPool::new(&reg);
        // First call allocates, second reuses. No panic is the check.
        for _ in 0..3 {
            let out = pool.parse("rust", b"fn x(){}").unwrap();
            assert!(!out.tree.root_node().has_error());
        }
    }

    #[test]
    fn unknown_plugin_errors() {
        let reg = PluginRegistry::with_v1_plugins();
        let pool = ParserPool::new(&reg);
        let err = pool.parse("klingon", b"qapla").unwrap_err().to_string();
        assert!(err.contains("unknown plugin"));
    }
}
