//! The pure-Rust separator search, over the entry points the module keeps
//! inside `decompose`.

mod cutter;
mod separator;

use super::{MAX_EXPANDED_BASE, select_random_st_pair, validate_graph_size};

#[test]
fn expanded_graph_size_guard_checks_its_exact_boundary() {
    let vertices = 3;
    let max_edges = ((MAX_EXPANDED_BASE - vertices) / 2) as usize;

    validate_graph_size(vertices as u32, max_edges).expect("the index limit is inclusive");
    assert!(validate_graph_size(vertices as u32, max_edges + 1).is_err());
}

/// The source and sink draw is part of the answer this module documents as a
/// function of the graph and the seed, so the stream is pinned here: moving it
/// moves every separator a stored seed produces.
#[test]
fn the_source_and_sink_draw_is_pinned_to_one_stream() {
    let (s, t) = select_random_st_pair(64, 7).expect("64 vertices leave a pair to draw");
    assert_eq!((s, t), (36, 59));

    // The pair never repeats a vertex, and a graph with one vertex has no pair
    // to draw at all.
    assert_ne!(s, t);
    assert!(select_random_st_pair(1, 7).is_none());
}
