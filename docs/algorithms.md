# Algorithms

goatd builds a tree decomposition three ways: by eliminating vertices in a
chosen order, by nested dissection over a multilevel bisector, and by
FlowCutter's flow-based separators. The portfolio runs them under one budget,
keeps the narrowest result, and spends what is left of the budget improving
it. This page says what each construction does and where it departs from the
published method. The API is on [docs.rs](https://docs.rs/goatd).

## Preprocessing

Every construction starts from the same reduction, run once and shared. Each
rule removes a vertex and records the bag it will occupy:

| rule | condition | bag |
| --- | --- | --- |
| islet | degree 0 | `{v}` |
| twig | degree 1, neighbour `u` | `{v, u}` |
| series | degree 2, neighbours `a` and `b` not adjacent | `{v, a, b}`, and the edge `a-b` is added |
| simplicial | the neighbours form a clique | `{v} ∪ N(v)` |
| almost simplicial | one edge missing among the neighbours, and `degree(v)` within the width lower bound so far | `{v} ∪ N(v)`, and the missing edge is added |

Islets and twigs are cleared first, which removes forest components whole.
The other rules run in the order listed and start again whenever one fires.
The recorded bags become the front of the elimination sequence, the
construction continues on the residual graph, and vertex ids never change.

## Elimination orders

An elimination order builds a decomposition one vertex at a time: the vertex
leaves a bag of itself and its remaining neighbours, which are joined into a
clique. Each order is a rule for which vertex goes next.

- **Min-fill** takes the vertex whose elimination adds the fewest edges. Fill
  counts live in a heap and only the neighbours of an eliminated vertex are
  rescored.
- **Min-degree** takes the vertex of smallest degree, on the same machinery.
- **Relative fill** takes the smallest fill per unit of degree, then the
  smallest degree.
- **Maximum cardinality search** numbers the vertices from the last down to
  the first, always taking the one with the most numbered neighbours, and
  eliminates along the numbering; on a chordal graph that adds no fill.
  **MCS-M** is the same search with a longer reach, and eliminating along it
  gives a minimal triangulation: no added edge can be removed without
  breaking chordality. Neither reads a seed, so each runs once.

The scores are exact; the orders differ in how they break ties. A
*deterministic* order breaks a tie by a seeded per-vertex salt. A *sampled*
order draws from the tied set with one caller-supplied weight per vertex,
where equal weights sample uniformly and a lower weight is likelier. The
portfolio's restarts widen the tied set into a band of scores near the
minimum, so that seeds still separate on a graph where one vertex holds the
minimum at every step.

## Vertex coordinates

`embedding::Embedding` places the vertices in a few dimensions by repeated
averaging over neighbours, whitening the cloud after every round. The rounds
are then subspace iteration on the lazy random walk: the axes settle on the
walk's slowest modes and the first approximates a Fiedler vector. Distance
from the centre says how peripheral a vertex is, and
`Embedding::rank_weights` turns that ranking into sampling weights that
eliminate peripheral vertices first.

## Nested dissection

Nested dissection bisects the graph, turns the crossing edges into a vertex
separator, recurses on both sides and eliminates the separator last. The
bisector is the usual multilevel scheme of coarsen, partition, uncoarsen and
refine, V-cycles included. The separator is a minimum vertex cover of the
crossing edges, found by augmenting-path matching, which by König's theorem
is the smallest separator covering them. Small subgraphs are finished by
min-fill.

The graph bisector is public on its own. A hypergraph bisector with FM and
flow-based refinement sits beside it; no construction here calls that one.

## FlowCutter

[FlowCutter](https://github.com/kit-algo/flow-cutter-pace17) finds balanced
separators by advancing max-flow cuts. goatd holds it twice:

- `flowcutter::decompose` is the vendored PACE 2017 C++ solver, which builds
  a whole decomposition. Over the PACE source it carries memory guards, work
  metering, an early-convergence limit and a density gate;
  [THIRD-PARTY.md](THIRD-PARTY.md) lists each change.
- `flowcutter::separator::find` is a Rust separator search, used to refine a
  decomposition: the decomposition is projected onto both sides of a
  separator and glued back at a new separator bag, and the replacement is
  kept only when `(width, total bag size)` improves.

## The portfolio

`portfolio::decompose` runs a fixed schedule and returns the narrowest
result, ranked by width and then total bag size. The candidates share one
graph build and one preprocessing pass. Each is handed the best width so far
and stops as soon as a bag of its own is too wide to win.

1. deterministic min-degree, first so that a valid decomposition exists
   early;
2. sampled min-degree;
3. nested dissection;
4. sampled min-fill;
5. sampled min-degree and nested dissection again on a second seed;
6. the two cardinality searches, where the residual is small enough for
   each;
7. a *diverse pass* over sampled fill and degree scores, then sampled
   min-fill *restarts* on fresh seeds while time remains;
8. a trailing FlowCutter candidate on what the restarts leave.

The sampled orders and the diverse pass run a second time on the embedding's
peripheral-first weights: the *hedge*. Those weights help some graphs and
hurt others, so both sets run rather than one being chosen.

**Budgets.** A configuration carries a soft and a hard deadline, both
measured from before preprocessing. The soft one stops the portfolio
starting further candidates; the hard one ends the run, and the solver sets
it to twice `--budget`. A run returns a decomposition whatever the clock
does: the first candidate to run out of time puts each unfinished component
in a bag of its own. Without a budget the schedule is bounded by count
instead: at most 100 restarts, no diverse pass, no FlowCutter candidate, and
none of the stages that take a share of the window.

**Large graphs.** The residual left after preprocessing picks the schedule.
Up to 10,000 vertices everything runs. Above that the portfolio times its
first candidate, prices a min-fill pass from it, and runs the whole schedule
only where the budget holds several such passes; otherwise it drops nested
dissection, the diverse pass and the hedge and spends the time on restarts.
Above 300,000 vertices a budget that will not pay for min-fill at all runs
min-degree alone. `PortfolioConfig::with_expensive_orders_up_to` moves the
upper line.

**Stopping.** `stop_flag` ends a run from outside it: every deadline check
answers as an expired hard deadline and the caller gets the best
decomposition so far. The solver sets it from `SIGTERM`. The library is
single-threaded throughout.

## Improving the answer

On a budgeted run, four stages take a share of the hard window off its end
and spend it on the answer the schedule produced. Each starts only where its
projected work fits the time it has, and its result is kept only where it is
narrower.

**The bipartite lift** runs first rather than last, on a bipartite graph.
One side is an independent set, so eliminating it adds no edges among the
eliminated vertices, and what is left is a projection whose decomposition
lifts to one of the whole graph, of width `max(projection width, largest
eliminated degree)`. On an incidence graph the projection of the clause side
is the primal graph, so the search that matters runs on the smaller graph.
The stage walks cutoffs on the side's degrees, eliminating only the vertices
below each, and keeps the narrowest lift. Its width is the incumbent every
later candidate is bounded against, and the stage runs only where the budget
pays for the projections it would build.

**Recombination** pools the bags of the best decomposition of every stage
and searches the pool for the narrowest tree whose bags all come from it.
The search is the dynamic programme of Bouchitté and Todinca over a list of
candidate bags rather than every potential maximal clique, as Tamaki (2019)
runs it. The pool holds the winner's own bags, so the answer is never wider.
After a first answer the widest bags, with their neighbours in the tree, are
re-triangulated by MCS-M and their cliques added to the list, and the search
repeats until a round adds nothing.

**The merge loop** is Tamaki's (2022) improvement loop over the same
programme. It builds a second list from scratch by randomised min-fill
draws, each made minimal; picks a bag of one list and a partner from the
other such that neither crosses the other; triangulates the piece the pair
picks out and adds its cliques; and searches the merged list, which admits
trees that neither list admits on its own.
`decomposition::decompose_by_merging` runs the loop without a portfolio in
front of it, which is the paper's algorithm.

**Local re-triangulation** takes the same pair step between the answer and
the trees the run already has: nested dissection, the diverse passes,
FlowCutter, which cut the graph where a randomised elimination does not. A
small piece is triangulated several times over and the programme run over
all of them together.

Two cheaper passes close the run. **Minimalization** removes added edges the
answer's triangulation does not need until none can be removed, which is
Rose, Tarjan and Lueker's characterisation of a minimal triangulation, and
rebuilds the bags; it never widens. **Vertex reinsertion** takes a vertex
out of the widest bags and rebuilds its attachment through neighbour and
separator bags, keeping a strict improvement in width and then total bag
size.

## Validation and reproducibility

`TreeDecomposition::validate` checks the bags, the tree, vertex and edge
coverage and the running intersection property, in time linear in the graph
and the decomposition. `TreeDecomposition::new` validates; `new_trusted`
checks only in debug builds.

Seeded, step-budgeted runs are reproducible. Wall-clock budgets are not,
since machine speed changes where they stop; while a caller holds the guard
from `meter::arm`, a duration budget advances by charged graph work instead,
and a budgeted run repeats.

## References

- Berry, Blair, Heggernes and Peyton, *Maximum cardinality search for
  computing minimal triangulations of graphs*, Algorithmica 39(4), 2004.
- Bouchitté and Todinca, *Treewidth and minimum fill-in: grouping the
  minimal separators*, SIAM Journal on Computing 31(1), 2001.
- Hamann and Strasser, *Graph bisection with Pareto optimization*,
  [arXiv:1504.03812](https://arxiv.org/abs/1504.03812); Strasser,
  *Computing tree decompositions with FlowCutter: PACE 2017 submission*,
  [arXiv:1709.08949](https://arxiv.org/abs/1709.08949).
- Rose, Tarjan and Lueker, *Algorithmic aspects of vertex elimination on
  graphs*, SIAM Journal on Computing 5(2), 1976.
- Tamaki, *Computing treewidth via exact and heuristic lists of minimal
  separators*, SEA 2019, and *Heuristic computation of exact treewidth*, SEA
  2022.
- The multilevel partitioning work is credited in
  [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md).
