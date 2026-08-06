//! TypeScript plugin — callable extraction.
//!
//! Reuses the JavaScript walker since tree-sitter-typescript produces
//! the same node kinds for callables, imports, and call expressions
//! (it just adds type annotations which the walker ignores).

use std::path::Path;

use cgg_core::{ids::FileId, FileFacts};
use tree_sitter::Tree;

use crate::LanguagePlugin;
use super::javascript::JsWalker;

#[derive(Debug)]
pub struct TypeScriptPlugin;

impl LanguagePlugin for TypeScriptPlugin {
    fn id(&self) -> &'static str {
        "typescript"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".ts", ".tsx", ".mts", ".cts"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["ts-node", "tsx", "deno", "bun"]
    }
    /// The TypeScript plugin reuses `JsWalker` wholesale, so it produces
    /// exactly the signals the JavaScript plugin does. Declaring the
    /// default here would tell the dead-code capability table that cgg
    /// never looked for TypeScript decorators — the difference between a
    /// finding and a guess, and wrong in the direction that hides a gap.
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            unreachable: true,
            dyn_uses: true,
            attributes: true,
            impls: true,
            value_refs: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_typescript::LANGUAGE_TSX.into()
    }

    fn extract(
        &self,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "typescript");
        let mut w = JsWalker::new(source, &mut facts);
        w.walk(tree.root_node());
        let mut out = facts;
        // Same passes the JavaScript plugin runs. They were missing
        // here, which meant TypeScript silently produced none of the
        // dead-code signals its `signals()` manifest now declares.
        if crate::deadcode_signals() {
            out.unreachable =
                super::cfg::unreachable_after_terminator(tree, &super::cfg::JS);
            out.dyn_uses = super::dynuse::extract(tree, source, "typescript");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::{ids::FileId, DefVariant};
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        TypeScriptPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.ts"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn typed_function_declaration() {
        let src = "function greet(name: string): void {}\nasync function fetch(): Promise<void> {}\n";
        let f = extract(src);
        let names: Vec<&str> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
        assert!(names.contains(&"greet"), "got: {names:?}");
        assert!(names.contains(&"fetch"), "got: {names:?}");
    }

    #[test]
    fn typed_arrow_function() {
        let src = "const add = (a: number, b: number): number => a + b;\n";
        let f = extract(src);
        assert!(f.definitions.iter().any(|d| d.simple_name == "add"));
    }

    #[test]
    fn class_with_types() {
        let src = r#"
class Service {
    private name: string;
    constructor(name: string) { this.name = name; }
    run(): void {}
    static create(): Service { return new Service(""); }
}
"#;
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f.definitions.iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(by["constructor"], DefVariant::Constructor);
        assert_eq!(by["run"], DefVariant::InherentMethod);
        assert_eq!(by["create"], DefVariant::StaticMethod);
    }

    #[test]
    fn import_from_captured() {
        let src = "import { helper } from './utils';\nimport type { Config } from './types';\n";
        let f = extract(src);
        // type-only imports are still import_statement nodes.
        assert!(f.imports.iter().any(|i| i.path == "./utils"));
    }

    #[test]
    fn call_expressions() {
        let src = "function f() { greet('x'); obj.run(); }\n";
        let f = extract(src);
        assert!(f.references.iter().any(|r| r.name == "greet"));
        assert!(f.references.iter().any(|r| r.name == "run" && r.receiver_hint == "obj"));
    }
}
