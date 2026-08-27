# Contributing

Bug reports, focused pull requests, documentation corrections, and new graph
families for tests are welcome. Describe the behavior you are changing and why;
for algorithm changes, include the graph shape that exposes the difference.

## Before opening a pull request

Run the same checks as CI:

```sh
cargo test --all-targets && cargo test --doc
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo fmt --check
```

Build setup, including the C++ compiler used for FlowCutter, is documented in
[docs/building.md](docs/building.md).

## Code guidelines

- Every successful graph-to-decomposition entry point returns a valid tree
  decomposition. Parsing, validation, and FlowCutter failures return `Error`;
  panics are reserved for documented programming contracts such as matching a
  weight vector to the graph's vertices.
- The library is single-threaded. Do not add thread pools or spawn threads;
  callers decide how instances are parallelized.
- Budgets and seeds are explicit arguments. Do not read process environment
  variables from library code.
- A CLI flag that has no effect for the selected order is a usage error that
  names the flag and the order that accepts it.
- Keep one implementation and one configuration owner for each operation.
  Extend shared code instead of adding a parallel path.
- Files under `vendor/treedecomp/upstream/` are third-party code. Existing
  changes are marked `// goatd:` and listed in `THIRD-PARTY.md`; prefer making
  new changes in the FFI shim.

## Tests

- Tests using only public or crate-visible items belong in `src/tests/`.
  CLI, format, and other public end-to-end tests belong in the top-level
  `tests/` directory.
- Tests needing private items belong in a `tests/` directory inside the module
  they exercise, such as `src/elimination/tests/`. Do not create a module
  directory whose only content is a test file.
- Keep only the `#[cfg(test)] mod tests;` registration in production files.
- Give each test a sentence-like name for one behavior. Use fixed seeds, no
  sleeps, and small graphs constructed in the test.
- A bug fix includes a regression test that fails on the unfixed parent.

## Documentation and commits

Update user-facing documentation with behavior changes. Keep prose direct and
avoid benchmark numbers that will go stale. Commit subjects are imperative and
name the behavior or contract changed.
