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

The same constructions are available through
[Python](bindings/python/README.md), [C and C++](bindings/c/README.md), and
[WebAssembly](bindings/wasm/README.md).

## Evaluation

The [solver comparison](https://tractables.github.io/goatd/comparison/) runs
the shipped portfolio and seven public baselines on 9,413 selected component
graphs derived from Model Counting Competition formulas. Each receives ten
seconds on one CPU.

<!-- Generated table; 9,413 selected graphs. -->

| Solver | Nontrivial | Exact best | Within +1 | Within +4 |
| --- | ---: | ---: | ---: | ---: |
| goatd portfolio | **9,347 (99.3%)** | **8,616 (91.5%)** | **9,073 (96.4%)** | **9,218 (97.9%)** |
| HTD | 9,203 (97.8%) | 1,936 (20.6%) | 3,566 (37.9%) | 6,730 (71.5%) |
| Jdrasil heuristic | 9,095 (96.6%) | 824 (8.8%) | 2,193 (23.3%) | 5,464 (58.0%) |
| Tamaki PACE 2017 | 8,240 (87.5%) | 656 (7.0%) | 1,290 (13.7%) | 3,305 (35.1%) |
| FlowCutter PACE 2017 | 9,115 (96.8%) | 260 (2.8%) | 389 (4.1%) | 1,938 (20.6%) |
| NetworkX min-degree | 8,999 (95.6%) | 132 (1.4%) | 147 (1.6%) | 215 (2.3%) |
| NetworkX min-fill | 8,279 (88.0%) | 53 (0.6%) | 151 (1.6%) | 1,426 (15.1%) |
| Arboretum heuristic | 5,549 (59.0%) | 31 (0.3%) | 91 (1.0%) | 931 (9.9%) |

Each solver runs at the setting its own documentation recommends for the
smallest width inside a fixed time limit, which for HTD is
`--opt width --iterations 0 --strategy challenge`. The anytime solvers keep
searching until the ten seconds are up and report whatever decomposition they
hold when the harness stops them. goatd stops itself at its own hard cutoff
just under ten seconds and writes what it has. Every decomposition is checked
by the same validator. The default selection omits graphs where pinned
NetworkX min-degree returns a validated width below 30. “Nontrivial” means a
validated decomposition narrower than `|V| - 1`. “Exact best” is the smallest
width observed among the displayed solvers, not a proven optimum.

## Building and contributing

Build setup is in [Building](docs/building.md). Contributions follow
[CONTRIBUTING.md](docs/CONTRIBUTING.md).

## Citing

goatd has no accompanying paper, so cite the software:

```bibtex
@misc{goatd,
  author       = {Van den Broeck, Guy},
  title        = {goatd: Greatest Of All Tree Decompositions},
  year         = {2026},
  howpublished = {\url{https://github.com/Tractables/goatd}},
  note         = {Rust library and command-line solver, version 0.1.2}
}
```

## Licence

Apache-2.0. Vendored code and modifications are recorded in
[THIRD-PARTY.md](docs/THIRD-PARTY.md);
[ACKNOWLEDGEMENTS.md](docs/ACKNOWLEDGEMENTS.md)
credits the work behind the algorithms.
