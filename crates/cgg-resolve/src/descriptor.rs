//! Interface descriptor → implementation linking.
//!
//! cgg parses `.proto`, GraphQL, OpenAPI, AsyncAPI and Smithy alongside
//! the languages that implement them. That combination is unusual, and
//! it makes one edge available that no single-language analysis can see:
//! the one from a *declaration* of an operation to the code serving it.
//!
//! ```text
//! service Greeter { rpc SayHello (Req) returns (Resp); }   // greeter.proto
//! func (s *server) SayHello(ctx, r *Req) (*Resp, error)    // server.go
//! ```
//!
//! Nothing in either file references the other. The `.proto` names no Go
//! symbol, and the Go file's link to the service runs through generated
//! code that is usually not committed. So the rpc looks unimplemented
//! and the handler looks uncalled — both wrong, and wrong in the
//! direction that hides an entire service's surface.
//!
//! This is the mirror of [`crate::ffi`] one level up. FFI links two
//! *implementations* across a language boundary; this links a
//! *declaration* to an implementation across one.
//!
//! # Why the match is deliberately narrow
//!
//! Method name alone is far too loose — `Get`, `Create` and `Update`
//! appear everywhere in every codebase. So a candidate must satisfy
//! **both** halves:
//!
//! 1. its method name matches the operation, case-insensitively (Java
//!    lowercases the first letter: `SayHello` → `sayHello`), and
//! 2. its owning type names the service — `GreeterImpl`,
//!    `GreeterServer`, `UnimplementedGreeterServer`, `greeterService`.
//!
//! That second condition is what keeps this from manufacturing edges. A
//! service whose implementation follows no naming convention is simply
//! not linked, which is the correct failure: a missing edge is a gap, a
//! wrong edge is a lie about where control goes.

use std::collections::HashMap;

use cgg_core::graph::{CallEdge, Confidence, Graph, Via};
use cgg_core::ids::{CallableId, ResolverId};

/// Languages whose "callables" are interface declarations rather than
/// code. A definition in one of these describes an operation someone
/// else implements.
const DESCRIPTOR_LANGUAGES: &[&str] = &["proto", "graphql", "smithy"];

/// Which family a descriptor language belongs to, for the edge payload.
fn family_of(language: &str) -> &'static str {
    match language {
        "proto" => "grpc",
        "graphql" => "graphql",
        "smithy" => "smithy",
        _ => "descriptor",
    }
}

/// Link every descriptor operation to its implementation.
pub fn link_descriptors(graph: &Graph) -> Vec<CallEdge> {
    let resolver = ResolverId::new("descriptor-linker");
    let mut out: Vec<CallEdge> = Vec::new();

    // Most trees contain no descriptors at all. Check before building an
    // index over every callable in the graph — this pass runs on every
    // run, and an allocation proportional to the whole tree to then find
    // nothing is exactly the kind of cost a default-on phase must not
    // impose.
    if !graph
        .callables
        .values()
        .any(|n| DESCRIPTOR_LANGUAGES.contains(&n.language.as_str()))
    {
        return out;
    }

    // Implementation candidates, indexed by lowercased method name.
    // Built once: the descriptor side is small, the implementation side
    // is the whole tree.
    let mut by_method: HashMap<String, Vec<CallableId>> = HashMap::new();
    for (id, n) in &graph.callables {
        if DESCRIPTOR_LANGUAGES.contains(&n.language.as_str())
            || n.synthetic
            || n.qualified_name.starts_with('<')
        {
            continue;
        }
        by_method
            .entry(n.simple_name.to_ascii_lowercase())
            .or_default()
            .push(*id);
    }

    for (op_id, op) in &graph.callables {
        if !DESCRIPTOR_LANGUAGES.contains(&op.language.as_str()) {
            continue;
        }
        // Only operations, not the service/message declarations that
        // contain them. `Greeter.SayHello` has an owner; `Greeter` does
        // not.
        let Some(service) = crate::names::owner_from_qn(&op.qualified_name) else {
            continue;
        };
        let service_lc = service
            .rsplit(['.', ':'])
            .next()
            .unwrap_or(service)
            .to_ascii_lowercase();
        if service_lc.is_empty() {
            continue;
        }

        let Some(cands) = by_method.get(&op.simple_name.to_ascii_lowercase()) else {
            continue;
        };
        for cand in cands {
            let Some(node) = graph.callables.get(cand) else {
                continue;
            };
            // The owning type has to name the service. Without this the
            // match is a bare method name, which is no evidence at all.
            let Some(owner) = crate::names::owner_from_qn(&node.qualified_name) else {
                continue;
            };
            if !owner.to_ascii_lowercase().contains(&service_lc) {
                continue;
            }
            out.push(CallEdge {
                src: *op_id,
                dst: *cand,
                site_line: op.start_line,
                site_byte: op.start_byte,
                // An inference from a naming convention, never an
                // observation — the same bar entry nodes are held to.
                confidence: Confidence::Low,
                via: Via::Descriptor(family_of(&op.language).to_string()),
                resolver: resolver.clone(),
            });
        }
    }

    out.sort_by_key(|e| (e.src.as_u32(), e.dst.as_u32()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, CallableNode};
    use cgg_core::ids::FileId;

    fn node(id: u32, qn: &str, lang: &str) -> CallableNode {
        CallableNode {
            id: CallableId::new(id),
            qualified_name: qn.into(),
            simple_name: qn.rsplit(['.', ':']).next().unwrap_or(qn).into(),
            kind: CallableKind::Function,
            language: lang.into(),
            file: FileId::new(0),
            ..Default::default()
        }
    }

    fn graph_with(nodes: &[(u32, &str, &str)]) -> Graph {
        let mut g = Graph::new();
        for (id, qn, lang) in nodes {
            g.add_callable(node(*id, qn, lang));
        }
        g
    }

    #[test]
    fn rpc_links_to_a_conventionally_named_implementation() {
        let g = graph_with(&[
            (0, "Greeter.SayHello", "proto"),
            (1, "server.GreeterServer.SayHello", "go"),
        ]);
        let e = link_descriptors(&g);
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(e[0].src.as_u32(), 0);
        assert_eq!(e[0].dst.as_u32(), 1);
    }

    #[test]
    fn java_lowercases_the_first_letter_and_still_links() {
        let g = graph_with(&[
            (0, "Greeter.SayHello", "proto"),
            (1, "com.x.GreeterImpl.sayHello", "java"),
        ]);
        assert_eq!(link_descriptors(&g).len(), 1);
    }

    #[test]
    fn a_bare_method_name_match_is_not_enough() {
        // The failure this guards: `Get` appears in every codebase. An
        // owner that does not name the service is not evidence.
        let g = graph_with(&[(0, "Greeter.Get", "proto"), (1, "cache.Store.Get", "go")]);
        assert!(link_descriptors(&g).is_empty());
    }

    #[test]
    fn the_service_declaration_itself_links_to_nothing() {
        let g = graph_with(&[(0, "Greeter", "proto"), (1, "x.Greeter.Greeter", "go")]);
        assert!(link_descriptors(&g).is_empty());
    }
}
