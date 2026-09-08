# Algorithms

goatd provides several tree-decomposition constructions and a portfolio that
combines them. This page describes what differs from a textbook implementation
or from the vendored upstream code.

## The portfolio

`portfolio::candidates` runs a fixed schedule and returns every distinct
decomposition it produces, each with the bags an adjacent bag contains
contracted and the list sorted by width and then total bag size, so its head
is what `portfolio::decompose` returns; candidates that built the same bags
under the same tree, which different elimination orders often do, count once.
`portfolio::candidates_traced` adds to each the stage, seed and pass that
produced it. The schedule:

1. deterministic min-degree;
2. sampled min-degree;
3. nested dissection;
4. sampled min-fill;
5. sampled min-degree, second seed;
6. nested dissection, second seed.

All six share one graph build and one preprocessing pass, then compute their
own order for the residual.
Min-degree goes first so that a valid decomposition exists early, and every
later candidate is handed the best width so far and stops as soon as one of its
bags is too wide to win. Fill counts are computed once, when the first
fill-based order needs them, and reused by the rest.

Two cardinality-search candidates follow the fixed orders, each on a residual
small enough for it. Neither is part of the schedule choice below: each has a
vertex gate of its own. They are described under *Cardinality searches*.

With time left the portfolio keeps going: first a diverse pass over sampled
fill/degree scores, where what the candidates above cost says it fits, then
further min-fill seeds, which the rest of this page calls the restarts. A
trailing candidate hands the graph to FlowCutter.

The residual left after preprocessing picks between three schedules. At or
below 10,000 vertices all of the above runs. Above that line it runs where the
budget is wide enough for it: the portfolio times its first candidate, prices
one min-fill pass over the residual from that, and runs the whole schedule
while the time the soft deadline has left holds fifteen such passes. Where it
does not, and up to 300,000 vertices, the min-fill candidate still runs but
stops at half the time the restart deadline has left when it starts, so the
restarts keep a share of the budget; nested dissection, the diverse pass and
the hedge are skipped; a candidate that reaches the soft deadline stops there
as everywhere else, but the portfolio keeps starting candidates and restarts
until the restart deadline rather than the soft one, which on a budget over
4.75 seconds is the soft deadline anyway, since the second stage then goes to
the FlowCutter candidate; and the restarts are sampled min-fill when the initial
min-fill produced a decomposition and sampled min-degree when it did not.
Above 300,000 vertices the same price decides again at a different fraction:
the paced schedule runs where the time the soft deadline has left holds two of
those passes, which is what the min-fill candidate there would run to, and
otherwise only the min-degree candidates and sampled min-degree restarts run.
`PortfolioConfig::with_expensive_orders_up_to` moves the upper boundary; the
10,000-vertex line is fixed, and a run with no soft budget has no window to
price a pass against, so there that line is the whole rule. The FlowCutter
candidate runs on a residual of any size, under its own vertex cap.

`portfolio::decompose` returns one decomposition rather than all of them. It
contracts bags contained in a neighbouring bag, in each candidate that can
still win, then minimizes `(treewidth, total bag size)`.
`sampled_min_fill_candidates` is the smaller set, for callers that rank the
candidates themselves.

### Budgets

A configuration carries a soft and a hard deadline, both measured from before
preprocessing. The soft one stops the portfolio starting further candidates and
samples; the hard one ends the run. In the solver the hard deadline defaults to
twice `--budget`, so a 4.75-second budget pairs with a 9.5-second cutoff. A
caller that needs time to write the result out can bring the hard deadline in
on its own.

A run returns a decomposition whatever the clock does. The first candidate to
run out of time puts each unfinished residual component in a bag of its own and
attaches the elimination bags it did build, at a cost linear in the residual.
On a residual running the whole schedule later candidates just stop, since a
valid decomposition already exists; on a paced one every candidate completes,
and the one that left the smallest residual wins. At the soft deadline that completion
covers only the component in hand, and the ones behind it get their own orders
against the hard deadline.

