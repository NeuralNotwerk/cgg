//! Output format trait and `OutputFormat` enum.
//!
//! Task 9 fleshes out all four formatters behind this single trait.
//! Earlier tasks (5, 4 demos) may implement the minimal mermaid/json
//! writers for their own end-to-end proofs.

#![deny(missing_debug_implementations)]
#![warn(unreachable_pub)]

pub mod dot;
pub mod graphml;
pub mod json;
pub mod mermaid;
pub mod node_ids;

pub use dot::DotFormatter;
pub use graphml::GraphmlFormatter;
pub use json::JsonFormatter;
pub use mermaid::MermaidFormatter;
pub use node_ids::{NodeIds, NodeNamer};

use cgg_core::Graph;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;

/// User-facing output format selected via `-t`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Mermaid,
    Json,
    Dot,
    Graphml,
}

impl OutputFormat {
    /// The node-id scheme this format uses when the caller did not pick
    /// one.
    ///
    /// Mermaid defaults to [`NodeIds::Short`] because its consumer is a
    /// coding agent's context window, where the id's token cost is the
    /// binding constraint and nothing reads the ids back. Every other
    /// format defaults to [`NodeIds::Hash`]: dot and graphml go to
    /// layout tools that do not care either way, and JSON's ids are the
    /// content-derived identity `--from-graph` replays, which
    /// [`Self::node_ids_are_identity`] refuses to let anything override.
    pub fn default_node_ids(&self) -> NodeIds {
        match self {
            OutputFormat::Mermaid => NodeIds::Short,
            OutputFormat::Json | OutputFormat::Dot | OutputFormat::Graphml => {
                NodeIds::Hash
            }
        }
    }

    /// Whether this format's node ids are load-bearing identity rather
    /// than a rendering detail, and so cannot be renumbered.
    ///
    /// True only for JSON. Its ids are the stable content-derived hashes
    /// `cgg::stable_ids` exists to produce: `--from-graph` reads that
    /// document back, and consumers diff ids across runs to tell an
    /// unchanged callable from a new one. Positional ids would break
    /// both, silently.
    pub fn node_ids_are_identity(&self) -> bool {
        matches!(self, OutputFormat::Json)
    }

    pub fn default_extension(&self) -> &'static str {
        match self {
            OutputFormat::Mermaid => "mmd",
            OutputFormat::Json => "json",
            OutputFormat::Dot => "dot",
            OutputFormat::Graphml => "graphml",
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutputFormat::Mermaid => "mermaid",
            OutputFormat::Json => "json",
            OutputFormat::Dot => "dot",
            OutputFormat::Graphml => "graphml",
        })
    }
}

/// The contract every formatter implements.
pub trait GraphFormatter: Send + Sync + fmt::Debug {
    fn format(&self) -> OutputFormat;
    /// Render the whole graph to `out`. Implementations must not panic
    /// on empty graphs.
    fn render(&self, graph: &Graph, out: &mut dyn io::Write) -> io::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_is_the_only_format_that_numbers_by_default() {
        assert_eq!(OutputFormat::Mermaid.default_node_ids(), NodeIds::Short);
        for f in [OutputFormat::Json, OutputFormat::Dot, OutputFormat::Graphml] {
            assert_eq!(f.default_node_ids(), NodeIds::Hash, "{f}");
        }
    }

    #[test]
    fn only_json_ids_are_identity() {
        assert!(OutputFormat::Json.node_ids_are_identity());
        for f in [
            OutputFormat::Mermaid,
            OutputFormat::Dot,
            OutputFormat::Graphml,
        ] {
            assert!(!f.node_ids_are_identity(), "{f}");
        }
    }

    #[test]
    fn default_extension_is_stable() {
        assert_eq!(OutputFormat::Mermaid.default_extension(), "mmd");
        assert_eq!(OutputFormat::Graphml.default_extension(), "graphml");
    }
}
