//! Mutable graph for elimination-based tree-decomposition.
//!
//! Adjacency is `Vec<Vec<u32>>` rather than `Vec<FxHashSet<u32>>`: Vec
//! iteration is more cache-friendly than hashbrown bucket scanning at the
//! degrees min-fill's hot path sees. Vertices are never
//! removed from the top-level vector; elimination marks them inactive and
//! clears their row.
//!
//! Dense, small graphs additionally maintain a flat bitset adjacency
//! alongside the Vec; most methods below have a bitset-mode and a
//! sparse-mode path.
//!
//! A row that grows past [`ROW_INDEX_THRESH`] additionally carries a map from
//! neighbour to its position in that row. Scanning such a row is what made
//! eliminating a vertex next to a hub cost the hub's degree rather than the
//! eliminated vertex's own; the map answers both "is `u` here" and "where is
//! `u`" without the scan. Short rows keep the plain Vec, which is what the
//! cache-friendliness argument above is about.

use rustc_hash::FxHashMap;

/// Maximum graph size for which bitset adjacency is indexed by vertex id.
/// At n = 16384: 16384 * 256 words * 8 bytes = 32 MB per graph.
const BITSET_THRESH: usize = 16384;

/// Slot value for a vertex the bitset does not cover.
const NO_SLOT: u32 = u32::MAX;

/// Row length at which a membership map starts being maintained. Below it the
/// linear scan wins: a few hundred contiguous `u32`s are a handful of cache
/// lines, while the map costs a hash and a random probe per lookup.
pub(super) const ROW_INDEX_THRESH: usize = 256;

/// A row at most this many times longer than the bag being eliminated is
/// stamped and read through the marker rather than probed through its map;
/// see `eliminate_with_nbrs_marker`.
const WALK_ROW_FACTOR: usize = 8;

/// Whether an edge list is already sorted, deduplicated and oriented `u < v`,
/// which is what [`crate::Graph::edges`] holds and what lets `from_edges` skip
/// the per-edge membership test.
fn is_canonical(edges: &[(u32, u32)]) -> bool {
    let mut previous: Option<(u32, u32)> = None;
    for &(u, v) in edges {
        if u >= v {
            return false;
        }
        if let Some(last) = previous
            && last >= (u, v)
        {
            return false;
        }
        previous = Some((u, v));
    }
    true
}

/// Whether a graph of `n` vertices and `num_edges` edges is kept as a flat
/// bitset indexed by vertex id: small enough for the bitset to fit and dense
/// enough for it to win.
fn bitset_mode(n: usize, num_edges: usize) -> bool {
    n <= BITSET_THRESH && num_edges.saturating_mul(128) > n.saturating_mul(n)
}

/// Whether a residual of `num_active` vertices and `num_edges` edges gets a
/// bitset re-indexed over its active vertices.
///
/// The condition is the same break-even as everywhere else — a row of bits
/// costs `num_active / 64` words against the marker path's average degree of
/// `2·num_edges / num_active`, so bits win once
/// `128·num_edges > num_active²` — and it is also the size bound. Rearranged,
/// it says the bitset's `num_active² / 8` bytes are at most `16·num_edges`,
/// twice what the adjacency rows already hold at `8·num_edges`, and the build
/// releases those rows and their membership maps. So the residual's density
/// caps the bitset against memory the graph is already holding, and there is
/// no vertex count to pick: a residual dense enough to be worth covering is
/// one whose rows already cost half of what covering it costs.
fn residual_bitset_mode(num_active: usize, num_edges: usize) -> bool {
    (num_edges as u128) * 128 > (num_active as u128) * (num_active as u128)
}

/// What a fill collector learnt about `v`'s elimination from the graph before
/// it, for [`EliminationGraph::eliminate_prepared`]: how many vertices each
/// neighbour gains and, in sparse mode, which ones and where `v` sits in
/// its row.
pub(super) struct PreparedFill<'a> {
    /// |N(v) \ N[u]| for a neighbour u, by vertex id.
    pub(super) gained: &'a [u32],
    /// Sparse mode only: the i-th neighbour gains
    /// `partners[starts[i]..starts[i + 1]]`, in the order the unprepared
    /// elimination pushes them, and `v` is at `v_position[i]` in its row.
    pub(super) partners: &'a [u32],
    pub(super) starts: &'a [u32],
    pub(super) v_position: &'a [u32],
}

/// Two distinct rows of `w` words each, starting at `a` and `b`, borrowed
/// together so a pass over both carries no bounds check per word.
#[inline(always)]
fn rows_mut(bitset: &mut [u64], a: usize, b: usize, w: usize) -> (&mut [u64], &mut [u64]) {
    debug_assert!(a != b);
    if a < b {
        let (low, high) = bitset.split_at_mut(b);
        (&mut low[a..a + w], &mut high[..w])
    } else {
        let (low, high) = bitset.split_at_mut(a);
        (&mut high[..w], &mut low[b..b + w])
    }
}

/// Build the membership map of one adjacency row.
fn build_row_index(row: &[u32], slot: &mut Option<Box<FxHashMap<u32, u32>>>) {
    let mut index: FxHashMap<u32, u32> = FxHashMap::default();
    index.reserve(row.len());
    for (position, &neighbour) in row.iter().enumerate() {
        index.insert(neighbour, position as u32);
    }
    *slot = Some(Box::new(index));
}

fn hardware_popcount_available() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::arch::is_x86_feature_detected!("popcnt")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

#[cfg(test)]
pub(super) fn intersection_popcount(left: &[u64], right: &[u64]) -> u64 {
    #[cfg(target_arch = "x86_64")]
    if hardware_popcount_available() {
        // SAFETY: the runtime check above establishes the target feature.
        return unsafe { intersection_popcount_popcnt(left, right) };
    }
    intersection_popcount_by(left, right, |word| word.count_ones() as u64)
}

#[cfg(all(test, target_arch = "x86_64"))]
#[target_feature(enable = "popcnt")]
unsafe fn intersection_popcount_popcnt(left: &[u64], right: &[u64]) -> u64 {
    intersection_popcount_by(left, right, |word| {
        std::arch::x86_64::_popcnt64(word as i64) as u64
    })
}