`PortfolioConfig::with_sampling_patience` asks the restarts to stop once they
stall: after a floor number of them, a run whose last improvement on the
portfolio's best decomposition is in the first half of the restarts it has done
gives the rest of the time back to the caller, and the trailing FlowCutter
candidate stops after half its window without a narrower decomposition instead
of running the window out. The floor is at most half the restarts the schedule
draws, so the rule fires on a schedule bounded by its count as well as on one
bounded by a deadline. It is off unless the caller asks for it: replaying the
rule over a corpus run, the time it saves costs width on about one small graph
in nine.

The restarts run past the soft deadline into the hard window, stopping 1.5
seconds short of it to leave the FlowCutter candidate that much to run in.
Under `PortfolioConfig::standard_with_budget` the restart deadline is what ends
them: one more starts only while what the previous one cost still fits before
it. The sampling count caps how many seeds are drawn, not how long they run, so
it is what stops the restarts of a run with no deadline to run to, which is
`standard()`, `sampled_min_fill()`, and any configuration with
`PortfolioConfig::with_restarts_to_deadline` turned off. On a residual over the
300,000-vertex limit that the budget leaves with min-degree alone, the restarts
stop at the soft deadline as the initial candidates do, and between 10,000 and
300,000 vertices they stop there too under a soft budget over 4.75 seconds,
where the FlowCutter candidate runs to its window and the second stage is worth
more to it than to more restarts.
Over 300,000 vertices there is one exception: the FlowCutter candidate's own
work model may say it could
not start and stop inside the hard window on a graph this size; then they run to
the hard deadline less a reserve for bagging the residual and writing the
result, which grows with the vertex and edge counts and is held between 50 ms
and 4 s. That reserve replaces the 1.5 seconds between 10,000 and 300,000
vertices too, on a graph the model declines. On a run with no hard deadline they
stop at the soft deadline.

`standard_with_budget` asks for the whole schedule at every budget. What a
short one can afford is decided when the run gets there: the diverse pass runs
on a residual the schedule admits, and there while what the initial orders cost,
divided between them, projects one more candidate of that shape to fit in half
the time the restart deadline has left; and the FlowCutter candidate runs when
the window it is left passes the test in the next paragraph. That test is what
decides the second stage of a short budget too, so a residual over the limit
keeps it only where the trailing candidate would not have used it.

The FlowCutter candidate takes what is left, less two estimated restarts, since
the vendored backend tests its deadline only between restarts and the result
still has to be copied out; over a 4.75-second window that estimate is capped at
half of what is left, so a graph whose restarts are expensive still gets a
candidate rather than none. It is skipped when what remains is too short to
seed it, or when the backend's setup and first restart would outlast it. On a
window of 4.75 seconds or less it also stops once it has gone 500 milliseconds
without finding a narrower decomposition, or after 50 restarts. On a longer
window neither applies and the window is what ends it: the candidate is the last
thing the portfolio runs, so time it leaves is time nobody uses.

Last of all, on a graph small enough for it, the portfolio drops the fill its
winner does not need and keeps the better of the two. That pass cannot widen a
bag, and it starts only while the work it projects fits in what the hard
deadline has left; it is described under *Cardinality searches*.

`stop_flag` ends a run from outside it: every deadline check in the library and
in the vendored backend then answers as an expired hard deadline, and the
caller gets the best decomposition found so far. The solver sets the flag from
a `SIGTERM` handler. Both standard configurations hedge, and the library is
single-threaded throughout.

## Preprocessing

Every elimination construction starts from the same deterministic reduction,
which the portfolio runs once and shares:

| rule | condition | action |
| --- | --- | --- |
| islet | degree 0 | remove `v` and record `{v}` |
| twig | degree 1, with neighbour `u` | remove `v` and record `{v, u}` |
| series | degree 2, with non-adjacent neighbours `a` and `b` | add `a-b`, remove `v`, and record `{v, a, b}` |
| simplicial | the live neighbours of `v` form a clique | remove `v` and record `{v} ∪ N(v)` |
| almost simplicial | the live neighbours have exactly one missing edge, and `degree(v)` does not exceed the running width lower bound | add the missing edge, remove `v`, and record `{v} ∪ N(v)` |

