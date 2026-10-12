"use strict";

let solver = null;

// The page passes its version stamp to every dependency.
Promise.resolve().then(() => {
  importScripts("runtime.js" + location.search, "goatd.js" + location.search);
  return createGoatd({ locateFile: file => file + location.search });
}).then(module => {
  solver = module;
  postMessage({ ready: true });
}).catch(failure => postMessage({ loadError: String(failure) }));

onmessage = event => {
  const { graph, order, seed, budget } = event.data;
  try {
    const started = performance.now();
    const td = goatdRuntime.decompose(solver, graph, order, seed, budget);
    postMessage({ td, elapsed: performance.now() - started });
  } catch (failure) {
    postMessage({ error: String(failure) });
  }
};