#[inline(always)]
fn intersection_popcount_by(
    left: &[u64],
    right: &[u64],
    popcount: impl Fn(u64) -> u64 + Copy,
) -> u64 {
    debug_assert_eq!(left.len(), right.len());

    let mut count_0 = 0u64;
    let mut count_1 = 0u64;
    let mut count_2 = 0u64;
    let mut count_3 = 0u64;
    let (left_chunks, left_tail) = left.as_chunks::<4>();
    let (right_chunks, right_tail) = right.as_chunks::<4>();
    for (left, right) in left_chunks.iter().zip(right_chunks) {
        count_0 += popcount(left[0] & right[0]);
        count_1 += popcount(left[1] & right[1]);
        count_2 += popcount(left[2] & right[2]);
        count_3 += popcount(left[3] & right[3]);
    }
    let tail = left_tail
        .iter()
        .zip(right_tail)
        .map(|(&left, &right)| popcount(left & right))
        .sum::<u64>();

    count_0 + count_1 + count_2 + count_3 + tail
}

#[inline(always)]
fn difference_popcount_by(
    left: &[u64],
    right: &[u64],
    popcount: impl Fn(u64) -> u64 + Copy,
) -> u64 {
    debug_assert_eq!(left.len(), right.len());

    let mut count_0 = 0u64;
    let mut count_1 = 0u64;
    let mut count_2 = 0u64;
    let mut count_3 = 0u64;
    let (left_chunks, left_tail) = left.as_chunks::<4>();
    let (right_chunks, right_tail) = right.as_chunks::<4>();
    for (left, right) in left_chunks.iter().zip(right_chunks) {
        count_0 += popcount(left[0] & !right[0]);
        count_1 += popcount(left[1] & !right[1]);
        count_2 += popcount(left[2] & !right[2]);
        count_3 += popcount(left[3] & !right[3]);
    }
    let tail = left_tail
        .iter()
        .zip(right_tail)
        .map(|(&left, &right)| popcount(left & !right))
        .sum::<u64>();

    count_0 + count_1 + count_2 + count_3 + tail
}

/// Mutable graph used by goatd during preprocessing, min-fill, and nested
/// dissection. Supports active/inactive vertices for constant-time elimination.
pub(super) struct EliminationGraph {
    pub(super) adj: Vec<Vec<u32>>,
    /// Position of each neighbour within `adj[v]`, for the rows long enough to
    /// be worth one. `None` for a short row, and for every row once the graph
    /// is in bitset mode, where `adj` stops being maintained.
    row_index: Vec<Option<Box<FxHashMap<u32, u32>>>>,
    pub(super) active: Vec<bool>,
    pub(super) num_active: usize,
    /// Count of undirected edges among active vertices. Enables O(1)
    /// clique-residual detection: the residual is complete iff
    /// `num_edges == num_active*(num_active-1)/2`.
    pub(super) num_edges: usize,
    /// Live degree in bitset mode. Adjacency rows stop being maintained after
    /// promotion, while this cache is updated with each bitset mutation.
    bitset_degree: Vec<u32>,
    /// Stamp-marker scratch for deduping fill-edge additions in
    /// O(Σdeg + k²) instead of O(k²·deg_avg). u16 halves memory footprint vs
    /// u32; the stamp wraps and clears the marker array when it does.
    elim_marker: Vec<u16>,
    elim_stamp: u16,
    /// Flat bitset adjacency over *slots*: the vertex in slot `i` occupies
    /// words `i * bitset_words .. (i+1) * bitset_words`, and bit `j` in that
    /// slice is set iff there is an edge to the vertex in slot `j`. Empty when
    /// bitset mode is disabled.
    pub(super) bitset: Vec<u64>,
    /// Number of u64 words per slot in `bitset`. 0 iff bitset is disabled.
    pub(super) bitset_words: usize,
    /// Slot of each vertex, or [`NO_SLOT`]. On a graph small enough to index
    /// by vertex id this is the identity; on a large graph whose residual got
    /// a bitset it covers the vertices that were active at promotion, which is
    /// what keeps the bitset to the residual's size and not the graph's.
    bitset_slot: Vec<u32>,
    /// The vertex each slot stands for, so a set bit can be reported as the
    /// vertex id every caller works in.
    slot_vertex: Vec<u32>,
    /// Whether slots are a re-indexing of the active vertices rather than
    /// vertex ids. False keeps the hot paths free of the extra lookup, which
    /// is what the graphs small enough to index by vertex id always did.
    bitset_compact: bool,
    hardware_popcount: bool,
}

/// Written out rather than derived so that `clone_from` refills the buffers a
/// working copy already has. The portfolio takes a copy of the preprocessed
/// residual for every candidate it runs, and on a graph with a few hundred
/// thousand vertices a fresh copy means an allocation per adjacency row.
///
/// Both halves destructure the whole struct, so a field added later does not
/// compile until it is handled in each.
impl Clone for EliminationGraph {
    fn clone(&self) -> Self {
        let Self {
            adj,
            row_index,
            active,
            num_active,
            num_edges,
            bitset_degree,
            elim_marker,
            elim_stamp,
            bitset,
            bitset_words,
            bitset_slot,
            slot_vertex,
            bitset_compact,
            hardware_popcount,
        } = self;
        Self {
            adj: adj.clone(),
            row_index: row_index.clone(),
            active: active.clone(),
            num_active: *num_active,
            num_edges: *num_edges,
            bitset_degree: bitset_degree.clone(),
            elim_marker: elim_marker.clone(),
            elim_stamp: *elim_stamp,
            bitset: bitset.clone(),
            bitset_words: *bitset_words,
            bitset_slot: bitset_slot.clone(),
            slot_vertex: slot_vertex.clone(),
            bitset_compact: *bitset_compact,
            hardware_popcount: *hardware_popcount,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        let Self {
            adj,
            row_index,
            active,
            num_active,
            num_edges,
            bitset_degree,
            elim_marker,
            elim_stamp,
            bitset,
            bitset_words,
            bitset_slot,
            slot_vertex,
            bitset_compact,
            hardware_popcount,
        } = source;
        self.adj.clone_from(adj);
        self.row_index.clone_from(row_index);
        self.active.clone_from(active);
        self.num_active = *num_active;
        self.num_edges = *num_edges;
        self.bitset_degree.clone_from(bitset_degree);
        self.elim_marker.clone_from(elim_marker);
        self.elim_stamp = *elim_stamp;
        self.bitset.clone_from(bitset);
        self.bitset_words = *bitset_words;
        self.bitset_slot.clone_from(bitset_slot);
        self.slot_vertex.clone_from(slot_vertex);
        self.bitset_compact = *bitset_compact;
        self.hardware_popcount = *hardware_popcount;
    }
}

impl EliminationGraph {
    pub(super) fn new(n: usize) -> Self {
        EliminationGraph {
            adj: vec![Vec::new(); n],
            row_index: (0..n).map(|_| None).collect(),
            active: vec![true; n],
            num_active: n,
            num_edges: 0,
            bitset_degree: Vec::new(),
            elim_marker: vec![0u16; n],
            elim_stamp: 0,
            bitset: Vec::new(),
            bitset_words: 0,
            bitset_slot: Vec::new(),
            slot_vertex: Vec::new(),
            bitset_compact: false,
            hardware_popcount: hardware_popcount_available(),
        }
    }

