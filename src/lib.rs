//! goatd — Greatest Of All Tree Decompositions: tree decompositions of graphs.
//!
//! A graph goes in as an edge list ([`Graph`]) and a [`TreeDecomposition`]
//! comes out, by one of three routes:
//!
//! * [`elimination`] — greedy elimination orders (min-fill, min-degree, nested
//!   dissection, and the two sampled variants that break ties by a per-vertex
//!   weight), each run after a safe-reduction preprocessing pass; a schedule
//!   that runs several under a time budget and keeps the narrowest; and a
//!   refinement pass that re-cuts a decomposition along FlowCutter separators.
//! * [`flowcutter`] — the FlowCutter treewidth solver (PACE 2017), vendored in
//!   C++ and driven in process, under a wall-clock or a step budget.
//! * [`flowcutter_rs`] — one balanced vertex separator from a pure-Rust port of
//!   FlowCutter's separator search.
//!
//! Beside them: [`multilevel_bisect`] and [`multilevel_hg_bisect`], the
//! multilevel graph and hypergraph bisectors the orders above are built on;
//! [`td_ops`], surgery on a decomposition — rooting, projecting onto a vertex
//! subset, gluing two at a separator; and PACE `.gr`/`.td` reading and writing
//! on the two types ([`Graph::from_gr`], [`TreeDecomposition::to_td`]).
//!
//! Every decomposition returned covers each vertex and each edge and has the
//! running intersection property. Nothing here spawns a thread or reads the
//! process environment; a search that reads a clock says so in its signature,
//! and the clock it reads is [`meter::now`] — arm the work meter ([`meter`])
//! and every budget in the crate is spent in graph work instead of wall time,
//! so the same budget picks the same decomposition on every machine. The
//! `goatd` binary runs the same routes over PACE files.
//!
//! ```
//! use goatd::Graph;
//! use goatd::elimination::{Config, elimination_td};
//!
//! // The 4-cycle with one chord: treewidth 2.
//! let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)]);
//! let td = elimination_td(&graph, Config::MinFill, 0, None);
//! assert_eq!(td.treewidth(), 2);
//!
//! let text = td.to_td(graph.num_vertices);
//! assert!(text.starts_with("s td "));
//! ```

mod bisect_seed;
mod deadline;
mod error;
mod fm_common;
mod graph;
mod pace;
mod rng;
mod td;

pub mod elimination;
pub mod flowcutter;
pub mod flowcutter_rs;
pub mod meter;
pub mod multilevel_bisect;
pub mod multilevel_hg_bisect;
pub mod td_ops;

pub use error::Error;
pub use graph::{Graph, local_index, restrict_to_subset};
pub use rng::Xorshift64;
pub use td::{TdBag, TreeDecomposition};

#[cfg(test)]
mod tests;
