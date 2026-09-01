"use strict";

const BenchmarkPresentation = (() => {
  const profileDeltas = Object.freeze([0, 1, 2, 4, 8]);
  const omittedSolverIds = Object.freeze(["goatd-portfolio-refined"]);
  const widthFloorSolverId = "networkx-min-degree";
  const defaultMinimumMinDegreeWidth = 30;
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

  function minDegreeWidth(instance) {
    const result = instance.results?.[widthFloorSolverId];
    return result?.status === "ok" && Number.isFinite(result.width)
      ? result.width
      : null;
  }

  function meetsMinimumMinDegreeWidth(instance, minimumWidth) {
    const width = minDegreeWidth(instance);
    return minimumWidth === 0 || !Number.isFinite(width) || width >= minimumWidth;
  }

  function defaultInstances(instances, _solverIds, statistics) {
    return instances.filter((instance) => {
      return !statistics.isTreeComponent(instance)
        && meetsMinimumMinDegreeWidth(instance, defaultMinimumMinDegreeWidth);
    });
  }

  return Object.freeze({
    aggregateMetricDefinitions,
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
