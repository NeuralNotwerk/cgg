//! Shared YAML/JSON tree helpers for the structured API-descriptor
//! plugins (OpenAPI/Swagger, AsyncAPI).
//!
//! Both formats are parsed with the YAML grammar — JSON is valid
//! flow-style YAML, so one set of helpers covers block mappings, flow
//! mappings, sequences, and scalars. The plugins create one definition
//! per named shape (schema / message / channel / operation) spanning that
//! shape's value-node byte range, then [`collect_refs`] emits one
//! reference per `$ref` pointer. Because each `$ref` falls inside the byte
//! range of the shape that contains it, the standard byte-span resolvers
//! attribute every edge correctly with no descriptor-specific linking.

use cgg_core::{DefRecord, DefVariant, FileFacts, RefRecord};
use tree_sitter::Node;

/// Descend through YAML `flow_node` / `block_node` wrappers (skipping
/// anchors, tags, and comments) to the concrete value node underneath.
pub fn unwrap(node: Node) -> Node {
    let mut n = node;
    while matches!(n.kind(), "flow_node" | "block_node") {
        let mut chosen = None;
        let mut c = n.walk();
        for ch in n.named_children(&mut c) {
            if !matches!(ch.kind(), "anchor" | "tag" | "comment") {
                chosen = Some(ch);
            }
        }
        match chosen {
            Some(ch) => n = ch,
            None => break,
        }
    }
    n
}

/// Text of a scalar node, with surrounding YAML/JSON quotes stripped.
pub fn scalar_text(node: Node, src: &[u8]) -> String {
    let u = unwrap(node);
    let t = u.utf8_text(src).unwrap_or("").trim();
    let bytes = t.as_bytes();
    if bytes.len() >= 2 {
        let (f, l) = (bytes[0], bytes[bytes.len() - 1]);
        if (f == b'\'' && l == b'\'') || (f == b'"' && l == b'"') {
            return t[1..t.len() - 1].to_string();
        }
    }
    t.to_string()
}

fn is_pair(kind: &str) -> bool {
    matches!(kind, "block_mapping_pair" | "flow_pair")
}

fn is_mapping(kind: &str) -> bool {
    matches!(kind, "block_mapping" | "flow_mapping")
}

/// The (key, value-node) entries of a mapping node, in document order.
/// Returns empty if `node` is not a mapping.
pub fn mapping_entries<'a>(node: Node<'a>, src: &[u8]) -> Vec<(String, Node<'a>)> {
    let u = unwrap(node);
    if !is_mapping(u.kind()) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut c = u.walk();
    for ch in u.named_children(&mut c) {
        if !is_pair(ch.kind()) {
            continue;
        }
        let Some(k) = ch.child_by_field_name("key") else { continue };
        if let Some(v) = ch.child_by_field_name("value") {
            out.push((scalar_text(k, src), v));
        }
    }
    out
}

/// Follow a key path (`["components", "schemas"]`) from a mapping node.
pub fn get<'a>(node: Node<'a>, keys: &[&str], src: &[u8]) -> Option<Node<'a>> {
    let mut cur = node;
    for &k in keys {
        cur = mapping_entries(cur, src)
            .into_iter()
            .find(|(kk, _)| kk == k)
            .map(|(_, v)| v)?;
    }
    Some(cur)
}

/// The top value node of a parsed document (`stream` → `document` → node).
pub fn document_root(root: Node) -> Option<Node> {
    let mut c = root.walk();
    let doc = root.named_children(&mut c).find(|n| n.kind() == "document")?;
    let mut c2 = doc.walk();
    doc.named_children(&mut c2).find(|n| n.kind() != "comment")
}

/// Simple name of a JSON-pointer `$ref` (last `/`-separated segment).
/// `#/components/schemas/Pet` → `Pet`, `common.yaml#/Foo` → `Foo`.
pub fn ref_simple_name(pointer: &str) -> &str {
    pointer.rsplit('/').next().unwrap_or(pointer).trim()
}

/// Push a definition spanning `value`'s byte range, so every `$ref`
/// nested inside it resolves to this definition via byte containment.
pub fn push_def(facts: &mut FileFacts, name: &str, qualified: &str, value: Node, sig: String) {
    if name.is_empty() {
        return;
    }
    facts.definitions.push(DefRecord {
        simple_name: name.to_string(),
        qualified_name: qualified.to_string(),
        variant: DefVariant::FreeFunction,
        start_line: (value.start_position().row as u32) + 1,
        end_line: (value.end_position().row as u32) + 1,
        start_byte: value.start_byte() as u32,
        end_byte: value.end_byte() as u32,
        signature_hint: sig,
        visibility: String::new(),
        attributes: Vec::new(),
        ..Default::default()
    });
}

/// Add one definition per named entry under the mapping at `section`.
/// `kind` labels the signature hint (e.g. `"schema"`, `"message"`).
pub fn add_section_defs(top: Node, section: &[&str], kind: &str, src: &[u8], facts: &mut FileFacts) {
    let Some(container) = get(top, section, src) else { return };
    for (name, value) in mapping_entries(container, src) {
        push_def(facts, &name, &name, value, format!("{kind} {name}"));
    }
}

/// Recursively emit a reference for every `$ref` pointer under `node`.
pub fn collect_refs(node: Node, src: &[u8], facts: &mut FileFacts) {
    if is_pair(node.kind()) {
        if let Some(k) = node.child_by_field_name("key") {
            if scalar_text(k, src) == "$ref" {
                if let Some(v) = node.child_by_field_name("value") {
                    let name = ref_simple_name(&scalar_text(v, src)).to_string();
                    if !name.is_empty() {
                        let vn = unwrap(v);
                        facts.references.push(RefRecord {
                            name,
                            receiver_hint: String::new(),
                            site_line: (vn.start_position().row as u32) + 1,
                            site_byte: vn.start_byte() as u32,
                        });
                    }
                }
            }
        }
    }
    let mut c = node.walk();
    if c.goto_first_child() {
        loop {
            collect_refs(c.node(), src, facts);
            if !c.goto_next_sibling() {
                break;
            }
        }
    }
}
