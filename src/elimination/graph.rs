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

/// Maximum graph size for which full bitset adjacency is maintained.
/// At n = 16384: 16384 * 256 words * 8 bytes = 32 MB per graph.
const BITSET_THRESH: usize = 16384;

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
#[derive(Clone)]
pub(super) struct EliminationGraph {
    pub(super) adj: Vec<Vec<u32>>,
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
    /// Flat bitset adjacency: vertex `v` occupies words
    /// `v * bitset_words .. (v+1) * bitset_words`; bit `u` in that slice is
    /// set iff edge (v, u) exists. Empty when bitset mode is disabled.
    pub(super) bitset: Vec<u64>,
    /// Number of u64 words per vertex in `bitset`. 0 iff bitset is disabled.
    pub(super) bitset_words: usize,
    hardware_popcount: bool,
}

impl EliminationGraph {
    pub(super) fn new(n: usize) -> Self {
        EliminationGraph {
            adj: vec![Vec::new(); n],
            active: vec![true; n],
            num_active: n,
            num_edges: 0,
            bitset_degree: Vec::new(),
            elim_marker: vec![0u16; n],
            elim_stamp: 0,
            bitset: Vec::new(),
            bitset_words: 0,
            hardware_popcount: hardware_popcount_available(),
        }
    }

    pub(super) fn from_edges(n: u32, edges: &[(u32, u32)]) -> Self {
        let n = n as usize;
        let mut g = EliminationGraph::new(n);
        for &(u, v) in edges {
            assert!(
                (u as usize) < n && (v as usize) < n,
                "elimination edge ({u}, {v}) has an endpoint outside 0..{n}"
            );
            if u != v && !g.adj[u as usize].contains(&v) {
                g.adj[u as usize].push(v);
                g.adj[v as usize].push(u);
                g.num_edges += 1;
            }
        }
        if n <= BITSET_THRESH && g.num_edges.saturating_mul(128) > n.saturating_mul(n) {
            let w = n.div_ceil(64);
            g.bitset = vec![0u64; n * w];
            g.bitset_words = w;
            g.bitset_degree = g.adj.iter().map(|row| row.len() as u32).collect();
            for v in 0..n {
                for &u in g.adj[v].iter() {
                    g.bitset[v * w + u as usize / 64] |= 1u64 << (u as usize % 64);
                }
            }
        }
        g
    }

    /// True when promoting from adj-only to bitset-assisted representation is
    /// worthwhile: density has crossed the break-even where bitset's
    /// O(k · words) beats the marker path's O(k · avg_deg). With
    /// `avg_deg = 2·num_edges / num_active` and `words ≈ n/64`, that break-even
    /// is `128·num_edges > n · num_active` — `num_active`, not `n`, so
    /// promotion still fires when fill edges densify the graph mid-elimination
    /// even though `from_edges` saw it as sparse.
    pub(super) fn should_promote_bitset(&self) -> bool {
        if self.bitset_words > 0 {
            return false;
        }
        let n = self.adj.len();
        if n == 0 || n > BITSET_THRESH {
            return false;
        }
        self.num_edges.saturating_mul(128) > n.saturating_mul(self.num_active.max(1))
    }

    /// Allocate and populate the bitset adjacency from `adj`, switching the
    /// graph into bitset mode. After this, `adj` is no longer maintained, so
    /// a caller that reads `graph.adj` directly must not call this mid-loop.
    pub(super) fn promote_bitset(&mut self) {
        debug_assert_eq!(self.bitset_words, 0);
        let n = self.adj.len();
        if n == 0 || n > BITSET_THRESH {
            return;
        }
        let w = n.div_ceil(64);
        let mut bs = vec![0u64; n * w];
        self.bitset_degree = self.adj.iter().map(|row| row.len() as u32).collect();
        for v in 0..n {
            if !self.active[v] {
                continue;
            }
            for &u in self.adj[v].iter() {
                bs[v * w + u as usize / 64] |= 1u64 << (u as usize % 64);
            }
        }
        self.bitset = bs;
        self.bitset_words = w;
    }

