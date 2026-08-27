//! Tests of the graph partitioner's private scoring representation.

mod initial;

use super::{MAX_BISECTION_EDGES, validate_size};
use crate::Error;

#[test]
fn csr_size_guard_bounds_the_directed_arc_count() {
    validate_size(MAX_BISECTION_EDGES).expect("the documented edge limit is inclusive");
    assert!(matches!(
        validate_size(MAX_BISECTION_EDGES + 1),
        Err(Error::TooLarge(_))
    ));
}
