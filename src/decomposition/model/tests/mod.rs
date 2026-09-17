use crate::tests::td_fixture::make_td;

use super::TdBag;

#[test]
fn algorithm_bag_preserves_its_stable_vertex_order() {
    assert_eq!(
        TdBag::from_algorithm_order(vec![3, 1, 2]).vertices(),
        [3, 1, 2],
    );
}

/// The mass is the log2 of the sum over bags of 2^|bag|, which the comparison
/// package's validator computes the same way, and the separator is the largest
/// intersection of two adjacent bags.
#[test]
fn the_shape_numbers_are_the_bag_mass_and_the_widest_join() {
    // Bags of 3, 2 and 3 vertices in a path, sharing two vertices at the first
    // join and one at the second.
    let td = make_td(
        vec![vec![0, 1, 2], vec![1, 2], vec![2, 3, 4]],
        vec![(0, 1), (1, 2)],
    );
    let mass = td.bag_mass();
    let direct = (8f64 + 4f64 + 8f64).log2();
    assert!((mass - direct).abs() < 1e-12, "{mass} against {direct}");
    assert_eq!(td.max_separator(), 2);
}

/// The widest join is found wherever it sits in the tree, not only at the
/// first edge: each edge is taken once, from its lower-numbered bag.
#[test]
fn the_widest_join_is_found_at_the_far_end_of_the_tree() {
    // (0,1) shares one vertex, (1,2) shares three.
    let td = make_td(
        vec![vec![0, 1], vec![1, 2, 3, 4], vec![2, 3, 4, 5]],
        vec![(0, 1), (1, 2)],
    );
    assert_eq!(td.max_separator(), 3);
}

/// Nothing to compile and nothing to carry.
#[test]
fn an_empty_decomposition_has_no_mass_and_no_separator() {
    let empty = make_td(vec![], vec![]);
    assert_eq!(empty.bag_mass(), 0.0);
    assert_eq!(empty.max_separator(), 0);
}

/// The sum is scaled by the largest bag before the logarithm, so bags far past
/// the exponent range still give an answer.
#[test]
fn the_mass_of_huge_bags_does_not_overflow() {
    let bag: Vec<u32> = (0..3_000).collect();
    let other: Vec<u32> = (0..2_999).collect();
    let td = make_td(vec![bag, other], vec![(0, 1)]);
    let mass = td.bag_mass();
    assert!(mass.is_finite());
    // 2^3000 + 2^2999 = 1.5 * 2^3000.
    assert!((mass - (3_000.0 + 1.5f64.log2())).abs() < 1e-9, "{mass}");
    assert_eq!(td.max_separator(), 2_999);
}

mod validation;
