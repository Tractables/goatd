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

`bindings/python/` and `bindings/c/` each declare an empty `[workspace]`
table, so none of the above touches them. A change under either directory
additionally needs:

```sh
cargo fmt --check --manifest-path bindings/python/Cargo.toml
cargo clippy --manifest-path bindings/python/Cargo.toml --release --locked -- -D warnings

cargo fmt --check --manifest-path bindings/c/Cargo.toml
cargo clippy --manifest-path bindings/c/Cargo.toml --release --all-targets -- -D warnings
```

A change to `bindings/c/src/lib.rs` or `bindings/c/cbindgen.toml` also needs
the committed header checked against cbindgen at the version
`.github/workflows/c-bindings.yml` pins:

```sh
cargo install cbindgen --version 0.29.0 --locked
cbindgen --config bindings/c/cbindgen.toml --crate goatd-c \
         --output /tmp/goatd.h bindings/c
diff -u bindings/c/include/goatd.h /tmp/goatd.h
```

The core crate, the bindings and the citation metadata carry one version, so a
version bump changes `Cargo.toml`, every manifest under `bindings/`, the
`version` in `CITATION.cff` and the version in the `README.md` BibTeX entry.
`.github/scripts/check-versions.sh` checks that, and CI runs it on every
change. `date-released` in `CITATION.cff` is not checked; set it to the date
of the release being tagged.

Build setup, including the C++ compiler used for FlowCutter and the extra
tools the bindings need, is documented in [building.md](building.md).

## Code guidelines

- Every successful graph-to-decomposition entry point returns a valid tree
  decomposition. Invalid caller data, parsing failures, validation failures,
  and FlowCutter failures return `Error`. Panics are limited to documented
  programming contracts.
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

- Tests using only the public API belong in the top-level `tests/` directory.
- Tests needing private or crate-visible items belong in a `tests/` directory
  inside the module they exercise, such as `src/elimination/tests/`. Do not
  create a module directory whose only content is a test file.
- Keep only the `#[cfg(test)] mod tests;` registration in production files.
- Give each test a sentence-like name for one behavior. Use fixed seeds, no
  sleeps, and small graphs constructed in the test.
- A bug fix includes a regression test that fails on the unfixed parent.

## Documentation and commits

Update user-facing documentation with behavior changes. Keep prose direct and
avoid benchmark numbers that will go stale. Commit subjects are imperative and
name the behavior or contract changed.
