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
pub(crate) fn build_td_from_ranked_bags(
    ranked_bags: Vec<Vec<u32>>,
    rank: &[u32],
) -> TreeDecomposition {
    let n_bags = ranked_bags.len();
    debug_assert!(n_bags <= rank.len());
    debug_assert!(rank.iter().all(|&step| step < n_bags as u32));
    let n_bags_u32 = n_bags as u32;

    // Each bag has at most one parent, and as many further neighbours as it has
    // children, so the row lengths are known before anything is pushed. Sizing
    // the rows up front saves the regrowth a large decomposition pays on every
    // one of them: a graph of a few hundred thousand vertices produces that
    // many rows per candidate.
    let mut parent: Vec<u32> = Vec::with_capacity(n_bags);
    let mut degree: Vec<u32> = vec![0; n_bags];
    // The parent rule and the bag's own ordering both read the bag's
    // vertices, so they read them together: on a decomposition with a few
    // hundred thousand bags, visiting them a second time is a second pass
    // over all of it, and the second pass misses every line the first
    // brought in.
    let mut bags: Vec<TdBag> = Vec::with_capacity(n_bags);
    for (step, vertices) in ranked_bags.into_iter().enumerate() {
        let mut best = u32::MAX;
        for &u in &vertices {
            let r = rank[u as usize];
            if r > step as u32 && r < best {
                best = r;
            }
        }
        parent.push(best);
        if best < n_bags_u32 {
            degree[step] += 1;
            degree[best as usize] += 1;
        }
        bags.push(TdBag::new(vertices));
    }

    let mut adj: Vec<Vec<usize>> = degree
        .iter()
        .map(|&d| Vec::with_capacity(d as usize))
        .collect();
    for (step, &best) in parent.iter().enumerate() {
        if best < n_bags_u32 {
            adj[step].push(best as usize);
            adj[best as usize].push(step);
        }
    }

    TreeDecomposition::from_parts(rank.len() as u32, bags, adj)
}
