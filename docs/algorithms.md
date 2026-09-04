# Algorithms

goatd provides several tree-decomposition constructions and a portfolio that
combines them. This page describes the choices that differ from a textbook
implementation or from the upstream code.

## Portfolio

`portfolio::candidates` starts with deterministic min-degree on the original
graph, then shares one graph build and one preprocessing pass across five
elimination runs:

1. deterministic min-degree on the original graph;
2. sampled min-degree;
3. nested dissection;
4. sampled min-fill;
5. sampled min-degree with a second seed;
6. nested dissection with a second seed.

The inexpensive order runs first so a valid candidate exists early. Later
runs receive the best width already found and stop when a bag is too wide to
win. With time left, the portfolio tries sampled fill/degree scores followed
by further sampled min-fill seeds; on a very large residual it uses sampled
min-degree and skips the expensive fixed orders instead. Initial fill counts
are computed only when a fill-based order first runs, then reused across the
remaining scores and seeds. The best-only path contracts bags
contained in an adjacent bag in each candidate that can still win, then
minimizes `(treewidth, total bag size)`. `sampled_min_fill_candidates` exposes
the smaller variant used by callers that want to apply their own ranking.

A soft budget, measured from before preprocessing, stops the portfolio from
starting further candidates and samples. A candidate stopped by either budget
puts each unfinished residual component in one bag and attaches the partial
elimination bags to it, as long as it is the first candidate. This completion
is linear in the residual size. At the soft budget it applies only to the
component the candidate was working on: the components after it have the rest
of the hard budget and get their own orders, against the hard deadline alone,
and fall back to one bag each when it passes. Later candidates can stop without completion
because a valid candidate already exists. Only the hard budget ends the
portfolio: the deterministic min-degree and min-fill orders return at the soft
budget when the residual is still large, because cheap-mode elimination at
that scale can overshoot the hard budget by seconds, and what they return is a
complete decomposition that the trailing FlowCutter candidate still runs
after. `PortfolioConfig::standard_with_budget`, which the CLI uses for
a budgeted standard portfolio, offers 100 extra samples below a 4.75-second
soft budget and 1,000 at or above it, and in either case draws seeds on past
that count until the restart deadline: the count caps how many seeds are drawn,
not the clock, and a graph whose candidates are quick would otherwise finish
the schedule with budget unspent. `PortfolioConfig::with_restarts_to_deadline`
turned off stops the restarts at the count, which is what `standard()` and
`sampled_min_fill()` do; a run with no soft deadline stops at the count either
way. The restart deadline is the hard deadline less a 1.5-second reserve for
the trailing FlowCutter candidate, so the restarts and the weighted stages run
past the soft deadline: on residuals small enough for the expensive orders,
more restart time lowers width on many more graphs than a longer FlowCutter
tail does. On a residual above the 10,000-vertex cutoff, and on a run with no
hard deadline, the restarts stop at the soft deadline as the initial
candidates do. An extra sample that reaches the restart deadline stops there;
the rest of the hard-budget interval is left to the trailing FlowCutter
candidate. That candidate is skipped when the interval is too short to seed
it, and also when the graph is large enough that the backend's setup and
first restart alone outlast the interval: the setup pass runs to the end
whatever the clock says, so on such a graph the run would return well after
the hard deadline. The estimate uses the work-unit model in *Flow-based
separators*. Once the search is under way the backend does test the deadline
inside it — between the restarts, between the cells of one partition, and
between the augmentations of one cut — and a search stopped that way returns
the best decomposition it has already recorded. By default, a 4.75-second soft budget has a 9.5-second hard
deadline. Callers that need more time to write the result can set an earlier
hard budget independently without changing the soft schedule. Both standard
configurations hedge, which adds the candidates described under *The hedge*.
The library remains single-threaded throughout.

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
bound used to admit the almost-simplicial rule. Under a budget the pass stops
at the soft deadline and hands the rest of the graph to the elimination order;
the rules it has applied by then stand, and the ones it has not are the
order's to do.