    /// The slot holding `v`. Only valid in bitset mode, and only for a vertex
    /// the bitset covers — every vertex that was active when it was built.
    #[inline]
    fn slot(&self, v: u32) -> usize {
        if !self.bitset_compact {
            return v as usize;
        }
        let slot = self.bitset_slot[v as usize];
        debug_assert_ne!(slot, NO_SLOT, "vertex {v} is outside the bitset");
        slot as usize
    }

    /// The vertex in `slot`.
    #[inline]
    fn vertex_at(&self, slot: usize) -> u32 {
        if !self.bitset_compact {
            return slot as u32;
        }
        self.slot_vertex[slot]
    }

    /// The slot holding `v`, for the callers outside this module that read
    /// `bitset` directly.
    #[inline]
    pub(super) fn bitset_slot_of(&self, v: u32) -> usize {
        self.slot(v)
    }

    /// The vertex in `slot`, for those same callers.
    #[inline]
    pub(super) fn bitset_vertex_at(&self, slot: usize) -> u32 {
        self.vertex_at(slot)
    }

    /// Whether `neighbour` is in `vertex`'s adjacency row. Sparse mode only.
    #[inline]
    fn row_contains(&self, vertex: u32, neighbour: u32) -> bool {
        match &self.row_index[vertex as usize] {
            Some(index) => index.contains_key(&neighbour),
            None => self.adj[vertex as usize].contains(&neighbour),
        }
    }

    /// Append `neighbour` to `vertex`'s row, keeping its map in step and
    /// building one if the row has just grown long enough to want it. Sparse
    /// mode only.
    #[inline]
    fn row_push(&mut self, vertex: u32, neighbour: u32) {
        let vertex = vertex as usize;
        let position = self.adj[vertex].len();
        self.adj[vertex].push(neighbour);
        if let Some(index) = self.row_index[vertex].as_deref_mut() {
            index.insert(neighbour, position as u32);
            return;
        }
        if position + 1 >= ROW_INDEX_THRESH {
            build_row_index(&self.adj[vertex], &mut self.row_index[vertex]);
        }
    }

    /// Remove `neighbour` from `vertex`'s row, exactly as the scan-and-
    /// `swap_remove` it replaces would, so row order does not depend on
    /// whether the row is indexed. Sparse mode only.
    #[inline]
    fn row_swap_remove(&mut self, vertex: u32, neighbour: u32) {
        let vertex = vertex as usize;
        let position = match &mut self.row_index[vertex] {
            Some(index) => match index.remove(&neighbour) {
                Some(position) => position as usize,
                None => return,
            },
            None => match self.adj[vertex].iter().position(|&w| w == neighbour) {
                Some(position) => position,
                None => return,
            },
        };
        let row = &mut self.adj[vertex];
        let last = row.len() - 1;
        let moved = row[last];
        row.swap_remove(position);
        if position != last
            && let Some(index) = &mut self.row_index[vertex]
        {
            index.insert(moved, position as u32);
        }
    }

    /// Whether `vertex`'s row currently carries a membership map.
    #[cfg(test)]
    pub(super) fn row_is_indexed(&self, vertex: u32) -> bool {
        self.row_index[vertex as usize].is_some()
    }

    /// Whether two graphs hold the same state: the same rows in the same
    /// order, the same membership maps, the same bitset and the same counts.
    ///
    /// The stamp marker is left out. It is scratch space for one call — every
    /// reader stamps before it reads — so two graphs can agree on everything a
    /// later step observes while carrying different stamps.
    #[cfg(test)]
    pub(super) fn same_state_as(&self, other: &Self) -> bool {
        self.adj == other.adj
            && self.row_index == other.row_index
            && self.active == other.active
            && self.num_active == other.num_active
            && self.num_edges == other.num_edges
            && self.bitset_degree == other.bitset_degree
            && self.bitset == other.bitset
            && self.bitset_words == other.bitset_words
            && self.bitset_slot == other.bitset_slot
            && self.slot_vertex == other.slot_vertex
            && self.bitset_compact == other.bitset_compact
    }

    /// Empty `vertex`'s row, as elimination does, and drop its map with it.
    #[inline]
    fn clear_row(&mut self, vertex: u32) {
        self.adj[vertex as usize].clear();
        self.row_index[vertex as usize] = None;
    }

    pub(super) fn from_edges(n: u32, edges: &[(u32, u32)]) -> Self {
        let n = n as usize;
        let mut g = EliminationGraph::new(n);
        for &(u, v) in edges {
            assert!(
                (u as usize) < n && (v as usize) < n,
                "elimination edge ({u}, {v}) has an endpoint outside 0..{n}"
            );
        }
        if is_canonical(edges) {
            // The edge count is known before a row is filled, so whether the
            // graph goes to bitset mode is too, and the row maps that mode
            // drops are not built in the first place.
            let index_rows = !bitset_mode(n, edges.len());
            g.fill_rows_from_canonical(edges, index_rows);
        } else {
            for &(u, v) in edges {
                if u != v && !g.row_contains(u, v) {
                    g.row_push(u, v);
                    g.row_push(v, u);
                    g.num_edges += 1;
                }
            }
        }
        if bitset_mode(n, g.num_edges) {
            g.build_bitset(false);
        }
        g
    }

    /// Fill the adjacency rows from an edge list already in the form
    /// [`crate::Graph::edges`] guarantees.
    ///
    /// Every production caller passes such a list, and there the membership
    /// test the general path runs per edge answers "no" every time: with the
    /// list sorted and deduplicated, no edge can already be in a row. Counting
    /// the degrees first also lets each row be allocated once and each map be
    /// built once, instead of growing the row and inserting into the map edge
    /// by edge. The rows come out in the order the general path leaves them:
    /// a vertex sees its neighbours below it in increasing order, then those
    /// above it, because that is the order the sorted list visits them in.
    /// With `index_rows` false no row gets a membership map, for a caller that
    /// is about to switch the graph to bitset mode and drop them anyway.
    fn fill_rows_from_canonical(&mut self, edges: &[(u32, u32)], index_rows: bool) {
        let mut degree = vec![0u32; self.adj.len()];
        for &(u, v) in edges {
            degree[u as usize] += 1;
            degree[v as usize] += 1;
        }
        for (row, &count) in self.adj.iter_mut().zip(degree.iter()) {
            row.reserve_exact(count as usize);
        }
        for &(u, v) in edges {
            self.adj[u as usize].push(v);
            self.adj[v as usize].push(u);
        }
        if index_rows {
            for vertex in 0..self.adj.len() {
                if self.adj[vertex].len() >= ROW_INDEX_THRESH {
                    build_row_index(&self.adj[vertex], &mut self.row_index[vertex]);
                }
            }
        }
        self.num_edges = edges.len();
    }

