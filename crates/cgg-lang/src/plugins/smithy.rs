//! Smithy IDL plugin — API model topology extraction.
//!
//! Smithy has no functions; it describes services. We map its shape graph
//! onto cgg's callable/edge model so the API topology shows up as a call
//! graph:
//!
//! * Every shape (`service`, `resource`, `operation`, `structure`, `union`,
//!   `list`, `map`, `enum`, simple shapes) becomes a definition, qualified
//!   by namespace as `namespace#ShapeName` (Smithy's canonical absolute id).
//! * Every shape reference inside a shape body becomes a reference:
//!   `service` → operations/resources, `resource` → lifecycle operations /
//!   sub-resources / identifier types, `operation` → input/output/errors,
//!   `structure`/`union` members → their target shapes, `list`/`map`/`set`
//!   → their member shapes.
//!
//! Traits (`@required`, `@error`, `@references(...)`, …) are skipped so the
//! graph stays structural rather than drowning in trait-name edges. Smithy
//! prelude primitives (`String`, `Integer`, `Timestamp`, …) are skipped as
//! references for the same reason — they resolve to nothing in-model.
//!
//! The vendored grammar is compiled in `build.rs`; see
//! `vendor/smithy/PROVENANCE.md`.

use std::path::Path;
use cgg_core::{ids::FileId, DefRecord, DefVariant, FileFacts, RefRecord};
use tree_sitter::{Node, Tree};
use tree_sitter_language::LanguageFn;
use crate::{LanguagePlugin, ResolverKind};

unsafe extern "C" {
    fn tree_sitter_smithy() -> *const ();
}
/// Raw binding to the vendored Smithy grammar's C entry point.
const SMITHY_LANGUAGE: LanguageFn = unsafe { LanguageFn::from_raw(tree_sitter_smithy) };

/// Smithy prelude primitives — referenced everywhere, defined nowhere
/// in-model, so they'd only ever land in the external bucket. Skip them.
const PRELUDE: &[&str] = &[
    "String", "Blob", "Boolean", "Byte", "Short", "Integer", "Long",
    "Float", "Double", "BigInteger", "BigDecimal", "Timestamp", "Document",
    "Unit", "PrimitiveBoolean", "PrimitiveByte", "PrimitiveShort",
    "PrimitiveInteger", "PrimitiveLong", "PrimitiveFloat", "PrimitiveDouble",
];

#[derive(Debug)]
pub struct SmithyPlugin;

impl LanguagePlugin for SmithyPlugin {
    fn id(&self) -> &'static str { "smithy" }
    fn extensions(&self) -> &'static [&'static str] { &[".smithy"] }
    fn shebangs(&self) -> &'static [&'static str] { &[] }
    fn resolver_kind(&self) -> ResolverKind { ResolverKind::Custom }
    fn ts_language(&self) -> tree_sitter::Language { SMITHY_LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "smithy");
        let namespace = find_namespace(tree.root_node(), source);
        let mut w = SmithyWalker { source, namespace, facts: &mut facts };
        w.walk(tree.root_node());
        facts
    }
}

/// Locate the model's `namespace` declaration (the dotted name node, not
/// the `namespace` keyword token).
fn find_namespace(root: Node, source: &[u8]) -> String {
    fn rec(n: Node, source: &[u8]) -> Option<String> {
        if n.kind() == "namespace_statement" {
            let mut c = n.walk();
            for child in n.children(&mut c) {
                if child.kind() == "namespace" && child.is_named() {
                    return Some(child.utf8_text(source).unwrap_or("").to_string());
                }
            }
        }
        let mut c = n.walk();
        for child in n.children(&mut c) {
            if let Some(ns) = rec(child, source) { return Some(ns); }
        }
        None
    }
    rec(root, source).unwrap_or_default()
}

/// Reduce a (possibly absolute) shape id to its simple name:
/// `ns#Shape` → `Shape`, `ns.Shape` → `Shape`, `Shape$member` → `Shape`.
fn simple_shape_name(text: &str) -> &str {
    let after_hash = text.rsplit('#').next().unwrap_or(text);
    let no_member = after_hash.split('$').next().unwrap_or(after_hash);
    no_member.rsplit('.').next().unwrap_or(no_member).trim()
}

struct SmithyWalker<'a> {
    source: &'a [u8],
    namespace: String,
    facts: &'a mut FileFacts,
}

impl<'a> SmithyWalker<'a> {
    fn text(&self, n: Node) -> &str { n.utf8_text(self.source).unwrap_or("") }

    fn walk(&mut self, node: Node) {
        if node.kind() == "shape_statement" {
            self.record_shape(node);
            return;
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop { self.walk(c.node()); if !c.goto_next_sibling() { break; } }
        }
    }