Islet and twig removal runs to a fixed point first, which clears forest
components without leaving a width-2 series bag behind. The pass then tries
series, simplicial and almost simplicial in that order, and starts again
whenever a rule fires. Series and simplicial eliminations raise the running
lower bound that admits the almost-simplicial rule. Under a budget the pass
stops at the soft deadline and leaves the rest of the graph to the elimination
order.

The recorded bags are the front of the elimination sequence. A solver continues
on the residual graph, and the decomposition builder attaches the prefix bags
in reverse elimination order. Vertex ids never change: a removed vertex is
marked inactive rather than renumbered.

## Min-fill and min-degree

Min-fill eliminates the vertex whose removal adds the fewest edges among its
neighbours. Fill counts live in a heap, only dirty neighbours are rescored, and
dense neighbourhoods use bitsets where sparse ones use a stamped marker array.
Min-degree shares the elimination and bag-building path with a cheaper key, and
refreshes the heap entry of every affected neighbour, including when a degree
falls.

Each score is exact. What varies between the orders is how they break ties.

- A salted deterministic order adds a seeded per-vertex value after the primary
  key. The portfolio's initial min-degree instead breaks equal degrees by heap
  insertion order.
- A sampled order draws from the whole minimum-key tie set, with one
  caller-supplied weight per vertex; uniform weights give uniform sampling. A
  vertex is drawn with mass `u32::MAX - weight + 1`, linear in the weight, so
  weight 0 is the likeliest and `u32::MAX` is drawn about once in four billion
  against it. Weights within a small factor of each other give a mild bias.
- The restarts widen that set into a band, taking every vertex whose fill is
  within a fixed distance of the smallest, so seeds still separate on a graph
  where one vertex holds the minimum at every step.
  `PortfolioConfig::with_sample_band` sets the width, which the standard
  configurations leave at 3, and 0 restores the exact minimum;
  `with_sample_band_alternate` sends even-numbered
  restarts to the minimum and odd-numbered ones to the band. Only the restarts
  read the band. Every other candidate runs its own score's exact minimum.

Besides fill and degree, a sampled order can minimize
`fill + degree_coefficient * degree` for a signed coefficient.

## Vertex coordinates

`embedding::Embedding::compute` places the vertices by repeated lazy
random-walk averaging. Each round moves every vertex halfway toward the mean of
its neighbours, then whitens the cloud: recentre it, rotate it onto the
eigenvectors of its covariance with cyclic Jacobi, and rescale every axis to
unit standard deviation. Without the whitening the averaging collapses the
graph onto one point. With it the rounds are subspace iteration on the lazy
random walk, so the axes settle on the walk's slowest modes in descending order
of variance, and the leading one approximates a Fiedler vector.

Since whitening pins the frame only up to a rotation, the stopping test watches
quantities a rotation leaves alone: the loop stops after `patience` consecutive
rounds in which no squared eccentricity and no squared edge length moved by
more than the tolerance, at the round cap, or on the caller's stop signal. The
defaults are 1e-4 in whitened units and 1,000 rounds. A round costs
`O(m·d + n·d²)` and is charged to the construction meter.

Distance from the centre of the cloud is how peripheral a vertex is.
`Embedding::rank_weights` turns that order into sampling weights. They are
spread over the whole `u32` range because a sampled order draws a tied vertex
with mass `u32::MAX - weight + 1`, so literal ranks would differ by a few parts
in 2^32 and draw almost uniformly.

## The hedge

Peripheral-first sampling weights help some graphs and hurt others, so the
standard configurations run both sets of candidates rather than choose between
them. The cost is the time the second set takes.

The plain candidates go first, on the caller's weights and the usual seeds. A
weighted stage then repeats, on one ranking, the fixed orders that read weights
and the diverse pass. Deterministic orders are not repeated, since they ignore
weights, and the restarts stay plain and keep the seeds and the position they
hold without a hedge. The incumbent width bound and both deadlines apply to
every candidate of both sets.

