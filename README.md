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
| goatd portfolio | **9,170 (97.4%)** | **8,240 (87.5%)** | **8,811 (93.6%)** | **9,040 (96.0%)** |
| Jdrasil heuristic | 9,095 (96.6%) | 1,736 (18.4%) | 3,412 (36.2%) | 6,056 (64.3%) |
| Tamaki PACE 2017 | 8,240 (87.5%) | 1,083 (11.5%) | 1,859 (19.7%) | 3,794 (40.3%) |
| FlowCutter PACE 2017 | 9,115 (96.8%) | 358 (3.8%) | 570 (6.1%) | 2,613 (27.8%) |
| HTD | 9,139 (97.1%) | 343 (3.6%) | 528 (5.6%) | 2,292 (24.3%) |
| NetworkX min-degree | 8,999 (95.6%) | 138 (1.5%) | 153 (1.6%) | 261 (2.8%) |
| NetworkX min-fill | 8,279 (88.0%) | 72 (0.8%) | 229 (2.4%) | 1,926 (20.5%) |
| Arboretum heuristic | 5,549 (59.0%) | 44 (0.5%) | 178 (1.9%) | 1,189 (12.6%) |

## Building and contributing

Build setup is in [Building](docs/building.md). Contributions follow
[CONTRIBUTING.md](docs/CONTRIBUTING.md).

## Licence

Apache-2.0. Vendored code and modifications are recorded in
[THIRD-PARTY.md](docs/THIRD-PARTY.md);
[ACKNOWLEDGEMENTS.md](docs/ACKNOWLEDGEMENTS.md)
credits the work behind the algorithms.
