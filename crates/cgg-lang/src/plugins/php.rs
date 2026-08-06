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
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "php");
        let mut w = PhpWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
            bases: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct PhpWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// `extends`/`implements` of the enclosing class, innermost last.
    bases: Vec<Vec<String>>,
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
            "class_declaration" | "interface_declaration" | "trait_declaration" => {
                let name = node
                    .child_by_field_name("name")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                self.bases.push(super::attrs::base_types(node, self.source));
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
            "function_call_expression"
            | "member_call_expression"
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
        for part in body.split(',') {
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

        let recv = node
            .child_by_field_name("object")
            .or_else(|| node.child_by_field_name("scope"))
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
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
            ..Default::default()
        });
        // Laravel needs BOTH `'Controller@method'` strings (<= L7) and
        // `[Controller::class, 'method']` arrays (>= L8); WordPress
        // needs `add_action('hook', 'fn')`. All three arrive here.
        let extra = super::registrar::capture(node, self.source, &context);
        self.facts.references.extend(extra);
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
        p.set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        PhpPlugin.extract(
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/x.php"),
            &tree,
            src.as_bytes(),
        )
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
}