    /// Clone preserving only the bitset; adj rows are allocated empty. Only
    /// valid when `bitset_words > 0`.
    pub(super) fn clone_bitset_only(&self) -> Self {
        debug_assert!(self.bitset_words > 0);
        EliminationGraph {
            adj: vec![Vec::new(); self.adj.len()],
            active: self.active.clone(),
            num_active: self.num_active,
            num_edges: self.num_edges,
            bitset_degree: self.bitset_degree.clone(),
            elim_marker: self.elim_marker.clone(),
            elim_stamp: self.elim_stamp,
            bitset: self.bitset.clone(),
            bitset_words: self.bitset_words,
            hardware_popcount: self.hardware_popcount,
        }
    }

    /// Add edge (u, v) using the bitset for O(1) existence check. Assumes
    /// `bitset_words > 0`.
    fn add_edge_bs(&mut self, u: u32, v: u32) -> bool {
        if u == v {
            return false;
        }
        let ui = u as usize;
        let vi = v as usize;
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
            self.bitset_degree[v as usize] as usize
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
        let left_start = left as usize * words;
        let right_start = right as usize * words;
        difference_popcount_by(
            &self.bitset[left_start..left_start + words],
            &self.bitset[right_start..right_start + words],
            popcount,
        )
    }

    pub(super) fn collect_live_nbrs_into(&self, v: u32, buf: &mut Vec<u32>) {
        let start_len = buf.len();
        if self.bitset_words > 0 {
            let vi = v as usize;
            let w = self.bitset_words;
            let vb = vi * w;
            for j in 0..w {
                let mut bits = self.bitset[vb + j];
                while bits != 0 {
                    let lsb = bits.trailing_zeros() as usize;
                    buf.push((j * 64 + lsb) as u32);
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
            let vi = v as usize;
            self.bitset[u as usize * w + vi / 64] & (1u64 << (vi % 64)) != 0
        } else {
            crate::meter::charge(self.adj[u as usize].len() as u64);
            self.adj[u as usize].contains(&v)
        }
    }

    pub(super) fn add_edge(&mut self, u: u32, v: u32) -> bool {
        if u == v {
            return false;
        }
        if self.bitset_words > 0 {
            self.add_edge_bs(u, v)
        } else {
            if self.adj[u as usize].contains(&v) {
                return false;
            }
            self.adj[u as usize].push(v);
            self.adj[v as usize].push(u);
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

    pub(super) fn eliminate(&mut self, v: u32) -> Vec<u32> {
        let neighbours = self.live_neighbours(v);
        self.eliminate_with_nbrs(v, &neighbours);
        neighbours
    }

    /// Eliminate vertex `v` given its pre-collected live neighbours. Avoids
    /// the extra `live_neighbours` allocation when the caller already has
    /// them.
    pub(super) fn eliminate_with_nbrs(&mut self, v: u32, neighbours: &[u32]) {
        self.eliminate_with_nbrs_impl(v, neighbours, None);
    }

    /// Eliminate `v` and record each fill edge once in canonical order.
    pub(super) fn eliminate_with_nbrs_record_fill(
        &mut self,
        v: u32,
        neighbours: &[u32],
        fill_edges: &mut Vec<(u32, u32)>,
    ) {
        fill_edges.clear();
        self.eliminate_with_nbrs_impl(v, neighbours, Some(fill_edges));
    }

    fn eliminate_with_nbrs_impl(
        &mut self,
        v: u32,
        neighbours: &[u32],
        fill_edges: Option<&mut Vec<(u32, u32)>>,
    ) {
        // The construction meter's single largest charge: one elimination is
        // the unit of work every goatd configuration loops over, so what this
        // costs sets the scale everything else in construction is charged
        // against.
        //
        // The sparse path's cost is not k². `eliminate_with_nbrs_marker` walks
        // every neighbour's whole adjacency row — once to stamp it, and to
        // find `v` in it — so it pays Σ deg(u) for u ∈ N(v) before it pays the
        // k² fill test. Around high-degree hubs that scan term can dominate k²
        // by orders of magnitude. Charging k² alone made elimination read some
        // fifty times cheaper than it runs, which let a configuration spend its
        // portfolio's whole window and more while the work clock believed it had
        // barely started. Measured on one such residual: 3.8 M units charged
        // against 265 ms of elimination.
        let k = neighbours.len() as u64;
        crate::meter::charge(if self.bitset_words > 0 {
            k.saturating_mul(self.bitset_words as u64)
        } else {
            self.nbr_scan_units(neighbours)
                .saturating_add(k.saturating_mul(k))
        });
        if self.bitset_words > 0 {
            self.eliminate_with_nbrs_bs(v, neighbours, fill_edges);
        } else {
            self.eliminate_with_nbrs_marker(v, neighbours, fill_edges);
        }
    }

    fn eliminate_with_nbrs_bs(
        &mut self,
        v: u32,
        neighbours: &[u32],
        fill_edges: Option<&mut Vec<(u32, u32)>>,
    ) {
        #[cfg(target_arch = "x86_64")]
        if self.hardware_popcount {
            // SAFETY: the flag is set only after runtime feature detection.
            return unsafe { self.eliminate_with_nbrs_bs_popcnt(v, neighbours, fill_edges) };
        }
        self.eliminate_with_nbrs_bs_by(v, neighbours, fill_edges, |word| word.count_ones());
    }

    #[cfg(target_arch = "x86_64")]
    #[target_feature(enable = "popcnt")]
    unsafe fn eliminate_with_nbrs_bs_popcnt(
        &mut self,
        v: u32,
        neighbours: &[u32],
        fill_edges: Option<&mut Vec<(u32, u32)>>,
    ) {
        self.eliminate_with_nbrs_bs_by(v, neighbours, fill_edges, |word| {
            std::arch::x86_64::_popcnt64(word as i64) as u32
        });
    }

    #[inline(always)]
    fn eliminate_with_nbrs_bs_by(
        &mut self,
        v: u32,
        neighbours: &[u32],
        mut fill_edges: Option<&mut Vec<(u32, u32)>>,
        popcount: impl Fn(u64) -> u32 + Copy,
    ) {
        let vi = v as usize;
        let w = self.bitset_words;
        let vb = vi * w;
        let mut pushes: usize = 0;

        for &u_raw in neighbours {
            let u = u_raw as usize;
            let ub = u * w;
            // The symmetric fill edge (bitset[wj] gaining bit u) is set when
            // wj's own outer-loop iteration runs, not here — bitset[wj] still
            // lacks bit u at that point, so u still shows up in wj's mask.
            for j in 0..w {
                let mut fill_mask = self.bitset[vb + j] & !self.bitset[ub + j];
                if j == vi / 64 {
                    fill_mask &= !(1u64 << (vi % 64));
                }
                if j == u / 64 {
                    fill_mask &= !(1u64 << (u % 64));
                }
                if let Some(edges) = fill_edges.as_deref_mut() {
                    let mut canonical = fill_mask;
                    while canonical != 0 {
                        let bit = canonical.trailing_zeros() as usize;
                        let other = (j * 64 + bit) as u32;
                        if u_raw < other {
                            edges.push((u_raw, other));
                        }
                        canonical &= canonical - 1;
                    }
                }
                self.bitset[ub + j] |= fill_mask;
                let added = popcount(fill_mask);
                self.bitset_degree[u] += added;
                pushes += added as usize;
            }
            self.bitset[ub + vi / 64] &= !(1u64 << (vi % 64));
            self.bitset_degree[u] -= 1;
        }

        for j in 0..w {
            self.bitset[vb + j] = 0;
        }
        self.bitset_degree[vi] = 0;
        if self.active[vi] {
            self.active[vi] = false;
            self.num_active -= 1;
        }
        self.num_edges -= neighbours.len();
        self.num_edges += pushes / 2;
    }

    fn eliminate_with_nbrs_marker(
        &mut self,
        v: u32,
        neighbours: &[u32],
        mut fill_edges: Option<&mut Vec<(u32, u32)>>,
    ) {
        let marker = self.elim_marker.as_mut_slice();
        let mut pushes: usize = 0;
        for &u_raw in neighbours {
            let u = u_raw as usize;
            self.elim_stamp = self.elim_stamp.wrapping_add(1);
            if self.elim_stamp == 0 {
                marker.fill(0);
                self.elim_stamp = 1;
            }
            let s = self.elim_stamp;
            let row = &mut self.adj[u];
            let mut v_pos = None;
            for (idx, &w) in row.iter().enumerate() {
                if w == v {
                    v_pos = Some(idx);
                }
                marker[w as usize] = s;
            }
            if let Some(v_pos) = v_pos {
                row.swap_remove(v_pos);
            }
            marker[u] = s;
            for &w in neighbours {
                let wi = w as usize;
                if marker[wi] != s {
                    marker[wi] = s;
                    row.push(w);
                    pushes += 1;
                    if u_raw < w
                        && let Some(edges) = fill_edges.as_deref_mut()
                    {
                        edges.push((u_raw, w));
                    }
                }
            }
        }
        self.adj[v as usize].clear();
        if self.active[v as usize] {
            self.active[v as usize] = false;
            self.num_active -= 1;
        }
        self.num_edges -= neighbours.len();
        self.num_edges += pushes / 2;
    }

    /// Remove vertex `v` without filling its neighbourhood — safe only when
    /// the caller already knows `v`'s removal cannot need a fill edge.
    /// Returns the vertex's live neighbours.
    pub(super) fn remove_without_fill(&mut self, v: u32) -> Vec<u32> {
        let neighbours = self.live_neighbours(v);
        self.remove_without_fill_nbrs(v, &neighbours);
        neighbours
    }

    /// Remove `v`, given its live neighbours, without filling — safe only
    /// when the caller has verified N(v) is already a clique (no fill edges
    /// needed). Cheaper than `eliminate_with_nbrs`: no stamp-marker work.
    pub(super) fn remove_without_fill_nbrs(&mut self, v: u32, nbrs: &[u32]) {
        // Simplicial elimination adds no fill, so there is no k² term. The
        // sparse path still searches each neighbour's row for `v` and pays the
        // same Σ deg(u) scan as the filling path; the bitset path clears one
        // bit per neighbour and then zeroes `v`'s own row, so it pays k plus
        // one pass over the words.
        crate::meter::charge(if self.bitset_words > 0 {
            (nbrs.len() as u64).saturating_add(self.bitset_words as u64)
        } else {
            self.nbr_scan_units(nbrs)
        });
        let vi = v as usize;
        if self.bitset_words > 0 {
            let w = self.bitset_words;
            for &u in nbrs {
                self.bitset[u as usize * w + vi / 64] &= !(1u64 << (vi % 64));
                self.bitset_degree[u as usize] -= 1;
            }
            let vb = vi * w;
            for j in 0..w {
                self.bitset[vb + j] = 0;
            }
            self.bitset_degree[vi] = 0;
        } else {
            for &u in nbrs {
                let row = &mut self.adj[u as usize];
                if let Some(pos) = row.iter().position(|&x| x == v) {
                    row.swap_remove(pos);
                }
            }
            self.adj[vi].clear();
        }
        if self.active[vi] {
            self.active[vi] = false;
            self.num_active -= 1;
        }
        self.num_edges -= nbrs.len();
    }

    /// Units for one pass over the adjacency rows of `nbrs` — what the sparse
    /// elimination paths actually pay, as opposed to the size of the
    /// neighbourhood they are handed.
    ///
    /// The metering guard keeps the summation off the un-metered path:
    /// [`crate::meter::charge`] is inert there, so counting for it
    /// would be pure overhead in every run that asked for no unit budget.
    #[inline]
    fn nbr_scan_units(&self, nbrs: &[u32]) -> u64 {
        if !crate::meter::is_armed() {
            return 0;
        }
        nbrs.iter()
            .map(|&u| self.adj[u as usize].len() as u64)
            .sum()
    }

    /// O(1) check: is the active residual a complete graph?
    pub(super) fn is_residual_clique(&self) -> bool {
        let n = self.num_active;
        let complete_edges = (n as u64) * (n.saturating_sub(1) as u64) / 2;
        n < 2 || self.num_edges as u64 == complete_edges
    }

    /// Is the live neighbourhood of `v` a clique?
    pub(super) fn is_simplicial(&self, v: u32) -> bool {
        if self.bitset_words > 0 {
            let vi = v as usize;
            let w = self.bitset_words;
            let vb = vi * w;
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
            let neighbours = &self.adj[v as usize];
            for i in 0..neighbours.len() {
                for j in (i + 1)..neighbours.len() {
                    if !self.contains_edge(neighbours[i], neighbours[j]) {
                        return false;
                    }
                }
            }
            true
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
        let vi = v as usize;
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
