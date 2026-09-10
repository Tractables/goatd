//! Exchange a fill edge for a clique on its common neighbourhood.
//!
//! The bags containing both ends form a connected subtree. Replace that
//! subtree by two bags: its vertex union without either endpoint. Each
//! outside branch meets at most one endpoint, so it attaches to one of the
//! replacement bags. This removes the chosen fill edge and completes its
//! common neighbourhood. A subsequent minimalization can remove other fill.

use std::time::{Duration, Instant};

use rustc_hash::FxHashSet;

use super::{RowSet, common_neighbourhood_is_clique, completion, minimalization_candidate};
use crate::{Graph, TreeDecomposition};

#[cfg(test)]
mod tests;

/// Work performed by an exchange search and its accepted improvements.
#[derive(Default, Debug)]
pub struct Stats {
    /// Candidate exchanges enumerated across all rounds.
    pub eligible: usize,
    /// Exchanges constructed and minimalized.
    pub tried: usize,
    /// Strict improvements accepted.
    pub improved: usize,
    /// Enumeration rounds begun.
    pub rounds: usize,
    /// Elapsed milliseconds, width, and log mass after each accepted move.
    pub improvements: Vec<(u128, u32, f64)>,
}

// Integer limbs prevent bag order or tiny floating-point differences from
// making an unchanged triangulation look like an improvement.
fn quality(tree: &TreeDecomposition) -> (u32, usize, Vec<u64>) {
    let mut mass = Vec::<u64>::new();
    for bag in tree.bags() {
        let size = bag.vertices().len();
        let mut index = size / 64;
        let mut add = 1u64 << (size % 64);
        loop {
            mass.resize(mass.len().max(index + 1), 0);
            let (sum, carry) = mass[index].overflowing_add(add);
            mass[index] = sum;
            if !carry {
                break;
            }
            index += 1;
            add = 1;
        }
    }
    mass.reverse();
    (tree.treewidth(), mass.len(), mass)
}

/// Replace the connected set of bags containing a non-input edge `uv`.
/// Callers provide valid input and enforce the chosen width ceiling.
fn flip(graph: &Graph, tree: &TreeDecomposition, u: u32, v: u32) -> Option<TreeDecomposition> {
    if graph.edges().binary_search(&(u.min(v), u.max(v))).is_ok() {
        return None;
    }
    let n = graph.num_vertices() as usize;
    let mut inside = vec![false; tree.bags().len()];
    let mut union = vec![false; n];
    for (i, bag) in tree.bags().iter().enumerate() {
        if bag.vertices().contains(&u) && bag.vertices().contains(&v) {
            inside[i] = true;
            for &w in bag.vertices() {
                union[w as usize] = true;
            }
        }
    }
    if !inside.iter().any(|&yes| yes) {
        return None;
    }
    let left: Vec<u32> = union
        .iter()
        .enumerate()
        .filter_map(|(w, &held)| (held && w != v as usize).then_some(w as u32))
        .collect();
    let right: Vec<u32> = union
        .iter()
        .enumerate()
        .filter_map(|(w, &held)| (held && w != u as usize).then_some(w as u32))
        .collect();
    let mut bags = vec![left, right];
    let mut map = vec![0usize; inside.len()];
    for (i, bag) in tree.bags().iter().enumerate() {
        if !inside[i] {
            map[i] = bags.len();
            bags.push(bag.vertices().to_vec());
        }
    }
    let mut edges = vec![(0, 1)];
    for (a, b) in tree
        .adjacency()
        .iter()
        .enumerate()
        .flat_map(|(a, neighbours)| {
            neighbours
                .iter()
                .copied()
                .filter(move |&b| b > a)
                .map(move |b| (a, b))
        })
    {
        match (inside[a], inside[b]) {
            (false, false) => edges.push((map[a], map[b])),
            (true, true) => (),
            _ => {
                let outside = if inside[a] { b } else { a };
                let attach = usize::from(tree.bags()[outside].vertices().contains(&v));
                edges.push((map[outside], attach));
            }
        }
    }
    TreeDecomposition::new_trusted(graph, bags, edges).ok()
}

