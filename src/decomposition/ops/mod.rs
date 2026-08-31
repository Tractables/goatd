//! Tree-decomposition surgery: rooting a decomposition into a walkable
//! forest, projecting one onto a vertex subset, and gluing two back together
//! at a shared separator.
//!
//! Every function here that returns a decomposition preserves the running
//! intersection property (RIP): a vertex's bags form a connected subtree of
//! the result whenever they did in the input. The per-function docs say how.

use std::collections::VecDeque;

use rustc_hash::FxHashSet;

use super::{TdBag, TreeDecomposition};
use crate::Error;
use crate::graph::index_by_vertex;

/// A decomposition rooted for a downward walk: what one breadth-first sweep
/// over the bag tree leaves behind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootedForest {
    /// Bag indices in breadth-first order, so every bag follows its parent and
    /// precedes its children. Reversed, it is a leaves-first order.
    order: Vec<usize>,
    /// Each bag's parent. Component roots have no parent.
    parent: Vec<Option<usize>>,
    /// Each bag's distance from its component root.
    depth: Vec<usize>,
    /// The bag each component was entered at, in the order the walk entered
    /// them.
    component_roots: Vec<usize>,
}

impl RootedForest {
    /// Bag indices in breadth-first order.
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    /// Each bag's parent; component roots contain `None`.
    pub fn parents(&self) -> &[Option<usize>] {
        &self.parent
    }

    /// Each bag's distance from its component root.
    pub fn depths(&self) -> &[usize] {
        &self.depth
    }

    /// The chosen root of each component.
    pub fn component_roots(&self) -> &[usize] {
        &self.component_roots
    }
}

/// Root a bag forest at `roots` and walk it breadth-first.
///
/// A decomposition need not be connected — a projection that drops a separator
/// leaves several components behind — so this roots a forest rather than a
/// tree: `roots` is tried in order, and each entry that a previous one has not
/// already reached opens a new component. Ending `roots` with `0..n` therefore
/// says "these bags first, then whatever they missed", and starting from
/// `0..n` alone says "no preference": either way every bag is reached exactly
/// once.
fn rooted_forest_from_adjacency(
    adj: &[Vec<usize>],
    roots: impl IntoIterator<Item = usize>,
) -> RootedForest {
    let n = adj.len();
    let mut parent = vec![None; n];
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
                    parent[nb] = Some(t);
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    decomposition: TreeDecomposition,
    local_to_original: Vec<u32>,
}

impl Projection {
    /// The projected decomposition, over local ids `0..k`.
    pub fn decomposition(&self) -> &TreeDecomposition {
        &self.decomposition
    }

    /// Maps each local vertex id back to the original vertex id.
    pub fn local_to_original(&self) -> &[u32] {
        &self.local_to_original
    }