`PortfolioConfig::standard` and `standard_with_budget` hedge on eccentricity
rankings in the dimensions of `portfolio::DEFAULT_HEDGE_DIMS`,
`[3, 1, 2, 4, 8, 5, 6, 7]`, which is every dimension the embedding has, with
the one that helps most on its own in front. A ranking is computed when the
first candidate that reads it asks for it, so a run that ends inside the plain
pass pays for none of them. `PortfolioConfig::with_hedge` takes `Hedge::Off` or
a `Hedge::Passes` carrying a `HedgeSeries`, where
`HedgeSeries::eccentricity_dims` gives one dimension per stage and
`HedgeSeries::random` draws the weights at random as a control.

A stage is as many candidates as the diverse pass and takes them from the
restarts, so the budget decides how many run. The plain pass is the portfolio's
own measurement of what a stage costs. The stages get
`PortfolioConfig::with_hedge_reserve` of whatever the restart deadline had left
when that pass ended, half of it by default, and another stage starts only
while the stages' spend and one more measurement fit inside that share. One
stage runs on any budget. A run with no soft budget runs the whole series, and
the trace reports each stage left unrun.

## Tracing a run

`portfolio::decompose_traced` reports each candidate to a caller-supplied sink
as it finishes: which candidate of the schedule it was (`portfolio::Stage`),
the seed, the pass of a hedge, whether it produced a decomposition or stopped
at the width bound or a deadline, and how far into the portfolio it finished. A
candidate that produced one also says whether the portfolio would return it, so
the winner is reported rather than inferred, and carries two more numbers about
the bags for a caller ranking the candidates itself: the bag mass, `log2` of
the sum over bags of `2^|bag|`, and the largest number of vertices two adjacent
bags share. `portfolio::decompose` is the same run with the sink discarded, and
does not compute them.

## Cardinality searches

Completing every bag of a tree decomposition to a clique gives a chordal graph
containing the input — a triangulation — whose maximal cliques are the bags of
a decomposition of the same width. A triangulation is minimal when none of the
edges it added can be taken out again without breaking chordality. A minimal
triangulation is not a narrowest one, but it never has an edge the width is
paying for and nothing needs.

Maximum cardinality search numbers the vertices from `n` down to 1, always
taking one with the most numbered neighbours; on a chordal graph the numbers
read backwards are a perfect elimination ordering. MCS-M is the same search
with a longer reach: a vertex counts the numbered vertices it can reach along a
path whose interior vertices all count lower than the path's endpoint.
Eliminating along the numbering MCS-M produces fills the graph to a minimal
triangulation (Berry, Blair, Heggernes and Peyton, *Maximum cardinality search
for computing minimal triangulations of graphs*, Algorithmica 39(4), 2004). The
two searches differ only in how one step collects the vertices whose count goes
up, so they are one function with a switch.

Three things come out of that, all of them optional gated additions to the
portfolio rather than replacements for anything.

**Maximum cardinality search as a candidate.** `Order::MaximumCardinality`
runs the plain search on the preprocessed residual and eliminates along the
numbering reversed. On a chordal residual that adds no fill at all; on any
other it adds whatever the numbering happens to need, with no minimality
guarantee. It costs one scan of the unnumbered vertices per vertex plus one
pass over the edges, which is cheap enough to run on residuals far larger than
MCS-M can be run on, so `PortfolioConfig::with_maximum_cardinality` has a gate
of its own. The candidate runs before the MCS-M one, which leaves MCS-M a
tighter width bound to abort on.

**MCS-M as a candidate.** `Order::MinimalTriangulation` runs MCS-M on the
preprocessed residual and eliminates along the result. It reads no seed and no
weights, so it is one candidate rather than a family of them, and so is the
plain search above it. It costs one traversal of the residual per vertex, which
is why `PortfolioConfig::with_minimal_triangulation` gates it on the residual's
vertex count. Both candidates run against the soft deadline and give up
part-way rather than taking the restarts' time. One step of MCS-M can walk the
whole residual, so the shared search reads the clock while it walks rather than
only between steps; both reaches stop the same way. On most graphs the greedy
orders are narrower and the portfolio keeps them.

