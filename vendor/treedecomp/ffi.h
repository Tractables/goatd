// C FFI wrapper for meelgroup/treedecomp (in-process FlowCutter).
#ifndef TREEDECOMP_FFI_H
#define TREEDECOMP_FFI_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

// Opaque handle to a tree decomposition result.
typedef struct TdResult TdResult;

// Run FlowCutter on a graph with `num_nodes` nodes.
// `edges` is a flat array of [u0,v0, u1,v1, ...] (0-indexed), length = 2*num_edges.
// `steps` controls the computation budget (higher = more time, better quality).
// `iters` controls the number of FlowCutter iterations.
// Returns a TdResult handle (caller must free with td_free), or NULL when the
// backend fails.
//
// The last four arguments meter the construction. `iters_done` (may be NULL)
// receives the restart iterations the loop actually consumed and
// `greedy_touches` (may be NULL) the graph elements the greedy pre-passes swept,
// so the caller charges measured work rather than a modelled estimate of it.
// `unit_budget` bounds the whole construction in the unit `greedy_touches` is
// counted in, and `units_per_iter` is what one restart iteration costs in that
// unit; `unit_budget = 0` arms no budget.
TdResult* td_compute(int num_nodes, int num_edges,
                     const int* edges, int64_t steps, int iters,
                     int64_t* iters_done, int64_t* greedy_touches,
                     int64_t unit_budget, int64_t units_per_iter);

// Run with a wall-clock timeout and optional early convergence detection.
// Stops early if the treewidth hasn't improved for `patience_ms` milliseconds,
// or `patience_unit_budget` work units while metered. Zero means no early
// stopping for that clock.
//
// `tight_gates` selects shortened pre-loop heuristics for a timeout that is
// expected to stop the search. Zero keeps the untimed heuristic limits and
// uses the timeout only as a stopping condition.
//
// The last four arguments mean what they mean for td_compute. A nonzero
// A nonzero `unit_budget` stands the wall deadline down, so that work alone
// decides where the search stops.
TdResult* td_compute_timed_patience(int num_nodes, int num_edges,
                                    const int* edges, int64_t steps, int iters,
                                    int64_t timeout_ms, int64_t patience_ms,
                                    int64_t patience_unit_budget,
                                    int tight_gates, int64_t* iters_done,
                                    int64_t* greedy_touches,
                                    int64_t unit_budget, int64_t units_per_iter);

// Get the number of bags in the tree decomposition.
int td_num_bags(const TdResult* td);

// Get the size of bag `bag_idx`.
int td_bag_size(const TdResult* td, int bag_idx);

// Copy the vertices of bag `bag_idx` into `out` (must have room for td_bag_size elements).
// Vertices are 0-indexed.
void td_bag_vertices(const TdResult* td, int bag_idx, int* out);

// Get the number of neighbors of bag `bag_idx` in the TD tree.
int td_bag_num_neighbors(const TdResult* td, int bag_idx);

// Copy the neighbor bag indices of bag `bag_idx` into `out`.
void td_bag_neighbors(const TdResult* td, int bag_idx, int* out);

// Free a TdResult.
void td_free(TdResult* td);

// Self-test of the vendored FlowCutter k-way id-heap: push/pop max-ordering,
// contains/get_key, and — the bug #18 regression — that the child-index
// arithmetic (k*pos+1) does not overflow signed int32 at large heap positions.
// Returns 0 on success, or a nonzero check id identifying the first failing
// assertion (see heap_selftest.cpp). Runs in milliseconds and allocates only
// modest memory.
int treedecomp_heap_selftest(void);

#ifdef __cplusplus
}
#endif

#endif