    /// Consume the projection as `(decomposition, local_to_original)`.
    pub fn into_parts(self) -> (TreeDecomposition, Vec<u32>) {
        (self.decomposition, self.local_to_original)
    }
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
pub(super) fn project_td_keeping_global_ids(
    td: &TreeDecomposition,
    keep: &[u32],
) -> Option<TreeDecomposition> {
    let n = td.bags.len();
    if n == 0 {
        return None;
    }

    let keep: FxHashSet<u32> = keep.iter().copied().collect();
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

    let parent_in_td = rooted_forest_from_adjacency(&td.adj, 0..n).parent;

    for &old_i in &non_empty {
        let new_i = old_to_new[old_i].unwrap();
        let mut ancestor = parent_in_td[old_i];
        while let Some(old_ancestor) = ancestor {
            if let Some(new_j) = old_to_new[old_ancestor] {
                new_adj[new_i].push(new_j);
                new_adj[new_j].push(new_i);
                break;
            }
            ancestor = parent_in_td[old_ancestor];
        }
    }

    let new_bags: Vec<TdBag> = non_empty
        .iter()
        .map(|&old_id| TdBag::new(projected[old_id].clone()))
        .collect();

    Some(TreeDecomposition::from_parts(
        td.num_vertices,
        new_bags,
        new_adj,
    ))
}

/// Project a tree decomposition onto a vertex subset and renumber the result
/// into a local id space.
///
/// [`project_td_keeping_global_ids`] does the bag filtering, empty-bag
/// contraction and tree rebuild. This function also relabels `keep` in sorted
/// order to local ids `0..k`, so
/// the returned decomposition numbers its vertices `0..k` and
/// [`Projection::local_to_original`] maps them back.
fn project(td: &TreeDecomposition, keep: &[u32]) -> Result<Projection, Error> {
    let mut sorted: Vec<u32> = keep.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    if let Some(&vertex) = sorted.iter().find(|&&vertex| vertex >= td.num_vertices) {
        return Err(Error::InvalidInput(format!(
            "projected vertex {vertex} is outside 0..{}",
            td.num_vertices
        )));
    }
    let global_to_local = index_by_vertex(&sorted);

    if sorted.is_empty() {
        return Ok(Projection {
            decomposition: TreeDecomposition::from_parts(0, Vec::new(), Vec::new()),
            local_to_original: Vec::new(),
        });
    }

    let Some(mut projected) = project_td_keeping_global_ids(td, &sorted) else {
        return Err(Error::InvalidDecomposition(
            "none of the projected vertices occurs in a bag".into(),
        ));
    };

    let represented: FxHashSet<u32> = projected
        .bags
        .iter()
        .flat_map(|bag| bag.vertices.iter().copied())
        .collect();
    if let Some(&missing) = sorted
        .iter()
        .find(|&&vertex| !represented.contains(&vertex))
    {
        return Err(Error::InvalidDecomposition(format!(
            "projected vertex {missing} occurs in no bag"
        )));
    }

    // Relabelling is order-preserving (a local id is the rank of its global id
    // in `sorted`), so bags that came back sorted by global id stay sorted.
    for bag in &mut projected.bags {
        for v in &mut bag.vertices {
            *v = global_to_local[&*v];
        }
    }
    projected.num_vertices = sorted.len() as u32;

    Ok(Projection {
        decomposition: projected,
        local_to_original: sorted,
    })
}

fn bag_is_subset(left: &TdBag, right: &TdBag, left_sorted: bool, right_sorted: bool) -> bool {
    if left.vertices.len() > right.vertices.len() {
        return false;
    }
    if !left_sorted {
        return left.vertices.iter().all(|vertex| {
            if right_sorted {
                right.vertices.binary_search(vertex).is_ok()
            } else {
                right.vertices.contains(vertex)
            }
        });
    }
    if !right_sorted {
        return left
            .vertices
            .iter()
            .all(|vertex| right.vertices.contains(vertex));
    }

    let mut right_index = 0;
    for vertex in &left.vertices {
        while right_index < right.vertices.len() && right.vertices[right_index] < *vertex {
            right_index += 1;
        }
        if right.vertices.get(right_index) != Some(vertex) {
            return false;
        }
        right_index += 1;
    }
    true
}

impl TreeDecomposition {
    /// Contract every bag contained in an adjacent bag.
    ///
    /// Contracting such an edge preserves the running intersection property:
    /// every vertex in the removed bag remains in the retained endpoint, and
    /// the removed bag's other neighbours are reattached there. Width cannot
    /// increase, while each contraction lowers total bag size.
    pub(crate) fn compact_subsumed_bags(self) -> Self {
        let bag_count = self.bags.len();
        if bag_count < 2 {
            return self;
        }

        // Each bag chooses at most one adjacent superset. Strict containment
        // points toward a larger bag; equal bags point toward the lower index,
        // so these links cannot cycle.
        let mut target: Vec<Option<usize>> = vec![None; bag_count];
        let bag_is_sorted: Vec<bool> = self
            .bags
            .iter()
            .map(|bag| bag.vertices.windows(2).all(|pair| pair[0] < pair[1]))
            .collect();
        for bag in 0..bag_count {
            for &neighbour in &self.adj[bag] {
                let bag_size = self.bags[bag].vertices.len();
                let neighbour_size = self.bags[neighbour].vertices.len();
                if bag_size > neighbour_size
                    || (bag_size == neighbour_size && neighbour > bag)
                    || !bag_is_subset(
                        &self.bags[bag],
                        &self.bags[neighbour],
                        bag_is_sorted[bag],
                        bag_is_sorted[neighbour],
                    )
                {
                    continue;
                }
                let replace = target[bag].is_none_or(|current| {
                    let current_size = self.bags[current].vertices.len();
                    neighbour_size > current_size
                        || (neighbour_size == current_size && neighbour < current)
                });
                if replace {
                    target[bag] = Some(neighbour);
                }
            }
        }

        let mut representative = vec![usize::MAX; bag_count];
        for start in 0..bag_count {
            let mut root = start;
            while let Some(next) = target[root] {
                root = if representative[next] == usize::MAX {
                    next
                } else {
                    representative[next]
                };
            }
            let mut bag = start;
            while representative[bag] == usize::MAX {
                representative[bag] = root;
                let Some(next) = target[bag] else {
                    break;
                };
                bag = next;
            }
        }

        let mut old_to_new = vec![usize::MAX; bag_count];
        let mut bags = Vec::new();
        for (old, bag) in self.bags.into_iter().enumerate() {
            if representative[old] == old {
                old_to_new[old] = bags.len();
                bags.push(bag);
            }
        }

        let mut adj = vec![Vec::new(); bags.len()];
        for (left, neighbours) in self.adj.iter().enumerate() {
            for &right in neighbours {
                if right <= left {
                    continue;
                }
                let left = old_to_new[representative[left]];
                let right = old_to_new[representative[right]];
                if left != right {
                    adj[left].push(right);
                    adj[right].push(left);
                }
            }
        }
        for neighbours in &mut adj {
            neighbours.sort_unstable();
            neighbours.dedup();
        }

        TreeDecomposition::from_parts(self.num_vertices, bags, adj)
    }

