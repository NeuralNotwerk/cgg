//! OpenAPI / Swagger plugin — REST API topology extraction.
//!
//! Detected by content (a root `openapi:` / `swagger:` key — see
//! `detect.rs`), so it claims only genuine API documents among `.yaml` /
//! `.yml` / `.json` files. Both YAML and JSON are parsed with the YAML
//! grammar (JSON is flow-style YAML).
//!
//! Definitions:
//! * every reusable component — `components/schemas`, `parameters`,
//!   `requestBodies`, `responses`, `headers` (OpenAPI 3) and top-level
//!   `definitions` (Swagger 2);
//! * every operation under `paths/<path>/<method>` (named by its
//!   `operationId`, falling back to `"<METHOD> <path>"`).
//!
//! References: every `$ref` pointer, attributed by byte containment to the
//! component or operation it sits inside — yielding operation → schema and
//! schema → schema edges (the API's request/response/model topology).

use crate::LanguagePlugin;
use crate::plugins::structured;
use cgg_core::{FileFacts, ids::FileId};
use std::path::Path;
use tree_sitter::Tree;

/// Attribute on every operation definition, which the descriptor linker
/// and the dead-code roots read (schemas and other components carry none).
pub const OPERATION_ATTR: &str = "openapi-operation";

/// connexion's extension naming the Python module (or class) that holds
/// the handlers of a path item or operation: a bare `operationId` under
/// it is `<controller>.<operationId>`.
const ROUTER_CONTROLLER: &str = "x-openapi-router-controller";

/// HTTP methods recognised as operations under a path item.
const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace",
];

#[derive(Debug)]
pub struct OpenApiPlugin;

impl LanguagePlugin for OpenApiPlugin {
    fn id(&self) -> &'static str {
        "openapi"
    }
    fn extensions(&self) -> &'static [&'static str] {
        &[".yaml", ".yml", ".json"]
    }
    fn shebangs(&self) -> &'static [&'static str] {
        &[]
    }
    fn ts_language(&self) -> tree_sitter::Language {
        tree_sitter_yaml::LANGUAGE.into()
    }

    fn extract(
        &self,
        _ctx: &crate::ExtractCtx<'_>,
        file: FileId,
        path: &Path,
        tree: &Tree,
        source: &[u8],
    ) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "openapi");
        let root = tree.root_node();
        let Some(top) = structured::document_root(root) else {
            return facts;
        };

        for (section, kind) in [
            (&["components", "schemas"][..], "schema"),
            (&["components", "parameters"][..], "parameter"),
            (&["components", "requestBodies"][..], "requestBody"),
            (&["components", "responses"][..], "response"),
            (&["components", "headers"][..], "header"),
            (&["definitions"][..], "schema"), // Swagger 2.0
        ] {
            structured::add_section_defs(top, section, kind, source, &mut facts);
        }

        // Operations: paths -> <path> -> <method>.
        if let Some(paths) = structured::get(top, &["paths"], source) {
            for (path_str, methods) in structured::mapping_entries(paths, source) {
                let path_controller = controller(methods, source);
                for (method, op) in structured::mapping_entries(methods, source) {
                    if !METHODS.contains(&method.as_str()) {
                        continue;
                    }
                    let op_id = structured::get(op, &["operationId"], source)
                        .map(|n| structured::scalar_text(n, source))
                        .filter(|s| !s.is_empty());
                    let named = op_id.is_some();
                    let mut op_id = op_id.unwrap_or_else(|| {
                        format!("{} {path_str}", method.to_uppercase())
                    });
                    if named
                        && !op_id.contains('.')
                        && let Some(c) =
                            controller(op, source).or_else(|| path_controller.clone())
                    {
                        op_id = format!("{c}.{op_id}");
                    }
                    let at = facts.definitions.len();
                    structured::push_def(
                        &mut facts,
                        &op_id,
                        &op_id,
                        op,
                        format!("operation {op_id}"),
                    );
                    if let Some(d) = facts.definitions.get_mut(at) {
                        d.attributes.push(OPERATION_ATTR.to_string());
                    }
                }
            }
        }

        structured::collect_refs(root, source, &mut facts);
        facts
    }
}

