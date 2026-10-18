//! Solidity plugin — callable extraction.

use crate::LanguagePlugin;
use cgg_core::{DefRecord, DefVariant, FileFacts, ImportRecord, RefRecord, ids::FileId};
use std::path::Path;
use tree_sitter::{Node, Tree};

/// Attribute on a function declared inside a `library`.
pub const LIBRARY_MEMBER: &str = "library-member";

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
            in_library: Vec::new(),
        };
        w.walk(tree.root_node());
        facts
    }
}

struct SolidityWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
    scope: Vec<String>,
    /// Per entry of `scope`: whether it is a `library`. A library's
    /// functions are called through the library (`L.f(x)`) or attached
    /// to a type (`using L for T`), never by bare name from outside it.
    in_library: Vec<bool>,
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
                    self.in_library.push(node.kind() == "library_declaration");
                    self.walk_children(node);
                    self.scope.pop();
                    self.in_library.pop();
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
                self.record_base_constructor_calls(node);
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
            // `using L for T;` / `using L for *;`: library `L`'s functions
            // are callable as members of `T` (of every type for `*`),
            // `x.f(a)` meaning `L.f(x, a)`. Recorded as an import of
            // kind `using-for`: path `L`, alias `T` (or `*`).
            "using_directive" => {
                if let Some(lib) = self.child_kind(node, "type_alias") {
                    let target = node
                        .child_by_field_name("source")
                        .map(|t| self.text(t).trim().to_string())
                        .unwrap_or_else(|| {
                            let mut c = node.walk();
                            let any = node.children(&mut c).any(|ch| ch.kind() == "*");
                            if any { "*".to_string() } else { String::new() }
                        });
                    let lib = self.text(lib).trim().to_string();
                    if !lib.is_empty() && !target.is_empty() {
                        self.facts.imports.push(ImportRecord {
                            kind: "using-for".into(),
                            path: lib,
                            alias: target,
                            site_line: (node.start_position().row as u32) + 1,
                            site_byte: node.start_byte() as u32,
                        });
                    }
                }
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
            attributes: if self.in_library.last() == Some(&true) {
                vec![LIBRARY_MEMBER.to_string()]
            } else {
                Vec::new()
            },
            ..Default::default()
        });
    }

    /// A call at `site` of contract `ty`'s constructor.
    fn push_constructor_call(&mut self, site: Node, ty: String) {
        self.facts.references.push(RefRecord {
            name: "constructor".to_string(),
            receiver_hint: ty,
            site_line: (site.start_position().row as u32) + 1,
            site_byte: site.start_byte() as u32,
            typed_receiver: true,
            ..Default::default()
        });
    }

    /// `constructor(…) Base(x) {}`: an invocation in a constructor's
    /// header that names one of the contract's bases calls that base's
    /// constructor. Any other name there is a modifier.
    fn record_base_constructor_calls(&mut self, ctor: Node) {
        let Some(contract) = std::iter::successors(ctor.parent(), |n| n.parent())
            .find(|n| n.kind() == "contract_declaration")
        else {
            return;
        };
        let bases: Vec<String> = contract
            .named_children(&mut contract.walk())
            .filter(|c| c.kind() == "inheritance_specifier")
            .filter_map(|c| {
                c.named_children(&mut c.walk())
                    .find(|t| t.kind() == "user_defined_type")
                    .map(|t| self.text(t).trim().to_string())
            })
            .collect();
        let invocations: Vec<Node> = ctor
            .named_children(&mut ctor.walk())
            .filter(|c| c.kind() == "modifier_invocation")
            .collect();
        for inv in invocations {
            let Some(name) = inv
                .named_children(&mut inv.walk())
                .find(|c| c.kind() == "identifier")
                .map(|c| self.text(c).to_string())
            else {
                continue;
            };
            if bases.iter().any(|b| b == &name) {
                self.push_constructor_call(inv, name);
            }
        }
    }

    fn record_call(&mut self, node: Node) {
        // call_expression -> expression -> (identifier | member_expression)
        let Some(callee) = node.child(0) else { return };
        let inner = if callee.kind() == "expression" {
            callee.child(0).unwrap_or(callee)
        } else {
            callee
        };
        // `new C(…)`: contract `C`'s constructor, looked up on `C`.
        if inner.kind() == "new_expression" {
            if let Some(ty) = inner
                .named_children(&mut inner.walk())
                .find(|c| c.kind() == "type_name")
                .and_then(|t| t.named_child(0))
                .filter(|t| t.kind() == "user_defined_type")
                .map(|t| self.text(t).trim().to_string())
                .filter(|t| !t.is_empty())
            {
                self.push_constructor_call(node, ty);
            }
            return;
        }
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

    /// WS16 (b): a library's functions are marked; a contract's are not.
    #[test]
    fn library_functions_are_marked() {
        let f = extract(
            "library L { function a() internal {} }\ncontract C { function b() public {} }\n",
        );
        let marked: Vec<(&str, bool)> = f
            .definitions
            .iter()
            .map(|d| {
                (
                    d.qualified_name.as_str(),
                    d.attributes.iter().any(|a| a == LIBRARY_MEMBER),
                )
            })
            .collect();
        assert_eq!(marked, vec![("L.a", true), ("C.b", false)]);
    }

    #[test]
    fn new_and_base_constructor_invocations_call_constructors() {
        let f = extract(
            "contract B { constructor(uint x) {} }\n\
             contract C is B { constructor(uint y) B(y) onlyOwner {} function m() public { C c = new C(1); uint[] memory a = new uint[](3); } }",
        );
        let refs: Vec<(&str, &str, bool)> = f
            .references
            .iter()
            .map(|r| (r.name.as_str(), r.receiver_hint.as_str(), r.typed_receiver))
            .collect();
        assert!(refs.contains(&("constructor", "B", true)), "{refs:?}");
        assert!(refs.contains(&("constructor", "C", true)), "{refs:?}");
        // Near miss: a modifier is not a base, and `new uint[]` no contract.
        assert_eq!(refs.len(), 2, "{refs:?}");
    }
}
