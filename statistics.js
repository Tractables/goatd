"use strict";

const BenchmarkStatistics = (() => {
  function median(values) {
    if (values.length === 0) return null;
    const sorted = [...values].sort((a, b) => a - b);
    const middle = Math.floor(sorted.length / 2);
    if (sorted.length % 2 === 1) return sorted[middle];
    return (sorted[middle - 1] + sorted[middle]) / 2;
  }

  function resultFor(instance, solverId) {
    return instance?.results?.[solverId] || { status: "unavailable" };
  }

  function validResults(instance, solverIds) {
    return solverIds
      .map((solverId) => resultFor(instance, solverId))
      .filter((result) => result.status === "ok");
  }

  function bestObserved(instance, solverIds, metric) {
    const values = validResults(instance, solverIds)
      .map((result) => result[metric])
      .filter(Number.isFinite);
    return values.length > 0 ? Math.min(...values) : null;
  }

  function widthQuality(instance, solverIds, solverId) {
    const result = resultFor(instance, solverId);
    if (result.status !== "ok" || !Number.isFinite(result.width)) return null;
    const widths = validResults(instance, solverIds)
      .map((entry) => entry.width)
      .filter(Number.isFinite);
    if (widths.length === 0) return null;
    const best = Math.min(...widths);
    const worst = Math.max(...widths);
    return {
      best,
      worst,
      delta: result.width - best,
      fraction: worst === best ? 0 : (result.width - best) / (worst - best),
    };
  }

  function isTreeComponent(instance) {
    return instance?.component_count === undefined
      && Number.isInteger(instance?.vertices)
      && instance.vertices > 1
      && Number.isInteger(instance?.edges)
      && instance.edges === instance.vertices - 1;
  }

  function percentageExcess(value, baseline) {
    if (!Number.isFinite(value) || !Number.isFinite(baseline)) return null;
    if (baseline === 0) return value === 0 ? 0 : null;
    return (100 * (value - baseline)) / baseline;
  }

  function aggregateSolver(instances, solverIds, solverId) {
    const valid = [];
    const widthDeltas = [];
    const totalSizeExcesses = [];
    const bagCountExcesses = [];
    let bestWidths = 0;
    let atBudget = 0;

    instances.forEach((instance) => {
      const result = resultFor(instance, solverId);
      if (result.status !== "ok") return;
      valid.push(result);
      if (result.budget_reached) atBudget += 1;

      const bestWidth = bestObserved(instance, solverIds, "width");
      if (Number.isFinite(bestWidth) && Number.isFinite(result.width)) {
        widthDeltas.push(result.width - bestWidth);
        if (result.width === bestWidth) bestWidths += 1;
      }

      const totalSizeExcess = percentageExcess(
        result.total_bag_size,
        bestObserved(instance, solverIds, "total_bag_size"),
      );
      if (Number.isFinite(totalSizeExcess)) totalSizeExcesses.push(totalSizeExcess);

      const bagCountExcess = percentageExcess(
        result.bag_count,
        bestObserved(instance, solverIds, "bag_count"),
      );
      if (Number.isFinite(bagCountExcess)) bagCountExcesses.push(bagCountExcess);
    });

    return {
      valid,
      bestWidths,
      atBudget,
      medianWidthDelta: median(widthDeltas),
      medianTotalSizeExcess: median(totalSizeExcesses),
      medianBagCountExcess: median(bagCountExcesses),
      medianElapsed: median(valid.map((result) => result.elapsed_ms).filter(Number.isFinite)),
    };
  }

  function share(count, total) {
    return total > 0 ? (100 * count) / total : null;
  }

  function widthProfileShare(instances, solverIds, solverId, maximumDelta) {
    const within = instances.reduce((count, instance) => {
      const result = resultFor(instance, solverId);
      const best = bestObserved(instance, solverIds, "width");
      return result.status === "ok"
        && Number.isFinite(result.width)
        && Number.isFinite(best)
        && result.width - best <= maximumDelta
        ? count + 1
        : count;
    }, 0);
    return share(within, instances.length);
  }

  return Object.freeze({
    aggregateSolver,
    bestObserved,
    median,
    percentageExcess,
    resultFor,
    share,
    isTreeComponent,
    validResults,
    widthQuality,
    widthProfileShare,
  });
})();

if (typeof module !== "undefined") module.exports = BenchmarkStatistics;
