//! Guards on the size of the two structs the pipeline allocates most.
//!
//! These are not style checks. `CallableNode` is held one-per-callable
//! for the whole run and `CallEdge` one-per-call-site, so a field added
//! to either is multiplied by hundreds of thousands on a real tree.
//!
//! The numbers here were earned: 0.8.1 added `Option<RollupMeta>` inline
//! to `CallableNode` and grew it from 208 to 272 bytes — 31%, paid by
//! every callable of every run for a field that is `None` unless someone
//! passed `--rollup`. A paired A/B against 0.8.0 over the benchmark
//! corpus measured about +2.9% wall clock. Boxing the field put it back.
//!
//! If a change here is deliberate, update the number and say in the
//! commit what the field buys and what it costs. If it is not, box the
//! field instead — that is what the last one needed.

use cgg_core::graph::{CallEdge, CallableNode, RollupMeta};

#[test]
fn callable_node_has_not_quietly_grown() {
    assert_eq!(
        std::mem::size_of::<CallableNode>(),
        216,
        "CallableNode changed size — see this file's header before \
         updating the number"
    );
}

#[test]
fn call_edge_has_not_quietly_grown() {
    // `weight` cost nothing: it landed in padding that already existed.
    assert_eq!(
        std::mem::size_of::<CallEdge>(),
        88,
        "CallEdge changed size — see this file's header"
    );
}

#[test]
fn rollup_meta_is_boxed_where_it_is_stored() {
    // The point of the box: the optional field costs a pointer, not the
    // whole struct, on every node that does not use it.
    assert!(
        std::mem::size_of::<RollupMeta>()
            > std::mem::size_of::<Option<Box<RollupMeta>>>(),
        "if RollupMeta ever gets smaller than a pointer, drop the box"
    );
}
