"use strict";

// The solver in a worker of its own, so the page stays live while a
// construction runs, and a run can be stopped: the page ends this worker and
// starts another. One message in per run, {graph, order, seed, budget}; out,
// {ready: true} once the module is up, then {td, elapsed} per run, or {error}
// when the call throws.
importScripts("goatd.js");

let solver = null;

createGoatd().then((module) => {
  solver = module;
  postMessage({ ready: true });
});

onmessage = (event) => {
  const { graph, order, seed, budget } = event.data;
  try {
    const started = performance.now();
    const pointer = solver.ccall(
      "goatd_decompose",
      "number",
      ["string", "number", "number", "number"],
      [graph, order, seed, budget],
    );
    const td = solver.UTF8ToString(pointer);
    solver.ccall("goatd_string_free", null, ["number"], [pointer]);
    postMessage({ td, elapsed: performance.now() - started });
  } catch (failure) {
    postMessage({ error: String(failure) });
  }
};
