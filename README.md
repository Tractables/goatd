# goatd — Greatest Of All Tree Decompositions

Tree decompositions of graphs, as a Rust library and a command-line solver.
A graph goes in as an edge list and a tree decomposition comes out, by one of
three routes:

- **elimination orders** — min-fill, min-degree and nested dissection, each
  run after a safe-reduction preprocessing pass, with two sampled variants
  that break ties by a per-vertex weight; a schedule that runs several under a
  time budget and keeps the narrowest; and a refinement pass that re-cuts a
  decomposition along FlowCutter separators;
- **FlowCutter** — the PACE 2017 treewidth solver, vendored in C++ and driven
  in process under a wall-clock or a step budget;
- **multilevel bisection** — the graph and hypergraph bisectors the orders
  above are built on, usable on their own.

Every decomposition returned covers each vertex and each edge and has the
running intersection property. The library spawns no thread and reads no
environment variable; a search that reads a clock says so in its signature,
and every other route is deterministic for a given seed.

## The solver

```sh
cargo install goatd
goatd graph.gr > graph.td
```

`goatd` reads a PACE `.gr` file (or `-` for stdin) and writes a PACE `.td` to
stdout or to `--out`. `--order` picks the route: `minfill` (the default),
`mindegree`, `nested-dissection`, `flowcutter`, or `schedule`, which runs
several orders under one budget. `--seed` fixes the tie-breaking, `--budget
<ms>` bounds the run, `--steps <n>` gives FlowCutter a step budget so a run
repeats exactly, `--ties sample` breaks min-fill and min-degree ties by
weighted sampling (with `--weights <file>`, one integer per vertex), and
`--refine` finishes with the FlowCutter-cut refinement. A flag the chosen
order cannot act on is an error naming both; `goatd --help` has the full list.

## The library

```toml
[dependencies]
goatd = "0.1"
```

```rust
use goatd::Graph;
use goatd::elimination::{Config, elimination_td};

// The 4-cycle with one chord: treewidth 2.
let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)]);
let td = elimination_td(&graph, Config::MinFill, 0, None);
assert_eq!(td.treewidth(), 2);
println!("{}", td.to_td(graph.num_vertices));
```

`Graph::from_gr` and `TreeDecomposition::from_td` read the PACE formats;
`to_gr` and `to_td` write them. `goatd::flowcutter::flowcutter_td` runs
FlowCutter under an `FcBudget`; `goatd::elimination::refined_td` is the
scheduled-and-refined construction the solver's `--order schedule --refine`
runs; `goatd::td_ops` roots, projects and glues decompositions. The rustdoc
has the rest.

## Building

The FlowCutter backend is C++ and is compiled by `build.rs` with the system
C++ compiler, so a build needs one that speaks C++20: `build.rs` looks for
`g++-14`, `g++-13`, `g++-12`, then plain `g++`, and `GOATD_CXX` names one
outright (GCC 12 or newer, or a recent Clang). Nothing else is needed: no
CMake, no system libraries, no network.

## Licence

Apache-2.0. The vendored FlowCutter sources are BSD-2-Clause and the graph
layer they sit on is MIT; [`THIRD-PARTY.md`](THIRD-PARTY.md) lists every
component and the modifications made to it, and
[`ACKNOWLEDGEMENTS.md`](ACKNOWLEDGEMENTS.md) credits the work the heuristics
come from.
