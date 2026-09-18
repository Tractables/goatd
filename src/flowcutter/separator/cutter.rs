//! The incremental cut search: grow a source side and a target side, augment
//! flow whenever one reaches the other, then pierce the current cut at the
//! frontier node scoring best on hop distance, and repeat.
//!
//! `BasicCutter` is one source-target pair. `Cutter` drives one of those and
//! commits to a new current cut only once the smaller side has grown as well
//! — the Pareto rule that makes the search anytime, so `super`'s outer loop
//! can stop at any point and still hold a usable separator.
//!
//! Everything here is in the vertex-split index space `expanded` defines: nodes
//! and arcs are the split graph's, so a cut is an edge cut that only becomes a
//! vertex separator once the caller maps it back. Piercing scores read hop
//! distances taken once at `init` and never rescored, so they stay a fixed
//! heuristic as the cut moves.

use super::*;

const SOURCE_SIDE: usize = 0;

const TARGET_SIDE: usize = 1;

/// Per-arc flow value, stored offset by 1 to fit in u8 for compactness.
/// Actual flow = stored - 1, so 0/1/2 ↔ flow -1/0/1.
struct Flow {
    f: Vec<u8>,
}

impl Flow {
    fn new(arc_count: usize) -> Self {
        Flow {
            f: vec![1u8; arc_count],
        }
    }
    fn clear(&mut self) {
        self.f.fill(1);
    }
    #[inline]
    fn get(&self, a: u32) -> i8 {
        (self.f[a as usize] as i8) - 1
    }
    #[inline]
    fn increase(&mut self, a: u32, back: u32) {
        self.f[a as usize] += 1;
        self.f[back as usize] = 2 - self.f[a as usize];
    }
    #[inline]
    fn decrease(&mut self, a: u32, back: u32) {
        self.f[a as usize] -= 1;
        self.f[back as usize] = 2 - self.f[a as usize];
    }
}

struct NodeSet {
    inside: Vec<bool>,
    count: u32,
    extra: Option<u32>,
}

impl NodeSet {
    fn new(n: u32) -> Self {
        NodeSet {
            inside: vec![false; n as usize],
            count: 0,
            extra: None,
        }
    }
    fn clear(&mut self) {
        self.inside.fill(false);
        self.count = 0;
        self.extra = None;
    }
    fn set_extra(&mut self, x: u32) {
        debug_assert!(!self.inside[x as usize]);
        debug_assert!(self.extra.is_none());
        self.inside[x as usize] = true;
        self.count += 1;
        self.extra = Some(x);
    }
}

fn bfs_hop_distance(g: &OrigGraph, source: u32, dist: &mut [i32], queue: &mut Vec<u32>) {
    for d in dist.iter_mut() {
        *d = i32::MAX;
    }
    dist[source as usize] = 0;
    queue.clear();
    queue.push(source);
    let mut head = 0usize;
    while head < queue.len() {
        let x = queue[head];
        head += 1;
        let dx = dist[x as usize];
        exp_out_arcs(g, x, |xy| {
            let y = exp_head(g, xy);
            if dist[y as usize] > dx + 1 {
                dist[y as usize] = dx + 1;
                queue.push(y);
            }
        });
    }
}

struct BasicCutter {
    assim: [NodeSet; 2],
    /// Arc IDs, not node IDs: arcs leaving `assim[side]` that carry flow.
    front: [Vec<u32>; 2],
    reach: [NodeSet; 2],
    /// Nodes the reachable search has set since the last reset, so the reset
    /// can restore those entries instead of rewriting the whole array.
    reach_touched: [Vec<u32>; 2],
    /// Arc IDs indexed by node: the arc used to reach each node, for walking
    /// an augmenting path back to its source.
    predecessor: [Vec<u32>; 2],
    flow: Flow,
    tmp_dfs: Vec<u32>,
    /// Queue for the two hop-distance searches `init` runs. Held here so that
    /// re-initializing a cutter does not allocate one per initialization;
    /// `bfs_hop_distance` clears it before each use.
    bfs_queue: Vec<u32>,
    /// Hop distances from the two original endpoints, fixed at `init` and
    /// never rescored as the cut grows.
    node_dist: [Vec<i32>; 2],
    cut_available: bool,
}

const NO_PRED: u32 = u32::MAX;