    /// Release the row maps. Bitset mode answers both questions they are there
    /// for, and `adj` stops being maintained, so keeping them would be memory
    /// spent on stale data.
    fn drop_row_indexes(&mut self) {
        for slot in self.row_index.iter_mut() {
            *slot = None;
        }
    }

    /// True when promoting from adj-only to bitset-assisted representation is
    /// worthwhile.
    ///
    /// A graph small enough to index by vertex id promotes on density alone:
    /// the bitset's O(k · words) beats the marker path's O(k · avg_deg) once
    /// `128·num_edges > n · num_active` — `num_active`, not `n`, so promotion
    /// still fires when fill edges densify the graph mid-elimination even
    /// though `from_edges` saw it as sparse.
    ///
    /// A larger graph promotes on its residual instead: once elimination has
    /// left a dense core, a bitset over the active vertices is the residual's
    /// size and not the graph's. This is where an elimination on a dense core
    /// costs its degree squared membership tests against the rows, and a row
    /// of bits turns that into degree times words. See
    /// [`residual_bitset_mode`] for why the density condition is also the size
    /// bound.
    pub(super) fn should_promote_bitset(&self) -> bool {
        if self.bitset_words > 0 {
            return false;
        }
        let n = self.adj.len();
        if n == 0 {
            return false;
        }
        if n <= BITSET_THRESH {
            return self.num_edges.saturating_mul(128) > n.saturating_mul(self.num_active.max(1));
        }
        residual_bitset_mode(self.num_active, self.num_edges)
    }

    /// Allocate and populate the bitset adjacency from `adj`, switching the
    /// graph into bitset mode. After this, `adj` is no longer maintained, so
    /// a caller that reads `graph.adj` directly must not call this mid-loop.
    ///
    /// A graph of at most [`BITSET_THRESH`] vertices is indexed by vertex id;
    /// a larger one is indexed over its active vertices. Either way the rows
    /// are released, since they are neither read nor maintained afterwards.
    pub(super) fn promote_bitset(&mut self) {
        debug_assert_eq!(self.bitset_words, 0);
        let n = self.adj.len();
        if n == 0 {
            return;
        }
        self.build_bitset(n > BITSET_THRESH);
    }

    /// Build the bitset from the adjacency rows and release them. `compact`
    /// indexes it over the active vertices; otherwise slots are vertex ids.
    fn build_bitset(&mut self, compact: bool) {
        let n = self.adj.len();
        // Slots are vertex ids unless the graph is too large for that, in
        // which case only the active vertices get one.
        let mut bitset_slot = Vec::new();
        let mut slot_vertex = Vec::new();
        let slots = if compact {
            bitset_slot = vec![NO_SLOT; n];
            slot_vertex.reserve(self.num_active);
            for (v, &live) in self.active.iter().enumerate() {
                if live {
                    bitset_slot[v] = slot_vertex.len() as u32;
                    slot_vertex.push(v as u32);
                }
            }
            slot_vertex.len()
        } else {
            n
        };
        self.bitset_compact = compact;
        let w = slots.div_ceil(64);
        let mut bitset = vec![0u64; slots * w];
        let mut degree = vec![0u32; slots];
        for v in 0..n {
            if !self.active[v] {
                continue;
            }
            let vs = if compact { bitset_slot[v] as usize } else { v };
            degree[vs] = self.adj[v].len() as u32;
            let vb = vs * w;
            for &u in self.adj[v].iter() {
                let us = if compact {
                    bitset_slot[u as usize] as usize
                } else {
                    u as usize
                };
                bitset[vb + us / 64] |= 1u64 << (us % 64);
            }
        }
        if !compact {
            // An inactive vertex keeps the degree its row still reports, as
            // the vertex-indexed build always has.
            for (v, row) in self.adj.iter().enumerate() {
                if !self.active[v] {
                    degree[v] = row.len() as u32;
                }
            }
        }
        // Every path that reads a row is guarded by `bitset_words == 0`, so
        // from here the rows are memory holding an answer nobody asks for.
        for row in self.adj.iter_mut() {
            *row = Vec::new();
        }
        self.bitset = bitset;
        self.bitset_degree = degree;
        self.bitset_words = w;
        self.bitset_slot = bitset_slot;
        self.slot_vertex = slot_vertex;
        self.drop_row_indexes();
    }

    /// Add edge (u, v) using the bitset for O(1) existence check. Assumes
    /// `bitset_words > 0`.
    fn add_edge_bs(&mut self, u: u32, v: u32) -> bool {
        if u == v {
            return false;
        }
        let ui = self.slot(u);
        let vi = self.slot(v);
        let w = self.bitset_words;
        let word_u = ui / 64;
        let bit_u = 1u64 << (ui % 64);
        if self.bitset[vi * w + word_u] & bit_u != 0 {
            return false;
        }
        self.bitset[vi * w + word_u] |= bit_u;
        self.bitset[ui * w + vi / 64] |= 1u64 << (vi % 64);
        self.bitset_degree[ui] += 1;
        self.bitset_degree[vi] += 1;
        self.num_edges += 1;
        true
    }

    pub(super) fn len(&self) -> usize {
        self.adj.len()
    }

    pub(super) fn degree(&self, v: u32) -> usize {
        if self.bitset_words > 0 {
            self.bitset_degree[self.slot(v)] as usize
        } else {
            self.adj[v as usize].len()
        }
    }

    pub(super) fn bitset_difference_count(&self, left: u32, right: u32) -> u64 {
        #[cfg(target_arch = "x86_64")]
        if self.hardware_popcount {
            // SAFETY: the flag is set only after runtime feature detection.
            return unsafe { self.bitset_difference_count_popcnt(left, right) };
        }
        self.bitset_difference_count_by(left, right, |word| word.count_ones() as u64)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "popcnt")]
    unsafe fn bitset_difference_count_popcnt(&self, left: u32, right: u32) -> u64 {
        self.bitset_difference_count_by(left, right, |word| {
            std::arch::x86_64::_popcnt64(word as i64) as u64
        })
    }