    fn record_shape(&mut self, shape_stmt: Node) {
        // `body:` is the concrete *_statement (service/operation/...).
        let Some(body) = shape_stmt.child_by_field_name("body") else { return };
        let Some(name_node) = body.child_by_field_name("name") else { return };
        let name = self.text(name_node).to_string();
        if name.is_empty() { return; }

        let kind = body.kind().strip_suffix("_statement").unwrap_or(body.kind());
        let qn = if self.namespace.is_empty() {
            name.clone()
        } else {
            format!("{}#{}", self.namespace, name)
        };
        let (sl, el) = (
            (shape_stmt.start_position().row as u32) + 1,
            (shape_stmt.end_position().row as u32) + 1,
        );
        self.facts.definitions.push(DefRecord {
            simple_name: name,
            qualified_name: qn,
            variant: DefVariant::FreeFunction,
            start_line: sl, end_line: el,
            start_byte: shape_stmt.start_byte() as u32,
            end_byte: shape_stmt.end_byte() as u32,
            signature_hint: format!("{kind} {}", self.text(name_node)),
            visibility: String::new(), attributes: Vec::new(),
        });

        // References live in the shape body, not in the leading traits
        // (those are a sibling field of `shape_statement`). Inner traits
        // (e.g. a member's `@required`) are skipped within `collect_refs`.
        self.collect_refs(body);
    }

    fn collect_refs(&mut self, node: Node) {
        match node.kind() {
            "trait" | "applied_traits" => return, // skip trait subtrees
            "shape_id" => {
                let raw = self.text(node).to_string();
                self.push_ref(&raw, node);
                return;
            }
            "identifier" => {
                // Bare value-position identifiers (operation `errors: [...]`).
                if node.parent().map(|p| p.kind()) == Some("operation_errors") {
                    let raw = self.text(node).to_string();
                self.push_ref(&raw, node);
                }
                return;
            }
            _ => {}
        }
        let mut c = node.walk();
        if c.goto_first_child() {
            loop { self.collect_refs(c.node()); if !c.goto_next_sibling() { break; } }
        }
    }

    fn push_ref(&mut self, raw: &str, node: Node) {
        let name = simple_shape_name(raw);
        if name.is_empty() || PRELUDE.contains(&name) { return; }
        self.facts.references.push(RefRecord {
            name: name.to_string(),
            receiver_hint: String::new(),
            site_line: (node.start_position().row as u32) + 1,
            site_byte: node.start_byte() as u32,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&SMITHY_LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        SmithyPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/m.smithy"), &tree, src.as_bytes())
    }

    const WEATHER: &str = r#"$version: "2"
namespace example.weather

service Weather {
    version: "2006-03-01"
    operations: [GetCity]
    resources: [City]
}

resource City {
    identifiers: { cityId: CityId }
    read: GetCity
}

@readonly
operation GetCity {
    input: GetCityInput
    output: GetCityOutput
    errors: [NoSuchResource]
}

@input
structure GetCityInput {
    @required
    cityId: CityId
}

@output
structure GetCityOutput {
    name: String
    coordinates: CityCoordinates
}

structure CityCoordinates {
    latitude: Float
    longitude: Float
}

@error("client")
structure NoSuchResource {
    resourceType: String
}

string CityId
"#;

    #[test]
    fn plugin_loads() {
        assert_eq!(SmithyPlugin.id(), "smithy");
        assert!(SmithyPlugin.extensions().contains(&".smithy"));
    }

    #[test]
    fn extracts_all_shapes_namespace_qualified() {
        let f = extract(WEATHER);
        let qns: Vec<&str> = f.definitions.iter().map(|d| d.qualified_name.as_str()).collect();
        assert!(qns.contains(&"example.weather#Weather"), "service def, got {qns:?}");
        assert!(qns.contains(&"example.weather#GetCity"), "operation def");
        assert!(qns.contains(&"example.weather#GetCityInput"), "structure def");
        assert!(qns.contains(&"example.weather#CityId"), "simple shape def");
    }

    #[test]
    fn service_references_operations_and_resources() {
        let f = extract(WEATHER);
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"GetCity"), "service->operation, got {refs:?}");
        assert!(refs.contains(&"City"), "service->resource");
        assert!(refs.contains(&"GetCityInput"), "operation->input");
        assert!(refs.contains(&"NoSuchResource"), "operation->error (bare ident)");
        assert!(refs.contains(&"CityCoordinates"), "structure member->shape");
    }

    #[test]
    fn prelude_primitives_are_not_references() {
        let f = extract(WEATHER);
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(!refs.contains(&"String"), "String is prelude, got {refs:?}");
        assert!(!refs.contains(&"Float"), "Float is prelude");
    }

    #[test]
    fn trait_shape_refs_excluded() {
        // @error("client") and @required must not produce edges.
        let f = extract(WEATHER);
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(!refs.contains(&"error"));
        assert!(!refs.contains(&"required"));
        assert!(!refs.contains(&"readonly"));
    }
}
