# goatd — Greatest Of All Tree Decompositions

<p align="center">
  <img src="docs/logo.png" alt="goatd logo" width="280">
</p>

<p align="center">
  <a href="https://github.com/Tractables/goatd/actions/workflows/ci.yml"><img
     src="https://github.com/Tractables/goatd/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
  <a href="https://crates.io/crates/goatd"><img
     src="https://img.shields.io/crates/v/goatd.svg" alt="crates.io"></a>
  <a href="https://docs.rs/goatd"><img
     src="https://docs.rs/goatd/badge.svg" alt="docs.rs"></a>
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
`goatd --help` for budgets, seeds, weighted ties, and refinement.

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

The [solver comparison](https://tractables.github.io/goatd/) evaluates the
shipped portfolio and seven public baselines on component graphs obtained from
Model Counting Competition CNFs after two seconds of preprocessing. Each
configuration receives ten seconds on one CPU, and every decomposition is
checked by the same validator.

The default view excludes tree components and focuses on harder cases: graphs
where the smallest validated width found is 30 or greater. It also retains 17
graphs for which no displayed configuration returned a validated
decomposition. “Exact best” means a tie with the smallest validated width
found, not a proven optimum.

<!-- Generated table; 6,640 selected graphs. -->

| Solver | Valid | Exact best | Within +1 | Within +4 |
| --- | ---: | ---: | ---: | ---: |
| goatd portfolio | 6,442 (97.0%) | **4,047 (60.9%)** | **5,203 (78.4%)** | **6,166 (92.9%)** |
| Jdrasil heuristic | 6,361 (95.8%) | 2,745 (41.3%) | 3,993 (60.1%) | 5,718 (86.1%) |
| FlowCutter PACE 2017 | **6,565 (98.9%)** | 1,239 (18.7%) | 1,774 (26.7%) | 3,921 (59.1%) |
| Tamaki PACE 2017 | 6,400 (96.4%) | 922 (13.9%) | 1,435 (21.6%) | 2,870 (43.2%) |
| HTD | 6,401 (96.4%) | 381 (5.7%) | 579 (8.7%) | 1,991 (30.0%) |
| NetworkX min-degree | 6,266 (94.4%) | 157 (2.4%) | 172 (2.6%) | 332 (5.0%) |
| NetworkX min-fill | 5,564 (83.8%) | 129 (1.9%) | 311 (4.7%) | 1,712 (25.8%) |
| Arboretum heuristic | 3,059 (46.1%) | 75 (1.1%) | 162 (2.4%) | 820 (12.3%) |

## Building and contributing

Build setup is in [Building](docs/building.md). Contributions follow
[CONTRIBUTING.md](docs/CONTRIBUTING.md).

## Licence

Apache-2.0. Vendored code and modifications are recorded in
[THIRD-PARTY.md](docs/THIRD-PARTY.md);
[ACKNOWLEDGEMENTS.md](docs/ACKNOWLEDGEMENTS.md)
credits the work behind the algorithms.
