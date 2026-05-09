//! Output format trait and `OutputFormat` enum.
//!
//! Task 9 fleshes out all four formatters behind this single trait.
//! Earlier tasks (5, 4 demos) may implement the minimal mermaid/json
//! writers for their own end-to-end proofs.

#![deny(missing_debug_implementations)]
#![warn(unreachable_pub)]

pub mod mermaid;

pub use mermaid::MermaidFormatter;

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
    fn default_extension_is_stable() {
        assert_eq!(OutputFormat::Mermaid.default_extension(), "mmd");
        assert_eq!(OutputFormat::Graphml.default_extension(), "graphml");
    }
}