impl BasicCutter {
    fn new(n_exp: u32, a_exp: u32) -> Self {
        BasicCutter {
            assim: [NodeSet::new(n_exp), NodeSet::new(n_exp)],
            front: [Vec::new(), Vec::new()],
            reach: [NodeSet::new(n_exp), NodeSet::new(n_exp)],
            reach_touched: [Vec::new(), Vec::new()],
            predecessor: [vec![NO_PRED; n_exp as usize], vec![NO_PRED; n_exp as usize]],
            flow: Flow::new(a_exp as usize),
            tmp_dfs: Vec::with_capacity(n_exp as usize),
            bfs_queue: Vec::with_capacity(n_exp as usize),
            node_dist: [
                vec![i32::MAX; n_exp as usize],
                vec![i32::MAX; n_exp as usize],
            ],
            cut_available: false,
        }
    }

    /// Start a search from the source-target pair `p`, on a graph of the size
    /// this cutter was built for.
    ///
    /// This is a complete reset: every field a search reads is either cleared
    /// here or overwritten before it is read, so a cutter that has already run
    /// behaves like a new one.
    fn init(&mut self, g: &OrigGraph, p: (u32, u32)) {
        for s in 0..2 {
            self.assim[s].clear();
            self.reach[s].clear();
            self.reach_touched[s].clear();
            self.front[s].clear();
            for p in self.predecessor[s].iter_mut() {
                *p = NO_PRED;
            }
        }
        self.flow.clear();

        self.assim[SOURCE_SIDE].set_extra(p.0);
        self.reach[SOURCE_SIDE].set_extra(p.0);
        self.assim[TARGET_SIDE].set_extra(p.1);
        self.reach[TARGET_SIDE].set_extra(p.1);

        bfs_hop_distance(
            g,
            p.0,
            &mut self.node_dist[SOURCE_SIDE],
            &mut self.bfs_queue,
        );
        bfs_hop_distance(
            g,
            p.1,
            &mut self.node_dist[TARGET_SIDE],
            &mut self.bfs_queue,
        );

        self.grow_reachable_sets(g, SOURCE_SIDE);
        self.grow_assimilated_sets(g);

        self.cut_available = true;
    }

    fn is_saturated(&self, g: &OrigGraph, direction: usize, arc: u32) -> bool {
        let arc = if direction == TARGET_SIDE {
            exp_back(g, arc)
        } else {
            arc
        };
        let cap = exp_capacity(g.arc_count, arc);
        let flow = self.flow.get(arc);
        cap == flow
    }

    /// Grows `reach[pierced_side]`; on hitting the opposite assimilated set it
    /// augments `flow` and continues, then conditionally regrows the other
    /// side once no augmenting path remains.
    fn grow_reachable_sets(&mut self, g: &OrigGraph, pierced_side: usize) {
        let my_src = pierced_side;
        let my_tgt = 1 - pierced_side;

        let mut was_flow_augmented = false;

        loop {
            let mut target_hit = None;

            let extra = match self.reach[my_src].extra.take() {
                Some(x) => x,
                None => break,
            };

            self.tmp_dfs.clear();
            self.tmp_dfs.push(extra);
            'dfs: while let Some(x) = self.tmp_dfs.pop() {
                let mut found_in_iter = None;
                let mut stop = false;
                exp_out_arcs(g, x, |xy| {
                    if stop {
                        return;
                    }
                    let y = exp_head(g, xy);
                    if self.reach[my_src].inside[y as usize] {
                        return;
                    }
                    if self.is_saturated(g, my_src, xy) {
                        return;
                    }
                    self.predecessor[my_src][y as usize] = xy;
                    self.reach[my_src].inside[y as usize] = true;
                    self.reach_touched[my_src].push(y);
                    self.reach[my_src].count += 1;
                    if self.assim[my_tgt].inside[y as usize] {
                        found_in_iter = Some(y);
                        stop = true;
                        return;
                    }
                    self.tmp_dfs.push(y);
                });
                if found_in_iter.is_some() {
                    target_hit = found_in_iter;
                    break 'dfs;
                }
            }

            if let Some(target) = target_hit {
                self.augment_along_path(g, my_src, target, pierced_side == SOURCE_SIDE);
                self.reset_reachable(my_src);
                was_flow_augmented = true;
            } else {
                break;
            }
        }