The recorded bags form the beginning of the elimination sequence. A solver
continues on the residual graph, then the decomposition builder attaches these
prefix bags in reverse elimination order. The graph keeps its original vertex
ids; removed vertices are marked inactive rather than renumbered.

The portfolio performs this work once. Five elimination candidates share the
reduced graph and prefix, then compute their own order for the residual graph.
The initial deterministic min-degree candidate uses the original graph.

## Min-fill and min-degree

Min-fill eliminates the vertex whose removal introduces the fewest missing
edges among its neighbors. Fill counts are maintained in a heap and only dirty
neighbors are rescored. Dense neighborhoods use bitsets; sparse neighborhoods
use a stamped marker array. Min-degree uses the same elimination and bag
construction path with a cheaper degree key.

Minimum-degree heap entries are refreshed for every affected neighbour after
an elimination, including when its degree decreases. The portfolio's initial
form breaks equal-degree ties by heap insertion order; the standalone form
uses the caller's seeded salt.

The salted deterministic forms use a seeded per-vertex value after the primary
key. The sampled forms instead draw from the complete minimum-key tie set. Besides
fill and degree, a sampled order can minimize
`fill + degree_coefficient * degree` for a signed coefficient. A caller may
supply one weight per vertex; uniform weights give uniform sampling. Each
score remains exact while the orders explore different elimination paths.

These choices extend the standard greedy heuristics with shared reductions,
incremental scoring, explicit weighted ties, incumbent-width pruning, and
budget-aware completion.

## Vertex coordinates

`embedding::Embedding::compute` places the vertices by repeated lazy
random-walk averaging: each round moves every vertex halfway toward the mean
of its neighbours, then whitens the cloud — recentre it, rotate it onto the
eigenvectors of its covariance with cyclic Jacobi, and rescale every axis to
unit standard deviation. Without the whitening the averaging collapses the
graph onto one point. With it the averaging is subspace iteration on the lazy
random walk: the axes settle on the walk's slowest modes, they come out in
descending order of variance, and the leading one approximates a Fiedler
vector.

Whitening pins the frame only up to a rotation, since axes with close
eigenvalues can come back swapped or flipped, so the loop watches quantities a
rotation leaves alone. It stops after `patience` consecutive rounds in which no
squared eccentricity and no squared edge length changed by more than the
tolerance (1e-4 in whitened units by default), at the round cap (1,000 by
default), or when the caller's stop signal fires, and returns the last
coordinates. A round costs `O(m·d + n·d²)` and is charged to the construction
meter.

The distance of a vertex from the centre of the cloud says how peripheral it
is. `Embedding::rank_weights` turns that order into sampling weights, spread
over the whole `u32` range: a sampled order draws a tied vertex with mass
`u32::MAX - weight + 1`, so literal ranks would differ in mass by a few parts
in 2^32 and draw almost uniformly.

## The hedge

Peripheral-first sampling weights help some graphs and hurt others.
`PortfolioConfig::with_hedge` runs the portfolio's own candidates and the
weighted ones instead of choosing between them, and the cost is the time the
second set takes.

The plain candidates go first: the diverse pass runs on the caller's weights
and the seeds it always had. Then the fixed orders that read the weights run
again on the ranking, and the diverse pass follows on the ranking and on those
same seeds. Every ordinary restart stays plain, on the seed sequence and in
the order a portfolio without the hedge runs, so no restart that portfolio
would have reached goes unrun. Nothing repeats a deterministic order, which
ignores weights. Ordering matters under a budget: where the schedule finishes,
the second set is free, and where the budget binds, the second set costs later
candidates rather than displacing the first. The incumbent width bound and the
deadlines apply to every candidate of both sets.