    /// Root this decomposition's bag forest and walk it breadth-first.
    ///
    /// Each entry in `roots` that has not already been reached opens a new
    /// component. Any component not named in `roots` is then rooted at its
    /// first bag, so every bag occurs in the result.
    ///
    /// # Errors
    ///
    /// Returns an error when a root is not a bag index.
    pub fn rooted_forest(
        &self,
        roots: impl IntoIterator<Item = usize>,
    ) -> Result<RootedForest, Error> {
        let roots: Vec<usize> = roots.into_iter().collect();
        if let Some(&root) = roots.iter().find(|&&root| root >= self.bags.len()) {
            return Err(Error::InvalidInput(format!(
                "root bag {root} is outside 0..{}",
                self.bags.len()
            )));
        }
        Ok(rooted_forest_from_adjacency(
            &self.adj,
            roots.into_iter().chain(0..self.bags.len()),
        ))
    }

    /// Project onto `keep`, renumbering its sorted unique vertex ids to `0..k`.
    /// Projecting onto an empty set returns an empty decomposition.
    ///
    /// # Errors
    ///
    /// Returns an error when a requested vertex is outside this
    /// decomposition's vertex range or occurs in no bag.
    pub fn project(&self, keep: &[u32]) -> Result<Projection, Error> {
        project(self, keep)
    }
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
        let src = (0..td.bags.len()).find(|&i| td.bags[i].vertices.contains(&v))?;
        if src != anchor {
            match bag_path_bfs(&td.adj, src, anchor) {
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
    let parent = rooted_forest_from_adjacency(adj, [src]).parent;
    let mut path = vec![dst];
    let mut x = dst;
    while x != src {
        x = parent[x]?;
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
pub(super) fn glue_at_separator(
    mut td_a: TreeDecomposition,
    mut td_b: TreeDecomposition,
    sep: &[u32],
) -> Option<TreeDecomposition> {
    if td_a.num_vertices != td_b.num_vertices {
        return None;
    }
    let num_vertices = td_a.num_vertices;
    let anchor_a = augment_for_separator(&mut td_a, sep)?;
    let anchor_b = augment_for_separator(&mut td_b, sep)?;

    let mut sep_sorted: Vec<u32> = sep.to_vec();
    sep_sorted.sort_unstable();
    sep_sorted.dedup();
    let sep_bag = TdBag::new(sep_sorted);

    let a_len = td_a.bags.len();
    let b_len = td_b.bags.len();
    let mut bags: Vec<TdBag> = Vec::with_capacity(1 + a_len + b_len);
    bags.push(sep_bag);
    bags.extend(td_a.bags);
    bags.extend(td_b.bags);

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); bags.len()];

    let a_offset = 1;
    for (i, nbs) in td_a.adj.into_iter().enumerate() {
        for nb in nbs {
            if nb > i {
                adj[a_offset + i].push(a_offset + nb);
                adj[a_offset + nb].push(a_offset + i);
            }
        }
    }
    let b_offset = a_offset + a_len;
    for (i, nbs) in td_b.adj.into_iter().enumerate() {
        for nb in nbs {
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

    Some(TreeDecomposition::from_parts(num_vertices, bags, adj))
}

#[cfg(test)]
mod tests;
