use super::super::native::reconstruct_native_adjacency;

#[test]
fn readback_orders_lower_bags_before_native_forward_neighbors() {
    let native = vec![vec![4, 2, 1], vec![3, 0], vec![0], vec![1], vec![0]];

    assert_eq!(
        reconstruct_native_adjacency(&native),
        vec![vec![4, 2, 1], vec![0, 3], vec![0], vec![1], vec![0]],
    );
}
