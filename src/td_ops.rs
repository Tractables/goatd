//! Tree-decomposition surgery: rooting a decomposition into a walkable
//! forest, projecting one onto a vertex subset, and gluing two back together
//! at a shared separator.
//!
//! Every function here that returns a decomposition preserves the running
//! intersection property (RIP): a vertex's bags form a connected subtree of
//! the result whenever they did in the input. The per-function docs say how.

use std::collections::VecDeque;

use rustc_hash::FxHashSet;

use crate::{TdBag, TreeDecomposition, local_index};

/// A decomposition rooted for a downward walk: what one breadth-first sweep
/// over the bag tree leaves behind.
pub struct RootedForest {
    /// Bag indices in breadth-first order, so every bag follows its parent and
    /// precedes its children. Reversed, it is a leaves-first order.
    pub order: Vec<usize>,
    /// Each bag's parent, [`NO_PARENT`] at a component root and at a bag no
    /// root reached.
    pub parent: Vec<usize>,
    /// Each bag's distance from its component root; `0` at a bag the walk did
    /// not reach.
    pub depth: Vec<usize>,
    /// The bag each component was entered at, in the order the walk entered
    /// them.
    pub component_roots: Vec<usize>,
}

/// [`RootedForest::parent`] at a bag with no parent.
pub const NO_PARENT: usize = usize::MAX;

/// Root a decomposition's bag tree at `roots` and walk it breadth-first.
///
/// A decomposition need not be connected — a projection that drops a separator
/// leaves several components behind — so this roots a forest rather than a
/// tree: `roots` is tried in order, and each entry that a previous one has not
/// already reached opens a new component. Ending `roots` with `0..n` therefore
/// says "these bags first, then whatever they missed", and starting from
/// `0..n` alone says "no preference": either way every bag is reached exactly
/// once.
pub fn rooted_forest(adj: &[Vec<usize>], roots: impl IntoIterator<Item = usize>) -> RootedForest {
    let n = adj.len();
    let mut parent = vec![NO_PARENT; n];
    let mut depth = vec![0usize; n];
    let mut order = Vec::with_capacity(n);
    let mut visited = vec![false; n];
    let mut component_roots = Vec::new();
    let mut queue = VecDeque::new();
    for start in roots {
        if visited[start] {
            continue;
        }
        component_roots.push(start);
        visited[start] = true;
        queue.push_back(start);
        while let Some(t) = queue.pop_front() {
            order.push(t);
            for &nb in &adj[t] {
                if !visited[nb] {
                    visited[nb] = true;
                    parent[nb] = t;
                    depth[nb] = depth[t] + 1;
                    queue.push_back(nb);
                }
            }
        }
    }
    RootedForest {
        order,
        parent,
        depth,
        component_roots,
    }
}

/// Result of projecting a decomposition onto a vertex subset.
pub struct ProjectedTd {
    /// The projection, over local ids `0..k`.
    pub td: TreeDecomposition,
    /// Mapping from local ids (0..k) back to the original vertex ids.
    pub local_to_global: Vec<u32>,
}

