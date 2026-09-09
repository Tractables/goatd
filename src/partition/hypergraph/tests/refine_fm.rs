use crate::partition::hypergraph::model::Hypergraph;
use crate::partition::hypergraph::refine_fm::{RegionScratch, localized_fm_pass};

/// A path of `n` vertices as two-pin hyperedges, split down the middle with the
/// two vertices either side of the cut swapped onto the wrong sides.
fn misplaced_path(n: usize) -> (Hypergraph, Vec<u8>) {
    let hyperedges: Vec<Vec<u32>> = (0..n as u32 - 1).map(|v| vec![v, v + 1]).collect();
    let mut part: Vec<u8> = (0..n).map(|v| u8::from(v >= n / 2)).collect();
    part[n / 2 - 1] = 1;
    part[n / 2] = 0;
    (Hypergraph::from_hyperedges(n, &hyperedges, None), part)
}

/// Pins the move the pass makes and the moves it rolls back. The pass commits
/// the best prefix of its move sequence, which here is one move: the vertex
/// swapped across the cut goes back, leaving a single cut hyperedge. The moves
/// after it do not pay and are rolled back, so the two sides come out 11 and 9
/// rather than even.
#[test]
fn a_localized_pass_keeps_only_the_move_that_pays() {
    let (hg, start) = misplaced_path(20);
    let mut part = start;
    let mut scratch = RegionScratch::new();

    assert!(localized_fm_pass(&hg, &mut part, 9, 0.2, &mut scratch));
    let one_cut_hyperedge: Vec<u8> = (0..20).map(|v| u8::from(v >= 11)).collect();
    assert_eq!(part, one_cut_hyperedge);
}

/// The pass keeps its per-vertex arrays between calls and clears only the
/// entries it wrote, so a scratch that has already been used has to give what a
/// fresh one gives.
#[test]
fn a_localized_pass_on_used_scratch_matches_one_on_fresh_scratch() {
    let (hg, start) = misplaced_path(20);

    let mut scratch = RegionScratch::new();
    let mut first = start.clone();
    let first_improved = localized_fm_pass(&hg, &mut first, 9, 0.2, &mut scratch);

    // Same call, same scratch: a region mark, a lock or a stale gain left behind
    // by the first call would change what the second one does.
    let mut second = start.clone();
    let second_improved = localized_fm_pass(&hg, &mut second, 9, 0.2, &mut scratch);
    assert_eq!(first_improved, second_improved);
    assert_eq!(first, second);

    let mut fresh = RegionScratch::new();
    let mut third = start;
    let third_improved = localized_fm_pass(&hg, &mut third, 9, 0.2, &mut fresh);
    assert_eq!(first_improved, third_improved);
    assert_eq!(first, third);
}
