/* goatd — Greatest Of All Tree Decompositions.
 *
 * Copyright the goatd authors. Licensed under the Apache License, Version 2.0.
 *
 * The ABI is unstable before 1.0: struct layouts, status codes and function
 * signatures can change in any release. Build against the goatd version you
 * ship with, and check goatd_version() at run time.
 */

#ifndef GOATD_H
#define GOATD_H

/* Generated from bindings/c/src/lib.rs by cbindgen. Do not edit by hand; CI
 * checks that this file still matches the source. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/**
 * The call succeeded.
 */
#define GOATD_OK 0

/**
 * The arguments broke a documented contract: a null pointer, a vertex id
 * outside the graph, or an option the chosen order cannot act on.
 */
#define GOATD_ERROR_INVALID_INPUT 1

/**
 * The decomposition handed to `goatd_validate` is not a tree decomposition
 * of the given graph.
 */
#define GOATD_ERROR_INVALID_DECOMPOSITION 2

/**
 * The graph exceeds a limit of the chosen construction.
 */
#define GOATD_ERROR_TOO_LARGE 3

/**
 * The FlowCutter backend returned nothing.
 */
#define GOATD_ERROR_NO_DECOMPOSITION 4

/**
 * goatd panicked. The panic did not cross into the caller, but the library
 * state behind it is no longer trustworthy; report it as a bug.
 */
#define GOATD_ERROR_PANIC 5

/**
 * An error this version of the bindings has no code for. The message says
 * what happened.
 */
#define GOATD_ERROR_OTHER 6

/**
 * Greedy min-fill elimination.
 */
#define GOATD_ORDER_MIN_FILL 0

/**
 * Greedy min-degree elimination.
 */
#define GOATD_ORDER_MIN_DEGREE 1

/**
 * Multilevel nested dissection.
 */
#define GOATD_ORDER_NESTED_DISSECTION 2

/**
 * The vendored FlowCutter solver.
 */
#define GOATD_ORDER_FLOWCUTTER 3

/**
 * Several orders under one budget, keeping the narrowest result. This is the
 * strongest setting; give it a `budget_ms`.
 */
#define GOATD_ORDER_PORTFOLIO 4

/**
 * How a decomposition is constructed. Start from `goatd_options_default` and
 * change what you need: a field the chosen order cannot act on is an error,
 * not a silently ignored value.
 */
typedef struct GoatdOptions {
  /**
   * One of the `GOATD_ORDER_` values.
   */
  uint32_t order;
  /**
   * Tie-breaking seed. One seed gives one decomposition. Not accepted by
   * `GOATD_ORDER_FLOWCUTTER`, which does not break ties this way.
   */
  uint64_t seed;
  /**
   * Milliseconds the construction may spend, or 0 for no limit. It is the
   * soft deadline of the elimination orders and of the portfolio,
   * FlowCutter's run time, and the refinement's deadline.
   */
  uint64_t budget_ms;
  /**
   * `GOATD_ORDER_FLOWCUTTER` only: a step budget in place of a clock, for a
   * run that repeats exactly. 0 leaves it unset. Give either this or
   * `budget_ms`, not both.
   */
  uint64_t steps;
  /**
   * `GOATD_ORDER_MIN_FILL` and `GOATD_ORDER_MIN_DEGREE` only: break ties by
   * weighted sampling from the whole tie set instead of by salt.
   */
  bool sample_ties;
  /**
   * With `sample_ties`, one weight per vertex; a smaller weight is
   * eliminated earlier. Null weighs every vertex the same.
   */
  const uint32_t *tie_weights;
  /**
   * Number of entries in `tie_weights`, which must be the graph's vertex
   * count.
   */
  size_t tie_weights_len;
  /**
   * Re-cut the decomposition along FlowCutter separators before returning
   * it. Accepted with every order.
   */
  bool refine;
} GoatdOptions;

/**
 * What a goatd call returned: `GOATD_OK`, or one of the `GOATD_ERROR_`
 * values. `goatd_last_error_message` describes the failure in words.
 */
typedef int32_t GoatdStatus;

