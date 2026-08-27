use super::sorted_lists_intersect;

#[test]
fn sorted_holder_intersection_detects_only_a_shared_bag() {
    assert!(sorted_lists_intersect(&[1, 4, 9], &[0, 4, 8]));
    assert!(!sorted_lists_intersect(&[1, 4, 9], &[0, 3, 8]));
}

mod validation;
