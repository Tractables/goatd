//! Ownership, metering, and validation at the vendored C++ boundary.

use std::os::raw::c_int;

use super::{Budget, BudgetKind, TimeoutBehavior, duration_ms};
use crate::{Error, Graph, TdBag, TreeDecomposition};

// FFI safety invariants:
//
// `NativeDecomposition::compute` is the only constructor for a foreign handle
// and rejects null before wrapping it. `Drop` frees that handle exactly once.
// Edge and readback buffers remain live for each call, and each readback buffer
// uses the size reported immediately beforehand by the same handle.
mod ffi {
    use std::os::raw::c_int;

    #[repr(C)]
    pub(super) struct TdResult {
        _private: [u8; 0],
    }

    // SAFETY: this mirrors `vendor/treedecomp/ffi.h`, which is compiled into
    // this crate. Compute calls transfer ownership of their result; readback
    // calls borrow it; `td_free` releases it.
    unsafe extern "C" {
        pub(super) fn td_compute(
            num_nodes: c_int,
            num_edges: c_int,
            edges: *const c_int,
            steps: i64,
            iterations: c_int,
            iterations_done: *mut i64,
            greedy_touches: *mut i64,
            unit_budget: i64,
            units_per_iteration: i64,
        ) -> *mut TdResult;
        pub(super) fn td_compute_timed_patience(
            num_nodes: c_int,
            num_edges: c_int,
            edges: *const c_int,
            steps: i64,
            iterations: c_int,
            timeout_ms: i64,
            patience_ms: i64,
            adapt_search: c_int,
            iterations_done: *mut i64,
            greedy_touches: *mut i64,
            unit_budget: i64,
            units_per_iteration: i64,
        ) -> *mut TdResult;
        pub(super) fn td_num_bags(td: *const TdResult) -> c_int;
        pub(super) fn td_bag_size(td: *const TdResult, bag_index: c_int) -> c_int;
        pub(super) fn td_bag_vertices(td: *const TdResult, bag_index: c_int, out: *mut c_int);
        pub(super) fn td_bag_num_neighbors(td: *const TdResult, bag_index: c_int) -> c_int;
        pub(super) fn td_bag_neighbors(td: *const TdResult, bag_index: c_int, out: *mut c_int);
        pub(super) fn td_free(td: *mut TdResult);
    }
}

/// One owned result from the vendored backend.
struct NativeDecomposition(*mut ffi::TdResult);

impl Drop for NativeDecomposition {
    fn drop(&mut self) {
        // SAFETY: the constructor accepts only a live owned handle, and this is
        // its only release point.
        unsafe { ffi::td_free(self.0) };
    }
}

impl NativeDecomposition {
    /// Run the backend. `None` means it returned no decomposition or its share
    /// of a metered budget could not pay for setup.
    fn compute(num_vertices: u32, flat_edges: &[c_int], budget: Budget) -> Option<Self> {
        let num_edges = flat_edges.len() / 2;
        let vertices = u64::from(num_vertices);
        let elements = graph_elements(vertices, num_edges as u64);

        // A timed budget is measured on the work clock while the meter is
        // armed, so converting it at that clock's rate gives the native loop
        // the same share in work units. Zero means no work-unit limit.
        let unit_budget = match budget.kind {
            BudgetKind::Timed { timeout, .. } if crate::meter::is_armed() => {
                let milliseconds = timeout.as_millis().min(u64::MAX as u128) as u64;
                let units = milliseconds.saturating_mul(crate::meter::UNITS_PER_MS);
                i64::try_from(units).unwrap_or(i64::MAX)
            }
            _ => 0,
        };

        let setup_units = SETUP_UNITS_PER_ELEMENT.saturating_mul(elements);
        if unit_budget > 0 && setup_units >= unit_budget as u64 {
            crate::meter::charge(DECLINE_UNITS_PER_ELEMENT.saturating_mul(elements));
            return None;
        }

        let search_units = (unit_budget.max(0) as u64).saturating_sub(setup_units);
        let search_budget = i64::try_from(search_units).unwrap_or(i64::MAX);
        let units_per_iteration =
            i64::try_from(iteration_work_units(vertices, num_edges as u64)).unwrap_or(i64::MAX);

        let mut iterations_done = 0i64;
        let mut greedy_touches = 0i64;

        let raw = match budget.kind {
            BudgetKind::Timed {
                timeout,
                patience,
                iterations,
                steps,
                timeout_behavior,
            } => {
                let timeout_ms = duration_ms(timeout);
                let patience_ms = patience.map(duration_ms).unwrap_or(0);
                let iterations = i32::try_from(iterations)
                    .expect("public FlowCutter validation bounds iterations");
                let steps =
                    i64::try_from(steps).expect("public FlowCutter validation bounds steps");
                let steps = match timeout_behavior {
                    TimeoutBehavior::AdaptSearch => steps,
                    TimeoutBehavior::StopOnly => scaled_steps(steps, num_edges),
                };
                // SAFETY: all input and output buffers remain live for the
                // call; a non-null returned handle transfers ownership.
                unsafe {
                    ffi::td_compute_timed_patience(
                        num_vertices as c_int,
                        num_edges as c_int,
                        flat_edges.as_ptr(),
                        steps,
                        iterations,
                        timeout_ms,
                        patience_ms,
                        timeout_behavior.as_ffi(),
                        &mut iterations_done,
                        &mut greedy_touches,
                        search_budget,
                        units_per_iteration,
                    )
                }
            }
            BudgetKind::Steps { steps, iterations } => {
                let steps =
                    i64::try_from(steps).expect("public FlowCutter validation bounds steps");
                let iterations = i32::try_from(iterations)
                    .expect("public FlowCutter validation bounds iterations");
                // SAFETY: as for the timed call above.
                unsafe {
                    ffi::td_compute(
                        num_vertices as c_int,
                        num_edges as c_int,
                        flat_edges.as_ptr(),
                        scaled_steps(steps, num_edges),
                        iterations,
                        &mut iterations_done,
                        &mut greedy_touches,
                        search_budget,
                        units_per_iteration,
                    )
                }
            }
        };

        charge_build(vertices, num_edges as u64, iterations_done, greedy_touches);

        (!raw.is_null()).then_some(Self(raw))
    }
}