/**
 * A tree decomposition, flattened into arrays.
 *
 * Bag `i` holds the vertices `bag_vertices[bag_offsets[i]]` up to but not
 * including `bag_vertices[bag_offsets[i + 1]]`, so `bag_offsets` has
 * `num_bags + 1` entries and its last entry is the length of `bag_vertices`.
 * `tree_edges` holds `2 * num_tree_edges` bag indices, one undirected edge
 * per pair.
 *
 * `goatd_decompose` fills the struct the caller supplies and takes ownership
 * of nothing; the three arrays inside belong to the caller and are released
 * together by `goatd_decomposition_free`.
 */
typedef struct GoatdDecomposition {
  /**
   * Vertices in the graph this decomposition was built for.
   */
  uint32_t num_vertices;
  /**
   * Number of bags.
   */
  size_t num_bags;
  /**
   * `num_bags + 1` offsets into `bag_vertices`.
   */
  const size_t *bag_offsets;
  /**
   * Bag contents, concatenated in bag order.
   */
  const uint32_t *bag_vertices;
  /**
   * Number of edges between bags.
   */
  size_t num_tree_edges;
  /**
   * `2 * num_tree_edges` bag indices.
   */
  const size_t *tree_edges;
  /**
   * Vertices in the largest bag, less one. An upper bound on the graph's
   * treewidth.
   */
  uint32_t treewidth;
} GoatdDecomposition;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * The version of these bindings, which is the version of the goatd release
 * they wrap. The string is static and outlives every other call.
 */
const char *goatd_version(void);

/**
 * Why the last call on this thread failed, as a NUL-terminated string, or
 * the empty string if it succeeded. Never null.
 *
 * The message belongs to goatd and is replaced by the next call on the same
 * thread; copy it if you need to keep it. Errors are recorded per thread, so
 * a message never crosses from one thread to another.
 */
const char *goatd_last_error_message(void);

/**
 * The defaults: min-fill, seed 0, no budget, no sampling, no refinement.
 */
struct GoatdOptions goatd_options_default(void);

/**
 * Decompose the graph on vertices `0..num_vertices` whose `num_edges`
 * undirected edges are the pairs in `edges`.
 *
 * On `GOATD_OK`, `*out` describes the decomposition and the caller releases
 * it with `goatd_decomposition_free`; on any other status `*out` is
 * untouched. `*out` is overwritten rather than merged, so free an earlier
 * result before reusing the storage.
 *
 * # Safety
 *
 * `edges` must point to `2 * num_edges` vertex ids, or be null when
 * `num_edges` is zero; `options` and `out` must each point to storage for one
 * value of their type; and `options->tie_weights`, when it is not null, must
 * point to `options->tie_weights_len` weights.
 */
GoatdStatus goatd_decompose(uint32_t num_vertices,
                            const uint32_t *edges,
                            size_t num_edges,
                            const struct GoatdOptions *options,
                            struct GoatdDecomposition *out);

/**
 * Release the arrays in a decomposition `goatd_decompose` produced and leave
 * the struct empty, so calling this twice is harmless. The struct itself
 * belongs to the caller.
 *
 * # Safety
 *
 * `decomposition` must be null or point to a value `goatd_decompose` filled
 * in and nothing has freed since.
 */
void goatd_decomposition_free(struct GoatdDecomposition *decomposition);

/**
 * Check a decomposition against its graph with goatd's own validator: bag
 * contents, an acyclic bag tree, vertex and edge coverage, and the running
 * intersection property.
 *
 * Returns `GOATD_OK` when it holds and `GOATD_ERROR_INVALID_DECOMPOSITION`
 * with a message naming the first violation when it does not. The
 * decomposition need not have come from `goatd_decompose`.
 *
 * # Safety
 *
 * `edges` must point to `2 * num_edges` vertex ids, or be null when
 * `num_edges` is zero, and `decomposition` must point to one
 * `GoatdDecomposition` whose arrays have the lengths its fields describe.
 */
GoatdStatus goatd_validate(uint32_t num_vertices,
                           const uint32_t *edges,
                           size_t num_edges,
                           const struct GoatdDecomposition *decomposition);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* GOATD_H */
