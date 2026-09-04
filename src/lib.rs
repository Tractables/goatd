//! goatd — Greatest Of All Tree Decompositions: tree decompositions of graphs.
//!
//! A [`Graph`] goes in and a [`TreeDecomposition`] comes out.
//!
//! [`elimination`] provides min-fill, min-degree, and nested-dissection orders.
//! [`embedding`] places the vertices in space and ranks them by how peripheral
//! they are, which is one source of the sampling weights those orders break
//! ties with. [`flowcutter`] provides the vendored FlowCutter decomposer and a Rust
//! separator search. [`portfolio`] combines constructions, and
//! [`decomposition`] contains the result type and its separator-based
//! refinement. [`partition`] exposes
//! the multilevel graph and hypergraph bisectors used by those constructions.
//!
//! [`Graph::from_gr`] and [`TreeDecomposition::to_td`] handle the PACE formats.
//! [`TreeDecomposition::validate`] checks a result against its graph. The
//! library is single-threaded. [`meter::arm`] makes duration budgets advance by
//! charged graph work instead of wall time when repeatable stopping points are
//! needed. [`stop_flag`] ends a running solve early and returns the best
//! decomposition found so far.
//!
//! ```
//! use goatd::Graph;
//! use goatd::elimination::{Order, decompose};
//!
//! // The 4-cycle with one chord: treewidth 2.
//! let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)]);
//! let td = decompose(&graph, Order::MinFill, 0, None)?;
//! assert_eq!(td.treewidth(), 2);
//!
//! let text = td.to_td();
//! assert!(text.starts_with("s td "));
//! # Ok::<(), goatd::Error>(())
//! ```

#![deny(missing_docs)]

mod deadline;
pub mod decomposition;
pub mod elimination;
pub mod embedding;
mod error;
pub mod flowcutter;
mod graph;
pub mod meter;
mod pace;
pub mod partition;
pub mod portfolio;
mod rng;
mod stop;

pub use decomposition::{TdBag, TreeDecomposition};
pub use error::Error;
pub use graph::Graph;
pub use stop::stop_flag;

#[cfg(test)]
mod tests;
