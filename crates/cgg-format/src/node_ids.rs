//! How a formatter names nodes in its output.
//!
//! The graph's own [`CallableId`] is a 52-bit content hash rendered in
//! base36 — about ten characters, deliberately non-sequential so a
//! consumer cannot mistake it for an index to diff across runs. That is
//! the right identity for the JSON document, which `--from-graph` reads
//! back and which is the only format anything re-ingests.
//!
//! It is the wrong identity for a diagram. A mermaid node id appears
//! once in the node declaration and again in every edge that touches it
//! — on cgg's own graph that is 2,232 nodes against 3,792 edge pairs, so
//! roughly three mentions per node — and a random base36 string is worst
//! case for a BPE tokenizer, costing several tokens for what is
//! semantically one opaque handle. The primary consumer of mermaid here
//! is a coding agent reading it in a context window, where those tokens
//! are the scarce resource. Measured over `cgg ./crates -t mermaid`,
//! numbering the nodes takes the output from 275,772 to 209,237 bytes
//! (-24.1%) and from 127,536 to 80,360 `o200k_base` tokens (**-37.0%**).
//! The token saving is half again the byte saving because a random
//! base36 string costs roughly one token per two characters, while `N7`
//! is one token outright.
//!
//! # Why not the qualified name
//!
//! The obvious alternative — use `cgg_walk::walk` as the node id and
//! drop the label line — is both wrong and bigger. Wrong, because
//! qualified names are not unique: on cgg's own tree 41 of 2,202
//! callables share one with another callable (`cgg::cgg` ×9, `cgg::write`
//! ×8, `cgg::run` ×4), so 41 nodes and 25 distinct edge pairs would
//! silently merge. Across the benchmark corpus the population that
//! shares `(language, file, owner, qualified_name)` is 17.7% — see the
//! overload discussion in `cgg::stable_ids::StableIds::callable`. Bigger,
//! because a name repeated on every edge costs more than a short handle
//! plus one label line: the same measurement gives 359,861 bytes, 34%
//! *worse* than the hash it was meant to replace.
//!
//! Numbering is the only one of the three that is both smaller and
//! collision-free, and it is collision-free by construction rather than
//! by luck.

use cgg_core::Graph;
use cgg_core::ids::CallableId;
use serde::{Deserialize, Serialize};

/// Which naming scheme a formatter uses for node ids.
///
/// Serialized in lowercase, matching the `--node-ids` spellings, so the
/// C ABI's JSON options document names it the same way the CLI does.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeIds {
    /// The full base36 content hash — stable across runs, so two
    /// renderings of different revisions diff meaningfully.
    #[default]
    Hash,
    /// The node's position in the graph, numbered from zero. Compact and
    /// unique within one document, but positional: inserting a callable
    /// early renumbers everything after it, so two renderings of
    /// different revisions do not diff meaningfully.
    Short,
}

impl NodeIds {
    /// The scheme a run actually renders with: the caller's explicit
    /// choice, else the format's own default — except for a format whose
    /// ids are identity, which is always [`NodeIds::Hash`].
    ///
    /// One definition, called by both `cgg::emit` (which renders the
    /// artifact) and `cgg::RunOptions` (whose `--rollup` budget has to be
    /// measured against the renderer that will actually run). Deriving it
    /// twice would let the budget be measured against one document while
    /// a different one is emitted, which is the bug this exists to make
    /// unrepresentable.
    pub fn resolve(choice: Option<NodeIds>, format: crate::OutputFormat) -> NodeIds {
        if format.node_ids_are_identity() {
            return NodeIds::Hash;
        }
        choice.unwrap_or_else(|| format.default_node_ids())
    }
}

impl std::str::FromStr for NodeIds {
    type Err = String;

    /// Parses the `--node-ids` spellings, so the CLI, Python and Node all
    /// accept exactly the same two words.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "short" => Ok(NodeIds::Short),
            "hash" => Ok(NodeIds::Hash),
            other => Err(format!(
                "unknown node-id scheme {other:?}: expected \"short\" or \"hash\""
            )),
        }
    }
}

/// Renders node ids for one graph under one scheme.
///
/// Holds no side table. `Graph::callables` is an `IndexMap`, so a node's
/// ordinal is already `get_index_of` — one hash lookup, the same cost a
/// side map would charge, with nothing to allocate or keep in sync.
#[derive(Debug)]
pub struct NodeNamer<'g> {
    graph: &'g Graph,
    scheme: NodeIds,
    /// Prefix under [`NodeIds::Hash`] — each formatter's existing one,
    /// so nothing about hashed output moves.
    hash_prefix: &'static str,
    /// Prefix under [`NodeIds::Short`]. Mermaid uses a different letter
    /// from its hashed form so a reader can tell at a glance which
    /// scheme produced a document; `C0` is a legal hash id too.
    short_prefix: &'static str,
}

impl<'g> NodeNamer<'g> {
    pub fn new(
        graph: &'g Graph,
        scheme: NodeIds,
        hash_prefix: &'static str,
        short_prefix: &'static str,
    ) -> Self {
        Self {
            graph,
            scheme,
            hash_prefix,
            short_prefix,
        }
    }

