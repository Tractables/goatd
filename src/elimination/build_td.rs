//! Elimination order → `TreeDecomposition`.
//!
//! Builds the bag tree with the elimination-order clique-tree rule (parent =
//! earliest-eliminated neighbour still in the bag) — not the junction-tree
//! construction the vendored C++ FlowCutter uses for the same job. Both yield
//! valid tree decompositions and are not expected to agree; this one does not
//! dedup non-maximal bags.

use crate::{TdBag, TreeDecomposition};

/// Build a `TreeDecomposition` from a per-step elimination record.
///
/// `steps[s]` is the eliminated vertex followed by the live neighbours that
/// were in its bag. `rank[v]` is the step at which vertex `v` was eliminated.
pub(super) fn build_td_from_steps(steps: Vec<Vec<u32>>, rank: &[u32]) -> TreeDecomposition {
    let n_bags = steps.len();
    debug_assert_eq!(n_bags, rank.len());
    debug_assert!(rank.iter().all(|&step| step < n_bags as u32));
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n_bags];
    let n_bags_u32 = n_bags as u32;
    for (step, vertices) in steps.iter().enumerate() {
        if vertices.len() <= 1 {
            continue;
        }
        let mut best = u32::MAX;
        for &u in &vertices[1..] {
            let r = rank[u as usize];
            if r < best {
                best = r;
            }
        }
        if best < n_bags_u32 {
            adj[step].push(best as usize);
            adj[best as usize].push(step);
        }
    }
    let bags = steps.into_iter().map(TdBag::new).collect();

    TreeDecomposition::from_parts(rank.len() as u32, bags, adj)
}
