//! PHP plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct PhpPlugin;

impl LanguagePlugin for PhpPlugin {
    fn id(&self) -> &'static str {
        "php"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".php", ".phtml"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &["php"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            dyn_uses: true,
            attributes: true,
            impls: true,
            value_refs: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_php::LANGUAGE_PHP.into()
    }

    fn extract(
        &self,
        ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "php");
        let mut w = PhpWalker {
            ctx: *ctx,
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
            namespace: String::new(),
        };
        w.walk(tree.root_node());
        if ctx.deadcode_signals {
            facts.dyn_uses = super::dynuse::extract(tree, source, "php");
        }
        facts
    }
}

struct PhpWalker<'a> {
    source: &'a [u8],
    /// Per-run extraction switches; see `crate::ExtractCtx`.
    ctx: crate::ExtractCtx<'a>,
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// `extends`/`implements` of the enclosing class, innermost last.
    bases: Vec<Vec<String>>,
    /// The namespace the walk is in (`namespace App\Http;`).
    namespace: String,
}

impl<'a> PhpWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.to_string()
        } else {
            format!("{}::{simple}", self.scope.join("::"))
        }
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "namespace_use_declaration" | "namespace_definition" => {
                // PHP's only import record. Without it every Symfony,
                // Laravel and CodeIgniter rule is ungated and therefore
                // inert — detection is the premise the matchers rest on.
                self.record_use(node);
                self.walk_children(node);
                return;
            }
            "class_declaration"
            | "interface_declaration"
            | "trait_declaration"
            | "enum_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                // Every type the file declares, by its fully qualified
                // name, so a `use` of it can be told from a `use` of a
                // dependency's (WS14). Bases stay on the methods.
                if !name.is_empty() && self.scope.is_empty() {
                    let fqn = if self.namespace.is_empty() {
                        name.clone()
                    } else {
                        format!("{}\\{name}", self.namespace)
                    };
                    self.facts.classes.push(cgg_core::ClassDecl {
                        class_qn: fqn,
                        base_types: Vec::new(),
                        line: (node.start_position().row as u32) + 1,
                    });
                }
                // `use T1, T2;` in the body mixes the traits' methods in:
                // `$this->m()` reaches them as it reaches a base's.
                let mut bases = super::attrs::base_types(node, self.source);
                if let Some(body) = node.child_by_field_name("body") {
                    let mut c = body.walk();
                    for u in body
                        .named_children(&mut c)
                        .filter(|n| n.kind() == "use_declaration")
                    {
                        let mut uc = u.walk();
                        for n in u
                            .named_children(&mut uc)
                            .filter(|n| matches!(n.kind(), "name" | "qualified_name"))
                        {
                            let t = self.text(n).to_string();
                            if !bases.contains(&t) {
                                bases.push(t);
                            }
                        }
                    }
                }
                self.bases.push(bases);
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                self.bases.pop();
                return;
            }
            "function_definition" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::FreeFunction);
                    if is_conditional(node)
                        && let Some(d) = self.facts.definitions.last_mut()
                    {
                        d.conditional = true;
                    }
                }
                self.walk_children(node);
                return;
            }
            "method_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.record_def(&name, node, DefVariant::InherentMethod);
                }
                self.walk_children(node);
                return;
            }
            // `require_once __DIR__ . '/x.php';`: the file a statement
            // loads (WS14), so copies of one function in several files can
            // be told apart by what the caller loads.
            "require_expression"
            | "require_once_expression"
            | "include_expression"
            | "include_once_expression" => {
                if let Some(arg) = node.named_child(0) {
                    let path = self.text(arg).trim().to_string();
                    if !path.is_empty() && path.len() <= 400 {
                        self.facts.imports.push(ImportRecord {
                            kind: "php-require".into(),
                            path,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                self.walk_children(node);
                return;
            }
            "function_call_expression"
            | "member_call_expression"
            // `$x?->m()` is `$x->m()` when `$x` is not null.
            | "nullsafe_member_call_expression"
            | "scoped_call_expression" => {
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

    fn record_def(&mut self, name: &str, node: Node, variant: DefVariant) {
        let qn = self.qn(name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name.to_string(),
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility: String::new(),
            // Symfony puts its whole routing surface in `#[Route(...)]`.
            attributes: super::attrs::collect(node, self.source),
            base_types: self.bases.last().cloned().unwrap_or_default(),
            ..Default::default()
        });
    }

    /// `use App\Http\Controllers\C;` / `use function ns\f;` /
    /// `namespace App;`
    fn record_use(&mut self, node: Node) {
        let raw = self.text(node).trim().to_string();
        let body = raw
            .trim_start_matches("namespace")
            .trim_start_matches("use")
            .trim()
            .trim_start_matches("function")
            .trim_start_matches("const")
            .trim()
            .trim_end_matches(&[';', '{'][..])
            .trim();
        if body.is_empty() || body.len() > 400 {
            return;
        }
        let kind = if raw.starts_with("namespace") {
            "package"
        } else {
            "use"
        };
        if kind == "package" {
            self.namespace = body.trim_start_matches('\\').to_string();
        }
        // `use A\{B, C as D};`: each item under the group's prefix.
        let expanded: Vec<String> = match (body.find('{'), body.rfind('}')) {
            (Some(open), Some(close)) if open < close => {
                let prefix = body[..open].trim();
                body[open + 1..close]
                    .split(',')
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .map(|p| format!("{prefix}{p}"))
                    .collect()
            }
            _ => body.split(',').map(|p| p.trim().to_string()).collect(),
        };
        for part in &expanded {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let (path, alias) = match part.split_once(" as ") {
                Some((p, a)) => (p.trim().to_string(), a.trim().to_string()),
                None => (part.to_string(), String::new()),
            };
            if path.is_empty() {
                continue;
            }
            self.facts.imports.push(ImportRecord {
                kind: kind.into(),
                path,
                alias,
                site_line: (node.start_position().row as u32) + 1,
                site_byte: node.start_byte() as u32,
            });
        }
    }

    fn record_call(&mut self, node: Node) {
        // `f()` and `$o->m()` name the callee under `function`;
        // `C::m()` names it under `name`, with the class under `scope`.
        // Reading only `function` silently dropped every static call,
        // which is the entire Laravel routing vocabulary.
        let func = node
            .child_by_field_name("function")
            .or_else(|| node.child_by_field_name("name"))
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        if func.is_empty() {
            return;
        }

        // require_once/include as import
        if func == "require_once"
            || func == "include"
            || func == "include_once"
            || func == "require"
        {
            let args = node.child_by_field_name("arguments");
            if let Some(a) = args.and_then(|a| a.child(0)) {
                let path = self
                    .text(a)
                    .trim_matches('\'')
                    .trim_matches('"')
                    .to_string();
                if !path.is_empty() {
                    self.facts.imports.push(ImportRecord {
                        kind: func.clone(),
                        path,
                        alias: String::new(),
                        site_line: (node.start_position().row as u32) + 1,
                        site_byte: node.start_byte() as u32,
                    });
                    return;
                }
            }
        }

        // `($f)->m()`, `(new Foo())->m()`: the value or the class the
        // receiver's shape stands for (`crate::receiver`).
        let mut typed_receiver = false;
        let recv = node
            .child_by_field_name("object")
            .map(|n| {
                let p = crate::receiver::peel("php", n, self.source);
                match p.type_name() {
                    Some(t) => {
                        typed_receiver = true;
                        t
                    }
                    None => self.text(p.node).to_string(),
                }
            })
            .or_else(|| {
                node.child_by_field_name("scope")
                    .map(|n| self.text(n).to_string())
            })
            .unwrap_or_default();
        // `$this->m()` is a call on the enclosing class's instance: spelled
        // `this` the way every other language spells it, so the resolver's
        // own-class reading applies (the written `$this` matched nothing).
        let recv = if recv == "$this" {
            "this".to_string()
        } else {
            recv
        };
        let context = if recv.is_empty() {
            func.clone()
        } else {
            format!("{recv}::{func}")
        };
        self.facts.references.push(RefRecord {
            name: func,
            receiver_hint: recv,
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
            typed_receiver,
            ..Default::default()
        });
        // Laravel needs BOTH `'Controller@method'` strings (<= L7) and
        // `[Controller::class, 'method']` arrays (>= L8); WordPress
        // needs `add_action('hook', 'fn')`. All three arrive here.
        let extra = super::registrar::capture(&self.ctx, node, self.source, &context);
        self.facts.references.extend(extra);
    }
}

/// Whether a function declaration sits in a conditional statement of
/// the code around it (`if (!function_exists('f')) { function f() … }`),
/// rather than unconditionally at the top of its file or namespace.
fn is_conditional(def: Node) -> bool {
    let mut cur = def.parent();
    while let Some(n) = cur {
        match n.kind() {
            "if_statement" | "else_clause" | "else_if_clause" => return true,
            "function_definition"
            | "method_declaration"
            | "class_declaration"
            | "program" => return false,
            _ => cur = n.parent(),
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::ids::FileId;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        PhpPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.php"),
            &tree,
            src.as_bytes(),
        )
    }

    // WS18: stacked attribute groups are separate attributes, so
    // `#[Test]` under `#[Group('x')]` is still seen.
    #[test]
    fn stacked_attribute_groups_are_separate_attributes() {
        let f = extract(
            "<?php\nclass T { #[Group('dav')]\n #[Test]\n public function it_adds() {} #[Test] public function b() {} }\n",
        );
        assert_eq!(
            f.definitions[0].attributes,
            vec!["#[Group('dav')]", "#[Test]"]
        );
        assert_eq!(f.definitions[1].attributes, vec!["#[Test]"]);
    }

    #[test]
    fn used_traits_are_bases() {
        let f =
            extract("<?php\nclass A extends B { use T1, \\N\\T2; function m() {} }\n");
        assert_eq!(f.definitions[0].base_types, vec!["B", "T1", "\\N\\T2"]);
    }

    #[test]
    fn this_receiver_is_spelled_this() {
        let f = extract(
            "<?php\nclass A { function r() { $this->h(); $o->h(); } function h() {} }\n",
        );
        let recv: Vec<&str> = f
            .references
            .iter()
            .filter(|r| r.name == "h")
            .map(|r| r.receiver_hint.as_str())
            .collect();
        assert_eq!(recv, vec!["this", "$o"]);
    }

    #[test]
    fn plugin_loads() {
        let plugin = PhpPlugin;
        assert_eq!(plugin.id(), "php");
        assert!(plugin.extensions().contains(&".php"));
        assert!(plugin.shebangs().contains(&"php"));
    }

    #[test]
    fn extracts_definitions() {
        let src = "<?php\nclass Service {\n  public function run() {}\n}\n";
        let f = extract(src);
        assert!(!f.definitions.is_empty(), "should extract definitions");
    }

    #[test]
    fn extracts_references() {
        let src = "<?php\nfunction main() { greet(); }\n";
        let f = extract(src);
        assert!(!f.references.is_empty(), "should extract references");
    }

    /// WS14: a grouped `use` names each class under its prefix, and every
    /// class is recorded by its fully qualified name.
    #[test]
    fn grouped_use_and_class_fqns() {
        let f = extract(
            "<?php\nnamespace App\\Http;\nuse Foo\\{Bar, Baz as Q};\nclass C { function m() {} }\n",
        );
        let uses: Vec<(&str, &str)> = f
            .imports
            .iter()
            .filter(|i| i.kind == "use")
            .map(|i| (i.path.as_str(), i.alias.as_str()))
            .collect();
        assert_eq!(uses, vec![("Foo\\Bar", ""), ("Foo\\Baz", "Q")]);
        assert_eq!(f.classes.len(), 1);
        assert_eq!(f.classes[0].class_qn, "App\\Http\\C");
    }

    /// WS14b: a function declared inside an `if` (a `function_exists`
    /// fallback, either block syntax) is marked conditional; one at file
    /// level or in a namespace is not.
    #[test]
    fn a_guarded_function_is_conditional() {
        let f = extract(
            "<?php\nfunction a() {}\nif (!function_exists('b')) :\n  function b() {}\nendif;\n\
             if (!function_exists('c')) { function c() {} }\n",
        );
        let cond: Vec<(&str, bool)> = f
            .definitions
            .iter()
            .map(|d| (d.simple_name.as_str(), d.conditional))
            .collect();
        assert_eq!(
            cond,
            vec![("a", false), ("b", true), ("c", true)],
            "{cond:?}"
        );
    }
}