    /// The id as it appears in the output, prefix included.
    ///
    /// An id the graph does not contain falls back to its hash form even
    /// under [`NodeIds::Short`]. Edges are pruned alongside their
    /// endpoints, so this should not arise — but the failure mode if it
    /// ever did must not be *reusing another node's number*, which would
    /// draw an arrow to the wrong function with nothing in the output to
    /// show for it.
    pub fn name(&self, id: CallableId) -> String {
        match self.scheme {
            NodeIds::Hash => format!("{}{}", self.hash_prefix, id.token()),
            NodeIds::Short => match self.graph.callables.get_index_of(&id) {
                Some(i) => format!("{}{}", self.short_prefix, i),
                None => format!("{}{}", self.hash_prefix, id.token()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgg_core::graph::{CallableKind, CallableNode};
    use cgg_core::ids::FileId;

    fn graph_of(ids: &[u64]) -> Graph {
        let mut g = Graph::new();
        for (i, raw) in ids.iter().enumerate() {
            g.add_callable(CallableNode {
                id: CallableId::new_u64(*raw),
                qualified_name: format!("f{i}"),
                simple_name: format!("f{i}"),
                kind: CallableKind::Function,
                language: "rust".into(),
                file: FileId::new(0),
                ..Default::default()
            });
        }
        g
    }

    use crate::OutputFormat;

    #[test]
    fn resolve_prefers_the_explicit_choice_then_the_format_default() {
        assert_eq!(
            NodeIds::resolve(None, OutputFormat::Mermaid),
            NodeIds::Short
        );
        assert_eq!(NodeIds::resolve(None, OutputFormat::Dot), NodeIds::Hash);
        assert_eq!(
            NodeIds::resolve(Some(NodeIds::Hash), OutputFormat::Mermaid),
            NodeIds::Hash
        );
        assert_eq!(
            NodeIds::resolve(Some(NodeIds::Short), OutputFormat::Dot),
            NodeIds::Short
        );
    }

    /// JSON ids are identity, so nothing can renumber them — not even an
    /// explicit ask. Without this, a `--rollup` budget measured for a
    /// JSON run would be computed against a document that never exists.
    #[test]
    fn resolve_never_renumbers_a_format_whose_ids_are_identity() {
        for choice in [None, Some(NodeIds::Short), Some(NodeIds::Hash)] {
            assert_eq!(
                NodeIds::resolve(choice, OutputFormat::Json),
                NodeIds::Hash,
                "{choice:?}"
            );
        }
    }

    #[test]
    fn the_two_spellings_parse_and_nothing_else_does() {
        assert_eq!("short".parse::<NodeIds>(), Ok(NodeIds::Short));
        assert_eq!("hash".parse::<NodeIds>(), Ok(NodeIds::Hash));
        assert!("Short".parse::<NodeIds>().is_err());
        assert!("".parse::<NodeIds>().is_err());
        let e = "sequential".parse::<NodeIds>().unwrap_err();
        assert!(e.contains("sequential") && e.contains("short"), "{e}");
    }

    #[test]
    fn hash_scheme_renders_the_base36_token() {
        let g = graph_of(&[0, 35, 36]);
        let n = NodeNamer::new(&g, NodeIds::Hash, "C", "N");
        assert_eq!(n.name(CallableId::new_u64(0)), "C0");
        assert_eq!(n.name(CallableId::new_u64(35)), "Cz");
        assert_eq!(n.name(CallableId::new_u64(36)), "C10");
    }

    #[test]
    fn short_scheme_renders_the_ordinal() {
        let g = graph_of(&[7_919, 104_729, 15_485_863]);
        let n = NodeNamer::new(&g, NodeIds::Short, "C", "N");
        assert_eq!(n.name(CallableId::new_u64(7_919)), "N0");
        assert_eq!(n.name(CallableId::new_u64(104_729)), "N1");
        assert_eq!(n.name(CallableId::new_u64(15_485_863)), "N2");
    }

    /// An id the graph does not contain has no ordinal. It must fall back
    /// to its hash rather than to some number, because every number in
    /// range already belongs to a real node — reusing one would draw an
    /// arrow to the wrong function and leave nothing in the output to say
    /// so.
    #[test]
    fn an_unknown_id_falls_back_to_its_hash() {
        let g = graph_of(&[7_919]);
        let n = NodeNamer::new(&g, NodeIds::Short, "C", "N");
        assert_eq!(n.name(CallableId::new_u64(7_919)), "N0");
        assert_eq!(n.name(CallableId::new_u64(104_729)), "C28t5");
    }

    /// Both prefixes are honoured, so a formatter that already had one
    /// (`n` in dot and graphml) keeps its hashed output byte-identical.
    #[test]
    fn each_scheme_uses_its_own_prefix() {
        let g = graph_of(&[7_919]);
        let id = CallableId::new_u64(7_919);
        assert_eq!(NodeNamer::new(&g, NodeIds::Hash, "n", "n").name(id), "n63z");
        assert_eq!(NodeNamer::new(&g, NodeIds::Short, "n", "n").name(id), "n0");
    }
}
