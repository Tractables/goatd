use crate::elimination::vertex_cover_separator::*;

#[test]
fn trivial_path_separator_is_single_middle_vertex() {
    let edges = vec![(0u32, 1), (1, 2), (2, 3), (3, 4)];
    let part = vec![0u8, 0, 1, 1, 1];
    let r = minimum_vertex_cover_separator(5, &edges, &part);
    assert_eq!(r.separator.len(), 1);
    assert!(r.separator[0] == 1 || r.separator[0] == 2);
    assert_eq!(r.side_a.len() + r.side_b.len(), 4);
}

#[test]
fn cover_beats_smaller_boundary_on_star_crossing() {
    let edges = vec![(1u32, 10), (1, 11), (1, 12), (0, 1)];
    let mut part = vec![0u8; 13];
    part[10..=12].fill(1);
    let r = minimum_vertex_cover_separator(13, &edges, &part);
    assert_eq!(r.separator, vec![1]);
}

#[test]
fn cover_strictly_smaller_than_smaller_boundary() {
    // A={1,2,3}, B={10}, all cross-edges converge on the hub — sanity-checks
    // the general property (cover <= smaller-boundary) on an asymmetric case.
    let edges = vec![(1u32, 10), (2, 10), (3, 10), (0, 1), (0, 2), (0, 3)];
    let mut part = vec![0u8; 11];
    part[10] = 1;
    let r = minimum_vertex_cover_separator(11, &edges, &part);
    assert_eq!(r.separator, vec![10]);
}

#[test]
fn empty_cut_yields_empty_separator() {
    let edges = vec![(0u32, 1), (1, 2)];
    let part = vec![0u8; 3];
    let r = minimum_vertex_cover_separator(3, &edges, &part);
    assert!(r.separator.is_empty());
    assert_eq!(r.side_a.len(), 3);
    assert!(r.side_b.is_empty());
}

#[test]
fn a_left_vertex_retries_a_right_vertex_the_one_before_it_took() {
    // Left boundary {1, 2}, right boundary {3, 4}. Vertex 1 is matched to 3
    // first; 2's only cross-edge is to 3, so it has to reach through 1 and move
    // it to 4. That augmenting path exists only if 2's search starts with 3
    // unvisited rather than carrying 1's mark, so the matching is 2 and the
    // whole left boundary is the cover. With 3 still marked the matching would
    // be 1 and the cover would come out as the right boundary instead.
    let edges = vec![(1u32, 3), (1, 4), (2, 3)];
    let part = vec![0u8, 0, 0, 1, 1];
    let r = minimum_vertex_cover_separator(5, &edges, &part);

    assert_eq!(r.separator, vec![1, 2]);
    assert_eq!(r.side_a, vec![0]);
    assert_eq!(r.side_b, vec![3, 4]);
}
