"use strict";

const BenchmarkPresentation = (() => {
  const profileDeltas = Object.freeze([0, 1, 2, 4, 8]);
  const omittedSolverIds = Object.freeze(["goatd-portfolio-refined"]);
  const defaultMinimumBestWidth = 30;
  const aggregateMetricDefinitions = Object.freeze([
    Object.freeze([
      "Valid",
      "A tree decomposition accepted by the common validator. The denominator is every selected graph.",
    ]),
    Object.freeze([
      "Exact best",
      "The solver tied the smallest validated width observed for this graph. The reference is not a proven optimum.",
    ]),
    Object.freeze([
      "Within +1",
      "The solver returned a validated width at most one above the best observed width for this graph.",
    ]),
    Object.freeze([
      "Within +4",
      "The solver returned a validated width at most four above the best observed width for this graph.",
    ]),
  ]);

  function displayedSolvers(solvers) {
    const omitted = new Set(omittedSolverIds);
    return solvers.filter((solver) => !omitted.has(solver.id));
  }

  function defaultInstances(instances, solverIds, statistics) {
    return instances.filter((instance) => {
      const observedWidth = statistics.bestObserved(instance, solverIds, "width");
      return !statistics.isTreeComponent(instance)
        && (!Number.isFinite(observedWidth) || observedWidth >= defaultMinimumBestWidth);
    });
  }

  return Object.freeze({
    aggregateMetricDefinitions,
    defaultInstances,
    defaultMinimumBestWidth,
    displayedSolvers,
    omittedSolverIds,
    profileDeltas,
  });
})();

if (typeof module !== "undefined") module.exports = BenchmarkPresentation;