/// The `x-openapi-router-controller` a path item or operation declares.
fn controller(node: tree_sitter::Node, source: &[u8]) -> Option<String> {
    structured::get(node, &[ROUTER_CONTROLLER], source)
        .map(|n| structured::scalar_text(n, source))
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tree_sitter::Parser;

    fn extract(src: &str) -> FileFacts {
        let mut p = Parser::new();
        p.set_language(&tree_sitter_yaml::LANGUAGE.into()).unwrap();
        let tree = p.parse(src, None).unwrap();
        OpenApiPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/api.yaml"),
            &tree,
            src.as_bytes(),
        )
    }

    const PETSTORE: &str = r#"openapi: 3.0.0
info:
  title: Petstore
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        '200':
          content:
            application/json:
              schema:
                $ref: '#/components/schemas/Pets'
    post:
      requestBody:
        content:
          application/json:
            schema:
              $ref: '#/components/schemas/Pet'
      responses:
        '201':
          description: created
components:
  schemas:
    Pet:
      type: object
      properties:
        category:
          $ref: '#/components/schemas/Category'
    Category:
      type: object
    Pets:
      type: array
      items:
        $ref: '#/components/schemas/Pet'
"#;

    #[test]
    fn plugin_loads() {
        assert_eq!(OpenApiPlugin.id(), "openapi");
        assert!(OpenApiPlugin.extensions().contains(&".yaml"));
    }

    #[test]
    fn extracts_schema_and_operation_defs() {
        let f = extract(PETSTORE);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        for want in ["Pet", "Category", "Pets", "listPets"] {
            assert!(names.contains(&want), "missing def {want}, got {names:?}");
        }
        // post has no operationId -> synthesized "POST /pets"
        assert!(
            names.iter().any(|n| n.contains("POST")),
            "synthesized op name, got {names:?}"
        );
    }

    /// BACKLOG P1 (connexion): a bare `operationId` under an
    /// `x-openapi-router-controller` (on the operation or its path item)
    /// is qualified by it; a dotted one is kept; every operation, and no
    /// schema, carries the operation attribute.
    #[test]
    fn router_controller_qualifies_a_bare_operation_id() {
        let src = "openapi: 3.0.0\n\
            paths:\n\
            \x20 /a:\n\
            \x20   x-openapi-router-controller: app.api.items\n\
            \x20   get:\n\
            \x20     operationId: list_items\n\
            \x20   post:\n\
            \x20     operationId: app.api.other.create\n\
            \x20   put:\n\
            \x20     x-openapi-router-controller: app.api.special\n\
            \x20     operationId: put_item\n\
            \x20 /b:\n\
            \x20   get:\n\
            \x20     operationId: listPets\n\
            components:\n\
            \x20 schemas:\n\
            \x20   Pet:\n\
            \x20     type: object\n";
        let f = extract(src);
        let ops: Vec<&str> = f
            .definitions
            .iter()
            .filter(|d| d.attributes.iter().any(|a| a == OPERATION_ATTR))
            .map(|d| d.qualified_name.as_str())
            .collect();
        assert_eq!(
            ops,
            vec![
                "app.api.items.list_items",
                "app.api.other.create",
                "app.api.special.put_item",
                "listPets",
            ]
        );
        assert!(
            f.definitions
                .iter()
                .any(|d| d.simple_name == "Pet" && d.attributes.is_empty())
        );
    }

    #[test]
    fn ref_edges_operation_to_schema_and_schema_to_schema() {
        let f = extract(PETSTORE);
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(refs.contains(&"Pets"), "operation -> schema, got {refs:?}");
        assert!(refs.contains(&"Category"), "schema -> schema");
        assert!(refs.contains(&"Pet"), "array items -> schema");
    }

    #[test]
    fn refs_fall_inside_their_owning_def_byte_range() {
        // The intra-file linker needs each $ref site_byte inside exactly
        // one def's byte range. Verify the Category $ref sits within Pet.
        let f = extract(PETSTORE);
        let pet = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "Pet")
            .unwrap();
        let cat_ref = f.references.iter().find(|r| r.name == "Category").unwrap();
        assert!(
            cat_ref.site_byte >= pet.start_byte && cat_ref.site_byte < pet.end_byte,
            "Category $ref byte {} not within Pet [{}, {})",
            cat_ref.site_byte,
            pet.start_byte,
            pet.end_byte
        );
    }

    #[test]
    fn parses_json_documents_too() {
        let json = r##"{"openapi":"3.0.0","components":{"schemas":{"A":{"properties":{"b":{"$ref":"#/components/schemas/B"}}},"B":{"type":"object"}}}}"##;
        let f = extract(json);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(
            names.contains(&"A") && names.contains(&"B"),
            "json defs, got {names:?}"
        );
        assert!(f.references.iter().any(|r| r.name == "B"), "json $ref");
    }
}