    #[inline(always)]
    fn bitset_difference_count_by(
        &self,
        left: u32,
        right: u32,
        popcount: impl Fn(u64) -> u64 + Copy,
    ) -> u64 {
        let words = self.bitset_words;
        let left_start = self.slot(left) * words;
        let right_start = self.slot(right) * words;
        difference_popcount_by(
            &self.bitset[left_start..left_start + words],
            &self.bitset[right_start..right_start + words],
            popcount,
        )
    }

    pub(super) fn collect_live_nbrs_into(&self, v: u32, buf: &mut Vec<u32>) {
        let start_len = buf.len();
        if self.bitset_words > 0 {
            let w = self.bitset_words;
            let vb = self.slot(v) * w;
            for j in 0..w {
                let mut bits = self.bitset[vb + j];
                while bits != 0 {
                    let lsb = bits.trailing_zeros() as usize;
                    buf.push(self.vertex_at(j * 64 + lsb));
                    bits &= bits - 1;
                }
            }
        } else {
            buf.extend_from_slice(&self.adj[v as usize]);
        }
        // Both paths touch every live neighbour once; the bitset path also
        // walks every word of `v`'s row, including empty words.
        crate::meter::charge(
            ((buf.len() - start_len) as u64).saturating_add(self.bitset_words as u64),
        );
    }

    pub(super) fn contains_edge(&self, u: u32, v: u32) -> bool {
        if self.bitset_words > 0 {
            crate::meter::charge(1);
            let w = self.bitset_words;
            let vi = self.slot(v);
            self.bitset[self.slot(u) * w + vi / 64] & (1u64 << (vi % 64)) != 0
        } else {
            crate::meter::charge(self.row_lookup_units(u));
            self.row_contains(u, v)
        }
    }

    /// Units one membership test on `vertex`'s row costs: a probe if the row
    /// is indexed, a scan of the whole row otherwise.
    #[inline]
    fn row_lookup_units(&self, vertex: u32) -> u64 {
        if self.row_index[vertex as usize].is_some() {
            1
        } else {
            self.adj[vertex as usize].len() as u64
        }
    }

    pub(super) fn add_edge(&mut self, u: u32, v: u32) -> bool {
        if u == v {
            return false;
        }
        if self.bitset_words > 0 {
            self.add_edge_bs(u, v)
        } else {
            if self.row_contains(u, v) {
                return false;
            }
            self.row_push(u, v);
            self.row_push(v, u);
            self.num_edges += 1;
            true
        }
    }

    /// Return a copy of `v`'s live neighbour list. In bitset mode this reads
    /// set bits directly and is correct even though `adj` itself goes stale.
    pub(super) fn live_neighbours(&self, v: u32) -> Vec<u32> {
        let mut neighbours = Vec::new();
        self.collect_live_nbrs_into(v, &mut neighbours);
        neighbours
    }

    /// Collect `v`'s live neighbours, eliminate it, and return them. Every
    /// caller in the library needs the list for the bag it records, so it
    /// collects the list itself and calls `eliminate_with_nbrs`; this remains
    /// as the one-call form the tests check that path against.
    #[cfg(test)]
    pub(super) fn eliminate(&mut self, v: u32) -> Vec<u32> {
        let neighbours = self.live_neighbours(v);
        self.eliminate_with_nbrs(v, &neighbours);
        neighbours
    }

    /// Eliminate vertex `v` given its pre-collected live neighbours. Avoids
    /// the extra `live_neighbours` allocation when the caller already has
    /// them.
    pub(super) fn eliminate_with_nbrs(&mut self, v: u32, neighbours: &[u32]) {
        // The construction meter's single largest charge: one elimination is
        // the unit of work every goatd configuration loops over, so what this
        // costs sets the scale everything else in construction is charged
        // against.
        //
        // The sparse path's cost is not k² alone. For a neighbour whose row is
        // short, `eliminate_with_nbrs_marker` walks that whole row — once to
        // stamp it, and to find `v` in it — so it pays deg(u) before it pays
        // the k² fill test. Around high-degree hubs that scan term used to
        // dominate k² by orders of magnitude; charging k² alone made
        // elimination read some fifty times cheaper than it runs, which let a
        // configuration spend its portfolio's whole window and more while the
        // work clock believed it had barely started. Measured on one such
        // residual: 3.8 M units charged against 265 ms of elimination. A hub's
        // row is now indexed and costs a probe instead, which is what
        // `nbr_scan_units` reports.
        let k = neighbours.len() as u64;
        crate::meter::charge(if self.bitset_words > 0 {
            k.saturating_mul(self.bitset_words as u64)
        } else {
            self.nbr_scan_units(neighbours)
                .saturating_add(k.saturating_mul(k))
        });
        if self.bitset_words > 0 {
            self.eliminate_with_nbrs_bs(v, neighbours);
        } else {
            self.eliminate_with_nbrs_marker(v, neighbours);
        }
    }

