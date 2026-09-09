//! The priority queue min-degree pops from: one bucket per degree, each
//! bucket an ordered set keyed on `(tie, vertex)`.
//!
//! Min-degree ranks by `(degree, tie, vertex)` ascending, and a vertex's
//! degree changes every time one of its neighbours is eliminated. A binary
//! heap cannot move an entry, so the only way to record a new degree there is
//! to push a second entry and let the older one be discarded when it
//! surfaces; on a dense residual that leaves tens of millions of superseded
//! entries and spends most of the run popping them. Degree is a small integer,
//! so a bucket queue can move a vertex in one insert and one removal and hold
//! exactly one entry per active vertex.
//!
//! The ordered set inside a bucket is what keeps the pop order total: the
//! sampling cores' `BucketMap` holds each bucket as an unordered `Vec`,
//! because they want the whole tie set to draw from, and it cannot answer
//! "the smallest `(tie, vertex)` at this degree".

use std::collections::BTreeSet;

/// No bucket — the vertex is not in the queue.
const ABSENT: u32 = u32::MAX;

pub(super) struct DegreeQueue {
    /// `buckets[d]` holds `(tie, vertex)` for every queued vertex of degree
    /// `d`. Grown on demand to the largest degree ever queued.
    buckets: Vec<BTreeSet<(u64, u32)>>,
    /// The bucket each vertex sits in, or [`ABSENT`].
    degree: Vec<u32>,
    /// The tie key each queued vertex was filed under.
    tie: Vec<u64>,
    /// A lower bound on the smallest non-empty bucket. Exact after a pop.
    cursor: usize,
}

impl DegreeQueue {
    pub(super) fn new(vertices: usize) -> Self {
        DegreeQueue {
            buckets: Vec::new(),
            degree: vec![ABSENT; vertices],
            tie: vec![0; vertices],
            cursor: 0,
        }
    }

    /// File `v` under `degree` with tie key `tie`, moving it if it is already
    /// queued. One removal and one insert, both `O(log b)` in the bucket's
    /// size; nothing superseded is left behind.
    pub(super) fn set(&mut self, v: u32, degree: u64, tie: u64) {
        self.remove(v);
        let bucket = usize::try_from(degree).expect("degree fits a usize");
        if bucket >= self.buckets.len() {
            self.buckets.resize_with(bucket + 1, BTreeSet::new);
        }
        self.buckets[bucket].insert((tie, v));
        self.degree[v as usize] = u32::try_from(bucket).expect("degree fits a u32");
        self.tie[v as usize] = tie;
        self.cursor = self.cursor.min(bucket);
    }

    /// The degree `v` is filed under, or `None` if it is not queued.
    pub(super) fn degree_of(&self, v: u32) -> Option<u64> {
        let bucket = self.degree[v as usize];
        (bucket != ABSENT).then_some(u64::from(bucket))
    }

    /// Drop `v` from the queue if it is in it.
    pub(super) fn remove(&mut self, v: u32) {
        let bucket = self.degree[v as usize];
        if bucket == ABSENT {
            return;
        }
        self.buckets[bucket as usize].remove(&(self.tie[v as usize], v));
        self.degree[v as usize] = ABSENT;
    }

    /// The smallest `(degree, tie, vertex)` in the queue, removed.
    pub(super) fn pop_min(&mut self) -> Option<(u64, u64, u32)> {
        while self.cursor < self.buckets.len() && self.buckets[self.cursor].is_empty() {
            self.cursor += 1;
        }
        let bucket = self.buckets.get_mut(self.cursor)?;
        let (tie, v) = bucket.pop_first()?;
        self.degree[v as usize] = ABSENT;
        Some((self.cursor as u64, tie, v))
    }
}
