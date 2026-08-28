use super::{TdBag, sorted_lists_intersect};

#[test]
fn algorithm_bag_preserves_its_stable_vertex_order() {
    assert_eq!(
        TdBag::from_algorithm_order(vec![3, 1, 2]).vertices(),
        [3, 1, 2],
    );
}

#[test]
fn sorted_holder_intersection_detects_only_a_shared_bag() {
    assert!(sorted_lists_intersect(&[1, 4, 9], &[0, 4, 8]));
    assert!(!sorted_lists_intersect(&[1, 4, 9], &[0, 3, 8]));
}

mod validation;
