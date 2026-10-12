"use strict";

// Shared by the page, worker and module smoke test.
const goatdRuntime = {
  unsigned(value, name) {
    const number = Number(value);
    if (!Number.isInteger(number) || number < 0 || number > 0xffffffff) {
      throw new Error(`${name} must be an integer from 0 to 4294967295`);
    }
    return number;
  },

  decompose(solver, graph, order, seed, budget) {
    order = this.unsigned(order, "order");
    seed = this.unsigned(seed, "seed");
    budget = this.unsigned(budget, "budget");
    const bytes = new TextEncoder().encode(graph + "\0");
    // ccall's string conversion uses the stack, which cannot hold large graphs.
    const input = solver._malloc(bytes.length) >>> 0;
    if (!input) throw new Error("could not allocate graph input");
    let output = 0;
    try {
      solver.HEAPU8.set(bytes, input);
      output = solver._goatd_decompose(input, order, seed, budget) >>> 0;
      if (!output) throw new Error("the solver returned no decomposition");
      return solver.UTF8ToString(output);
    } finally {
      if (output) solver._goatd_string_free(output);
      solver._free(input);
    }
  },
};

if (typeof module !== "undefined") module.exports = goatdRuntime;
