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
//! wrong edge is a lie about where control goes. The owner may also name
//! the service through a declared base (`class Hyperopt(SuggestionServicer)`,
//! a Go struct embedding `UnimplementedSuggestionServer`), and names are
//! compared with `_` removed (`say_hello` serves `SayHello`). Generated
//! files and client, stub, mock and `Unimplemented*` classes are never
//! targets: they carry the rpc's name without serving it.
//!
//! An OpenAPI operation whose `operationId` is a dotted path (connexion,
//! prefixed by `x-openapi-router-controller`) names its Python handler
//! outright, and links to the callable with that qualified name.

use std::collections::{HashMap, HashSet};

use cgg_core::FileFacts;
use cgg_core::frameworks::rules::{
    GRPC_GENERATED_FILE_MARKERS, RPC_STUB_OWNER_PREFIXES, RPC_STUB_OWNER_SUFFIXES,
};
use cgg_core::graph::{CallEdge, CallableKind, CallableNode, Confidence, Graph, Via};
use cgg_core::ids::{CallableId, ResolverId};

/// Languages whose "callables" are interface declarations rather than
/// code. A definition in one of these describes an operation someone
/// else implements.
const DESCRIPTOR_LANGUAGES: &[&str] = &["proto", "graphql", "smithy", "openapi"];

/// The host languages an OpenAPI `operationId` names a handler in by its
/// dotted path (connexion resolves `pkg.module.function`).
const OPENAPI_HANDLER_LANGUAGES: &[&str] = &["python"];

/// Whether a descriptor callable is an operation the outside world calls:
/// a proto `rpc` or an OpenAPI operation. Each is a dead-code root: the
/// code it links to is served to remote clients.
pub fn is_descriptor_operation(node: &CallableNode) -> bool {
    match node.language.as_str() {
        "proto" => node.kind == CallableKind::Method,
        "openapi" => node
            .attributes
            .iter()
            .any(|a| a == cgg_lang::plugins::openapi::OPERATION_ATTR),
        _ => false,
    }
}

/// Lowercase with `_` removed: `say_hello`, `SayHello` and `sayHello`
/// compare equal.
fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '_')
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// A file a gRPC / Connect generator wrote.
fn is_generated(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    GRPC_GENERATED_FILE_MARKERS.iter().any(|m| {
        if m.ends_with('.') {
            name.contains(m)
        } else {
            name.ends_with(m)
        }
    })
}

/// A generated or test class that carries an rpc's name without serving
/// it (`UnimplementedGreeterServer`, `GreeterClient`, `MockGreeter`).
fn is_stub_owner(owner: &str) -> bool {
    let o = owner.to_ascii_lowercase();
    RPC_STUB_OWNER_PREFIXES.iter().any(|p| o.starts_with(p))
        || RPC_STUB_OWNER_SUFFIXES.iter().any(|p| o.ends_with(p))
}

/// Which family a descriptor language belongs to, for the edge payload.
fn family_of(language: &str) -> &'static str {
    match language {
        "proto" => "grpc",
        "graphql" => "graphql",
        "smithy" => "smithy",
        "openapi" => "openapi",
        _ => "descriptor",
    }
}

