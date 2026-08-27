# Algorithms

goatd provides several tree-decomposition constructions and a portfolio that
combines them. This page describes the choices that differ from a textbook
implementation or from the upstream code.

## Portfolio

`five_slot_portfolio` shares one graph build and one preprocessing pass across
five elimination runs:

1. sampled min-degree;
2. nested dissection;
3. sampled min-fill;
4. sampled min-degree with a second seed;
5. nested dissection with a second seed.

The inexpensive order runs first so a valid candidate exists early. Later
runs receive the best width already found and stop when a bag is too wide to
win. With time left, the portfolio tries further sampled min-fill seeds. The
winner minimizes `(treewidth, total bag size)`. `single_slot_portfolio` exposes
the smaller variant used by callers that want to apply their own ranking.

A soft budget changes expensive greedy bookkeeping to a cheaper mode. A hard
budget completes the first candidate with a valid path decomposition of the
remaining graph; later candidates can stop without completing because a valid
candidate already exists. The library remains single-threaded throughout.

## Safe reductions

Every elimination construction starts with a fixed-point pass over five
reductions: isolated vertices, leaves, degree-two series vertices, simplicial
vertices, and almost-simplicial vertices. The last rule is applied only when
the vertex degree does not exceed the lower bound accumulated by earlier safe
eliminations. The emitted prefix bags can therefore be attached to a
decomposition of the residual graph without increasing its width.

The reduced graph, prefix bags, connected components, and initial fill counts
are computed once and reused by the portfolio.

## Min-fill and min-degree

Min-fill eliminates the vertex whose removal introduces the fewest missing
edges among its neighbors. Fill counts are maintained in a heap and only dirty
neighbors are rescored. Dense neighborhoods use bitsets; sparse neighborhoods
use a stamped marker array. Min-degree uses the same elimination and bag
construction path with a cheaper degree key.

The deterministic forms use a seeded per-vertex salt after the primary keys.
The sampled forms instead draw from the complete minimum-key tie set. A caller
may supply one weight per vertex; uniform weights give uniform sampling. This
keeps the usual min-fill or min-degree criterion while allowing the portfolio
to explore materially different orders.

These choices extend the standard greedy heuristics with shared reductions,
incremental scoring, explicit weighted ties, incumbent-width pruning, and
budget-aware completion.

## Nested dissection and multilevel bisection

Nested dissection is not a separate partitioning primitive. It repeatedly
calls goatd's multilevel graph bisector, turns the crossing edges of that
bisection into a vertex separator, recurses on both remaining sides, and
eliminates the separator last.

The bisector follows the usual coarsen, initial-partition, uncoarsen, and
refine sequence, including V-cycles. For a proposed bisection, goatd builds the
bipartite graph of crossing edges and computes a minimum vertex cover using
augmenting-path matching. By König's theorem this is the smallest separator
obtained by covering those crossing edges, and it is never larger than taking
all boundary vertices from the smaller side.

The same multilevel graph bisector is public on its own. A separate public
hypergraph bisector minimizes cut hyperedges, with FM and flow-based
refinement; it is not used to disguise a hypergraph as a graph.

## Flow-based separators

[FlowCutter](https://github.com/kit-algo/flow-cutter-pace17) explores the
tradeoff between cut size and balance by repeatedly advancing max-flow cuts.
goatd contains two related implementations:

- `flowcutter_rs` is a Rust separator search. It returns the separator and its
  two sides, and is used by decomposition refinement.
- `flowcutter` is the vendored PACE 2017 C++ tree-decomposition builder. It
  constructs a full decomposition and remains useful because its complete
  search is not the same operation as the Rust separator call.

The Rust separator search uses one cutter per restart instead of increasing a
multi-cutter batch, and uses goatd's seeded RNG. Refinement projects an existing
decomposition onto both sides of a separator, glues the projections at a new
separator bag, and accepts the replacement only when `(treewidth, total bag
size)` improves. Recursion applies the same monotone check.

The C++ builder carries several practical changes over the PACE source:

- memory guards for the dense adjacency matrix and bag-adjacency graph;
- 64-bit heap-position arithmetic;
- bounded greedy-order passes and work-unit metering;
- an early-convergence patience limit;
- a density gate for a shortcut order that is expensive on clique-dominated
  graphs.

The complete list of source changes and licences is in
[THIRD-PARTY.md](../THIRD-PARTY.md).

## Decomposition operations

`td_ops` provides the graph-only operations used by refinement: rooting a bag
forest, projecting a decomposition onto a vertex subset while preserving
global ids, and gluing two decompositions at a separator. These functions are
public so other construction and local-improvement algorithms can reuse the
same path.

## Correctness and reproducibility

`TreeDecomposition::validate` checks bag ids, the bag forest, vertex and edge
coverage, and the running intersection property. The test suite validates all
five elimination configurations on every undirected graph through five
vertices, tests the separator and FlowCutter routes on larger graph families,
and exercises malformed decompositions one invariant at a time.

Seeded, step-budgeted runs are reproducible. Wall-clock budgets are explicitly
different: they may stop after different amounts of work on different
machines. `meter::arm` replaces wall time with charged graph work when a caller
needs budget decisions to be repeatable.

The main algorithmic sources are the
[FlowCutter bisection paper](https://arxiv.org/abs/1504.03812), the
[PACE 2017 decomposition paper](https://arxiv.org/abs/1709.08949), and the
multilevel partitioning work credited in
[ACKNOWLEDGEMENTS.md](../ACKNOWLEDGEMENTS.md).
