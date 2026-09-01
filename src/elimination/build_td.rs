//! Elimination order → `TreeDecomposition`.
//!
//! Builds the bag tree with the elimination-order clique-tree rule (parent =
//! earliest-eliminated neighbour still in the bag) — not the junction-tree
//! construction the vendored C++ FlowCutter uses for the same job. Both yield
//! valid tree decompositions and are not expected to agree; this one does not
//! dedup non-maximal bags.

use crate::{TdBag, TreeDecomposition};

/// Build a `TreeDecomposition` from elimination bags and their vertex ranks.
///
/// An ordinary elimination bag has one vertex whose rank is the bag index and
/// zero or more later-ranked neighbours. A deadline completion may instead
/// put every vertex of an unfinished residual component in one bag and assign
/// all of them that bag's rank.
pub(super) fn build_td_from_ranked_bags(
    ranked_bags: Vec<Vec<u32>>,
    rank: &[u32],
) -> TreeDecomposition {
    let n_bags = ranked_bags.len();
    debug_assert!(n_bags <= rank.len());
    debug_assert!(rank.iter().all(|&step| step < n_bags as u32));
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n_bags];
    let n_bags_u32 = n_bags as u32;
    for (step, vertices) in ranked_bags.iter().enumerate() {
        let mut best = u32::MAX;
        for &u in vertices {
            let r = rank[u as usize];
            if r > step as u32 && r < best {
                best = r;
            }
        }
        if best < n_bags_u32 {
            adj[step].push(best as usize);
            adj[best as usize].push(step);
        }
    }
    let bags = ranked_bags.into_iter().map(TdBag::new).collect();

    TreeDecomposition::from_parts(rank.len() as u32, bags, adj)
}
