//! The static graph the separator search runs on: an undirected edge list
//! turned into paired directed arcs, with CSR out-arc ranges per vertex and,
//! for each arc, the id of its reverse.
//!
//! Built once per separator computation and read-only after that — the flow
//! lives in the cutter, and the vertex-split graph the search actually
//! traverses is derived from these ids arithmetically in `expanded`.
//!
//! Arcs are sorted by `(tail, head)` and every arc has its reverse present:
//! `back_arc` is filled by binary search within that ordering, and the arc
//! numbering is the index space `expanded` maps onto. Neither survives a
//! reordering of the arc arrays.

pub(super) struct OrigGraph {
    pub(super) n: u32,
    /// Directed arc tails (length 2*|E_undirected|), sorted by (tail, head).
    pub(super) tail: Vec<u32>,
    /// Directed arc heads (length 2*|E_undirected|), parallel to `tail`.
    pub(super) head: Vec<u32>,
    /// `back_arc[a]` is the arc id of (head[a], tail[a]).
    pub(super) back_arc: Vec<u32>,
    /// CSR offsets: `out_arc_offsets[v]..out_arc_offsets[v + 1]` is the arc-id
    /// range leaving `v`.
    out_arc_offsets: Vec<u32>,
}

impl OrigGraph {
    pub(super) fn build(n: u32, edges: &[(u32, u32)]) -> Option<Self> {
        let mut arcs: Vec<(u32, u32)> = Vec::with_capacity(edges.len() * 2);
        for &(u, v) in edges {
            debug_assert!(u < n && v < n && u != v);
            arcs.push((u, v));
            arcs.push((v, u));
        }
        if arcs.is_empty() {
            return None;
        }

        arcs.sort_unstable();
        arcs.dedup();

        let arc_count = arcs.len();
        let mut tail = Vec::with_capacity(arc_count);
        let mut head = Vec::with_capacity(arc_count);
        for (u, v) in arcs {
            tail.push(u);
            head.push(v);
        }

        // Depends on `tail` already sorted by (tail, head) from the sort above.
        let mut out_arc_offsets = vec![0u32; (n + 1) as usize];
        for &t in &tail {
            out_arc_offsets[(t + 1) as usize] += 1;
        }
        for v in 1..=n as usize {
            out_arc_offsets[v] += out_arc_offsets[v - 1];
        }

        let mut back_arc = vec![0u32; arc_count];
        for i in 0..arc_count {
            let u = tail[i];
            let v = head[i];
            let lo = out_arc_offsets[v as usize] as usize;
            let hi = out_arc_offsets[(v + 1) as usize] as usize;
            let slice = &head[lo..hi];
            let pos = slice.binary_search(&u).expect("symmetric graph");
            back_arc[i] = (lo + pos) as u32;
        }

        Some(OrigGraph {
            n,
            tail,
            head,
            back_arc,
            out_arc_offsets,
        })
    }

    #[inline]
    pub(super) fn out_arcs(&self, v: u32) -> std::ops::Range<u32> {
        self.out_arc_offsets[v as usize]..self.out_arc_offsets[v as usize + 1]
    }
}

pub(super) fn is_connected(g: &OrigGraph) -> bool {
    if g.n == 0 {
        return false;
    }
    let mut seen = vec![false; g.n as usize];
    let mut stack = vec![0u32];
    seen[0] = true;
    let mut count = 1u32;
    while let Some(v) = stack.pop() {
        for a in g.out_arcs(v) {
            let h = g.head[a as usize];
            if !seen[h as usize] {
                seen[h as usize] = true;
                count += 1;
                stack.push(h);
            }
        }
    }
    count == g.n
}