**Dropping fill the bags do not need.** Removing one edge `uv` from a chordal
graph leaves it chordal exactly when the common neighbourhood of `u` and `v` is
a clique, and a triangulation is minimal exactly when no single added edge can
be removed (Rose, Tarjan and Lueker, *Algorithmic aspects of vertex elimination
on graphs*, SIAM Journal on Computing 5(2), 1976). So dropping removable added
edges until none is left gives a minimal triangulation.
`decomposition::minimalize_triangulation` does that to a decomposition's own
completion and rebuilds the bags from a perfect elimination ordering of what
remains. Dropping edges cannot enlarge a clique, so the pass never widens; when
it improves neither the width nor the total bag size, the input comes back
unchanged. It holds two bitsets over the graph's vertices, which is why
`PortfolioConfig::with_triangulation_refinement` gates it on the vertex count.
The portfolio applies it to its winner, whatever candidate produced it, and
hands the result back as one more candidate.

How long the pass takes does not follow the vertex count. It follows the bags
of the decomposition being rebuilt, and how many sweeps the edge-dropping needs
is not known until it has run. So the vertex gate bounds the memory and the
clock bounds the time: completing the bags costs one insert per pair of a bag,
which the decomposition says in advance, and the portfolio starts the pass only
while that projection fits in what is left of the hard deadline. After that,
every loop in the pass reads the clock on a stride. The completion and the
rebuild hand back the input decomposition when they run out of time; a sweep cut
part-way keeps the edges it had already dropped, since taking a removable edge
out of a chordal graph leaves it chordal whether or not the sweep finishes. A
graph that runs out of time therefore loses the improvement and keeps its
decomposition.

## Nested dissection and multilevel bisection

Nested dissection is not a separate partitioning primitive. It repeatedly calls
goatd's multilevel graph bisector, turns the crossing edges of a bisection into
a vertex separator, recurses on both remaining sides, and eliminates the
separator last.

The bisector follows the usual coarsen, initial-partition, uncoarsen and refine
sequence, V-cycles included. For a proposed bisection, goatd builds the
bipartite graph of crossing edges and computes a minimum vertex cover by
augmenting-path matching. By König's theorem that is the smallest separator
covering those edges, and it is never larger than all the boundary vertices on
the smaller side. Small subgraphs, and a level whose bisection leaves nothing
to recurse on, are ordered by min-fill against the hard deadline; the vertices
it does not reach follow in a fixed order.

The graph bisector is public on its own, as is a separate hypergraph bisector
that minimizes cut hyperedges with FM and flow-based refinement. Hypergraph
coarsening matches vertices by their total shared hyperedge weight, so explicit
weights and repeated hyperedges affect both the coarsening and the cut
objective.

Partition refinement belongs to the partitioner: it improves the temporary 0/1
bisection during uncoarsening and returns another bisection. Decomposition
refinement is under `decomposition`, and rewrites complete bags and a bag tree
around a separator supplied by FlowCutter.

## Flow-based separators

