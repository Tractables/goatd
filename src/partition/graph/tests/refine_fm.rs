use crate::partition::common::BisectionStop;
use crate::partition::graph::csr::build_csr;
use crate::partition::graph::refine_fm::{RegionScratch, localized_fm_pass};

/// A path of `n` vertices, split down the middle with the two vertices either
/// side of the cut swapped onto the wrong sides.
fn misplaced_path(n: u32) -> (Vec<(u32, u32)>, Vec<u8>) {
    let edges: Vec<(u32, u32)> = (0..n - 1).map(|v| (v, v + 1)).collect();
    let mut part: Vec<u8> = (0..n).map(|v| u8::from(v >= n / 2)).collect();
    part[(n / 2 - 1) as usize] = 1;
    part[(n / 2) as usize] = 0;
    (edges, part)
}

/// Pins the move the pass makes and the moves it rolls back. The pass commits
/// the best prefix of its move sequence, which here is one move: the vertex
/// swapped across the cut goes back, leaving a single cut edge. The moves after
/// it do not pay and are rolled back, so the two sides come out 11 and 9 rather
/// than even. Checked against the same input before the pass kept its arrays in
/// scratch: same partition.
#[test]
fn a_localized_pass_keeps_only_the_move_that_pays() {
    let (edges, start) = misplaced_path(20);
    let graph = build_csr(20, &edges);
    let mut part = start;
    let mut scratch = RegionScratch::new();
    let mut stop = BisectionStop::new(None);

    assert!(localized_fm_pass(
        &graph,
        &mut part,
        9,
        0.2,
        &mut scratch,
        &mut stop
    ));
    let one_cut_edge: Vec<u8> = (0..20).map(|v| u8::from(v >= 11)).collect();
    assert_eq!(part, one_cut_edge);
}

/// The pass keeps its per-vertex arrays between calls and clears only the
/// entries it wrote, so a scratch that has already been used has to give what a
/// fresh one gives.
#[test]
fn a_localized_pass_on_used_scratch_matches_one_on_fresh_scratch() {
    let (edges, start) = misplaced_path(20);
    let graph = build_csr(20, &edges);
    let mut stop = BisectionStop::new(None);

    let mut scratch = RegionScratch::new();
    let mut first = start.clone();
    let first_improved = localized_fm_pass(&graph, &mut first, 9, 0.2, &mut scratch, &mut stop);

    // Same call, same scratch: a region mark, a lock or a boundary answer left
    // behind by the first call would change what the second one does.
    let mut second = start.clone();
    let second_improved = localized_fm_pass(&graph, &mut second, 9, 0.2, &mut scratch, &mut stop);
    assert_eq!(first_improved, second_improved);
    assert_eq!(first, second);

    let mut fresh = RegionScratch::new();
    let mut third = start;
    let third_improved = localized_fm_pass(&graph, &mut third, 9, 0.2, &mut fresh, &mut stop);
    assert_eq!(first_improved, third_improved);
    assert_eq!(first, third);
}