/// Link every descriptor operation to its implementation.
pub fn link_descriptors(graph: &Graph, facts: &[FileFacts]) -> Vec<CallEdge> {
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

    // Implementation candidates, indexed by folded method name.
    // Built once: the descriptor side is small, the implementation side
    // is the whole tree.
    let mut by_method: HashMap<String, Vec<CallableId>> = HashMap::new();
    for (id, n) in &graph.callables {
        // `<…>` names are cgg's own sentinels, except a Rust trait impl
        // (`<MyGreeter as Greeter>::say_hello`), which is tonic's server.
        if DESCRIPTOR_LANGUAGES.contains(&n.language.as_str())
            || n.synthetic
            || (n.qualified_name.starts_with('<') && !n.qualified_name.contains(" as "))
        {
            continue;
        }
        by_method.entry(fold(&n.simple_name)).or_default().push(*id);
    }

    // A proto `message` is a data type: a class of that name carries
    // its field getters (`GetRulesReply.GetRules`), never an rpc.
    let messages: HashSet<String> = graph
        .callables
        .values()
        .filter(|n| n.language == "proto" && n.kind != CallableKind::Method)
        .map(|n| fold(&n.simple_name))
        .collect();

    // Declared bases per (language, owner type), from class records and
    // from the bases a method's own record carries.
    let mut bases: HashMap<(&str, &str), Vec<&str>> = HashMap::new();
    for f in facts {
        if DESCRIPTOR_LANGUAGES.contains(&f.language.as_str()) {
            continue;
        }
        for c in &f.classes {
            let slot = bases
                .entry((f.language.as_str(), crate::dispatch::type_stem(&c.class_qn)))
                .or_default();
            slot.extend(c.base_types.iter().map(String::as_str));
        }
        for d in &f.definitions {
            if d.base_types.is_empty() {
                continue;
            }
            let Some(owner) = crate::names::owner_from_qn(&d.qualified_name) else {
                continue;
            };
            let slot = bases
                .entry((f.language.as_str(), crate::dispatch::type_stem(owner)))
                .or_default();
            slot.extend(d.base_types.iter().map(String::as_str));
        }
    }
    let file_path = |n: &CallableNode| -> String {
        graph
            .files
            .get(&n.file)
            .map(|f| f.path.to_string_lossy().into_owned())
            .unwrap_or_default()
    };

    for (op_id, op) in &graph.callables {
        if !DESCRIPTOR_LANGUAGES.contains(&op.language.as_str()) {
            continue;
        }
        if op.language == "openapi" {
            if is_descriptor_operation(op) {
                link_openapi_operation(
                    graph, *op_id, op, &by_method, &resolver, &mut out,
                );
            }
            continue;
        }
        // Only operations, not the service/message declarations that
        // contain them. `Greeter.SayHello` has an owner; `Greeter` does
        // not.
        // A protobuf `message`/`enum` is a data shape: its qualified name's
        // prefix is the package, not a service. Only an `rpc` (a method of
        // its `service`) is implemented by host-language code.
        if op.language == "proto" && op.kind != cgg_core::graph::CallableKind::Method {
            continue;
        }
        let Some(service) = crate::names::owner_from_qn(&op.qualified_name) else {
            continue;
        };
        let service_lc = fold(service.rsplit(['.', ':']).next().unwrap_or(service));
        if service_lc.is_empty() {
            continue;
        }

        let Some(cands) = by_method.get(&fold(&op.simple_name)) else {
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
            let stem = crate::dispatch::type_stem(owner);
            if is_stub_owner(stem)
                || messages.contains(&fold(stem))
                || is_generated(&file_path(node))
            {
                continue;
            }
            // Written in the owner (`GreeterImpl`, Rust's `<MyGreeter as
            // Greeter>`), or in a base it declares (`SuggestionServicer`,
            // an embedded `UnimplementedSuggestionServer`).
            let raw = crate::names::raw_owner(&node.qualified_name).unwrap_or(owner);
            let named_by_base = || {
                bases
                    .get(&(node.language.as_str(), stem))
                    .is_some_and(|bs| {
                        bs.iter().any(|b| {
                            let b = crate::dispatch::type_stem(b);
                            let bl = b.to_ascii_lowercase();
                            !RPC_STUB_OWNER_SUFFIXES.iter().any(|x| bl.ends_with(x))
                                && fold(b).contains(&service_lc)
                        })
                    })
            };
            if !fold(raw).contains(&service_lc) && !named_by_base() {
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
                weight: 1,
            });
        }
    }

    // Discovery order, not hash order.
    out.sort_by_cached_key(|e| {
        (
            graph.callables.get_index_of(&e.src),
            graph.callables.get_index_of(&e.dst),
        )
    });
    out.dedup_by(|a, b| a.src == b.src && a.dst == b.dst);
    out
}

