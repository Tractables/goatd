# goatd from C and C++

<p align="center">
  <a href="https://github.com/Tractables/goatd"><img
     src="https://raw.githubusercontent.com/Tractables/goatd/main/docs/logo.png"
     alt="goatd logo" width="280"></a>
</p>

<p align="center">
  <a href="https://github.com/Tractables/goatd/releases/latest"><img
     src="https://img.shields.io/github/v/release/Tractables/goatd"
     alt="GitHub release"></a>
  <a href="https://github.com/Tractables/goatd/actions/workflows/c-bindings.yml"><img
     src="https://github.com/Tractables/goatd/actions/workflows/c-bindings.yml/badge.svg"
     alt="C bindings"></a>
  <a href="https://github.com/Tractables/goatd/blob/main/bindings/c/include/goatd.h"><img
     src="https://img.shields.io/badge/API-C%20%2F%20C%2B%2B-blue"
     alt="C and C++ API"></a>
  <a href="https://github.com/Tractables/goatd/blob/main/LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"
     alt="License: Apache-2.0"></a>
</p>

A C ABI over the goatd library: a graph goes in as a vertex count and a flat
edge array, and a tree decomposition comes out as bag offsets, a concatenated
vertex array, and the edges between bags. The orders, budgets, seeds and
refinement are the ones the `goatd` command line exposes.

**The ABI is unstable before 1.0.** Struct layouts, status codes and
signatures can change in any release. Build against the version you ship with
and check `goatd_version()` at run time.

This is a separate crate with its own `Cargo.toml`. It builds the `goatd`
sources beside it and carries the same version, which `goatd_version()`
reports.

## Install from a release

Each GitHub release attaches a `goatd-c-<tag>-<target>` archive
(`.tar.gz` on Linux and macOS, `.zip` on Windows) for each of
`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, and
`x86_64-pc-windows-msvc`. No Rust toolchain is needed to use one: it holds
this header, the shared and static library, `LICENSE`, `THIRD-PARTY.md`, and
`NATIVE-STATIC-LIBS.txt`, the system libraries a static link needs on that
platform (see Linking below).

## Building

```sh
cargo build --release --manifest-path bindings/c/Cargo.toml
```

The first build compiles goatd's vendored C++ FlowCutter with the platform's
own compiler; [docs/building.md](../../docs/building.md) covers the compiler
it looks for. Everything lands in `bindings/c/target/release`:

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| shared | `libgoatd_c.so` | `libgoatd_c.dylib` | `goatd_c.dll`, `goatd_c.dll.lib` |
| static | `libgoatd_c.a` | `libgoatd_c.a` | `goatd_c.lib` |

The header is `bindings/c/include/goatd.h`.

## Linking

Against the shared library nothing else is needed; the C++ runtime is already
linked into it.

```sh
cc -Ibindings/c/include prog.c bindings/c/target/release/libgoatd_c.so -o prog
```

A static link is different: FlowCutter is C++, so the final link has to pull
in the C++ runtime and the system libraries Rust's standard library uses.
`-lstdc++` on Linux, `-lc++` on macOS, and on Windows the MSVC C++ runtime,
which `link.exe` resolves on its own. The exact list for a toolchain comes
from rustc:

```sh
cargo rustc --release --manifest-path bindings/c/Cargo.toml -- \
    --print native-static-libs
```

which on Linux prints something like `-lgcc_s -lutil -lrt -lpthread -lm -ldl
-lc -lstdc++`. Pass that after the archive:

```sh
cc -Ibindings/c/include prog.c bindings/c/target/release/libgoatd_c.a \
   -lstdc++ -lm -lpthread -ldl -o prog
```

On Windows, compile with `/MD`: Rust and the vendored C++ both use the dynamic
CRT, and mixing it with the static one fails at link time.

`goatd-c.pc.in` is a pkg-config template. Fill in the two placeholders when
installing:

```sh
sed -e 's|@PREFIX@|/usr/local|' -e 's|@VERSION@|0.1.2|' \
    bindings/c/goatd-c.pc.in > /usr/local/lib/pkgconfig/goatd-c.pc
```

## Use the API

`example/example.c` builds a graph whose width is known, decomposes it three
ways, checks the bags against the graph, and frees the result. CI compiles and
runs it against both libraries on Linux and macOS and against the DLL on
Windows.

```c
GoatdOptions options = goatd_options_default();
options.order = GOATD_ORDER_PORTFOLIO;
options.budget_ms = 1000;

GoatdDecomposition td;
if (goatd_decompose(num_vertices, edges, num_edges, &options, &td) != GOATD_OK) {
    fprintf(stderr, "%s\n", goatd_last_error_message());
    return 1;
}
for (size_t bag = 0; bag < td.num_bags; bag++) {
    for (size_t i = td.bag_offsets[bag]; i < td.bag_offsets[bag + 1]; i++) {
        use(td.bag_vertices[i]);
    }
}
goatd_decomposition_free(&td);
```

The caller owns the three arrays inside `GoatdDecomposition` and releases them
all with `goatd_decomposition_free`; it owns the struct itself, which goatd
only writes into. Nothing else the API returns is owned by the caller:
`goatd_version()` is static, and the string from
`goatd_last_error_message()` belongs to goatd and lives until the next call on
that thread.

An option that means nothing for the chosen order — a step budget with an
elimination order, say — is an error naming both, not a value that is quietly
dropped.

goatd is single-threaded. Each thread may call it independently, on its own
graph; error messages are recorded per thread.

## Regenerate the header

`include/goatd.h` is generated from `src/lib.rs` by
[cbindgen](https://github.com/mozilla/cbindgen) and committed. CI regenerates
it and fails if the committed copy has drifted, so change `src/lib.rs` and
then:

```sh
cargo install cbindgen --version 0.29.0 --locked
cbindgen --config bindings/c/cbindgen.toml --crate goatd-c \
         --output bindings/c/include/goatd.h bindings/c
```
