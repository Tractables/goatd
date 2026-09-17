// C FFI wrapper for meelgroup/treedecomp (in-process FlowCutter).
#include <cstddef>  // goatd: size_t — not guaranteed transitively on all stdlibs
#include <cstdint>  // goatd: int64_t
#include <memory>
#include <utility>
#include "ffi.h"
#include "TreeDecomposition.hpp"
#include "IFlowCutter.hpp"

struct TdResult {
    TWD::TreeDecomposition td;
    // Cache adjacency as vectors since TWD::Graph stores adj_list as protected
    std::vector<std::vector<int>> adj;
};

static void add_edges(TWD::Graph& graph, int num_edges, const int* edges) {
    for (int i = 0; i < num_edges; i++) {
        graph.addEdge(edges[2 * i], edges[2 * i + 1]);
    }
}

// Build the graph the backend imports from, and release it again.
//
// TWD::Graph holds one adjacency bitset per vertex — n * ceil(n/64) * 8 bytes,
// over a gigabyte at the Rust side's 100,000-vertex guard. importGraph copies
// the whole graph into its own arc arrays and keeps no reference to it, so
// scoping it here keeps that matrix off the peak the search runs at.
static std::unique_ptr<TWD::IFlowCutter> import_graph(int num_nodes, int num_edges,
                                                      const int* edges) {
    TWD::Graph g(num_nodes);
    add_edges(g, num_edges, edges);
    auto fc = std::make_unique<TWD::IFlowCutter>(num_nodes, num_edges, /*verb=*/0);
    fc->importGraph(g);
    return fc;
}

static TdResult* store_result(TWD::TreeDecomposition td) {
    auto result = std::make_unique<TdResult>();
    result->td = std::move(td);
    const int num_bags = result->td.numNodes();
    result->adj.resize(num_bags);
    for (int bag = 0; bag < num_bags; bag++) {
        result->adj[bag] = result->td.Neighbors(bag);
    }
    return result.release();
}

extern "C" {

TdResult* td_compute(int num_nodes, int num_edges,
                     const int* edges, int64_t steps, int iters,
                     int64_t* iters_done, int64_t* greedy_touches,
                     int64_t unit_budget, int64_t units_per_iter) {
    try {
        auto fc = import_graph(num_nodes, num_edges, edges);
        return store_result(fc->constructTD(steps, iters, iters_done,
                                            greedy_touches, unit_budget,
                                            units_per_iter));
    } catch (...) {
        return nullptr;
    }
}

TdResult* td_compute_timed_patience(int num_nodes, int num_edges,
                                    const int* edges, int64_t steps, int iters,
                                    int64_t timeout_ms, int64_t patience_ms,
                                    int64_t patience_unit_budget,
                                    int tight_gates, int64_t* iters_done,
                                    int64_t* greedy_touches,
                                    int64_t unit_budget, int64_t units_per_iter) {
    try {
        auto fc = import_graph(num_nodes, num_edges, edges);
        return store_result(fc->constructTD_timed_patience(
            steps, iters, timeout_ms, patience_ms, patience_unit_budget,
            tight_gates != 0,
            iters_done, greedy_touches, unit_budget, units_per_iter));
    } catch (...) {
        return nullptr;
    }
}

int td_num_bags(const TdResult* td) {
    return td->td.numNodes();
}

int td_bag_size(const TdResult* td, int bag_idx) {
    // Bags() is non-const in TWD, but we stored the result — cast away const
    auto& bags = const_cast<TWD::TreeDecomposition&>(td->td).Bags();
    return static_cast<int>(bags[bag_idx].size());
}

void td_bag_vertices(const TdResult* td, int bag_idx, int* out) {
    auto& bags = const_cast<TWD::TreeDecomposition&>(td->td).Bags();
    const auto& bag = bags[bag_idx];
    for (size_t i = 0; i < bag.size(); i++) {
        out[i] = bag[i];
    }
}

int td_bag_num_neighbors(const TdResult* td, int bag_idx) {
    return static_cast<int>(td->adj[bag_idx].size());
}

void td_bag_neighbors(const TdResult* td, int bag_idx, int* out) {
    const auto& nb = td->adj[bag_idx];
    for (size_t i = 0; i < nb.size(); i++) {
        out[i] = nb[i];
    }
}

void td_free(TdResult* td) {
    delete td;
}

void td_set_stop_flag(const unsigned char* flag) {
    TWD::g_external_stop = flag;
}

} // extern "C"
