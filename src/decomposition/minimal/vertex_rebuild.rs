//! Remove one vertex, simplify the remaining triangulation, and reinsert it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::exchange::{compact, quality};
use super::minimalization_candidate;
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

fn rebuild(
    graph: &Graph,
    tree: &TreeDecomposition,
    vertex: u32,
    deadline: Instant,
) -> Option<TreeDecomposition> {
    rebuild_candidate::<false>(graph, tree, vertex, deadline)
}

fn rebuild_candidate<const DIRECT: bool>(
    graph: &Graph,
    tree: &TreeDecomposition,
    vertex: u32,
    deadline: Instant,
) -> Option<TreeDecomposition> {
    if graph.num_vertices() <= 1 {
        return None;
    }
    let lower = |v: u32| v - u32::from(v > vertex);
    let mut required = vec![false; graph.num_vertices() as usize - 1];
    for &(a, b) in graph.edges() {
        if a == vertex {
            required[lower(b) as usize] = true;
        } else if b == vertex {
            required[lower(a) as usize] = true;
        }
    }
    if !required.iter().any(|&yes| yes) {
        return None;
    }
    let keep: Vec<_> = (0..graph.num_vertices()).filter(|&v| v != vertex).collect();
    let small_graph = graph.induced_subgraph(&keep).ok()?;
    let (small, _) = tree.project(&keep).ok()?.into_parts();
    let small = compact(small);
    let small = minimalization_candidate(&small, &small_graph, Some(deadline)).unwrap_or(small);
    let small = compact(small);
    let kept = support(&small, &required);
    let mut bags: Vec<Vec<u32>> = small
        .bags()
        .iter()
        .enumerate()
        .map(|(index, bag)| {
            let mut bag: Vec<_> = bag
                .vertices()
                .iter()
                .map(|&v| v + u32::from(v >= vertex))
                .collect();
            if kept[index] && !DIRECT {
                bag.push(vertex);
            }
            bag
        })
        .collect();
    // Deletion can disconnect the graph, and minimalization can return one
    // bag tree per component. Join their selected supports through the
    // reinserted vertex; components without a neighbour stay separate.
    let mut tree_edges = edges(&small);
    let mut attachment: Vec<_> = if DIRECT {
        (0..kept.len()).collect()
    } else {
        Vec::new()
    };
    if DIRECT {
        let original = |v: u32| v + u32::from(v >= vertex);
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
    }

    let mut seen = vec![false; kept.len()];
    let mut first = None;
    for root in 0..kept.len() {
        if !kept[root] || seen[root] {
            continue;
        }
        if let Some(first) = first {
            tree_edges.push(if DIRECT {
                (attachment[first], attachment[root])
            } else {
                (first, root)
            });
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
    let candidate = compact(candidate);
    if DIRECT {
        Some(candidate)
    } else {
        let candidate =
            minimalization_candidate(&candidate, graph, Some(deadline)).unwrap_or(candidate);
        Some(compact(candidate))
    }
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

/// Rebuild a tree whose validity the caller has established.
///
/// # Panics
/// Debug builds assert input validity. Release callers must establish it.
pub fn improve_trusted(
    graph: &Graph,
    start: &TreeDecomposition,
    deadline: Instant,
) -> (TreeDecomposition, Stats) {
    search::<false>(graph, start, deadline)
}

/// Reinsert vertices using connecting bags built from their neighbours and separators.
///
/// # Panics
/// Debug builds assert input validity. Release callers must establish it.
pub fn improve_direct_trusted(
    graph: &Graph,
    start: &TreeDecomposition,
    deadline: Instant,
) -> (TreeDecomposition, Stats) {
    search::<true>(graph, start, deadline)
}

fn search<const DIRECT: bool>(
    graph: &Graph,
    start: &TreeDecomposition,
    deadline: Instant,
) -> (TreeDecomposition, Stats) {
    debug_assert!(start.validate(graph).is_ok());
    let mut best = compact(start.clone());
    let mut stats = Stats::default();
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
        let mut moved = false;
        for vertex in 0..graph.num_vertices() {
            if crate::deadline::expired(Some(deadline)) {
                break;
            }
            let candidate = if DIRECT {
                rebuild_candidate::<true>(graph, &best, vertex, deadline)
            } else {
                rebuild(graph, &best, vertex, deadline)
            };
            let Some(candidate) = candidate else {
                continue;
            };
            stats.tried += 1;
            if quality(&candidate) < quality(&best) {
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
