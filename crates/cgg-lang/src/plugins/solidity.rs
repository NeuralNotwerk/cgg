//! Solidity plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

#[derive(Debug)]
pub struct SolidityPlugin;

impl LanguagePlugin for SolidityPlugin {
    fn id(&self) -> &'static str {
        "solidity"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".sol"]
    }
    fn signals(&self) -> crate::PluginSignals {
        crate::PluginSignals {
            visibility: true,
            ..Default::default()
        }
    }

    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_solidity::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "solidity");
        let mut w = SolidityWalker {
            source,
            facts: &mut facts,
            scope: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct SolidityWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
}

impl<'a> SolidityWalker<'a> {
    fn text(&self, n: Node) -> &str {
        n.utf8_text(self.source).unwrap_or("")
    }
    fn qn(&self, simple: &str) -> String {
        if self.scope.is_empty() {
            simple.into()
        } else {
            format!("{}.{simple}", self.scope.join("."))
        }
    }
    fn child_kind<'n>(&self, node: Node<'n>, kind: &str) -> Option<Node<'n>> {
        let mut c = node.walk();
        if c.goto_first_child() {
            loop {
                if c.node().kind() == kind {
                    return Some(c.node());
                }
                if !c.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }

    fn walk(&mut self, node: Node) {
        match node.kind() {
            "contract_declaration" | "library_declaration" | "interface_declaration" => {
                let name = self
                    .child_kind(node, "identifier")
                    .map(|n| self.text(n).to_string())
                    .unwrap_or_default();
                if !name.is_empty() {
                    self.scope.push(name);
                    self.walk_children(node);
                    self.scope.pop();
                } else {
                    self.walk_children(node);
                }
                return;
            }
            "function_definition" | "modifier_definition" => {
                self.record_function(node, false);
                self.walk_children(node);
                return;
            }
            "constructor_definition" => {
                self.record_function(node, true);
                self.walk_children(node);
                return;
            }
            "import_directive" => {
                if let Some(s) = self.child_kind(node, "string") {
                    let raw = self.text(s);
                    let path = raw
                        .trim_matches(|c: char| c == '"' || c == '\'')
                        .to_string();
                    if !path.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "import".into(),
                            path,
                            alias: String::new(),
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
                return;
            }
            "call_expression" => {
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

    fn record_function(&mut self, node: Node, is_ctor: bool) {
        let name = if is_ctor {
            "constructor".to_string()
        } else {
            self.child_kind(node, "identifier")
                .map(|n| self.text(n).to_string())
                .unwrap_or_default()
        };
        if name.is_empty() {
            return;
        }
        let variant = if is_ctor {
            DefVariant::Constructor
        } else if self.scope.is_empty() {
            DefVariant::FreeFunction
        } else {
            DefVariant::InherentMethod
        };
        let qn = self.qn(&name);
        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        let visibility = self
            .child_kind(node, "visibility")
            .map(|n| self.text(n).to_string())
            .unwrap_or_default();
        // Normalize onto the shared enum as well as keeping the native
        // string. Without this every Solidity callable reads as
        // `Vis::Unknown`, which caps dead-code confidence for the whole
        // language — the analysis cannot claim "no out-of-tree caller
        // can exist" for a function it does not know is private.
        let vis = match visibility.as_str() {
            "public" | "external" => cgg_core::facts::Vis::Public,
            "internal" => cgg_core::facts::Vis::Internal,
            "private" => cgg_core::facts::Vis::Private,
            _ => cgg_core::facts::Vis::Unknown,
        };
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant,
            start_line: sl,
            end_line: el,
            start_byte: node.start_byte() as u32,
            end_byte: node.end_byte() as u32,
            signature_hint: super::extract_signature(self.text(node)),
            visibility,
            vis,
            attributes: Vec::new(),
            ..Default::default()
        });
    }

    fn record_call(&mut self, node: Node) {
        // call_expression -> expression -> (identifier | member_expression)
        let Some(callee) = node.child(0) else { return };
        let inner = if callee.kind() == "expression" {
            callee.child(0).unwrap_or(callee)
        } else {
            callee
        };
        let (name, receiver) = match inner.kind() {
            "identifier" => (self.text(inner).to_string(), String::new()),
            "member_expression" => {
                // member_expression has two identifier-bearing children:
                // object (first) and property (last). Tree-sitter-solidity
                // doesn't expose a field name; pick by position.
                let mut idents: Vec<Node> = inner
                    .children(&mut inner.walk())
                    .filter(|c| c.kind() == "identifier" || c.kind() == "expression")
                    .collect();
                let property = idents
                    .pop()
                    .map(|n| {
                        if n.kind() == "expression" {
                            n.child(0)
                                .map(|c| self.text(c).to_string())
                                .unwrap_or_default()
                        } else {
                            self.text(n).to_string()
                        }
                    })
                    .unwrap_or_default();
                let object = idents
                    .into_iter()
                    .next()
                    .map(|n| {
                        if n.kind() == "expression" {
                            n.child(0)
                                .map(|c| self.text(c).to_string())
                                .unwrap_or_default()
                        } else {
                            self.text(n).to_string()
                        }
                    })
                    .unwrap_or_default();
                (property, object)
            }
            _ => (self.text(inner).to_string(), String::new()),
        };
        if name.is_empty() {
            return;
        }
        self.facts.references.push(RefRecord {
            name,
            receiver_hint: receiver,
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
        p.set_language(&tree_sitter_solidity::LANGUAGE.into())
            .unwrap();
        let tree = p.parse(src, None).unwrap();
        SolidityPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/x.sol"),
            &tree,
            src.as_bytes(),
        )
    }

    #[test]
    fn contract_methods_and_ctor() {
        let src = "contract Token { constructor() {} function transfer(address to) public returns (bool) { return _send(to); } function _send(address t) internal returns (bool) { return true; } }";
        let f = extract(src);
        let by: std::collections::HashMap<_, _> = f
            .definitions
            .iter()
            .map(|d| (d.qualified_name.clone(), d.variant))
            .collect();
        assert_eq!(
            by.get("Token.constructor"),
            Some(&DefVariant::Constructor),
            "by: {by:?}"
        );
        assert_eq!(by.get("Token.transfer"), Some(&DefVariant::InherentMethod));
        assert_eq!(by.get("Token._send"), Some(&DefVariant::InherentMethod));
    }

    #[test]
    fn library_function_is_free() {
        let src = "library M { function add(uint a, uint b) internal pure returns (uint) { return a + b; } }";
        let f = extract(src);
        // M is pushed to scope so it's qualified.
        assert!(
            f.definitions.iter().any(|d| d.qualified_name == "M.add"),
            "defs: {:?}",
            f.definitions
        );
    }

    #[test]
    fn import_captured() {
        let src = "import \"./Other.sol\";";
        let f = extract(src);
        assert!(
            f.imports.iter().any(|i| i.path == "./Other.sol"),
            "imports: {:?}",
            f.imports
        );
    }

    #[test]
    fn call_captured() {
        let src = "contract T { function f() public { g(); h.k(); } }";
        let f = extract(src);
        assert!(
            f.references.iter().any(|r| r.name == "g"),
            "refs: {:?}",
            f.references
        );
        assert!(
            f.references
                .iter()
                .any(|r| r.name == "k" && r.receiver_hint == "h"),
            "refs: {:?}",
            f.references
        );
    }
}
