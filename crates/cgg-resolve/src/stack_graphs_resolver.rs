//! Stack-graphs resolver stub.
//!
//! The full stack-graphs integration was removed in the tree-sitter 0.26
//! upgrade because the upstream `tree-sitter-stack-graphs` crate pins
//! tree-sitter 0.24 (ABI 14). Our cross-file resolver + type propagation
//! handles the same resolution with better performance.

use cgg_core::{
    audit::AuditUnresolvedCall,
    graph::{CallEdge, Graph},
    ids::FileId,
    FileFacts,
};

#[derive(Debug, Default)]
pub struct ResolveOutput {
    pub edges: Vec<CallEdge>,
    pub unresolved: Vec<AuditUnresolvedCall>,
}

#[derive(Debug)]
pub struct FileInput<'a> {
    pub file: FileId,
    pub language: &'a str,
    pub source: &'a [u8],
}

pub fn resolve(
    _graph: &Graph,
    _facts: &[FileFacts],
    _inputs: &[FileInput<'_>],
) -> ResolveOutput {
    ResolveOutput::default()
}

pub fn resolve_light(
    _graph: &Graph,
    _facts: &[FileFacts],
    _inputs: &[FileInput<'_>],
) -> ResolveOutput {
    ResolveOutput::default()
}

pub fn is_sg_language(_lang: &str) -> bool {
    false
}
