//! GraphQL SDL plugin — type-system topology extraction.
//!
//! Maps a GraphQL schema (SDL) onto cgg's callable/edge model:
//!
//! * `type`, `interface`, `input`, `union`, `enum`, and `scalar` type
//!   definitions become definitions (GraphQL has no namespaces, so the
//!   qualified name is just the type name).
//! * Every named-type occurrence inside a definition becomes a reference:
//!   field result types, field-argument input types, `implements`
//!   interfaces, and `union` members. This yields the type dependency
//!   graph (`type` → field-type, `type` → implemented interface,
//!   `union` → member types).
//!
//! The five built-in scalars (`String`, `Int`, `Float`, `Boolean`, `ID`)
//! are skipped as references — they resolve to nothing in-schema and would
//! only add leaf noise.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, RefRecord};
use tree_sitter::{Node, Tree};
use crate::{LanguagePlugin, ResolverKind};

const BUILTIN_SCALARS: &[&str] = &["String", "Int", "Float", "Boolean", "ID"];

/// Type-definition node kinds that introduce a named type.
const DEF_KINDS: &[(&str, &str)] = &[
    ("object_type_definition", "type"),
    ("interface_type_definition", "interface"),
    ("input_object_type_definition", "input"),
    ("union_type_definition", "union"),
    ("enum_type_definition", "enum"),
    ("scalar_type_definition", "scalar"),
];

#[derive(Debug)]
pub struct GraphqlPlugin;

impl LanguagePlugin for GraphqlPlugin {
    fn id(&self) -> &'static str { "graphql" }
    fn extensions(&self) -> &'static [&'static str] { &[".graphql", ".gql", ".graphqls"] }
    fn shebangs(&self) -> &'static [&'static str] { &[] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_graphql::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "graphql");
        let mut w = GraphqlWalker { source, facts: &mut facts };
        w.walk(tree.root_node());
        facts
    }
}

struct GraphqlWalker<'a> {
    source: &'a [u8],
    facts: &'a mut FileFacts,
}

impl<'a> GraphqlWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }

    fn walk(&mut self, node: Node) {
        if let Some((_, keyword)) = DEF_KINDS.iter().find(|(k, _)| *k == node.kind()) {
            self.record(node, keyword);
            return;
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } }
        }
    }

    fn record(&mut self, node: Node, keyword: &str) {
        // The definition's own name is a direct `name` child; type
        // references are `named_type` nodes wrapping a `name`.
        let mut c = node.walk();
        let name_node = node.children(&mut c).find(|n| n.kind() == "name");
        let Some(name_node) = name_node else { return };
        let name = self.text(name_node).trim().to_string();
        if name.is_empty() { return; }

        let (sl, el) = (
            (node.start_position().row as u32) + 1,
            (node.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name.clone(),
            qualified_name: name.clone(),
            variant: DefVariant::FreeFunction,
            start_line: sl, end_line: el,
            start_byte: node.start_byte() as u32, end_byte: node.end_byte() as u32,
            signature_hint: format!("{keyword} {name}"),
            visibility: String::new(), attributes: Vec::new(),
        });

        self.collect_refs(node);
    }

    fn collect_refs(&mut self, node: Node) {
        if node.kind() == "named_type" {
            let mut c = node.walk();
            if let Some(name_node) = node.children(&mut c).find(|n| n.kind() == "name") {
                let name = self.text(name_node).trim();
                if !name.is_empty() && !BUILTIN_SCALARS.contains(&name) {
                    self.facts.references.push(RefRecord {
                        name: name.to_string(),
                        receiver_hint: String::new(),
                        site_line: (name_node.start_position().row as u32) + 1,
                        site_byte: name_node.start_byte() as u32,
                    });
                }
            }
            return;
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop { self.collect_refs(c.node()); if !c.goto_next_sibling() { break; } }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_graphql::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        GraphqlPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/s.graphql"), &tree, src.as_bytes())
    }

    const SCHEMA: &str = r#"type Query { city(filter: CityInput): City forecast: Forecast }
type City implements Node { name: String! coords: [Coord!]! }
type Coord { lat: Float }
interface Node { id: ID! }
input CityInput { id: ID! }
union Result = City | Forecast
type Forecast { rain: Float }
"#;

    #[test]
    fn plugin_loads() {
        assert_eq!(GraphqlPlugin.id(), "graphql");
        assert!(GraphqlPlugin.extensions().contains(&".graphql"));
    }

    #[test]
    fn extracts_type_defs() {
        let f = extract(SCHEMA);
        let names: Vec<&str> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
        for want in ["Query", "City", "Coord", "Node", "CityInput", "Result", "Forecast"] {
            assert!(names.contains(&want), "missing def {want}, got {names:?}");
        }
    }

    #[test]
    fn field_implements_and_union_references() {
        let f = extract(SCHEMA);
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"City"), "field result type, got {refs:?}");
        assert!(refs.contains(&"Coord"), "list field element type");
        assert!(refs.contains(&"Node"), "implements interface");
        assert!(refs.contains(&"CityInput"), "argument input type");
        assert!(refs.contains(&"Forecast"), "union member");
    }

    #[test]
    fn builtin_scalars_excluded() {
        let f = extract(SCHEMA);
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        for s in ["String", "Float", "ID"] {
            assert!(!refs.contains(&s), "{s} scalar should be excluded, got {refs:?}");
        }
    }
}