        if was_flow_augmented {
            self.reset_reachable(my_tgt);
            // No early exit here, unlike the my_src grow above: this needs the
            // full reachable set, not just the first augmenting path.
            let extra = match self.reach[my_tgt].extra.take() {
                Some(x) => x,
                None => return,
            };
            self.tmp_dfs.clear();
            self.tmp_dfs.push(extra);
            while let Some(x) = self.tmp_dfs.pop() {
                exp_out_arcs(g, x, |xy| {
                    let y = exp_head(g, xy);
                    if self.reach[my_tgt].inside[y as usize] {
                        return;
                    }
                    if self.is_saturated(g, my_tgt, xy) {
                        return;
                    }
                    self.predecessor[my_tgt][y as usize] = xy;
                    self.reach[my_tgt].inside[y as usize] = true;
                    self.reach_touched[my_tgt].push(y);
                    self.reach[my_tgt].count += 1;
                    self.tmp_dfs.push(y);
                });
            }
        }
    }

    fn augment_along_path(
        &mut self,
        g: &OrigGraph,
        my_src: usize,
        target: u32,
        pierced_from_source: bool,
    ) {
        let mut x = target;
        while !self.assim[my_src].inside[x as usize] {
            let xy = self.predecessor[my_src][x as usize];
            debug_assert!(xy != NO_PRED, "predecessor chain broken");
            let back = exp_back(g, xy);
            if pierced_from_source {
                self.flow.increase(xy, back);
            } else {
                self.flow.decrease(xy, back);
            }
            x = exp_tail(g, xy);
        }
    }

    /// Put `reach[side]` back to `assim[side]`.
    ///
    /// Only the nodes the search set since the last reset can differ, so those
    /// are the only ones restored. That relies on the assimilated set being a
    /// subset of the reachable one, which the reference implementation asserts
    /// (`flow_cutter.hpp`, "assimilated must be a subset of reachable") and
    /// which `current_cut_side` already assumes when it compares the two
    /// counts.
    fn reset_reachable(&mut self, side: usize) {
        debug_assert!(
            self.assim[side]
                .inside
                .iter()
                .zip(self.reach[side].inside.iter())
                .all(|(&a, &r)| !a || r),
            "assimilated must be a subset of reachable"
        );
        for &node in &self.reach_touched[side] {
            self.reach[side].inside[node as usize] = self.assim[side].inside[node as usize];
        }
        self.reach_touched[side].clear();
        self.reach[side].count = self.assim[side].count;
        self.reach[side].extra = self.assim[side].extra;
    }

    fn grow_assimilated_sets(&mut self, g: &OrigGraph) {
        let smaller = if self.reach[SOURCE_SIDE].count <= self.reach[TARGET_SIDE].count {
            SOURCE_SIDE
        } else {
            TARGET_SIDE
        };

        let extra = match self.assim[smaller].extra.take() {
            Some(x) => x,
            None => return,
        };

        self.tmp_dfs.clear();
        self.tmp_dfs.push(extra);
        while let Some(x) = self.tmp_dfs.pop() {
            exp_out_arcs(g, x, |xy| {
                let y = exp_head(g, xy);
                let f = self.flow.get(xy);
                if f != 0 {
                    self.front[smaller].push(xy);
                }
                if self.assim[smaller].inside[y as usize] {
                    return;
                }
                if self.is_saturated(g, smaller, xy) {
                    return;
                }
                self.assim[smaller].inside[y as usize] = true;
                self.assim[smaller].count += 1;
                self.tmp_dfs.push(y);
            });
        }

        let inside_ref = &self.assim[smaller].inside;
        self.front[smaller].retain(|&xy| !inside_ref[exp_head(g, xy) as usize]);
    }

    fn current_cut_side(&self) -> usize {
        // Arbitrary tie-break, chosen to match the ported reference implementation.
        let src_sat = self.reach[SOURCE_SIDE].count == self.assim[SOURCE_SIDE].count;
        let tgt_sat = self.reach[TARGET_SIDE].count == self.assim[TARGET_SIDE].count;
        if src_sat && (!tgt_sat || self.assim[SOURCE_SIDE].count <= self.assim[TARGET_SIDE].count) {
            SOURCE_SIDE
        } else {
            TARGET_SIDE
        }
    }

    fn current_cut(&self) -> &[u32] {
        &self.front[self.current_cut_side()]
    }
    fn current_smaller_size(&self) -> u32 {
        self.assim[self.current_cut_side()].count
    }

    fn is_on_smaller_side(&self, x: u32) -> bool {
        self.assim[self.current_cut_side()].inside[x as usize]
    }

    #[inline]
    fn score_pierce(&self, y: u32, side: usize, causes_aug: bool) -> i64 {
        let src_dist = self.node_dist[side][y as usize];
        let tgt_dist = self.node_dist[1 - side][y as usize];
        let mut score = (tgt_dist as i64).saturating_sub(src_dist as i64);
        if causes_aug {
            score = score.saturating_sub(1_000_000_000);
        }
        score
    }

    fn select_pierce_node(&self, g: &OrigGraph, side: usize) -> Option<u32> {
        let mut best = i64::MIN;
        let mut chosen: Option<u32> = None;
        for &xy in &self.front[side] {
            let y = exp_head(g, xy);
            if self.assim[1 - side].inside[y as usize] {
                continue;
            }
            let causes_aug = self.reach[1 - side].inside[y as usize];
            let s = self.score_pierce(y, side, causes_aug);
            if s > best {
                best = s;
                chosen = Some(y);
            }
        }
        chosen
    }

    /// Returns false once no further cut is reachable.
    fn advance(&mut self, g: &OrigGraph) -> bool {
        debug_assert!(self.cut_available);
        let side = self.current_cut_side();
        if self.assim[side].count >= n_exp(g.n) / 2 {
            self.cut_available = false;
            return false;
        }
        let py = self.select_pierce_node(g, side);
        let pierce = match py {
            Some(y) => y,
            None => {
                self.cut_available = false;
                return false;
            }
        };
        self.assim[side].set_extra(pierce);
        self.reach[side].set_extra(pierce);
        self.grow_reachable_sets(g, side);
        self.grow_assimilated_sets(g);
        self.cut_available = true;
        true
    }

    /// Advances while the next step would leave the cut the size it is, which
    /// is what the reference implementation does by default: it skips the
    /// sides that are not the maximum.
    fn advance_while_cut_holds(&mut self, g: &OrigGraph) {
        debug_assert!(self.cut_available);
        let mut guard = 0u32;
        loop {
            let side = self.current_cut_side();
            if self.assim[side].count >= n_exp(g.n) / 2 {
                break;
            }
            let Some(pierce) = self.select_pierce_node(g, side) else {
                break;
            };
            if self.reach[1 - side].inside[pierce as usize] {
                break;
            }
            self.assim[side].set_extra(pierce);
            self.reach[side].set_extra(pierce);
            self.grow_reachable_sets(g, side);
            self.grow_assimilated_sets(g);
            self.cut_available = true;
            guard += 1;
            if guard > 1_000_000 {
                break;
            }
        }
    }
}