/// Run the native backend and validate everything copied across the boundary.
pub(super) fn run(graph: &Graph, budget: Budget) -> Result<TreeDecomposition, Error> {
    let mut flat_edges = Vec::with_capacity(graph.edges.len() * 2);
    for &(left, right) in &graph.edges {
        flat_edges.push(left as c_int);
        flat_edges.push(right as c_int);
    }

    let native = NativeDecomposition::compute(graph.num_vertices, &flat_edges, budget)
        .ok_or(Error::NoDecomposition)?;
    extract(&native, graph)
}

/// Copy a result without normalizing it first, then validate the copied value.
/// Keeping the backend's adjacency intact lets validation detect asymmetric,
/// repeated, or cyclic bag edges instead of accidentally repairing them.
fn extract(native: &NativeDecomposition, graph: &Graph) -> Result<TreeDecomposition, Error> {
    // SAFETY: `native` owns a live handle. Each probe-then-fill pair uses that
    // same immutable handle and a buffer of the reported size.
    unsafe {
        let num_bags = ffi::td_num_bags(native.0);
        if num_bags == 0 {
            return Err(Error::NoDecomposition);
        }
        if num_bags < 0 {
            return Err(Error::InvalidDecomposition(
                "FlowCutter returned a negative bag count".into(),
            ));
        }
        let num_bags = num_bags as usize;
        let mut bags = Vec::with_capacity(num_bags);
        let mut adjacency = Vec::with_capacity(num_bags);

        for bag_index in 0..num_bags {
            let bag_size = ffi::td_bag_size(native.0, bag_index as c_int);
            if bag_size < 0 || bag_size as u32 > graph.num_vertices {
                return Err(Error::InvalidDecomposition(format!(
                    "FlowCutter bag {bag_index} has invalid size {bag_size}"
                )));
            }
            let mut vertices = vec![0; bag_size as usize];
            ffi::td_bag_vertices(native.0, bag_index as c_int, vertices.as_mut_ptr());
            let vertices = vertices
                .into_iter()
                .map(|vertex| {
                    u32::try_from(vertex)
                        .ok()
                        .filter(|&vertex| vertex < graph.num_vertices)
                        .ok_or_else(|| {
                            Error::InvalidDecomposition(format!(
                                "FlowCutter bag {bag_index} contains invalid vertex {vertex}"
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            bags.push(TdBag::new(vertices));

            let neighbor_count = ffi::td_bag_num_neighbors(native.0, bag_index as c_int);
            if neighbor_count < 0 || neighbor_count as usize > num_bags {
                return Err(Error::InvalidDecomposition(format!(
                    "FlowCutter bag {bag_index} has invalid neighbor count {neighbor_count}"
                )));
            }
            let mut neighbors = vec![0; neighbor_count as usize];
            ffi::td_bag_neighbors(native.0, bag_index as c_int, neighbors.as_mut_ptr());
            let neighbors = neighbors
                .into_iter()
                .map(|neighbor| {
                    usize::try_from(neighbor)
                        .ok()
                        .filter(|&neighbor| neighbor < num_bags)
                        .ok_or_else(|| {
                            Error::InvalidDecomposition(format!(
                                "FlowCutter bag {bag_index} has invalid neighbor {neighbor}"
                            ))
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            adjacency.push(neighbors);
        }

        let decomposition = TreeDecomposition::from_parts(graph.num_vertices, bags, adjacency);
        decomposition.validate(graph)?;
        Ok(decomposition)
    }
}

/// Fitted setup charge per vertex or directed edge touched by the backend.
const SETUP_UNITS_PER_ELEMENT: u64 = 6_000;
/// Fitted charge per arc and square root of the vertex count for one restart.
const ITERATION_UNITS_PER_FLOW: u64 = 50;
/// Cost of constructing the edge buffer when a metered build is declined.
const DECLINE_UNITS_PER_ELEMENT: u64 = 20;

fn graph_elements(vertices: u64, edges: u64) -> u64 {
    vertices.saturating_add(edges.saturating_mul(2))
}

/// Work charged for one FlowCutter restart on this graph.
pub(crate) fn iteration_work_units(vertices: u64, edges: u64) -> u64 {
    ITERATION_UNITS_PER_FLOW
        .saturating_mul(edges.saturating_mul(2))
        .saturating_mul(vertices.isqrt())
}

fn charge_build(vertices: u64, edges: u64, iterations_done: i64, greedy_touches: i64) {
    if !crate::meter::is_armed() {
        return;
    }
    let setup = SETUP_UNITS_PER_ELEMENT.saturating_mul(graph_elements(vertices, edges));
    let search =
        (iterations_done.max(0) as u64).saturating_mul(iteration_work_units(vertices, edges));
    crate::meter::charge(
        setup
            .saturating_add(search)
            .saturating_add(greedy_touches.max(0) as u64),
    );
}

/// Clamp the backend's step ceiling to the amount small graphs can use.
fn scaled_steps(steps: i64, num_edges: usize) -> i64 {
    steps.min(10_000i64.max(50 * num_edges as i64))
}
