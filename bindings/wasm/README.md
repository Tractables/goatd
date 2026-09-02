# goatd in the browser

<p align="center">
  <a href="https://github.com/Tractables/goatd"><img
     src="https://raw.githubusercontent.com/Tractables/goatd/main/docs/logo.png"
     alt="goatd logo" width="280"></a>
</p>

<p align="center">
  <a href="https://tractables.github.io/goatd/"><img
     src="https://img.shields.io/badge/run-in%20the%20browser-blue"
     alt="Run in the browser"></a>
  <a href="https://github.com/Tractables/goatd/actions/workflows/wasm.yml"><img
     src="https://github.com/Tractables/goatd/actions/workflows/wasm.yml/badge.svg"
     alt="Web build"></a>
  <a href="https://github.com/Tractables/goatd/blob/main/LICENSE"><img
     src="https://img.shields.io/badge/license-Apache--2.0-blue.svg"
     alt="License: Apache-2.0"></a>
</p>

A page that decomposes a PACE `.gr` graph in the tab and draws both the graph
and the decomposition, each lighting up the other under the pointer:
`index.html`, `styles.css`, `app.js`, `worker.js`, `logo.png` and one graph
file, `mcc2025-track1-093.gr`, beside the Emscripten build of the crate. The
solver runs in a worker, so the page stays live during a run and a run can be
cancelled. A row of example graphs runs from a 6×6 grid through the primal
graph of a Model Counting Competition CNF to a grid of 10,000 vertices, a
`.gr` file can be opened with a button or dropped on the page, the `.td`
text can be copied or saved, and the address bar carries the example and the
settings, so a result can be linked to. There is no framework, no
bundler and nothing fetched from anywhere else. It is served at
<https://tractables.github.io/goatd/> and follows `main`, so it may be ahead
of the latest release.

## Build locally

To build it by hand you need the
[Emscripten SDK](https://emscripten.org/docs/getting_started/downloads.html)
on PATH and the Rust target:

```sh
rustup target add wasm32-unknown-emscripten
cd bindings/wasm
CXX_wasm32_unknown_emscripten=em++ \
AR_wasm32_unknown_emscripten=emar \
CXXFLAGS_wasm32_unknown_emscripten=-fwasm-exceptions \
  cargo build --release
mkdir -p site
cp index.html styles.css app.js worker.js logo.png mcc2025-track1-093.gr \
   target/wasm32-unknown-emscripten/release/goatd.{js,wasm} site/
python3 -m http.server -d site
```

Opening `index.html` from the filesystem does not work: the module is fetched,
so the files have to come from a server.

## Publishing

`.github/workflows/wasm.yml` builds it, uploads the files as the `goatd-web`
artifact and, on `main`, commits them at the root of the `gh-pages` branch,
beside the solver comparison under `comparison/`. It stamps the two references
in `index.html` with the commit (`app.js?v=<sha>`), and the page passes the
stamp on to the worker, the module and the graph file it fetches, so a new
page never runs with an older file a browser still holds.
