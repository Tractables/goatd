# goatd — Greatest Of All Tree Decompositions

<p align="center">
  <img src="docs/logo.png" alt="goatd logo" width="280">
</p>

<p align="center">
  <a href="https://tractables.github.io/goatd/"><img
     src="https://img.shields.io/badge/run-in%20the%20browser-blue" alt="Run in the browser"></a>
  <a href="https://crates.io/crates/goatd"><img
     src="https://img.shields.io/crates/v/goatd.svg" alt="crates.io"></a>
  <a href="https://docs.rs/goatd"><img
     src="https://docs.rs/goatd/badge.svg" alt="docs.rs"></a>
  <a href="https://pypi.org/project/goatd/"><img
     src="https://img.shields.io/pypi/v/goatd.svg" alt="PyPI"></a>
  <a href="https://github.com/Tractables/goatd/releases/latest"><img
     src="https://img.shields.io/github/v/release/Tractables/goatd" alt="GitHub release"></a>
  <a href="https://github.com/Tractables/goatd/actions/workflows/ci.yml"><img
     src="https://github.com/Tractables/goatd/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
</p>

Tree decompositions of graphs, as a Rust library and command-line solver. The
portfolio runs several constructions and keeps the narrowest result:

- **portfolio search** — safe reductions, several seeds and construction
  methods, then optional separator refinement;
- **greedy elimination** — min-fill and min-degree, with deterministic or
  weighted-sampling tie breaking;
- **nested dissection** — recursive multilevel bisection, with each separator
  eliminated after its two sides;
- **flow-based separation** — balanced cuts for constructing and refining
  decompositions. goatd includes a Rust FlowCutter separator search and the
  vendored PACE 2017 FlowCutter decomposer.

See [Algorithms](docs/algorithms.md) for the details and the differences from
the upstream methods.

## Solver

```sh
cargo install goatd
goatd graph.gr > graph.td
```

`goatd` reads and writes the PACE `.gr` and `.td` formats. Choose `--order
minfill`, `mindegree`, `nested-dissection`, `flowcutter`, or `portfolio`; run
`goatd --help` for budgets, seeds, weighted ties, and refinement. The same
solver runs [in the browser](https://tractables.github.io/goatd/).

## Library

```toml
[dependencies]
goatd = "0.1"
```

The [`basic` example](examples/basic.rs) constructs a graph, computes a
decomposition, validates it, and writes it in PACE format:

```sh
cargo run --example basic
```

The public API also exposes graph and hypergraph bisection, the Rust separator
search, the C++ FlowCutter decomposer, and decomposition projection and
refinement. Rustdoc documents each entry point.

## Bindings

The same constructions are available from
[Python](bindings/python/README.md) and from
[C and C++](bindings/c/README.md).

## Evaluation

The [solver comparison](https://tractables.github.io/goatd/comparison/)
evaluates the shipped portfolio and seven public baselines on component graphs
obtained from Model Counting Competition CNFs after two seconds of
preprocessing. Each configuration receives ten seconds on one CPU, and every
decomposition is checked by the same validator.

The default view excludes tree components. It also excludes a graph when the
pinned NetworkX min-degree run returned a counted decomposition of width below
30. If that run did not return a counted decomposition, the graph remains.
This keeps the selected graphs fixed when new solver results are added.

A counted result passes validation, contains more than one bag, and has width
below the graph's one-bag width `|V| - 1`. A result that does not improve that
bound is counted like a timeout. “Exact best” means a tie with the smallest
counted width found, not a proven optimum.

<!-- Generated table; 9,413 selected graphs. -->

| Solver | Nontrivial | Exact best | Within +1 | Within +4 |
| --- | ---: | ---: | ---: | ---: |
| goatd portfolio | **9,176 (97.5%)** | **5,046 (53.6%)** | **7,127 (75.7%)** | **8,867 (94.2%)** |
| Jdrasil heuristic | 9,095 (96.6%) | 4,412 (46.9%) | 6,409 (68.1%) | 8,445 (89.7%) |
| Tamaki PACE 2017 | 8,240 (87.5%) | 2,068 (22.0%) | 3,048 (32.4%) | 5,233 (55.6%) |
| FlowCutter PACE 2017 | 9,115 (96.8%) | 1,160 (12.3%) | 1,877 (19.9%) | 5,499 (58.4%) |
| HTD | 9,139 (97.1%) | 410 (4.4%) | 774 (8.2%) | 3,610 (38.4%) |
| NetworkX min-degree | 8,999 (95.6%) | 140 (1.5%) | 156 (1.7%) | 363 (3.9%) |
| NetworkX min-fill | 8,279 (88.0%) | 137 (1.5%) | 445 (4.7%) | 3,188 (33.9%) |
| Arboretum heuristic | 5,549 (59.0%) | 89 (0.9%) | 334 (3.5%) | 1,876 (19.9%) |

## Building and contributing

Build setup is in [Building](docs/building.md). Contributions follow
[CONTRIBUTING.md](docs/CONTRIBUTING.md).

## Licence

Apache-2.0. Vendored code and modifications are recorded in
[THIRD-PARTY.md](docs/THIRD-PARTY.md);
[ACKNOWLEDGEMENTS.md](docs/ACKNOWLEDGEMENTS.md)
credits the work behind the algorithms.
