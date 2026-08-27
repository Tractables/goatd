# Building

goatd builds with Cargo. Its FlowCutter backend is C++ and needs a C++20
compiler; there are no CMake or system-library dependencies.

```sh
cargo build
cargo test --all-targets
```

`build.rs` tries `g++-14`, `g++-13`, `g++-12`, and then `g++`. Set
`GOATD_CXX` to use another GCC 12-or-newer compiler or a recent Clang:

```sh
GOATD_CXX=clang++ cargo build
```

The minimum supported Rust version is recorded as `rust-version` in
`Cargo.toml`.
