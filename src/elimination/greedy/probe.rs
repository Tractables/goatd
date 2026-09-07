//! Where a greedy elimination spends its wall time, for experiments.
//!
//! Off unless the environment variable `GOATD_ELIM_PROBE` names a file to
//! append to; when it is unset every call below is one branch on a `None`.
//! Reading the process environment is not something shipped library code
//! does, so this file is a measurement tool and not part of a release.
//!
//! One line per second of wall carries the state of the residual and the wall
//! time split over the phases of the loop, cumulative and since the previous
//! line, so the cost of one elimination and which phase it sits in can be read
//! per interval.

use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use super::{ElimExit, EliminationGraph};

/// Wall time between lines.
const STRIDE: Duration = Duration::from_secs(1);

/// Serial number per elimination run in the process, so the lines of the
/// portfolio's several candidates can be told apart in one file.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// The one place the environment is read.
fn probe_path() -> Option<&'static OsString> {
    static PATH: OnceLock<Option<OsString>> = OnceLock::new();
    PATH.get_or_init(|| std::env::var_os("GOATD_ELIM_PROBE"))
        .as_ref()
}

/// A phase of the elimination loop.
#[derive(Clone, Copy)]
pub(super) enum Phase {
    /// Heap pop, the stale-snapshot re-score and its re-push.
    Heap,
    /// `take_bag`: collecting the live neighbours and copying the bag.
    Bag,
    /// The elimination itself, fill edges included.
    Elim,
    /// Handing the bag to the sink.
    Sink,
    /// The core's reaction to the elimination.
    After,
}

const PHASES: usize = 5;

pub(super) struct ElimProbe {
    out: Option<std::fs::File>,
    run: u64,
    start: Instant,
    next: Duration,
    elims: u64,
    pops: u64,
    discards: u64,
    pushes: u64,
    ns: [u64; PHASES],
    last_at: Duration,
    last_elims: u64,
    last_pops: u64,
    last_discards: u64,
    last_pushes: u64,
    last_ns: [u64; PHASES],
}

impl ElimProbe {
    /// Open the probe if the environment names a file, and write the header
    /// line for this run. `kind` names the core, `n` and `edges` describe the
    /// graph it starts from.
    pub(super) fn start(kind: &str, n: usize, edges: usize) -> Self {
        let out = probe_path()
            .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
        let run = RUNS.fetch_add(1, Ordering::Relaxed);
        let mut probe = ElimProbe {
            out,
            run,
            start: Instant::now(),
            next: STRIDE,
            elims: 0,
            pops: 0,
            discards: 0,
            pushes: 0,
            ns: [0; PHASES],
            last_at: Duration::ZERO,
            last_elims: 0,
            last_pops: 0,
            last_discards: 0,
            last_pushes: 0,
            last_ns: [0; PHASES],
        };
        if let Some(file) = probe.out.as_mut() {
            let _ = writeln!(
                file,
                "probe-run run={run} core={kind} vertices={n} edges={edges}"
            );
        }
        probe
    }

    /// Whether the probe is writing anything.
    #[inline]
    pub(super) fn on(&self) -> bool {
        self.out.is_some()
    }

    /// The clock, if the probe is on.
    #[inline]
    pub(super) fn mark(&self) -> Option<Instant> {
        self.out.as_ref().map(|_| Instant::now())
    }

    /// Count one entry taken off the heap, and whether it was discarded
    /// without eliminating anything — an entry for an already-eliminated
    /// vertex, one the core has replaced, or one whose score had moved and is
    /// pushed back.
    #[inline]
    pub(super) fn popped(&mut self, discarded: bool) {
        if self.out.is_some() {
            self.pops += 1;
            self.discards += u64::from(discarded);
        }
    }

    /// Count entries pushed onto the heap.
    #[inline]
    pub(super) fn pushed(&mut self, entries: u64) {
        if self.out.is_some() {
            self.pushes += entries;
        }
    }

    /// Add the time since `mark` to `phase`.
    #[inline]
    pub(super) fn charge(&mut self, phase: Phase, mark: Option<Instant>) {
        if let Some(mark) = mark {
            self.ns[phase as usize] += mark.elapsed().as_nanos() as u64;
        }
    }

    /// Note the end of the initial scoring pass, which on a large graph is a
    /// visible share of the budget before the first elimination.
    pub(super) fn seeded(&mut self, graph: &EliminationGraph, cheap_mode: bool) {
        let at = self.start.elapsed();
        let (run, active, edges) = (self.run, graph.num_active, graph.num_edges);
        if let Some(file) = self.out.as_mut() {
            let _ = writeln!(
                file,
                "probe-seed run={run} ms={ms} active={active} edges={edges} cheap={cheap}",
                ms = at.as_millis(),
                cheap = u8::from(cheap_mode),
            );
            let _ = file.flush();
        }
        self.next = at + STRIDE;
        self.last_at = at;
    }