/// List nontrivial exchanges whose two replacement bags respect `width`.
fn candidates(
    graph: &Graph,
    tree: &TreeDecomposition,
    width: u32,
    deadline: Instant,
) -> Vec<(u32, u32)> {
    let n = graph.num_vertices() as usize;
    let words = n.div_ceil(64) as u64;
    let squares = tree.bags().iter().fold(0u64, |sum, bag| {
        let size = bag.vertices().len() as u64;
        sum.saturating_add(size.saturating_mul(size))
    });
    let projected = squares
        .saturating_mul(words)
        .saturating_add((n as u64).saturating_mul(n as u64));
    // Enumerating neighbours is preparation; leave the rest of the window
    // for constructing and minimalizing exchanges. This scales with the
    // available budget rather than imposing a fixed vertex-count cutoff.
    if Duration::from_millis(crate::meter::milliseconds_for_units(projected))
        > deadline.saturating_duration_since(crate::meter::now()) / 8
    {
        return Vec::new();
    }
    let Some(rows) = completion(tree, n, Some(deadline)) else {
        return Vec::new();
    };
    let mut neighbours = Vec::new();
    let mut common = vec![0u64; rows.words];
    let mut members = Vec::new();
    let original: FxHashSet<_> = graph.edges().iter().copied().collect();
    let mut found = Vec::new();
    for u in 0..n {
        RowSet::members(rows.row(u), &mut neighbours);
        for &v in neighbours.iter().filter(|&&v| v as usize > u) {
            if crate::deadline::expired(Some(deadline)) {
                return found;
            }
            if original.contains(&(u as u32, v)) {
                continue;
            }
            if !common_neighbourhood_is_clique(&rows, u, v as usize, &mut common, &mut members)
                && members.len() <= width as usize
            {
                found.push((u as u32, v));
            }
        }
    }
    found
}

/// Try exchanges followed by the existing minimal triangulation pass.
/// Only strict width-then-mass improvements become the next starting tree.
/// All comparisons use compacted bags. The deadline bounds the search;
/// validation and compaction also run when it has already expired.
///
/// # Errors
/// Returns an error when `start` is not a valid decomposition of `graph`.
pub fn improve(
    graph: &Graph,
    start: &TreeDecomposition,
    deadline: Instant,
) -> Result<(TreeDecomposition, Stats), crate::Error> {
    start.validate(graph)?;
    let started = Instant::now();
    let mut best = start.subsumed_bag_compaction().apply(start.clone());
    let mut stats = Stats::default();
    while !crate::deadline::expired(Some(deadline)) {
        stats.rounds += 1;
        let options = candidates(graph, &best, best.treewidth(), deadline);
        stats.eligible += options.len();
        let mut moved = false;
        for (u, v) in options {
            if crate::deadline::expired(Some(deadline)) {
                break;
            }
            let Some(candidate) = flip(graph, &best, u, v) else {
                continue;
            };
            debug_assert!(candidate.treewidth() <= best.treewidth());
            stats.tried += 1;
            let candidate = candidate.subsumed_bag_compaction().apply(candidate);
            let candidate =
                minimalization_candidate(&candidate, graph, Some(deadline)).unwrap_or(candidate);
            let candidate = candidate.subsumed_bag_compaction().apply(candidate);
            if quality(&candidate) < quality(&best) {
                best = candidate;
                stats.improved += 1;
                let log_mass = best.shape().0;
                stats.improvements.push((
                    started.elapsed().as_millis(),
                    best.treewidth(),
                    log_mass,
                ));
                moved = true;
                break;
            }
        }
        if !moved {
            break;
        }
    }
    Ok((best, stats))
}