/// Project a tree decomposition onto a vertex subset, preserving the original
/// vertex ids (no renumbering).
///
/// Vertices outside `keep` are dropped from every bag, bags left empty are
/// contracted away, and each surviving bag is reattached to its nearest
/// surviving ancestor.
///
/// The running intersection property is preserved: dropping a vertex removes
/// it from a connected subtree of bags, so what remains for each retained
/// vertex is still a connected subtree.
///
/// Returns `None` when every projected bag would be empty.
pub fn project_td_keeping_global_ids(
    td: &TreeDecomposition,
    keep: &FxHashSet<u32>,
) -> Option<TreeDecomposition> {
    let n = td.bags.len();
    if n == 0 {
        return None;
    }

    let projected: Vec<Vec<u32>> = td
        .bags
        .iter()
        .map(|bag| {
            bag.vertices
                .iter()
                .copied()
                .filter(|v| keep.contains(v))
                .collect()
        })
        .collect();

    let non_empty: Vec<usize> = (0..n).filter(|&i| !projected[i].is_empty()).collect();
    if non_empty.is_empty() {
        return None;
    }

    let mut old_to_new = vec![None; n];
    for (new_i, &old_i) in non_empty.iter().enumerate() {
        old_to_new[old_i] = Some(new_i);
    }

    let new_count = non_empty.len();
    let mut new_adj: Vec<Vec<usize>> = vec![Vec::new(); new_count];

    let parent_in_td = rooted_forest(&td.adj, 0..n).parent;

    for &old_i in &non_empty {
        let new_i = old_to_new[old_i].unwrap();
        let mut ancestor = parent_in_td[old_i];
        while ancestor != NO_PARENT {
            if let Some(new_j) = old_to_new[ancestor] {
                new_adj[new_i].push(new_j);
                new_adj[new_j].push(new_i);
                break;
            }
            ancestor = parent_in_td[ancestor];
        }
    }

    let new_bags: Vec<TdBag> = non_empty
        .iter()
        .enumerate()
        .map(|(new_id, &old_id)| {
            let mut verts = projected[old_id].clone();
            verts.sort_unstable();
            TdBag {
                id: new_id,
                vertices: verts,
            }
        })
        .collect();

    Some(TreeDecomposition {
        bags: new_bags,
        adj: new_adj,
    })
}

/// Project a tree decomposition onto a vertex subset and renumber the result
/// into a local id space.
///
/// Thin wrapper over [`project_td_keeping_global_ids`], which does the bag
/// filtering, empty-bag contraction and tree rebuild. The only extra work here
/// is the relabelling: `keep` sorted ascending becomes local ids `0..k`, so
/// the returned decomposition numbers its vertices `0..k` and
/// `local_to_global` maps them back.
pub fn project_td(td: &TreeDecomposition, keep: &FxHashSet<u32>) -> Option<ProjectedTd> {
    let mut sorted: Vec<u32> = keep.iter().copied().collect();
    sorted.sort_unstable();
    let global_to_local = local_index(&sorted);

    let mut projected = project_td_keeping_global_ids(td, keep)?;

    // Relabelling is order-preserving (a local id is the rank of its global id
    // in `sorted`), so bags that came back sorted by global id stay sorted.
    for bag in &mut projected.bags {
        for v in &mut bag.vertices {
            *v = global_to_local[&*v];
        }
    }

    Some(ProjectedTd {
        td: projected,
        local_to_global: sorted,
    })
}

// ---------------------------------------------------------------------------
// Separator glue
// ---------------------------------------------------------------------------

/// Find one bag that contains every vertex in `sep`, augmenting the
/// decomposition if necessary. Returns the index of that bag.
///
/// Strategy: pick the bag with the largest intersection with `sep`, then for
/// each missing `v ∈ sep`, BFS from a bag that contains `v` to the anchor bag
/// and add `v` to every bag on the path. This preserves RIP: `v`'s original
/// bag-subtree is extended by a connected path of bags, all containing `v`. A
/// `v` whose bag is in another component has no such path, so the two
/// components are joined by one edge first — see the branch below.
///
/// Bag widths may grow by up to `|sep \ anchor_bag|` in the worst case; the
/// refinement's `(width, total_bag_size)` guard catches cases where this
/// growth wipes out the benefit of the cut.
fn augment_for_separator(td: &mut TreeDecomposition, sep: &[u32]) -> Option<usize> {
    if td.bags.is_empty() {
        return None;
    }

    let sep_set: FxHashSet<u32> = sep.iter().copied().collect();

    let anchor = (0..td.bags.len()).max_by_key(|&i| {
        td.bags[i]
            .vertices
            .iter()
            .filter(|v| sep_set.contains(v))
            .count()
    })?;

    for &v in sep {
        if td.bags[anchor].vertices.contains(&v) {
            continue;
        }
        let src = (0..td.bags.len()).find(|&i| td.bags[i].vertices.contains(&v));
        match src {
            Some(src) if src != anchor => match bag_path_bfs(&td.adj, src, anchor) {
                Some(path) => {
                    for &b in &path {
                        if !td.bags[b].vertices.contains(&v) {
                            td.bags[b].vertices.push(v);
                        }
                    }
                }
                None => {
                    // `v`'s bag is in another component, so there is no path of
                    // bags to carry it along and writing it into both ends
                    // would leave its bags disconnected. One edge joins the two
                    // components first: between two components it can close no
                    // cycle, and it disconnects no vertex's bags, so `v` then
                    // travels it the way it would any other edge.
                    td.adj[src].push(anchor);
                    td.adj[anchor].push(src);
                    td.bags[anchor].vertices.push(v);
                }
            },
            _ => {
                td.bags[anchor].vertices.push(v);
            }
        }
    }

    for bag in td.bags.iter_mut() {
        bag.vertices.sort_unstable();
        bag.vertices.dedup();
    }

    Some(anchor)
}