[FlowCutter](https://github.com/kit-algo/flow-cutter-pace17) trades cut size
against balance by repeatedly advancing max-flow cuts. goatd contains two
implementations of it:

- `flowcutter::separator::find`, a Rust separator search that returns the
  separator and its two sides, used by decomposition refinement;
- `flowcutter::decompose`, the vendored PACE 2017 C++ builder, which constructs
  a whole decomposition.

The Rust search uses one cutter per restart rather than a growing multi-cutter
batch, and goatd's seeded RNG. Refinement projects an existing decomposition
onto both sides of a separator, glues the projections at a new separator bag,
and accepts the replacement only when `(treewidth, total bag size)` improves;
recursion applies the same monotone check.

The C++ builder carries several practical changes over the PACE source: memory
guards for the dense adjacency matrix and the bag-adjacency graph, 64-bit
heap-position arithmetic, bounded greedy-order passes with work-unit metering,
an early-convergence patience limit, and a density gate on a shortcut order
that is expensive on clique-dominated graphs.
[THIRD-PARTY.md](THIRD-PARTY.md) lists every source change and licence.

## Recombining the candidates' bags

Every candidate above produces a whole tree decomposition and the portfolio
keeps the narrowest, which throws away the good bags of all the others. The
last stage of a budgeted run keeps them instead: it collects the bags of every
decomposition the run produced and searches, over that pool, for the narrowest
tree decomposition whose bags all come from it.

The search is the dynamic programme of Bouchitté and Todinca restricted to a
list of candidate bags rather than run over every potential maximal clique of
the graph. A *block* is a connected component `C` of `G` less a pool bag,
carried with its separator `N(C)`; a *cap* of a block is a pool bag `Ω` with
`N(C) ⊆ Ω ⊆ C ∪ N(C)` and a vertex inside `C`. The width of a block is the
cheapest way to decompose `C ∪ N(C)` with `N(C)` in its top bag: everything in
one bag, or a cap with the blocks it leaves inside `C` under it. Blocks are
evaluated smallest first, so a block's sub-blocks are settled before it and one
pass is enough; the answer is the same expression over the whole graph,
minimised over the choice of top bag.

The tree that comes out is a valid decomposition whatever the pool holds, so no
bag is tested for being a potential maximal clique: a cap and the blocks below
it cover every edge inside `C ∪ N(C)`, and each vertex's bags form a subtree
because a block's bags stay inside it. The pool holds the winner's own bags, so
the search cannot come back wider than the portfolio already has, and the
portfolio keeps the result only where it is narrower.

`PortfolioConfig::with_recombination` gates the stage on a vertex count,
because the search costs one traversal of the graph per bag in the pool. What
it holds is capped separately, as a multiple of the graph's vertex count: 64
vertex ids per vertex in the pool and 64 again in the blocks. It gives up the
graph rather than exceed either. The stage takes a share of the hard window off
the end — an eighth, between 50 ms and 30 s — so the rest of the schedule
finishes that much earlier; a run with no hard budget has no share to give it
and does not run it. At its deadline the search hands back nothing rather than
a part-built answer, and the portfolio returns what it had.

The reference for the dynamic programme is Bouchitté and Todinca, "Treewidth
and minimum fill-in: grouping the minimal separators", SIAM Journal on
Computing 31(1), 2001. Running it over a heuristic list rather than the
complete one is Tamaki, "Computing treewidth via exact and heuristic lists of
minimal separators", 2019.

## Decomposition operations

`decomposition` holds the tree-decomposition type, validation, projection,
FlowCutter-based refinement, and the minimalization pass of *Cardinality
searches*. Refinement preserves global vertex ids while it projects each side
and glues them at a separator.

Public constructors canonicalize each bag's contents and the undirected bag
edges, so equivalent caller inputs expose the same rooted walk. Native
algorithms may keep a stable algorithm-defined vertex and neighbour order where
it carries useful traversal information, and FlowCutter preserves both at its
adapter boundary.

## Correctness and reproducibility

`TreeDecomposition::validate` checks bag contents, the bag forest, vertex and
edge coverage, and the running intersection property.

Elimination reads the clock on the work it has charged rather than on the
iterations it has run: once a millisecond's worth of charged work has passed,
or 64 iterations, whichever comes first. Iterations differ in cost by orders of
magnitude, so a count alone used to carry a run seconds past its hard deadline.

Seeded, step-budgeted runs are reproducible. Wall-clock budgets are not, since
machine speed and load change where they stop. While a caller holds the guard
returned by `meter::arm`, duration budgets advance by charged graph work
instead; dropping the guard restores wall-clock budgets.

The main algorithmic sources are the
[FlowCutter bisection paper](https://arxiv.org/abs/1504.03812), the
[PACE 2017 decomposition paper](https://arxiv.org/abs/1709.08949), and the
multilevel partitioning work credited in
[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md).
