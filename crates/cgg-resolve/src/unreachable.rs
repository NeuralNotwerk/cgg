//! Consumers of `FileFacts::unreachable`: call sites control flow can
//! never reach.
//!
//! A plugin records a region after an unconditional terminator
//! (`return`, `throw`, `break`, …; `cgg-lang/src/plugins/cfg.rs`). A call
//! written there never runs, so its edge is not evidence that the callee
//! is used, and the edge itself is not structurally certain. Regions are
//! only computed under `--dead-code`, so the default graph never changes.

use std::collections::HashMap;

use cgg_core::FileFacts;
use cgg_core::graph::{CallEdge, Confidence, Graph};
use cgg_core::ids::FileId;

/// Unreachable byte ranges per file, sorted by start.
#[derive(Debug, Default)]
pub struct UnreachableIndex {
    regions: HashMap<FileId, Vec<(u32, u32)>>,
}

impl UnreachableIndex {
    pub fn build(facts: &[FileFacts]) -> Self {
        let mut regions: HashMap<FileId, Vec<(u32, u32)>> = HashMap::new();
        for f in facts {
            if f.unreachable.is_empty() {
                continue;
            }
            let v = regions.entry(f.file).or_default();
            v.extend(f.unreachable.iter().map(|r| (r.start_byte, r.end_byte)));
            v.sort_unstable();
        }
        UnreachableIndex { regions }
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty()
    }

    /// Whether `e`'s call site lies in a region of its caller's own body.
    /// A callable defined inside the region (a hoisted JS `function`, a
    /// nested closure) starts after the region does: its calls run when
    /// it is called, so they are not covered.
    pub fn covers(&self, graph: &Graph, e: &CallEdge) -> bool {
        if e.site_byte == 0 || self.regions.is_empty() {
            return false;
        }
        let Some(src) = graph.callables.get(&e.src) else {
            return false;
        };
        if src.synthetic {
            return false;
        }
        let Some(v) = self.regions.get(&src.file) else {
            return false;
        };
        let at = e.site_byte;
        // Regions starting at or before the site; nested blocks can each
        // hold one, so any of them may contain it.
        let i = v.partition_point(|r| r.0 <= at);
        v[..i]
            .iter()
            .any(|&(s, end)| at < end && src.start_byte <= s && s < src.end_byte)
    }
}

/// Cap every edge whose call site is unreachable at `medium`. Returns
/// how many edges moved from `high`, so the caller can keep its
/// confidence histogram in step.
pub fn cap_unreachable_edges(graph: &mut Graph, facts: &[FileFacts]) -> u64 {
    let idx = UnreachableIndex::build(facts);
    if idx.is_empty() {
        return 0;
    }
    let capped: Vec<usize> = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.confidence == Confidence::High && idx.covers(graph, e))
        .map(|(i, _)| i)
        .collect();
    for &i in &capped {
        graph.edges[i].confidence = Confidence::Medium;
    }
    capped.len() as u64
}
