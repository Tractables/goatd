use super::super::{MAX_EDGES, MAX_VERTICES, vendor_size_guard_counts};
use crate::Error;

#[test]
fn the_vendor_size_guard_accepts_both_exact_limits() {
    vendor_size_guard_counts(MAX_VERTICES, MAX_EDGES).expect("the documented limits are inclusive");
}

#[test]
fn the_vendor_size_guard_names_an_excess_vertex_count() {
    let error = vendor_size_guard_counts(MAX_VERTICES + 1, 0)
        .expect_err("the adjacency matrix would be too large");

    assert!(matches!(error, Error::TooLarge(_)));
    assert!(error.to_string().contains(&(MAX_VERTICES + 1).to_string()));
}

#[test]
fn the_vendor_size_guard_names_an_excess_edge_count() {
    let error = vendor_size_guard_counts(1, MAX_EDGES + 1)
        .expect_err("the vendor allocations would be too large");

    assert!(matches!(error, Error::TooLarge(_)));
    assert!(error.to_string().contains(&(MAX_EDGES + 1).to_string()));
}