/// An OpenAPI operation whose `operationId` is a dotted path to its
/// handler (`annif.rest.list_vocabs`): the callable of that qualified
/// name, or, when the path and the qualified name differ only in a
/// leading package (`src.`), one whose name ends with it, on segment
/// boundaries, over at least two segments. A bare `operationId`
/// (`listPets`) names no handler.
fn link_openapi_operation(
    graph: &Graph,
    op_id: CallableId,
    op: &CallableNode,
    by_method: &HashMap<String, Vec<CallableId>>,
    resolver: &ResolverId,
    out: &mut Vec<CallEdge>,
) {
    let path: Vec<&str> = op.qualified_name.split('.').collect();
    if path.len() < 2 || path.iter().any(|s| s.is_empty() || s.contains(' ')) {
        return;
    }
    let Some(cands) = path.last().and_then(|m| by_method.get(&fold(m))) else {
        return;
    };
    for cand in cands {
        let Some(node) = graph.callables.get(cand) else {
            continue;
        };
        if !OPENAPI_HANDLER_LANGUAGES.contains(&node.language.as_str()) {
            continue;
        }
        let qn: Vec<&str> = node.qualified_name.split('.').collect();
        let n = qn.len().min(path.len());
        if n < 2 || qn[qn.len() - n..] != path[path.len() - n..] {
            continue;
        }
        out.push(CallEdge {
            src: op_id,
            dst: *cand,
            site_line: op.start_line,
            site_byte: op.start_byte,
            // The document names the handler's path: exact when it is
            // the whole qualified name, a match on its tail otherwise.
            confidence: if qn.len() == path.len() {
                Confidence::Medium
            } else {
                Confidence::Low
            },
            via: Via::Descriptor(family_of(&op.language).to_string()),
            resolver: resolver.clone(),
            weight: 1,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, CallableNode};
    use cgg_core::ids::FileId;

    fn node(id: u32, qn: &str, lang: &str) -> CallableNode {
        // A proto `Service.Rpc` is a method; a bare or package-prefixed
        // proto name in these tests is a message.
        let kind = if lang == "proto" && qn.split('.').count() == 2 {
            CallableKind::Method
        } else {
            CallableKind::Function
        };
        CallableNode {
            id: CallableId::new(id),
            qualified_name: qn.into(),
            simple_name: qn.rsplit(['.', ':']).next().unwrap_or(qn).into(),
            kind,
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
        let e = link_descriptors(&g, &[]);
        assert_eq!(e.len(), 1, "{e:?}");
        assert_eq!(e[0].src.as_u64(), 0);
        assert_eq!(e[0].dst.as_u64(), 1);
    }

    #[test]
    fn java_lowercases_the_first_letter_and_still_links() {
        let g = graph_with(&[
            (0, "Greeter.SayHello", "proto"),
            (1, "com.x.GreeterImpl.sayHello", "java"),
        ]);
        assert_eq!(link_descriptors(&g, &[]).len(), 1);
    }

    #[test]
    fn a_proto_message_in_a_package_links_to_nothing() {
        // `package a.state; message Iterator {}` — the prefix is the
        // package, and a message is no operation.
        let g = graph_with(&[
            (0, "a.state.Iterator", "proto"),
            (1, "StateStore::iterator", "scala"),
        ]);
        assert!(link_descriptors(&g, &[]).is_empty());
    }

    #[test]
    fn a_bare_method_name_match_is_not_enough() {
        // The failure this guards: `Get` appears in every codebase. An
        // owner that does not name the service is not evidence.
        let g = graph_with(&[(0, "Greeter.Get", "proto"), (1, "cache.Store.Get", "go")]);
        assert!(link_descriptors(&g, &[]).is_empty());
    }

    fn graph_in(nodes: &[(u32, &str, &str, &str)]) -> Graph {
        let mut g = Graph::new();
        for (i, (id, qn, lang, path)) in nodes.iter().enumerate() {
            let fid = FileId::new(i as u32);
            g.add_file(cgg_core::graph::FileRecord {
                id: fid,
                path: (*path).into(),
                language: (*lang).into(),
                ..Default::default()
            });
            let mut n = node(*id, qn, lang);
            n.file = fid;
            g.add_callable(n);
        }
        g
    }

    /// BACKLOG P1: generated files, clients, stubs, mocks,
    /// `Unimplemented*` bases and proto messages are never targets; a
    /// snake_case method serves its CamelCase rpc.
    #[test]
    fn stubs_generated_code_and_messages_are_not_implementations() {
        let g = graph_in(&[
            (0, "Greeter.SayHello", "proto", "a.proto"),
            (1, "HelloReply", "proto", "a.proto"),
            (2, "pb.GreeterClient.SayHello", "go", "a_grpc.pb.go"),
            (3, "pb.greeterClient.SayHello", "go", "x.go"),
            (4, "pb.UnimplementedGreeterServer.SayHello", "go", "y.go"),
            (5, "mock.MockGreeterServer.SayHello", "go", "m.go"),
            (6, "GreeterBlockingStub.sayHello", "java", "S.java"),
            (7, "srv.GreeterImpl.SayHello", "go", "srv.go"),
            (8, "<MyGreeter as Greeter>::say_hello", "rust", "lib.rs"),
            (9, "x.GreeterServer.SayHello", "go", "greeter.pb.go"),
        ]);
        let dsts: Vec<u64> = link_descriptors(&g, &[])
            .iter()
            .map(|e| e.dst.as_u64())
            .collect();
        assert_eq!(dsts, vec![7, 8], "{dsts:?}");
    }

    /// An owner that does not name the service links through a declared
    /// base that does (`class Hyperopt(SuggestionServicer)`, an embedded
    /// `UnimplementedSuggestionServer`); a client base does not count.
    #[test]
    fn a_declared_base_naming_the_service_links_the_owner() {
        let g = graph_in(&[
            (0, "Suggestion.GetSuggestions", "proto", "a.proto"),
            (
                1,
                "hyperopt.HyperoptService.GetSuggestions",
                "python",
                "h.py",
            ),
            (2, "v1.APIService.GetSuggestions", "go", "api.go"),
            (3, "w.Wrapper.GetSuggestions", "python", "w.py"),
        ]);
        let mut fh = FileFacts::new(FileId::new(1), "h.py".into(), "python");
        fh.classes.push(cgg_core::ClassDecl {
            class_qn: "hyperopt.HyperoptService".into(),
            base_types: vec!["api_pb2_grpc.SuggestionServicer".into()],
            line: 1,
        });
        let mut fg = FileFacts::new(FileId::new(2), "api.go".into(), "go");
        fg.classes.push(cgg_core::ClassDecl {
            class_qn: "v1.APIService".into(),
            base_types: vec!["pb.UnimplementedSuggestionServer".into()],
            line: 1,
        });
        let mut fw = FileFacts::new(FileId::new(3), "w.py".into(), "python");
        fw.classes.push(cgg_core::ClassDecl {
            class_qn: "w.Wrapper".into(),
            base_types: vec!["SuggestionClient".into()],
            line: 1,
        });
        let dsts: Vec<u64> = link_descriptors(&g, &[fh, fg, fw])
            .iter()
            .map(|e| e.dst.as_u64())
            .collect();
        assert_eq!(dsts, vec![1, 2], "{dsts:?}");
    }

    /// BACKLOG P1 (connexion): a dotted `operationId` links to the Python
    /// callable of that qualified name (exact: medium; a tail match past
    /// a leading package: low); a bare one, or another language's
    /// same-named function, links to nothing.
    #[test]
    fn an_openapi_operation_id_names_its_python_handler() {
        let mut g = graph_in(&[
            (0, "annif.rest.list_vocabs", "openapi", "api.yaml"),
            (1, "listPets", "openapi", "api.yaml"),
            (2, "annif.rest.list_vocabs", "python", "annif/rest.py"),
            (
                3,
                "src.annif.rest.list_vocabs",
                "python",
                "src/annif/rest.py",
            ),
            (4, "other.list_vocabs", "python", "other.py"),
            (5, "annif.rest.list_vocabs", "javascript", "r.js"),
            (6, "pets.listPets", "python", "pets.py"),
        ]);
        for id in [0u32, 1] {
            g.callables
                .get_mut(&CallableId::new(id))
                .unwrap()
                .attributes
                .push(cgg_lang::plugins::openapi::OPERATION_ATTR.into());
        }
        let got: Vec<(u64, Confidence)> = link_descriptors(&g, &[])
            .iter()
            .map(|e| (e.dst.as_u64(), e.confidence))
            .collect();
        assert_eq!(got, vec![(2, Confidence::Medium), (3, Confidence::Low)]);
    }

    #[test]
    fn the_service_declaration_itself_links_to_nothing() {
        let g = graph_with(&[(0, "Greeter", "proto"), (1, "x.Greeter.Greeter", "go")]);
        assert!(link_descriptors(&g, &[]).is_empty());
    }
}
