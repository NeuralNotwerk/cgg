//! AsyncAPI plugin — event/message API topology extraction.
//!
//! Detected by content (a root `asyncapi:` key — see `detect.rs`) among
//! `.yaml` / `.yml` / `.json` files, parsed with the YAML grammar.
//!
//! Definitions:
//! * reusable components — `components/schemas`, `messages`, `parameters`;
//! * every `channels/<name>` (both AsyncAPI 2 and 3);
//! * every `operations/<name>` (AsyncAPI 3).
//!
//! References: every `$ref` pointer, attributed by byte containment to the
//! channel / operation / message / schema it sits inside — yielding
//! channel → message, operation → channel/message, and message → schema
//! edges. AsyncAPI 2 `publish` / `subscribe` blocks live inside the
//! channel's byte range, so their message `$ref`s attach to the channel.

use crate::LanguagePlugin;
use crate::plugins::structured;
use cgg_core::{FileFacts, ids::FileId};
use std::path::Path;
use tree_sitter::Tree;

#[derive(Debug)]
pub struct AsyncApiPlugin;

impl LanguagePlugin for AsyncApiPlugin {
    fn id(&self) -> &'static str {
        "asyncapi"
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
        let mut facts = FileFacts::new(file, path.to_path_buf(), "asyncapi");
        let root = tree.root_node();
        let Some(top) = structured::document_root(root) else {
            return facts;
        };

        for (section, kind) in [
            (&["components", "schemas"][..], "schema"),
            (&["components", "messages"][..], "message"),
            (&["components", "parameters"][..], "parameter"),
        ] {
            structured::add_section_defs(top, section, kind, source, &mut facts);
        }

        // Channels and (AsyncAPI 3) operations are top-level maps whose
        // keys name the shape.
        structured::add_section_defs(top, &["channels"], "channel", source, &mut facts);
        structured::add_section_defs(
            top,
            &["operations"],
            "operation",
            source,
            &mut facts,
        );

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
        AsyncApiPlugin.extract(
            &crate::ExtractCtx::plain(),
            FileId::new(0),
            &PathBuf::from("/tmp/__cgg_test__/async.yaml"),
            &tree,
            src.as_bytes(),
        )
    }

    // AsyncAPI 2 style: channels with publish/subscribe.
    const STREETLIGHTS: &str = r#"asyncapi: 2.6.0
info:
  title: Streetlights
channels:
  light/measured:
    publish:
      message:
        $ref: '#/components/messages/LightMeasured'
components:
  messages:
    LightMeasured:
      payload:
        $ref: '#/components/schemas/LightMeasuredPayload'
  schemas:
    LightMeasuredPayload:
      type: object
      properties:
        lumens:
          type: integer
"#;

    #[test]
    fn plugin_loads() {
        assert_eq!(AsyncApiPlugin.id(), "asyncapi");
        assert!(AsyncApiPlugin.extensions().contains(&".yaml"));
    }

    #[test]
    fn extracts_channel_message_schema_defs() {
        let f = extract(STREETLIGHTS);
        let names: Vec<&str> = f
            .definitions
            .iter()
            .map(|d| d.simple_name.as_str())
            .collect();
        assert!(
            names.contains(&"light/measured"),
            "channel def, got {names:?}"
        );
        assert!(names.contains(&"LightMeasured"), "message def");
        assert!(names.contains(&"LightMeasuredPayload"), "schema def");
    }

    #[test]
    fn channel_to_message_and_message_to_schema_refs() {
        let f = extract(STREETLIGHTS);
        let refs: Vec<&str> = f.references.iter().map(|r| r.name.as_str()).collect();
        assert!(
            refs.contains(&"LightMeasured"),
            "channel -> message, got {refs:?}"
        );
        assert!(refs.contains(&"LightMeasuredPayload"), "message -> schema");
    }

    #[test]
    fn channel_ref_falls_inside_channel_def() {
        let f = extract(STREETLIGHTS);
        let chan = f
            .definitions
            .iter()
            .find(|d| d.simple_name == "light/measured")
            .unwrap();
        let msg_ref = f
            .references
            .iter()
            .find(|r| r.name == "LightMeasured")
            .unwrap();
        assert!(
            msg_ref.site_byte >= chan.start_byte && msg_ref.site_byte < chan.end_byte,
            "message $ref not within channel byte range"
        );
    }
}
