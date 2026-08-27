//! The pure-Rust separator search, over the entry points the module keeps
//! inside `decompose`.

mod separator;

use super::{MAX_EXPANDED_BASE, validate_graph_size};

#[test]
fn expanded_graph_size_guard_checks_its_exact_boundary() {
    let vertices = 3;
    let max_edges = ((MAX_EXPANDED_BASE - vertices) / 2) as usize;

    validate_graph_size(vertices as u32, max_edges).expect("the index limit is inclusive");
    assert!(validate_graph_size(vertices as u32, max_edges + 1).is_err());
}
