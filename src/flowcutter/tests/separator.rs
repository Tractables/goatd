use crate::flowcutter_rs::flowcutter_compute_separator;

#[test]
fn separator_on_path_graph_is_small() {
    let n = 10;
    let edges: Vec<(u32, u32)> = (0..9).map(|i| (i, i + 1)).collect();
    let r = flowcutter_compute_separator(n, &edges, 10_000, 3, 0).expect("separator on path");
    assert_eq!(r.separator.len() + r.side_a.len() + r.side_b.len(), n);
    assert!(!r.side_a.is_empty());
    assert!(!r.side_b.is_empty());
    assert!(
        r.separator.len() <= 2,
        "path separator too big: {:?}",
        r.separator
    );
}

#[test]
fn separator_on_disconnected_returns_none_or_valid() {
    // Two disjoint triangles.  FlowCutter may return an empty separator
    // (graph already disconnected) which our wrapper turns into None.
    let n = 6;
    let edges = vec![(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5)];
    let r = flowcutter_compute_separator(n, &edges, 10_000, 3, 0);
    if let Some(r) = r {
        assert!(!r.side_a.is_empty());
        assert!(!r.side_b.is_empty());
    }
}
