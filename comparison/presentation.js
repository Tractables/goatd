"use strict";

const BenchmarkPresentation = (() => {
  const profileDeltas = Object.freeze([0, 1, 2, 4, 8]);
  const omittedSolverIds = Object.freeze(["goatd-portfolio-refined"]);
  const canonicalPortfolioSolverId = "goatd-portfolio";
  const widthFloorSolverId = "networkx-min-degree";
  const defaultMinimumMinDegreeWidth = 30;
  const aggregateMetricDefinitions = Object.freeze([
    Object.freeze([
      "Nontrivial",
      "A tree decomposition accepted by the common validator that improves on the graph's one-bag width. The denominator is every selected graph.",
    ]),
    Object.freeze([
      "Exact best",
      "The solver tied the smallest counted width observed for this graph. The reference is not a proven optimum.",
    ]),
    Object.freeze([
      "Within +1",
      "The solver returned a counted width at most one above the best observed width for this graph.",
    ]),
    Object.freeze([
      "Within +4",
      "The solver returned a counted width at most four above the best observed width for this graph.",
    ]),
  ]);

  function displayedSolvers(solvers) {
    const omitted = new Set(omittedSolverIds);
    return solvers.filter((solver) => {
      if (omitted.has(solver.id)) return false;
      return !solver.id.startsWith("goatd-portfolio")
        || solver.id === canonicalPortfolioSolverId;
    });
  }

  function minDegreeWidth(instance, statistics) {
    const result = instance.results?.[widthFloorSolverId];
    return statistics.isCountedResult(instance, result)
      ? result.width
      : null;
  }

  function meetsMinimumMinDegreeWidth(instance, minimumWidth, statistics) {
    const width = minDegreeWidth(instance, statistics);
    return minimumWidth === 0 || !Number.isFinite(width) || width >= minimumWidth;
  }

  function defaultInstances(instances, _solverIds, statistics) {
    return instances.filter((instance) => {
      return !statistics.isTreeComponent(instance)
        && meetsMinimumMinDegreeWidth(instance, defaultMinimumMinDegreeWidth, statistics);
    });
  }

  return Object.freeze({
    aggregateMetricDefinitions,
    canonicalPortfolioSolverId,
    defaultInstances,
    defaultMinimumMinDegreeWidth,
    displayedSolvers,
    meetsMinimumMinDegreeWidth,
    minDegreeWidth,
    omittedSolverIds,
    profileDeltas,
    widthFloorSolverId,
  });
})();

if (typeof module !== "undefined") module.exports = BenchmarkPresentation;
