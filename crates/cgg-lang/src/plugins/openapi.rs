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

use std::path::Path;
use cgg_core::{ids::FileId, FileFacts};
use tree_sitter::Tree;
use crate::plugins::structured;
use crate::LanguagePlugin;

/// HTTP methods recognised as operations under a path item.
const METHODS: &[&str] = &["get", "put", "post", "delete", "options", "head", "patch", "trace"];

#[derive(Debug)]
pub struct OpenApiPlugin;

impl LanguagePlugin for OpenApiPlugin {
    fn id(&self) -> &'static str { "openapi" }
    fn extensions(&self) -> &'static [&'static str] { &[".yaml", ".yml", ".json"] }
    fn shebangs(&self) -> &'static [&'static str] { &[] }
    fn ts_language(&self) -> tree_sitter::Language { tree_sitter_yaml::LANGUAGE.into() }

    fn extract(&self, file: FileId, path: &Path, tree: &Tree, source: &[u8]) -> FileFacts {
        let mut facts = FileFacts::new(file, path.to_path_buf(), "openapi");
        let root = tree.root_node();
        let Some(top) = structured::document_root(root) else { return facts };

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
                for (method, op) in structured::mapping_entries(methods, source) {
                    if !METHODS.contains(&method.as_str()) {
                        continue;
                    }
                    let op_id = structured::get(op, &["operationId"], source)
                        .map(|n| structured::scalar_text(n, source))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| format!("{} {path_str}", method.to_uppercase()));
                    structured::push_def(&mut facts, &op_id, &op_id, op, format!("operation {op_id}"));
                }
            }
        }

        structured::collect_refs(root, source, &mut facts);
        facts
    }
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
        OpenApiPlugin.extract(FileId::new(0), &PathBuf::from("/tmp/__cgg_test__/api.yaml"), &tree, src.as_bytes())
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
        let names: Vec<&str> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
        for want in ["Pet", "Category", "Pets", "listPets"] {
            assert!(names.contains(&want), "missing def {want}, got {names:?}");
        }
        // post has no operationId -> synthesized "POST /pets"
        assert!(names.iter().any(|n| n.contains("POST")), "synthesized op name, got {names:?}");
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
        let pet = f.definitions.iter().find(|d| d.simple_name == "Pet").unwrap();
        let cat_ref = f.references.iter().find(|r| r.name == "Category").unwrap();
        assert!(
            cat_ref.site_byte >= pet.start_byte && cat_ref.site_byte < pet.end_byte,
            "Category $ref byte {} not within Pet [{}, {})",
            cat_ref.site_byte, pet.start_byte, pet.end_byte
        );
    }

    #[test]
    fn parses_json_documents_too() {
        let json = r##"{"openapi":"3.0.0","components":{"schemas":{"A":{"properties":{"b":{"$ref":"#/components/schemas/B"}}},"B":{"type":"object"}}}}"##;
        let f = extract(json);
        let names: Vec<&str> = f.definitions.iter().map(|d| d.simple_name.as_str()).collect();
        assert!(names.contains(&"A") && names.contains(&"B"), "json defs, got {names:?}");
        assert!(f.references.iter().any(|r| r.name == "B"), "json $ref");
    }
}
