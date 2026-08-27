# Agent Instructions — goatd

`CLAUDE.md` is the instruction file for this repository. `AGENTS.md` is a
relative symlink to it so agents read the same rules. Edit this file, not the
symlink.

goatd is a public Rust library and CLI: a graph goes in and a tree
decomposition comes out. Library code, solver code, tests, documentation, and
vendored dependencies live here. Every tracked file and commit is public.

[`CONTRIBUTING.md`](CONTRIBUTING.md) binds in full. In particular:

- Run the complete gate set before reporting a change finished, including
  `--all-targets` so the CLI and examples are covered.
- For a bug fix, add a regression test and confirm that it fails on the
  unfixed parent.
- Keep the library single-threaded. Consumers own parallelism across graph
  instances.
- Keep budgets and seeds explicit. Library code does not read the process
  environment.
- Reject an inert CLI flag with an error naming both the flag and the order
  that accepts it.
- Put public-surface tests in top-level `tests/`; put tests of private or
  crate-visible items in a `tests/` directory inside their module.
- Extend shared implementations, fixtures, and tables instead of creating a
  second path that must be kept in sync.
- Update [docs/algorithms.md](docs/algorithms.md) when an algorithm or its
  selection policy changes.
- The first build compiles the vendored C++ code. Compiler setup is in
  [docs/building.md](docs/building.md).