    /// Count one elimination and, once a second, write the state of the
    /// residual. `nbrs` are the live neighbours the elimination was given.
    pub(super) fn eliminated(
        &mut self,
        graph: &EliminationGraph,
        nbrs: &[u32],
        bag_len: usize,
        cheap_mode: bool,
        heap_len: usize,
    ) {
        if self.out.is_none() {
            return;
        }
        self.elims += 1;
        let at = self.start.elapsed();
        if at < self.next {
            return;
        }
        self.next = at + STRIDE;

        // Only sampled eliminations pay for these: they walk the neighbour
        // list again.
        let degree = nbrs.len();
        let sigma: u64 = nbrs.iter().map(|&u| graph.degree(u) as u64).sum();
        let indexed = nbrs.iter().filter(|&&u| graph.row_is_indexed(u)).count();

        let active = graph.num_active as u64;
        let edges = graph.num_edges as u64;
        // The gate the existing bitset promotion uses, read on the residual:
        // the bitset's k words per neighbour beats the row path's k times the
        // average degree once 128 * edges passes active squared.
        let gate = u128::from(edges) * 128 > u128::from(active) * u128::from(active);
        // What a bitset re-indexed over the residual would cost in memory.
        let words = (graph.num_active.div_ceil(64)) as u64;
        let bitset_mib = active * words * 8 / (1024 * 1024);

        let interval_ms = (at - self.last_at).as_secs_f64() * 1e3;
        let interval_elims = self.elims - self.last_elims;
        let interval_pops = self.pops - self.last_pops;
        let interval_discards = self.discards - self.last_discards;
        let interval_pushes = self.pushes - self.last_pushes;
        let per_elim_us = if interval_elims > 0 {
            interval_ms * 1e3 / interval_elims as f64
        } else {
            0.0
        };

        let cumulative = self.ns.map(|ns| ns / 1_000_000);
        let mut interval = [0u64; PHASES];
        for ((slot, ns), last) in interval.iter_mut().zip(self.ns).zip(self.last_ns) {
            *slot = (ns - last) / 1_000_000;
        }

        if let Some(file) = self.out.as_mut() {
            let _ = writeln!(
                file,
                "probe run={run} ms={ms} elims={elims} d_ms={interval_ms:.0} d_elims={interval_elims} \
us_per_elim={per_elim_us:.1} active={active} edges={edges} degree={degree} bag={bag_len} \
sigma={sigma} indexed={indexed} cheap={cheap} gate={gate} bitset_mib={bitset_mib} \
heap_len={heap_len} d_pops={interval_pops} d_discards={interval_discards} d_pushes={interval_pushes} \
ms_heap={c0} ms_bag={c1} ms_elim={c2} ms_sink={c3} ms_after={c4} \
d_heap={i0} d_bag={i1} d_elim={i2} d_sink={i3} d_after={i4}",
                run = self.run,
                ms = at.as_millis(),
                elims = self.elims,
                cheap = u8::from(cheap_mode),
                gate = u8::from(gate),
                c0 = cumulative[0],
                c1 = cumulative[1],
                c2 = cumulative[2],
                c3 = cumulative[3],
                c4 = cumulative[4],
                i0 = interval[0],
                i1 = interval[1],
                i2 = interval[2],
                i3 = interval[3],
                i4 = interval[4],
            );
            let _ = file.flush();
        }

        self.last_at = at;
        self.last_elims = self.elims;
        self.last_pops = self.pops;
        self.last_discards = self.discards;
        self.last_pushes = self.pushes;
        self.last_ns = self.ns;
    }

    /// Write the closing line of the run: how it ended and what was left.
    pub(super) fn finished(&mut self, graph: &EliminationGraph, exit: ElimExit) {
        let at = self.start.elapsed();
        let (elims, run) = (self.elims, self.run);
        let (pops, discards, pushes) = (self.pops, self.discards, self.pushes);
        let active = graph.num_active;
        let edges = graph.num_edges;
        let ns = self.ns;
        if let Some(file) = self.out.as_mut() {
            let _ = writeln!(
                file,
                "probe-end run={run} exit={exit:?} ms={ms} elims={elims} pops={pops} \
discards={discards} pushes={pushes} active={active} edges={edges} \
ms_heap={h} ms_bag={b} ms_elim={e} ms_sink={s} ms_after={a}",
                ms = at.as_millis(),
                h = ns[0] / 1_000_000,
                b = ns[1] / 1_000_000,
                e = ns[2] / 1_000_000,
                s = ns[3] / 1_000_000,
                a = ns[4] / 1_000_000,
            );
            let _ = file.flush();
        }
    }
}