    fn eliminate_with_nbrs_bs(&mut self, v: u32, neighbours: &[u32]) {
        #[cfg(target_arch = "x86_64")]
        if self.hardware_popcount {
            // SAFETY: the flag is set only after runtime feature detection.
            return unsafe { self.eliminate_with_nbrs_bs_popcnt(v, neighbours) };
        }
        self.eliminate_with_nbrs_bs_by(v, neighbours, |word| word.count_ones());
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "popcnt")]
    unsafe fn eliminate_with_nbrs_bs_popcnt(&mut self, v: u32, neighbours: &[u32]) {
        self.eliminate_with_nbrs_bs_by(v, neighbours, |word| {
            std::arch::x86_64::_popcnt64(word as i64) as u32
        });
    }

    #[inline(always)]
    fn eliminate_with_nbrs_bs_by(
        &mut self,
        v: u32,
        neighbours: &[u32],
        popcount: impl Fn(u64) -> u32 + Copy,
    ) {
        let vi = self.slot(v);
        let w = self.bitset_words;
        let vb = vi * w;
        let mut pushes: usize = 0;

        let v_word = vi / 64;
        let v_bit = 1u64 << (vi % 64);
        debug_assert!(
            self.bitset[vb + v_word] & v_bit == 0,
            "vertex {v} is its own neighbour"
        );

        for &u_raw in neighbours {
            let u = self.slot(u_raw);
            let ub = u * w;
            let u_word = u / 64;
            let u_bit = 1u64 << (u % 64);
            debug_assert!(
                self.bitset[vb + u_word] & u_bit != 0,
                "eliminating {v} with a vertex that is not its neighbour"
            );
            // Take u out of v's row for the scan below, rather than masking
            // its bit off once per word, and put it back before the next
            // neighbour reads the row. v's own bit needs no such care: the
            // graph holds no self-loop, so v's row never carries it.
            self.bitset[vb + u_word] &= !u_bit;
            // The symmetric fill edge (bitset[wj] gaining bit u) is set when
            // wj's own outer-loop iteration runs, not here — bitset[wj] still
            // lacks bit u at that point, so u still shows up in wj's mask.
            let (v_row, u_row) = rows_mut(&mut self.bitset, vb, ub, w);
            let mut added = 0u32;
            for (&v_word, u_word) in v_row.iter().zip(u_row.iter_mut()) {
                let fill_mask = v_word & !*u_word;
                *u_word |= fill_mask;
                added += popcount(fill_mask);
            }
            self.bitset_degree[u] += added;
            pushes += added as usize;
            self.bitset[vb + u_word] |= u_bit;
            self.bitset[ub + v_word] &= !v_bit;
            self.bitset_degree[u] -= 1;
        }

        self.bitset[vb..vb + w].fill(0);
        self.bitset_degree[vi] = 0;
        self.finish_elimination(v, neighbours.len(), pushes);
    }

    fn eliminate_with_nbrs_marker(&mut self, v: u32, neighbours: &[u32]) {
        // Every neighbour u gets the same question: which of v's other
        // neighbours are missing from u's row. Asked through u's membership
        // map, the k probes land in a different map for every u, and once k
        // is in the thousands those maps do not fit in cache together, so
        // nearly every probe misses and the loop waits on each one. Asked
        // through the marker, u's row is stamped with blind stores, which the
        // store buffer takes without stalling, and the k reads that follow
        // land on the bag's own slots, which stay in cache from one
        // neighbour to the next. So an indexed row is stamped too unless it
        // is far longer than the bag, where stamping it would pay its length
        // rather than the bag's and the map probe wins.
        //
        // The pushes are the missing neighbours in bag order and v leaves
        // each row by the same swap-remove, so a row ends up identical
        // whichever way its question was answered, and a map stays in step
        // with its row.
        let walk_limit = neighbours.len().saturating_mul(WALK_ROW_FACTOR);
        let mut pushes: usize = 0;
        for &u_raw in neighbours {
            let u = u_raw as usize;
            let row = &mut self.adj[u];
            if row.len() > walk_limit
                && let Some(index) = self.row_index[u].as_deref_mut()
            {
                if let Some(position) = index.remove(&v) {
                    let position = position as usize;
                    let last = row.len() - 1;
                    let moved = row[last];
                    row.swap_remove(position);
                    if position != last {
                        index.insert(moved, position as u32);
                    }
                }
                for &w in neighbours {
                    if w != u_raw && !index.contains_key(&w) {
                        index.insert(w, row.len() as u32);
                        row.push(w);
                        pushes += 1;
                    }
                }
                continue;
            }
            self.elim_stamp = self.elim_stamp.wrapping_add(1);
            if self.elim_stamp == 0 {
                self.elim_marker.fill(0);
                self.elim_stamp = 1;
            }
            let s = self.elim_stamp;
            let marker = self.elim_marker.as_mut_slice();
            let mut v_pos = None;
            for (idx, &w) in row.iter().enumerate() {
                if w == v {
                    v_pos = Some(idx);
                }
                marker[w as usize] = s;
            }
            marker[u] = s;
            let mut index = self.row_index[u].as_deref_mut();
            if let Some(v_pos) = v_pos {
                let last = row.len() - 1;
                let moved = row[last];
                row.swap_remove(v_pos);
                if let Some(index) = index.as_deref_mut() {
                    index.remove(&v);
                    if v_pos != last {
                        index.insert(moved, v_pos as u32);
                    }
                }
            }
            match index {
                Some(index) => {
                    for &w in neighbours {
                        let wi = w as usize;
                        if marker[wi] != s {
                            marker[wi] = s;
                            index.insert(w, row.len() as u32);
                            row.push(w);
                            pushes += 1;
                        }
                    }
                }
                None => {
                    for &w in neighbours {
                        let wi = w as usize;
                        if marker[wi] != s {
                            marker[wi] = s;
                            row.push(w);
                            pushes += 1;
                        }
                    }
                    if row.len() >= ROW_INDEX_THRESH {
                        build_row_index(&self.adj[u], &mut self.row_index[u]);
                    }
                }
            }
        }
        self.clear_row(v);
        self.finish_elimination(v, neighbours.len(), pushes);
    }

    /// Deactivate `v` and settle the counts once its rows are done:
    /// `pushes` is the number of row entries the fill edges added.
    #[inline]
    fn finish_elimination(&mut self, v: u32, degree: usize, pushes: usize) {
        if self.active[v as usize] {
            self.active[v as usize] = false;
            self.num_active -= 1;
        }
        self.num_edges -= degree;
        self.num_edges += pushes / 2;
    }

    /// [`eliminate_with_nbrs`](Self::eliminate_with_nbrs) with the fill
    /// edges already known. A neighbour that gains nothing only loses `v`,
    /// and one that gains gets its edges without a pass to find them. The
    /// graph ends up as the unprepared elimination leaves it, row for row.
    pub(super) fn eliminate_prepared(
        &mut self,
        v: u32,
        neighbours: &[u32],
        fill: &PreparedFill<'_>,
    ) {
        let k = neighbours.len();
        if self.bitset_words > 0 {
            let w = self.bitset_words;
            let vi = self.slot(v);
            let vb = vi * w;
            let v_word = vi / 64;
            let v_bit = 1u64 << (vi % 64);
            let mut filled = 0u64;
            let mut pushes = 0usize;
            for &u_raw in neighbours {
                let u = self.slot(u_raw);
                let ub = u * w;
                let gained = fill.gained[u_raw as usize];
                if gained > 0 {
                    // As in the unprepared pass: u's bit leaves v's row while
                    // u's row takes what it lacks of it.
                    filled += 1;
                    let u_word = u / 64;
                    let u_bit = 1u64 << (u % 64);
                    self.bitset[vb + u_word] &= !u_bit;
                    let (v_row, u_row) = rows_mut(&mut self.bitset, vb, ub, w);
                    debug_assert_eq!(
                        v_row
                            .iter()
                            .zip(u_row.iter())
                            .map(|(&a, &b)| (a & !b).count_ones())
                            .sum::<u32>(),
                        gained,
                        "prepared fill count of {u_raw} disagrees with the rows"
                    );
                    for (&from_v, into_u) in v_row.iter().zip(u_row.iter_mut()) {
                        *into_u |= from_v;
                    }
                    self.bitset[vb + u_word] |= u_bit;
                    self.bitset_degree[u] += gained;
                    pushes += gained as usize;
                }
                self.bitset[ub + v_word] &= !v_bit;
                self.bitset_degree[u] -= 1;
            }
            self.bitset[vb..vb + w].fill(0);
            self.bitset_degree[vi] = 0;
            crate::meter::charge((k as u64).saturating_add(filled.saturating_mul(w as u64)));
            self.finish_elimination(v, k, pushes);
            return;
        }
        debug_assert_eq!(fill.starts.len(), k + 1);
        debug_assert_eq!(fill.v_position.len(), k);
        let mut pushes = 0usize;
        for (i, &u_raw) in neighbours.iter().enumerate() {
            let u = u_raw as usize;
            let partners = &fill.partners[fill.starts[i] as usize..fill.starts[i + 1] as usize];
            debug_assert_eq!(partners.len(), fill.gained[u] as usize);
            let row = &mut self.adj[u];
            let index = self.row_index[u].as_deref_mut();
            let position = match &index {
                // The map has to drop v either way, and says where it was.
                Some(index) => {
                    index.get(&v).copied().expect("v is in its neighbour's row") as usize
                }
                None => fill.v_position[i] as usize,
            };
            debug_assert_eq!(row[position], v);
            let last = row.len() - 1;
            let moved = row[last];
            row.swap_remove(position);
            match index {
                Some(index) => {
                    index.remove(&v);
                    if position != last {
                        index.insert(moved, position as u32);
                    }
                    for &y in partners {
                        index.insert(y, row.len() as u32);
                        row.push(y);
                    }
                }
                None => {
                    row.extend_from_slice(partners);
                    if row.len() >= ROW_INDEX_THRESH {
                        build_row_index(&self.adj[u], &mut self.row_index[u]);
                    }
                }
            }
            pushes += partners.len();
        }
        self.clear_row(v);
        crate::meter::charge((k + pushes) as u64);
        self.finish_elimination(v, k, pushes);
    }

    /// Remove `v`, given its live neighbours, without filling — safe only
    /// when the caller has verified N(v) is already a clique (no fill edges
    /// needed). Cheaper than `eliminate_with_nbrs`: no stamp-marker work.
    pub(super) fn remove_without_fill_nbrs(&mut self, v: u32, nbrs: &[u32]) {
        // Simplicial elimination adds no fill, so there is no k² term. The
        // sparse path locates `v` in each neighbour's row, which costs a probe
        // on an indexed row and a scan of the row otherwise; the bitset path
        // clears one bit per neighbour and then zeroes `v`'s own row, so it
        // pays k plus one pass over the words.
        crate::meter::charge(if self.bitset_words > 0 {
            (nbrs.len() as u64).saturating_add(self.bitset_words as u64)
        } else {
            self.nbr_scan_units(nbrs)
        });
        let vi = v as usize;
        if self.bitset_words > 0 {
            let w = self.bitset_words;
            let vs = self.slot(v);
            for &u in nbrs {
                let us = self.slot(u);
                self.bitset[us * w + vs / 64] &= !(1u64 << (vs % 64));
                self.bitset_degree[us] -= 1;
            }
            let vb = vs * w;
            for j in 0..w {
                self.bitset[vb + j] = 0;
            }
            self.bitset_degree[vs] = 0;
        } else {
            for &u in nbrs {
                self.row_swap_remove(u, v);
            }
            self.clear_row(v);
        }
        if self.active[vi] {
            self.active[vi] = false;
            self.num_active -= 1;
        }
        self.num_edges -= nbrs.len();
    }

    /// Units the sparse elimination paths pay to find `v` in each row of
    /// `nbrs` — a probe per indexed row, a full pass over the others — as
    /// opposed to the size of the neighbourhood they are handed.
    ///
    /// The metering guard keeps the summation off the un-metered path:
    /// [`crate::meter::charge`] is inert there, so counting for it
    /// would be pure overhead in every run that asked for no unit budget.
    #[inline]
    fn nbr_scan_units(&self, nbrs: &[u32]) -> u64 {
        if !crate::meter::is_armed() {
            return 0;
        }
        nbrs.iter().map(|&u| self.row_lookup_units(u)).sum()
    }

    /// O(1) check: is the active residual a complete graph?
    pub(super) fn is_residual_clique(&self) -> bool {
        let n = self.num_active;
        let complete_edges = (n as u64) * (n.saturating_sub(1) as u64) / 2;
        n < 2 || self.num_edges as u64 == complete_edges
    }

    /// Is the live neighbourhood of `v` a clique?
    ///
    /// Takes `&mut self` for the stamp marker the sparse path answers from;
    /// the graph itself is unchanged.
    #[allow(clippy::wrong_self_convention)]
    pub(super) fn is_simplicial(&mut self, v: u32) -> bool {
        if self.bitset_words > 0 {
            let w = self.bitset_words;
            let vb = self.slot(v) * w;
            let vbs = &self.bitset[vb..vb + w];
            let mut words_scanned = 0u64;
            for j in 0..w {
                let mut word = vbs[j];
                while word != 0 {
                    let lsb = word.trailing_zeros() as usize;
                    let u = j * 64 + lsb;
                    let ub = u * w;
                    // v is not simplicial iff some other neighbour w2 of v is
                    // not a neighbour of u, i.e. N(v) & ~N(u) has a bit set
                    // besides u's own.
                    for (l, &v_word) in vbs.iter().enumerate() {
                        words_scanned += 1;
                        let non_nbrs = v_word & !self.bitset[ub + l];
                        let masked = if l == u / 64 {
                            non_nbrs & !(1u64 << (u % 64))
                        } else {
                            non_nbrs
                        };
                        if masked != 0 {
                            crate::meter::charge(words_scanned);
                            return false;
                        }
                    }
                    word &= word - 1;
                }
            }
            crate::meter::charge(words_scanned);
            true
        } else {
            self.simplicial_by_rows(v)
        }
    }

    /// [`is_simplicial`](Self::is_simplicial) against the adjacency rows.
    ///
    /// The pairs are the pairs of the nested scan, in the same order, so the
    /// same missing edge is the one that ends the walk. What differs is how
    /// each pair is answered: an indexed row is probed, and an unindexed one
    /// is stamped once and its `k - i - 1` remaining questions answered from
    /// the marker, rather than scanned again per pair. Each pair still costs
    /// what the same test costs [`contains_edge`](Self::contains_edge), so
    /// what a caller spends does not depend on which way it was answered.
    fn simplicial_by_rows(&mut self, v: u32) -> bool {
        let k = self.adj[v as usize].len();
        // The last neighbour has no pair of its own: every pair it is in has
        // already been tested from the other end.
        for i in 0..k.saturating_sub(1) {
            let u = self.adj[v as usize][i] as usize;
            let units = self.row_lookup_units(u as u32);
            if self.row_index[u].is_none() {
                self.elim_stamp = self.elim_stamp.wrapping_add(1);
                if self.elim_stamp == 0 {
                    self.elim_marker.fill(0);
                    self.elim_stamp = 1;
                }
                let stamp = self.elim_stamp;
                let marker = self.elim_marker.as_mut_slice();
                for &w in &self.adj[u] {
                    marker[w as usize] = stamp;
                }
            }
            let stamp = self.elim_stamp;
            let neighbours = &self.adj[v as usize];
            for &w in &neighbours[i + 1..] {
                crate::meter::charge(units);
                let present = match &self.row_index[u] {
                    Some(index) => index.contains_key(&w),
                    None => self.elim_marker[w as usize] == stamp,
                };
                if !present {
                    return false;
                }
            }
        }
        true
    }

    /// The one edge missing from `v`'s neighbourhood, if exactly one is.
    ///
    /// `None` when the neighbourhood is already a clique and when two or more
    /// edges are missing, which is what the almost-simplicial rule needs: it
    /// fires only on the single-missing-edge case, and the pair is then unique.
    ///
    /// The bitset path counts, for each u ∈ N(v), how many of v's other
    /// neighbours u is not adjacent to. The sum over N(v) counts every missing
    /// edge twice, so the scan can stop as soon as it passes two, and the two
    /// vertices contributing one each are the endpoints. That is O(k · words)
    /// against the O(k²) membership tests of the pairwise scan, and on a
    /// residual of a few thousand vertices with degrees in the thousands the
    /// difference is seconds.
    pub(super) fn almost_simplicial_nonedge(&self, v: u32) -> Option<(u32, u32)> {
        if self.bitset_words == 0 {
            let neighbours = &self.adj[v as usize];
            let mut missing: Option<(u32, u32)> = None;
            for i in 0..neighbours.len() {
                for j in (i + 1)..neighbours.len() {
                    if !self.contains_edge(neighbours[i], neighbours[j]) {
                        if missing.is_some() {
                            return None;
                        }
                        missing = Some((neighbours[i], neighbours[j]));
                    }
                }
            }
            return missing;
        }

        let w = self.bitset_words;
        let vb = self.slot(v) * w;
        let mut endpoints: [u32; 2] = [0, 0];
        let mut found = 0usize;
        let mut total = 0u64;
        let mut neighbours_scanned = 0u64;
        for j in 0..w {
            let mut bits = self.bitset[vb + j];
            while bits != 0 {
                let u = self.vertex_at(j * 64 + bits.trailing_zeros() as usize);
                bits &= bits - 1;
                neighbours_scanned += 1;
                // u itself is in N(v) and not in N(u), so one of the counted
                // bits is always u.
                let missing_at_u = self.bitset_difference_count(v, u) - 1;
                total += missing_at_u;
                if total > 2 {
                    crate::meter::charge(neighbours_scanned * w as u64);
                    return None;
                }
                if missing_at_u == 1 {
                    if found == 2 {
                        crate::meter::charge(neighbours_scanned * w as u64);
                        return None;
                    }
                    endpoints[found] = u;
                    found += 1;
                }
            }
        }
        crate::meter::charge(neighbours_scanned * w as u64);
        if total == 2 && found == 2 {
            Some((endpoints[0], endpoints[1]))
        } else {
            None
        }
    }

    /// Fill count of `v` via bitset intersection: for each u ∈ N(v),
    /// popcount(bitset[u] & bitset[v]) counts N(v) members adjacent to u;
    /// summed and halved gives edges within N(v). O(k · words) vs
    /// O(k · avg_deg) for the marker path.
    pub(super) fn fill_count_of_bs(&self, v: u32) -> u64 {
        #[cfg(target_arch = "x86_64")]
        if self.hardware_popcount {
            // SAFETY: the flag is set only after runtime feature detection.
            return unsafe { self.fill_count_of_bs_popcnt(v) };
        }
        self.fill_count_of_bs_by(v, |word| word.count_ones() as u64)
    }

    #[cfg(test)]
    pub(super) fn fill_count_of_bs_portable(&self, v: u32) -> u64 {
        self.fill_count_of_bs_by(v, |word| word.count_ones() as u64)
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "popcnt")]
    unsafe fn fill_count_of_bs_popcnt(&self, v: u32) -> u64 {
        self.fill_count_of_bs_by(v, |word| std::arch::x86_64::_popcnt64(word as i64) as u64)
    }

    #[inline(always)]
    fn fill_count_of_bs_by(&self, v: u32, popcount: impl Fn(u64) -> u64 + Copy) -> u64 {
        let vi = self.slot(v);
        let w = self.bitset_words;
        let vb = vi * w;
        let vbs = &self.bitset[vb..vb + w];
        let k = self.bitset_degree[vi] as u64;
        if k < 2 {
            return 0;
        }
        let total_pairs = k * (k - 1) / 2;

        // Hardware popcount makes dense scans win sooner. Keep the portable
        // path's earlier break-even for targets where each word costs more.
        let sparse_threshold = if self.hardware_popcount { w } else { 2 * w };
        let klen = k as usize;
        if k < sparse_threshold as u64 && klen <= 256 {
            return self.fill_count_of_bs_sparse(vbs, klen, w, total_pairs);
        }

        // Dense fallback: O(k · w).
        let mut edges = 0u64;
        for j in 0..w {
            let mut word = vbs[j];
            while word != 0 {
                let lsb = word.trailing_zeros() as usize;
                let u = j * 64 + lsb;
                let ub = u * w;
                let ubs = &self.bitset[ub..ub + w];
                word &= word - 1;
                edges += popcount(ubs[j] & word);
                edges += intersection_popcount_by(&ubs[j + 1..], &vbs[j + 1..], popcount);
            }
        }
        total_pairs - edges
    }

    #[inline(never)]
    fn fill_count_of_bs_sparse(&self, vbs: &[u64], klen: usize, w: usize, total_pairs: u64) -> u64 {
        let mut nbrs = [0u32; 256];
        let mut idx = 0;
        for (j, &v_word) in vbs.iter().enumerate() {
            let mut word = v_word;
            while word != 0 {
                let lsb = word.trailing_zeros() as usize;
                nbrs[idx] = (j * 64 + lsb) as u32;
                idx += 1;
                word &= word - 1;
            }
        }
        let mut edges = 0u64;
        for i in 0..klen {
            let u = nbrs[i] as usize;
            let ub = u * w;
            for &other in &nbrs[i + 1..klen] {
                let x = other as usize;
                let bit = (self.bitset[ub + (x >> 6)] >> (x & 63)) & 1;
                edges += bit;
            }
        }
        total_pairs - edges
    }
}
