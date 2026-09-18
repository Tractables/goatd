# goatd — Greatest Of All Tree Decompositions

<p align="center">
  <img src="docs/logo.png" alt="goatd logo" width="280">
</p>

<p align="center">
  <a href="https://tractables.github.io/goatd/"><img
     src="https://img.shields.io/badge/run-in%20the%20browser-blue" alt="Run in the browser"></a>
  <a href="https://crates.io/crates/goatd"><img
     src="https://img.shields.io/crates/v/goatd.svg" alt="crates.io"></a>
  <a href="https://pypi.org/project/goatd/"><img
     src="https://img.shields.io/pypi/v/goatd.svg" alt="PyPI"></a>
  <a href="https://github.com/Tractables/goatd/releases/latest"><img
     src="https://img.shields.io/github/v/release/Tractables/goatd" alt="GitHub release"></a>
  <a href="https://github.com/Tractables/goatd/actions/workflows/ci.yml"><img
     src="https://github.com/Tractables/goatd/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://docs.rs/goatd"><img
     src="https://docs.rs/goatd/badge.svg" alt="docs.rs"></a>
</p>

Tree decompositions of graphs: a Rust library and command-line solver, with
Python, C and browser bindings. A graph goes in and a tree decomposition comes
out. The portfolio runs greedy elimination, nested dissection and flow-based
separation under one time budget and keeps the narrowest result;
[Algorithms](docs/algorithms.md) describes each construction and where it
departs from the published method.

## Solver

```sh
cargo install goatd
goatd graph.gr --order portfolio --budget 5000 > graph.td
```

`goatd` reads and writes the PACE `.gr` and `.td` formats. The budget is a
soft limit in milliseconds and the run ends at twice it. Without options
goatd runs a single min-fill order with no time limit; `goatd --help` lists
the other orders, seeds, weighted ties and refinement. The same solver runs
[in the browser](https://tractables.github.io/goatd/).

## Library

Add `goatd = "0.2"` as a dependency. The [`basic` example](examples/basic.rs)
constructs a graph, runs the portfolio, validates the decomposition and writes
it in PACE format; `cargo run --example basic` runs it. The rest of the API is
on [docs.rs](https://docs.rs/goatd).

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
| goatd portfolio | **9,347 (99.3%)** | **9,213 (97.9%)** | **9,298 (98.8%)** | **9,320 (99.0%)** |
| HTD | 9,203 (97.8%) | 703 (7.5%) | 1,813 (19.3%) | 5,866 (62.3%) |
| Jdrasil heuristic | 9,095 (96.6%) | 407 (4.3%) | 1,144 (12.2%) | 4,449 (47.3%) |
| Tamaki PACE 2017 | 8,240 (87.5%) | 353 (3.8%) | 814 (8.6%) | 2,698 (28.7%) |
| FlowCutter PACE 2017 | 9,115 (96.8%) | 176 (1.9%) | 267 (2.8%) | 1,287 (13.7%) |
| NetworkX min-degree | 8,999 (95.6%) | 117 (1.2%) | 140 (1.5%) | 190 (2.0%) |
| NetworkX min-fill | 8,279 (88.0%) | 41 (0.4%) | 94 (1.0%) | 944 (10.0%) |
| Arboretum heuristic | 5,549 (59.0%) | 25 (0.3%) | 57 (0.6%) | 623 (6.6%) |

Each solver runs at the setting its own documentation recommends for the
smallest width in a fixed time (for HTD, `--opt width --iterations 0
--strategy challenge`), and every decomposition is checked by the same
validator. Graphs where a pinned NetworkX min-degree run returns a width below
30 are left out. “Nontrivial” is a validated decomposition narrower than
`|V| - 1`; “exact best” is the smallest width among the displayed solvers, not
a proven optimum.

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
  note         = {Rust library and command-line solver, version 0.2.1}
}
```

## Licence

Apache-2.0. Vendored code and modifications are recorded in
[THIRD-PARTY.md](docs/THIRD-PARTY.md);
[ACKNOWLEDGEMENTS.md](docs/ACKNOWLEDGEMENTS.md)
credits the work behind the algorithms.
