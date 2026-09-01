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
pinned NetworkX min-degree run returned a validated decomposition of width
below 30. If that run did not return a validated decomposition, the graph
remains. This keeps the selected graphs fixed when new solver results are
added. “Exact best” means a tie with the smallest validated width found, not a
proven optimum.

<!-- Generated table; 9,395 selected graphs. -->

| Solver | Valid | Exact best | Within +1 | Within +4 |
| --- | ---: | ---: | ---: | ---: |
| goatd portfolio | 9,193 (97.8%) | **5,063 (53.9%)** | **7,144 (76.0%)** | **8,884 (94.6%)** |
| Jdrasil heuristic | 9,112 (97.0%) | 4,429 (47.1%) | 6,426 (68.4%) | 8,462 (90.1%) |
| Tamaki PACE 2017 | 9,145 (97.3%) | 2,161 (23.0%) | 3,141 (33.4%) | 5,326 (56.7%) |
| FlowCutter PACE 2017 | **9,317 (99.2%)** | 1,291 (13.7%) | 2,011 (21.4%) | 5,635 (60.0%) |
| HTD | 9,156 (97.5%) | 427 (4.5%) | 791 (8.4%) | 3,627 (38.6%) |
| NetworkX min-degree | 9,016 (96.0%) | 157 (1.7%) | 173 (1.8%) | 380 (4.0%) |
| NetworkX min-fill | 8,296 (88.3%) | 154 (1.6%) | 462 (4.9%) | 3,205 (34.1%) |
| Arboretum heuristic | 5,581 (59.4%) | 106 (1.1%) | 351 (3.7%) | 1,893 (20.1%) |

## Building and contributing

Build setup is in [Building](docs/building.md). Contributions follow
[CONTRIBUTING.md](docs/CONTRIBUTING.md).

## Licence

Apache-2.0. Vendored code and modifications are recorded in
[THIRD-PARTY.md](docs/THIRD-PARTY.md);
[ACKNOWLEDGEMENTS.md](docs/ACKNOWLEDGEMENTS.md)
credits the work behind the algorithms.
