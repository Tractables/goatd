# goatd in the browser

A page that decomposes a PACE `.gr` graph in the tab and draws both the graph
and the decomposition: `index.html`, `styles.css` and `app.js` beside the
Emscripten build of the crate. There is no framework, no bundler and nothing
fetched from anywhere else. It is served at
<https://tractables.github.io/goatd/solve/> and follows `main`, so it may be
ahead of the latest release.

`.github/workflows/wasm.yml` builds it, uploads the files as the `goatd-web`
artifact and, on `main`, commits them under `solve/` on the `gh-pages` branch,
whose root is the solver comparison. To build it by hand you need the
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
cp index.html styles.css app.js \
   target/wasm32-unknown-emscripten/release/goatd.{js,wasm} site/
python3 -m http.server -d site
```

Opening `index.html` from the filesystem does not work: the module is fetched,
so the files have to come from a server.

The decomposition runs on the page's own thread, so the tab is unresponsive
while it works. Give the budget a value you are willing to wait for.
