# Contributing

Build prerequisites: a C++20 compiler ([`README.md`](README.md), *Building*).

## Checks

Before opening a pull request, run:

```sh
cargo test --all-targets && cargo test --doc
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo fmt --check
```

CI runs the same commands, plus a build on the MSRV from `Cargo.toml` and
`cargo package`.

## Code

- Every entry point that takes a graph returns a valid decomposition: each
  vertex and each edge in some bag, and the running intersection property.
  The FlowCutter builder is the one route that can fail, and it returns an
  `Error` that says why.
- The library does not spawn threads and does not read the process
  environment; callers run many instances in parallel and configure a run
  through arguments.
- A search that reads a clock takes its deadline or budget as an argument.
  Everything else is deterministic for a given seed, and a change that alters
  the decomposition a seed produces is a behaviour change to state in the
  commit.
- In the solver, a flag that has no effect under the chosen `--order` is an
  error naming both the flag and the order it needs.
- `vendor/treedecomp/upstream/` is third-party source. It has a few in-place
  fixes, each marked `// goatd:` and listed in `THIRD-PARTY.md`; further
  changes go in `ffi.cpp`.

## Tests

- Tests that use only `pub`/`pub(crate)` items go in `src/tests/`; tests that
  need a module's private items go in `src/<module>/tests/`; the solver and
  the PACE formats are tested through the public surface in `tests/`.
  Production files contain no `#[cfg(test)]` code other than the `mod tests;`
  line.
- A test name states the fact being checked, one per test.
- Fixed seeds; no sleeps, no external binaries beyond the crate's own.
- No third-party test data: fixtures are small graphs written or generated in
  the test.
- A bug fix comes with a regression test that fails on the parent commit, in
  the same commit.

## Docs and commits

- Docs state what exists, what to watch out for, and what is guaranteed. No
  numbers that go stale.
- Commit subjects are imperative and describe the behaviour changed.
