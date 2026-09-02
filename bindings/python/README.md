# goatd for Python

<p align="center">
  <a href="https://github.com/Tractables/goatd"><img
     src="https://raw.githubusercontent.com/Tractables/goatd/main/docs/logo.png"
     alt="goatd logo" width="280"></a>
</p>

<p align="center">
  <a href="https://pypi.org/project/goatd/"><img
     src="https://img.shields.io/pypi/v/goatd.svg" alt="PyPI"></a>
  <a href="https://pypi.org/project/goatd/"><img
     src="https://img.shields.io/pypi/pyversions/goatd.svg" alt="Python versions"></a>
  <a href="https://github.com/Tractables/goatd/actions/workflows/wheels.yml"><img
     src="https://github.com/Tractables/goatd/actions/workflows/wheels.yml/badge.svg"
     alt="Wheels"></a>
  <a href="https://github.com/Tractables/goatd/blob/main/LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"
     alt="License: Apache-2.0"></a>
</p>

Python bindings for [goatd](https://github.com/Tractables/goatd), built with
[PyO3](https://pyo3.rs) and [maturin](https://www.maturin.rs).

## Install

```sh
pip install goatd
```

Wheels cover CPython 3.10 and later on Linux x86-64, macOS arm64 and Windows
x64. Anywhere else, pip builds from the source distribution, which needs a
Rust toolchain and a C++20 compiler.

## Use

```python
import goatd

graph = goatd.Graph(4, [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)])
td = goatd.decompose(graph, order="portfolio", budget_ms=100)

td.treewidth        # 2
td.bags             # [[0, 1, 2], [0, 2, 3], [0, 3], [3]]
td.edges            # [(0, 1), (1, 2), (2, 3)] — pairs of positions in td.bags
td.validate(graph)  # raises goatd.Error if td does not decompose graph
print(td.to_td())   # PACE .td text
```

## Options and formats

`decompose` takes the solver's knobs under the solver's names: `order` is one
of `minfill`, `mindegree`, `nested-dissection`, `flowcutter` and `portfolio`;
`seed` breaks ties; `ties="sample"` and `weights` control weighted sampling for
the two greedy orders; `steps` gives flowcutter a repeatable step budget in
place of a clock; `refine=True` re-cuts the result along FlowCutter separators.
An argument the chosen order cannot act on raises `ValueError` naming both.
Budgets are milliseconds, so the name is `budget_ms` rather than the command
line's `--budget`.

`goatd.Graph.from_gr` and `TreeDecomposition.from_td` read the PACE formats;
`to_gr` and `to_td` write them.

goatd is single-threaded. The interpreter lock is released for the whole of a
solve, so a caller can decompose several graphs at once from Python threads.

## Building from source

The extension builds the goatd sources in the repository, so it needs a Rust
toolchain, a C++20 compiler for the vendored FlowCutter, and maturin.

PEP 639 resolves `license-files` against the directory holding
`pyproject.toml` and forbids `..`, so the two notice files are copied in from
the repository root first rather than kept here in a second copy:

```sh
mkdir -p bindings/python/notices
cp LICENSE docs/THIRD-PARTY.md bindings/python/notices/
pip install maturin
maturin build --release --manifest-path bindings/python/Cargo.toml
```

`maturin develop` installs the extension into the active virtualenv for
`pytest bindings/python/tests`.
