//! Test-side entry point into the search shell.
//!
//! These tests drive single `(config, seed)` pairs straight from raw edges.
//! Production never does — it always goes through `prebuild` +
//! `run_config_prebuilt`, which amortize graph construction and preprocessing
//! across a whole portfolio — so the raw-edges entry point lives here, next to
//! the module's private internals it needs.

use crate::elimination::graph::Graph;
use crate::elimination::minfill_core::ElimStop;
use crate::elimination::preprocess::preprocess;
use crate::elimination::width_opt::*;

/// Run a single `(config, seed)` pair from raw edges. Builds the graph and
/// runs preprocessing.
pub(super) fn run_config(
    num_vertices: u32,
    edges: &[(u32, u32)],
    config: Config<'_>,
    seed: u64,
) -> ConfigRun {
    let graph = Graph::from_edges(num_vertices, edges);
    let reduced = preprocess(graph);
    run_config_on_reduced(
        reduced,
        false,
        None,
        RunSpec {
            config,
            seed,
            stop: ElimStop::default(),
            force_emit: false,
        },
    )
}
