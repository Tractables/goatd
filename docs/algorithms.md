# Algorithms

goatd provides several tree-decomposition constructions and a portfolio that
combines them. This page describes the choices that differ from a textbook
implementation or from the upstream code.

## Portfolio

`portfolio::candidates` shares one graph build and one preprocessing pass
across five elimination runs:

1. sampled min-degree;
2. nested dissection;
3. sampled min-fill;
4. sampled min-degree with a second seed;
5. nested dissection with a second seed.

The inexpensive order runs first so a valid candidate exists early. Later
runs receive the best width already found and stop when a bag is too wide to
win. With time left, the portfolio tries further sampled min-fill seeds; on a
very large residual it uses sampled min-degree and skips the expensive fixed
orders instead. Initial fill counts are computed only when sampled min-fill
first runs, then reused across its seeds. The winner minimizes `(treewidth,
total bag size)`. `sampled_min_fill_candidates` exposes the smaller variant
used by callers that want to apply their own ranking.

A soft budget, measured from before preprocessing, stops the portfolio from
starting further candidates and samples. A hard budget completes the first
candidate with a valid path decomposition of the remaining graph; later
candidates can stop without completion because a valid candidate already
exists. The library remains single-threaded throughout.

## Preprocessing

Every elimination construction starts with the same deterministic reduction
pass:

| rule | condition | action |
| --- | --- | --- |
| islet | degree 0 | remove `v` and record `{v}` |
| twig | degree 1, with neighbour `u` | remove `v` and record `{v, u}` |
| series | degree 2, with non-adjacent neighbours `a` and `b` | add `a-b`, remove `v`, and record `{v, a, b}` |
| simplicial | the live neighbours of `v` form a clique | remove `v` and record `{v} ∪ N(v)` |
| almost simplicial | the live neighbours have exactly one missing edge, and `degree(v)` does not exceed the running width lower bound | add the missing edge, remove `v`, and record `{v} ∪ N(v)` |

Islet and twig elimination run to a fixed point first. This removes forest
components without introducing a width-2 series bag. The pass then scans the
series, simplicial, and almost-simplicial rules in that order and starts again
if any rule fired. Series and simplicial eliminations raise the running lower
bound used to admit the almost-simplicial rule.

The recorded bags form the beginning of the elimination sequence. A solver
continues on the residual graph, then the decomposition builder attaches these
prefix bags in reverse elimination order. The graph keeps its original vertex
ids; removed vertices are marked inactive rather than renumbered.

The portfolio performs this work once. Its elimination candidates share the
reduced graph and prefix, then compute their own order for the residual graph.

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

Partition refinement is part of the partitioner. It improves the temporary
0/1 bisection during uncoarsening and returns another bisection. Decomposition
refinement is under `decomposition`: it starts from complete bags and a bag
tree, then rewrites them around a separator supplied by FlowCutter.

## Flow-based separators

[FlowCutter](https://github.com/kit-algo/flow-cutter-pace17) explores the
tradeoff between cut size and balance by repeatedly advancing max-flow cuts.
goatd contains two related implementations:

- `flowcutter::separator::find` is a Rust separator search. It returns the
  separator and its two sides, and is used by decomposition refinement.
- `flowcutter::decompose` is the vendored PACE 2017 C++ tree-decomposition
  builder. It constructs a full decomposition and remains useful because its
  complete search is not the same operation as the Rust separator call.

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

`decomposition` contains the tree-decomposition type, validation, projection,
and FlowCutter-based refinement. Refinement preserves global vertex ids while
it projects each side and glues them at a separator.

Public constructors canonicalize each bag's contents and the undirected bag
edges, so equivalent caller inputs expose the same rooted walk. Native
algorithms may retain stable algorithm-defined vertex and neighbour order when
it carries useful traversal information; FlowCutter preserves both at its
adapter boundary.

## Correctness and reproducibility

`TreeDecomposition::validate` checks bag contents, the bag forest, vertex and edge
coverage, and the running intersection property.

Seeded, step-budgeted runs are reproducible. Machine speed and load can change
where a wall-clock budget stops. While a caller holds the guard returned by
`meter::arm`, duration budgets advance by charged graph work instead. Dropping
the guard restores wall-clock budgets.

The main algorithmic sources are the
[FlowCutter bisection paper](https://arxiv.org/abs/1504.03812), the
[PACE 2017 decomposition paper](https://arxiv.org/abs/1709.08949), and the
multilevel partitioning work credited in
[ACKNOWLEDGEMENTS.md](../ACKNOWLEDGEMENTS.md).