/// The Pareto commit rule over the one cutter the search runs.
///
/// Running a single cutter is deliberate: the C++ original starts several and
/// raises the count every 16 iterations, so what is given up here is the
/// best-of-several pick, not the rule below.
pub(super) struct Cutter {
    /// Kept between rounds so that a new round reuses these arrays instead of
    /// allocating a fresh set; [`Cutter::init`] resets what a search reads.
    inner: Option<BasicCutter>,
    /// Snapshot from the last commit, not a live read of `inner`: intervening
    /// `BasicCutter::advance` calls move that cutter's smaller-side count
    /// before the next Pareto comparison runs.
    current_smaller: u32,
}

impl Cutter {
    /// An empty shell. [`Cutter::init`] builds the inner cutter on first use
    /// and resets it afterwards, so one of these serves every iteration of a
    /// search rather than one iteration.
    pub(super) fn new() -> Self {
        Cutter {
            inner: None,
            current_smaller: 0,
        }
    }

    /// Start the cutter on the source-target pair `p`, reusing the cutter
    /// already held. Reuse is sound because `BasicCutter::init` is a complete
    /// reset and the graph, hence every array length, is the same.
    pub(super) fn init(&mut self, g: &OrigGraph, p: (u32, u32)) {
        let inner = self
            .inner
            .get_or_insert_with(|| BasicCutter::new(n_exp(g.n), a_exp(g.n, g.arc_count)));
        inner.init(g, p);
        inner.advance_while_cut_holds(g);
        self.current_smaller = inner.current_smaller_size();
    }

    fn inner(&self) -> &BasicCutter {
        self.inner.as_ref().expect("init precedes every read")
    }

    pub(super) fn current_cut_size(&self) -> usize {
        self.inner().current_cut().len()
    }
    pub(super) fn current_smaller_size(&self) -> u32 {
        self.current_smaller
    }
    pub(super) fn current_cut(&self) -> &[u32] {
        self.inner().current_cut()
    }
    pub(super) fn is_on_smaller_side(&self, x: u32) -> bool {
        self.inner().is_on_smaller_side(x)
    }

    pub(super) fn advance(&mut self, g: &OrigGraph) -> bool {
        if n_exp(g.n) / 2 == self.current_smaller {
            return false;
        }

        let inner = self.inner.as_mut().expect("init precedes every advance");
        loop {
            if !inner.cut_available || !inner.advance(g) {
                return false;
            }
            inner.advance_while_cut_holds(g);
            // FlowCutter's Pareto rule: only commit once the smaller side is
            // strictly larger too, not merely once the cut size has grown.
            let smaller = inner.current_smaller_size();
            if smaller > self.current_smaller {
                self.current_smaller = smaller;
                return true;
            }
        }
    }
}
