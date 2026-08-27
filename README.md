# goatd — Greatest Of All Tree Decompositions

<p align="center">
  <img src="assets/logo.png" alt="goatd logo" width="280">
</p>

<p align="center">
  <a href="https://github.com/Tractables/goatd/actions/workflows/ci.yml"><img
     src="https://github.com/Tractables/goatd/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License: Apache-2.0"></a>
  <!-- Once the crate is on crates.io, add:
       <a href="https://crates.io/crates/goatd"><img src="https://img.shields.io/crates/v/goatd.svg" alt="crates.io"></a>
       <a href="https://docs.rs/goatd"><img src="https://docs.rs/goatd/badge.svg" alt="docs.rs"></a> -->
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

## Building and contributing

Build setup is in [Building](docs/building.md). Contributions follow
[CONTRIBUTING.md](CONTRIBUTING.md).

## Licence

Apache-2.0. Vendored code and modifications are recorded in
[THIRD-PARTY.md](THIRD-PARTY.md); [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md)
credits the work behind the algorithms.
