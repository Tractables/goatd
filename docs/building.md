# Building

goatd builds with Cargo. Its FlowCutter backend is C++ and needs a C++20
compiler; there are no CMake or system-library dependencies.

```sh
cargo build
cargo test --all-targets
```

On Linux `build.rs` tries `g++-14`, `g++-13`, `g++-12`, and then `g++`; on
macOS and Windows it uses the platform compiler (Apple clang, MSVC). Set
`GOATD_CXX` to use another GCC 12-or-newer compiler or a recent Clang:

```sh
GOATD_CXX=clang++ cargo build
```

The minimum supported Rust version is recorded as `rust-version` in
`Cargo.toml`.

## Bindings

`bindings/python/`, `bindings/c/` and `bindings/wasm/` are Rust crates in
their own right, each with an empty `[workspace]` table so they build apart
from the crate above. Building any of them needs a Rust toolchain, even when
the consumer's own project is Python, C or a web page, plus the same C++20
compiler for FlowCutter described above.

The Python extension additionally needs [maturin](https://www.maturin.rs). The
C header is generated from `bindings/c/src/lib.rs` by
[cbindgen](https://github.com/mozilla/cbindgen); use the version pinned in
`.github/workflows/c-bindings.yml` (currently 0.29.0), since a different
version can produce a header that differs from the one committed to
`bindings/c/include/goatd.h`.

The browser build needs the
[Emscripten SDK](https://emscripten.org/docs/getting_started/downloads.html)
on PATH and the `wasm32-unknown-emscripten` target. rustc links that target
with Wasm exceptions, so the vendored C++ has to be compiled with
`-fwasm-exceptions` to match; `bindings/wasm/README.md` has the commands.
