//! TypeScript plugin — callable extraction.
//!
//! Reuses the JavaScript walker since tree-sitter-typescript produces
//! the same node kinds for callables, imports, and call expressions
//! (it just adds type annotations which the walker ignores).

use std::path::Path;

use cgg_core::{FileFacts, ids::FileId};
use tree_sitter::Tree;

use super::javascript::JsWalker;
use crate::LanguagePlugin;

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

    /// `.ts` / `.mts` / `.cts` files are parsed with the TypeScript
    /// grammar, `.tsx` (and anything else) with the TSX one. The two
    /// disagree on `<`: in a `.ts` file `<Foo>x` is a type assertion and
    /// `<T>(x: T) => x` a generic arrow function, and the TSX grammar
    /// reads both as an unclosed JSX element, turning the rest of the
    /// file into one error node whose calls were never recorded.
    fn ts_language_for(&self, path: &Path) -> tree_sitter::Language {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if matches!(ext, "ts" | "mts" | "cts") {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        } else {
            self.ts_language()
        }
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "typescript");
        let manifest = super::manifest::js_module_manifest(path);
        facts.packages = manifest.names;
        let own_package = manifest.own;
        let exported_as = manifest.exported_as;
        let mut w = JsWalker::new(*ctx, source, &mut facts);
        w.walk(tree.root_node());
        let mut out = facts;
        super::javascript::push_package_name(&mut out, own_package, exported_as);
        // Same passes the JavaScript plugin runs. They were missing
        // here, which meant TypeScript silently produced none of the
        // dead-code signals its `signals()` manifest now declares.
        if ctx.deadcode_signals {
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
    use cgg_core::{DefVariant, ids::FileId};
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        TypeScriptPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.ts"),
            &tree,
            src.as_bytes(),
        )
    }

    /// A `.ts` file is parsed with the TypeScript grammar: `<Foo>o` is a
    /// type assertion there, where the TSX grammar opened a JSX element
    /// and lost every later call. A `.tsx` file keeps the TSX grammar.
    #[test]
    fn ts_files_use_the_typescript_grammar() {
        let src = "function r(o: any) {\n  (<Foo>o).m();\n  later();\n}\n";
        let path = PathBuf::from("/tmp/__cgg_test__/x.ts");
        let mut p = Parser::new();
        p.set_language(&TypeScriptPlugin.ts_language_for(&path))
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        assert!(!tree.root_node().has_error());
        let f = TypeScriptPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &path,
            &tree,
            src.as_bytes(),
        );
        let m = f.references.iter().find(|r| r.name == "m").unwrap();
        assert_eq!((m.receiver_hint.as_str(), m.typed_receiver), ("Foo", true));
        assert!(f.references.iter().any(|r| r.name == "later"));
        let tsx = PathBuf::from("/tmp/__cgg_test__/x.tsx");
        let mut p = Parser::new();
        p.set_language(&TypeScriptPlugin.ts_language_for(&tsx))
            .unwrap();
        assert!(
            !p.parse("const a = <div/>;\n", None)
                .unwrap()
                .root_node()
                .has_error()
        );
    }

    /// `import x = require('./m')` binds `x` to the module, like a
    /// CommonJS `const x = require('./m')` (FINDINGS A4).
    #[test]
    fn import_equals_require_binds_the_module() {
        let f = extract("import x = require('./cjs');\nx.run();\n");
        let imp: Vec<(&str, &str, &str)> = f
            .imports
            .iter()
            .map(|i| (i.kind.as_str(), i.path.as_str(), i.alias.as_str()))
            .collect();
        assert_eq!(imp, vec![("import", "./cjs", "x")]);
    }

    /// `export = value` is CommonJS `module.exports = value`: a name, a
    /// required module, or a value cgg cannot read (FINDINGS R12).
    #[test]
    fn export_assignment_is_the_default_export() {
        let records = |src: &str| -> Vec<(String, String, String)> {
            extract(src)
                .imports
                .iter()
                .filter(|i| i.kind == "local-export" || i.kind == "re-export")
                .map(|i| (i.kind.clone(), i.path.clone(), i.alias.clone()))
                .collect()
        };
        let own =
            |k: &str, p: &str, a: &str| (k.to_string(), p.to_string(), a.to_string());
        assert_eq!(
            records("function load() {}\nexport = load;\n"),
            vec![own("local-export", "", "load as default")]
        );
        assert_eq!(
            records("export = require('./impl');\n"),
            vec![own("re-export", "./impl", "*, default")]
        );
        assert_eq!(
            records("import { make } from './m';\nexport = make();\n"),
            vec![own("local-export", "", "<value> as default")]
        );
    }

    #[test]
    fn typed_function_declaration() {
        let src = "function greet(name: string): void {}\nasync function fetch(): Promise<void> {}\n";
        let f = extract(src);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
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
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.clone(), d.variant))
            .collect();
        assert_eq!(by["constructor"], DefVariant::Constructor);
        assert_eq!(by["run"], DefVariant::InherentMethod);
        assert_eq!(by["create"], DefVariant::StaticMethod);
    }

    #[test]
    fn class_member_visibility_is_read_from_the_grammar() {
        // FINDINGS C5: TypeScript recorded no visibility at all.
        use cgg_core::Vis;
        let src = r#"
export class Service {
  constructor(private readonly name: string) {}
  private hidden(): void {}
  protected hook(): void {}
  public shown(): void {}
  plain(): void {}
  #hard(): void {}
  @Decorated("public")
  private decorated(): void {}
  private static make(): void {}
  private get value(): string { return ""; }
}
class Singleton { private constructor() {} }
"#;
        let f = extract(src);
        let vis = |qn: &str| {
            f.definitions
                .iter()
                .find(|d| d.qualified_name == qn)
                .unwrap_or_else(|| panic!("{qn} not extracted"))
                .vis
        };
        assert_eq!(vis("Service.hidden"), Vis::Private);
        assert_eq!(vis("Service.hook"), Vis::Protected);
        assert_eq!(vis("Service.shown"), Vis::Public);
        // No modifier: public, the language default for class members.
        assert_eq!(vis("Service.plain"), Vis::Public);
        // ECMAScript private name.
        assert_eq!(vis("Service.#hard"), Vis::Private);
        // A decorator argument is not a modifier.
        assert_eq!(vis("Service.decorated"), Vis::Private);
        assert_eq!(vis("Service.make"), Vis::Private);
        assert_eq!(vis("Service.value"), Vis::Private);
        // A parameter property's `private` belongs to the field, not to
        // the constructor.
        assert_eq!(vis("Service.constructor"), Vis::Public);
        assert_eq!(vis("Singleton.constructor"), Vis::Private);
        // The native token stays empty, so the capability table does not
        // read class members as module-level visibility support.
        assert!(f.definitions.iter().all(|d| d.visibility.is_empty()));
    }

    #[test]
    fn functions_and_object_methods_have_no_visibility() {
        // Near-miss: module-level visibility (`export`) is not modelled,
        // and an object literal's method has none of its own.
        let src = "export function f(): void {}\nfunction g(): void {}\nconst h = (): void => {};\nconst o = { m(): void {} };\n";
        let f = extract(src);
        for name in ["f", "g", "h", "m"] {
            let d = f
                .definitions
                .iter()
                .find(|d| d.simple_name == name)
                .unwrap_or_else(|| panic!("{name} not extracted"));
            assert_eq!(d.vis, cgg_core::Vis::Unknown, "{name}");
        }
    }

    #[test]
    fn import_from_captured() {
        let src =
            "import { helper } from './utils';\nimport type { Config } from './types';\n";
        let f = extract(src);
        // type-only imports are still import_statement nodes.
        assert!(f.imports.iter().any(|i| i.path == "./utils"));
    }

    #[test]
    fn call_expressions() {
        let src = "function f() { greet('x'); obj.run(); }\n";
        let f = extract(src);
        assert!(f.references.iter().any(|r| r.name == "greet"));
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "run" && r.receiver_hint == "obj")
        );
    }

    #[test]
    fn interface_and_abstract_members_are_bodiless_declarations() {
        let f = extract(
            "interface Shape extends Base { area(): number; size: number; }\nabstract class A extends B implements Shape, L<X> { abstract render(): void; draw() { this.render(); } }\ntype T = { z(): void };\n",
        );
        let get = |qn: &str| f.definitions.iter().find(|d| d.qualified_name == qn);
        let area = get("Shape.area").expect("interface member");
        assert_eq!(area.variant, DefVariant::TraitMethod);
        assert!(area.attributes.iter().any(|a| a == "abstract"));
        assert_eq!(area.base_types, vec!["Base"]);
        assert!(get("A.render").is_some(), "abstract member");
        // The abstract class owns its concrete members too.
        let draw = get("A.draw").expect("abstract class scope");
        assert_eq!(draw.base_types, vec!["B", "Shape", "L<X>"]);
        // An object type's member belongs to no class; a property is not
        // a method.
        assert!(f.definitions.iter().all(|d| d.simple_name != "z"));
        assert!(f.definitions.iter().all(|d| d.simple_name != "size"));
    }
}