/// Shortest path between two bag indices in the bag tree. Returns the
/// sequence of bag indices from `src` to `dst` inclusive, or `None` when the
/// two are in different components and no path exists.
fn bag_path_bfs(adj: &[Vec<usize>], src: usize, dst: usize) -> Option<Vec<usize>> {
    if src == dst {
        return Some(vec![src]);
    }
    let parent = rooted_forest(adj, [src]).parent;
    if parent[dst] == NO_PARENT {
        return None;
    }
    let mut path = vec![dst];
    let mut x = dst;
    while x != src {
        x = parent[x];
        path.push(x);
    }
    path.reverse();
    Some(path)
}

/// Glue two tree decompositions at a shared separator.
///
/// Both `td_a` and `td_b` must have already been projected to retain every
/// vertex in `sep` (the caller enforces this by passing `side ∪ sep` as the
/// keep-set to [`project_td_keeping_global_ids`]). Vertex ids are preserved
/// across both inputs.
///
/// The glued decomposition gets a new bag 0 containing exactly `sep`, with the
/// anchor bag from each side attached as a neighbour. Each side's bags are
/// augmented (if needed) so their anchor contains every `sep` vertex,
/// preserving RIP for the glued tree.
pub fn glue_at_separator(
    mut td_a: TreeDecomposition,
    mut td_b: TreeDecomposition,
    sep: &[u32],
) -> Option<TreeDecomposition> {
    let anchor_a = augment_for_separator(&mut td_a, sep)?;
    let anchor_b = augment_for_separator(&mut td_b, sep)?;

    let mut sep_sorted: Vec<u32> = sep.to_vec();
    sep_sorted.sort_unstable();
    sep_sorted.dedup();
    let sep_bag = TdBag {
        id: 0,
        vertices: sep_sorted,
    };

    let mut bags: Vec<TdBag> = Vec::with_capacity(1 + td_a.bags.len() + td_b.bags.len());
    bags.push(sep_bag);

    let a_offset = 1;
    for (i, bag) in td_a.bags.iter().enumerate() {
        bags.push(TdBag {
            id: a_offset + i,
            vertices: bag.vertices.clone(),
        });
    }

    let b_offset = a_offset + td_a.bags.len();
    for (i, bag) in td_b.bags.iter().enumerate() {
        bags.push(TdBag {
            id: b_offset + i,
            vertices: bag.vertices.clone(),
        });
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); bags.len()];

    for (i, nbs) in td_a.adj.iter().enumerate() {
        for &nb in nbs {
            if nb > i {
                adj[a_offset + i].push(a_offset + nb);
                adj[a_offset + nb].push(a_offset + i);
            }
        }
    }
    for (i, nbs) in td_b.adj.iter().enumerate() {
        for &nb in nbs {
            if nb > i {
                adj[b_offset + i].push(b_offset + nb);
                adj[b_offset + nb].push(b_offset + i);
            }
        }
    }

    adj[0].push(a_offset + anchor_a);
    adj[a_offset + anchor_a].push(0);
    adj[0].push(b_offset + anchor_b);
    adj[b_offset + anchor_b].push(0);

    Some(TreeDecomposition { bags, adj })
}
