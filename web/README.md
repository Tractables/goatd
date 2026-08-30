# goatd in the browser

A page that decomposes a PACE `.gr` graph in the tab: `index.html` beside the
Emscripten build of the crate. There is no framework and no bundler.

`.github/workflows/wasm.yml` builds it and uploads the three files as the
`goatd-web` artifact. To build it by hand you need the
[Emscripten SDK](https://emscripten.org/docs/getting_started/downloads.html)
on PATH and the Rust target:

```sh
rustup target add wasm32-unknown-emscripten
cd web
CXX_wasm32_unknown_emscripten=em++ \
AR_wasm32_unknown_emscripten=emar \
CXXFLAGS_wasm32_unknown_emscripten=-fexceptions \
  cargo build --release
mkdir -p site
cp index.html target/wasm32-unknown-emscripten/release/goatd.{js,wasm} site/
python3 -m http.server -d site
```

Opening `index.html` from the filesystem does not work: the module is fetched,
so the files have to come from a server.

The decomposition runs on the page's own thread, so the tab is unresponsive
while it works. Give the budget a value you are willing to wait for.
