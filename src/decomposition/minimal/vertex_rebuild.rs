//! Remove one vertex, simplify the remaining triangulation, and reinsert it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::{SharedCompletion, rebuild_candidate};
use crate::{Graph, TreeDecomposition};

#[cfg(test)]
mod tests;

/// Work performed by vertex reconstruction.
#[derive(Default, Debug)]
pub struct Stats {
    /// Vertices removed and reinserted.
    pub tried: usize,
    /// Strict improvements retained.
    pub improved: usize,
    /// Sweeps over the vertices begun.
    pub rounds: usize,
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

fn compact(mut tree: TreeDecomposition) -> TreeDecomposition {
    loop {
        let before = tree.bags().len();
        tree = tree.subsumed_bag_compaction().apply(tree);
        if tree.bags().len() == before {
            return tree;
        }
    }
}

fn edges(tree: &TreeDecomposition) -> Vec<(usize, usize)> {
    tree.adjacency()
        .iter()
        .enumerate()
        .flat_map(|(a, adjacent)| {
            adjacent
                .iter()
                .copied()
                .filter(move |&b| b > a)
                .map(move |b| (a, b))
        })
        .collect()
}

/// A connected support in each needed component, meeting every required vertex.
fn support(tree: &TreeDecomposition, required: &[bool]) -> Vec<bool> {
    let count = required.iter().filter(|&&yes| yes).count();
    let mut single = None;
    let mut occurrences = vec![0usize; required.len()];
    for (index, bag) in tree.bags().iter().enumerate() {
        let mut held = 0;
        for &v in bag.vertices() {
            if required[v as usize] {
                occurrences[v as usize] += 1;
                held += 1;
            }
        }
        if held == count
            && single
                .is_none_or(|old: usize| bag.vertices().len() < tree.bags()[old].vertices().len())
        {
            single = Some(index);
        }
    }
    if let Some(index) = single {
        let mut kept = vec![false; tree.bags().len()];
        kept[index] = true;
        return kept;
    }
    let mut kept = vec![true; tree.bags().len()];
    let mut degree: Vec<_> = tree.adjacency().iter().map(Vec::len).collect();
    let mut leaves: VecDeque<_> = degree
        .iter()
        .enumerate()
        .filter_map(|(i, &d)| (d <= 1).then_some(i))
        .collect();
    while let Some(leaf) = leaves.pop_front() {
        if !kept[leaf] || degree[leaf] > 1 {
            continue;
        }
        let removable = tree.bags()[leaf]
            .vertices()
            .iter()
            .all(|&v| !required[v as usize] || occurrences[v as usize] > 1);
        if !removable {
            continue;
        }
        kept[leaf] = false;
        for &v in tree.bags()[leaf].vertices() {
            if required[v as usize] {
                occurrences[v as usize] -= 1;
            }
        }
        for &other in &tree.adjacency()[leaf] {
            if kept[other] {
                degree[other] -= 1;
                if degree[other] <= 1 {
                    leaves.push_back(other);
                }
            }
        }
    }
    kept
}

/// The neighbours of every vertex, each row ascending.
fn adjacency(graph: &Graph) -> Vec<Vec<u32>> {
    let mut adjacency = vec![Vec::new(); graph.num_vertices() as usize];
    for &(a, b) in graph.edges() {
        adjacency[a as usize].push(b);
        adjacency[b as usize].push(a);
    }
    adjacency
}

/// Remove `vertex` from `tree`, whose bags `shared` has completed, and put
/// it back through connecting bags.
fn rebuild(
    graph: &Graph,
    neighbours: &[u32],
    shared: &mut SharedCompletion,
    tree: &TreeDecomposition,
    vertex: u32,
    deadline: Instant,
) -> Option<TreeDecomposition> {
    if graph.num_vertices() <= 1 || neighbours.is_empty() {
        return None;
    }
    let lower = |v: u32| v - u32::from(v > vertex);
    let mut required = vec![false; graph.num_vertices() as usize - 1];
    for &u in neighbours {
        required[lower(u) as usize] = true;
    }
    // Restricting the graph to the other vertices used to read its whole edge
    // list; the rebuild no longer needs that copy but is charged the same.
    crate::meter::charge(graph.edges().len() as u64);
    let small = crate::decomposition::ops::project_dropping_vertex(tree, vertex)?;
    let small = compact(small);
    let remaining_edges = graph.edges().len() - neighbours.len();
    let small =
        rebuild_candidate(&small, shared, vertex, remaining_edges, Some(deadline)).unwrap_or(small);
    let small = compact(small);
    let kept = support(&small, &required);
    let original = |v: u32| v + u32::from(v >= vertex);
    let mut bags: Vec<Vec<u32>> = small
        .bags()
        .iter()
        .map(|bag| bag.vertices().iter().copied().map(original).collect())
        .collect();
    // Deletion can disconnect the graph. Each selected support gets a new
    // attachment bag, and those bags connect through the restored vertex.
    let mut tree_edges = edges(&small);
    let mut attachment: Vec<_> = (0..kept.len()).collect();
    for (index, &on_support) in kept.iter().enumerate() {
        if on_support {
            attachment[index] = bags.len();
            let mut bag = vec![vertex];
            bag.extend(
                small.bags()[index]
                    .vertices()
                    .iter()
                    .copied()
                    .filter(|&v| required[v as usize])
                    .map(original),
            );
            bags.push(bag);
        }
    }
    for edge in &mut tree_edges {
        let (a, b) = *edge;
        if kept[a] && kept[b] {
            let separator: Vec<_> = small.bags()[a]
                .vertices()
                .iter()
                .copied()
                .filter(|v| small.bags()[b].vertices().binary_search(v).is_ok())
                .map(original)
                .collect();
            bags[attachment[a]].extend_from_slice(&separator);
            bags[attachment[b]].extend_from_slice(&separator);
            *edge = (attachment[a], attachment[b]);
        }
    }
    for (index, &on_support) in kept.iter().enumerate() {
        if on_support {
            tree_edges.push((index, attachment[index]));
        }
    }
    // A separator can occur on several support edges.
    for bag in &mut bags[kept.len()..] {
        bag.sort_unstable();
        bag.dedup();
    }

    let mut seen = vec![false; kept.len()];
    let mut first = None;
    for root in 0..kept.len() {
        if !kept[root] || seen[root] {
            continue;
        }
        if let Some(first) = first {
            tree_edges.push((attachment[first], attachment[root]));
        } else {
            first = Some(root);
        }
        let mut stack = vec![root];
        seen[root] = true;
        while let Some(bag) = stack.pop() {
            for &other in &small.adjacency()[bag] {
                if kept[other] && !seen[other] {
                    seen[other] = true;
                    stack.push(other);
                }
            }
        }
    }
    let candidate = TreeDecomposition::new_trusted(graph, bags, tree_edges).ok()?;
    Some(compact(candidate))
}

/// Rebuild vertices and retain strict width-then-mass improvements.
/// The deadline bounds the search; initial compaction also runs when expired.
///
/// # Errors
/// Returns an error when `start` is not valid for `graph`.
pub fn improve(
    graph: &Graph,
    start: &TreeDecomposition,
    deadline: Instant,
) -> Result<(TreeDecomposition, Stats), crate::Error> {
    start.validate(graph)?;
    Ok(improve_trusted(graph, start, deadline))
}

/// Reinsert vertices using connecting bags of neighbours and separators.
/// The caller establishes that the input tree is valid for the graph.
///
/// # Panics
/// Debug builds assert input validity. Release callers must establish it.
pub fn improve_trusted(
    graph: &Graph,
    start: &TreeDecomposition,
    deadline: Instant,
) -> (TreeDecomposition, Stats) {
    debug_assert!(start.validate(graph).is_ok());
    let mut best = compact(start.clone());
    let mut stats = Stats::default();
    // The neighbour lists and the shared completion are built for the first
    // round that runs, not before: the gate below turns most large graphs
    // away, and the completion is a quadratic allocation.
    let mut shared: Option<(Vec<Vec<u32>>, SharedCompletion)> = None;
    while !crate::deadline::expired(Some(deadline)) {
        let n = graph.num_vertices() as u64;
        let squares = best.bags().iter().fold(0u64, |sum, bag| {
            let size = bag.vertices().len() as u64;
            sum.saturating_add(size.saturating_mul(size))
        });
        let projected = squares
            .saturating_mul(n.div_ceil(64))
            .saturating_add(n.saturating_mul(n));
        if Duration::from_millis(crate::meter::milliseconds_for_units(projected))
            > deadline.saturating_duration_since(crate::meter::now()) / 8
        {
            break;
        }
        stats.rounds += 1;
        let best_quality = quality(&best);
        let (adjacency, shared) =
            shared.get_or_insert_with(|| (adjacency(graph), SharedCompletion::new(graph)));
        shared.complete(&best);
        let mut moved = false;
        for vertex in 0..graph.num_vertices() {
            if crate::deadline::expired(Some(deadline)) {
                break;
            }
            let candidate = rebuild(
                graph,
                &adjacency[vertex as usize],
                shared,
                &best,
                vertex,
                deadline,
            );
            let Some(candidate) = candidate else {
                continue;
            };
            stats.tried += 1;
            if quality(&candidate) < best_quality {
                best = candidate;
                stats.improved += 1;
                moved = true;
                break;
            }
        }
        if !moved {
            break;
        }
    }
    (best, stats)
}