`PortfolioConfig::standard` and `standard_with_budget` hedge on eccentricity
rankings in the dimensions of `portfolio::DEFAULT_HEDGE_DIMS`, which is
`[3, 1, 2, 4, 8, 5, 6, 7]` — every dimension the embedding has, since the
budget rule stops the series where there is no time for another stage. Each
placement runs when the first candidate that reads it asks
for it, after the plain diverse pass, so a run that ends inside the plain pass
never pays for any of them; a placement is charged to the construction meter
and stops at the soft deadline. A residual too large for the expensive orders
runs sampled min-degree, as it does without a hedge, and there is nothing there
to hedge. `with_hedge(Hedge::Off)` runs the schedule without any of it.

`Hedge::Passes` carries a `HedgeSeries`: one weighted stage per weighting, in
the order the series gives them, each stage being the fixed orders that read
weights and the diverse pass again. A series of one runs exactly what is
described above. Which graphs a weighting improves is close to arbitrary and
two weightings improve mostly different ones, so several stages collect more of
them; the incumbent width bounds the candidates of every stage after the first.
Three leads the default series because on its own it is the dimension that
helps most, and a budget that has room for one stage should spend it there.
`HedgeSeries::eccentricity_dims` takes one dimension per stage and
`HedgeSeries::random` runs the same schedule on weights drawn at random, the
control for a series that means something.

A stage is as many candidates as the diverse pass and it takes them from the
restarts, so several stages can leave a graph whose plain pass nearly filled
the budget with no restarts at all. The plain pass is the portfolio's own
measurement of what a stage costs — the same orders on other weights — so the
stages get `PortfolioConfig::with_hedge_reserve` of what the restart deadline had
left when the plain pass ended, half of it by default, and one more stage
starts only while what the stages have spent plus that measurement fits in the
share. The first stage is outside the rule: a hedge runs one weighted stage on
any budget, and the reserve decides how many follow it. A stage that has run
replaces the measurement when it was cheaper, since the incumbent bounds the
stages after the first. A refused stage refuses the ones behind it, the
restarts start where they always start. A run with no soft budget bounds no
stage and runs the whole series: the rule exists to leave the restarts their
time, and there is no deadline to take it from, which also keeps such a run's
schedule independent of how long any candidate took. Without a deadline the
diverse pass does not run either, so a stage there is only the fixed orders
that read weights. The trace reports each stage left unrun.

## Attributing a result

`portfolio::decompose_traced` reports each candidate to a caller-supplied sink
as it finishes: which candidate of the schedule it was (`portfolio::Stage`),
the seed, the pass of a hedge, whether it produced a decomposition or stopped
at the width bound or the deadline, and how far into the portfolio it
finished. A candidate that produced one also says whether the portfolio would
return it, so the winner is reported rather than inferred.
`portfolio::decompose` is the same run with the sink discarded.

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
all boundary vertices from the smaller side. Small subgraphs, and a level whose
bisection leaves nothing to recurse on, are ordered by min-fill; under a budget
that min-fill runs against the hard deadline too, and the vertices it has not
reached follow in a fixed order.

The same multilevel graph bisector is public on its own. A separate public
hypergraph bisector minimizes cut hyperedges, with FM and flow-based
refinement; it is not used to disguise a hypergraph as a graph. Hypergraph
coarsening matches vertices by their total shared hyperedge weight, so explicit
weights and repeated hyperedges affect both coarsening and the cut objective.

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
[THIRD-PARTY.md](THIRD-PARTY.md).

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

Elimination reads the clock on the work it has charged rather than on the
iterations it has run: once a millisecond's worth of charged work has passed,
or 64 iterations, whichever comes first. A count alone is the wrong interval
where iterations differ in cost by orders of magnitude — one min-fill score on
a dense residual takes milliseconds, and 64 of them used to carry a run seconds
past its hard deadline.

Seeded, step-budgeted runs are reproducible. Machine speed and load can change
where a wall-clock budget stops. While a caller holds the guard returned by
`meter::arm`, duration budgets advance by charged graph work instead. Dropping
the guard restores wall-clock budgets.

The main algorithmic sources are the
[FlowCutter bisection paper](https://arxiv.org/abs/1504.03812), the
[PACE 2017 decomposition paper](https://arxiv.org/abs/1709.08949), and the
multilevel partitioning work credited in
[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md).
