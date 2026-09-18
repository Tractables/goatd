//! goatd — Greatest Of All Tree Decompositions: tree decompositions of graphs.
//!
//! A [`Graph`] goes in and a [`TreeDecomposition`] comes out.
//!
//! [`elimination`] builds a decomposition from an elimination order;
//! [`elimination::Order`] is the list, min-fill, min-degree and nested
//! dissection among them. [`embedding`] places the vertices in space and ranks
//! them by how peripheral they are, which is one source of the sampling
//! weights those orders break ties with. [`flowcutter`] provides the vendored
//! FlowCutter decomposer and a Rust separator search. [`portfolio`] combines
//! constructions, and [`decomposition`] contains the result type and its
//! separator-based refinement. [`partition`] holds the multilevel bisectors:
//! nested dissection runs on the graph one, and there is a hypergraph one
//! beside it that nothing in the crate calls.
//!
//! [`Graph::from_gr`] and [`TreeDecomposition::to_td`] handle the PACE formats.
//! [`TreeDecomposition::validate`] checks a result against its graph. Beside
//! the width, [`TreeDecomposition::bag_mass`] is what a consumer compiling over
//! the bags pays and [`TreeDecomposition::max_separator`] what it carries
//! between two of them. The library is single-threaded. [`meter::arm`] makes
//! duration budgets advance by charged graph work instead of wall time when
//! repeatable stopping points are needed. [`stop_flag`] ends a running solve
//! early and returns the best decomposition found so far; it is one flag for
//! the whole process.
//!
//! ```
//! use std::time::Duration;
//!
//! use goatd::Graph;
//! use goatd::portfolio::decompose_standard;
//!
//! // The 4-cycle with one chord: treewidth 2.
//! let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)]);
//! let td = decompose_standard(&graph, 0, Some(Duration::from_millis(100)))?;
//! assert_eq!(td.treewidth(), 2);
//!
//! let text = td.to_td();
//! assert!(text.starts_with("s td "));
//! # Ok::<(), goatd::Error>(())
//! ```

#![deny(missing_docs)]

mod adjacency;
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
mod prefetch;
mod rng;
mod stop;

pub use decomposition::{TdBag, TreeDecomposition};
pub use error::Error;
pub use graph::Graph;
pub use stop::stop_flag;

#[cfg(test)]
mod tests;
