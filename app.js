"use strict";

const numberFormat = new Intl.NumberFormat("en");
const profileDeltas = [0, 1, 2, 4, 8, 16];
const profileStyles = {
  "goatd-portfolio": { color: "#2774ae", dash: "" },
  "goatd-portfolio-refined": { color: "#005587", dash: "8 3" },
  "flowcutter-pace17": { color: "#17232d", dash: "" },
  "htd-default": { color: "#009e73", dash: "9 4" },
  "tamaki-pace17": { color: "#d55e00", dash: "3 3" },
  "jdrasil-heuristic": { color: "#7b5aa6", dash: "12 4 3 4" },
  "networkx-min-fill": { color: "#b97900", dash: "7 3" },
  "networkx-min-degree": { color: "#3288bd", dash: "2 3" },
  "arboretum-heuristic": { color: "#8c4f64", dash: "10 3 2 3" },
};
const fallbackProfileColors = ["#4d4d4d", "#648fff", "#785ef0", "#dc267f", "#fe6100"];

const state = {
  data: null,
  query: "",
  group: "all",
  kind: "all",
  size: "all",
  status: "all",
  aggregateUnit: "components",
  structure: "non-tree",
  sort: "instance",
  direction: "asc",
  page: 1,
  pageSize: 50,
};

function element(tagName, className, textContent) {
  const node = document.createElement(tagName);
  if (className) node.className = className;
  if (textContent !== undefined) node.textContent = textContent;
  return node;
}

function svgElement(tagName, attributes = {}, textContent) {
  const node = document.createElementNS("http://www.w3.org/2000/svg", tagName);
  Object.entries(attributes).forEach(([name, value]) => node.setAttribute(name, String(value)));
  if (textContent !== undefined) node.textContent = textContent;
  return node;
}

function append(parent, ...children) {
  children.forEach((child) => parent.appendChild(child));
  return parent;
}

function formatInteger(value) {
  return Number.isFinite(value) ? numberFormat.format(value) : "—";
}

function formatElapsed(value) {
  if (!Number.isFinite(value)) return "—";
  if (value < 1000) return `${numberFormat.format(value)} ms`;
  if (value < 10000) return `${(value / 1000).toFixed(2)} s`;
  return `${(value / 1000).toFixed(1)} s`;
}

function formatNumber(value) {
  if (!Number.isFinite(value)) return "—";
  return Number.isInteger(value) ? numberFormat.format(value) : value.toFixed(1);
}

function formatPercentage(value) {
  return Number.isFinite(value) ? `${value.toFixed(1)}%` : "—";
}

function formatCountShare(count, total) {
  return total > 0
    ? `${formatInteger(count)}/${formatInteger(total)} (${formatPercentage(BenchmarkStatistics.share(count, total))})`
    : "—";
}

function solverIds() {
  return state.data.solvers.map((solver) => solver.id);
}

function resultFor(instance, solverId) {
  return BenchmarkStatistics.resultFor(instance, solverId);
}

function completedResults(instance) {
  return BenchmarkStatistics.validResults(instance, solverIds());
}

function labelForStatus(status) {
  const labels = {
    ok: "Complete",
    timeout: "Timeout",
    error: "Error",
    invalid: "Invalid",
    skipped: "Skipped",
    unavailable: "Unavailable",
  };
  return labels[status] || String(status).replaceAll("_", " ");
}

function validateData(data) {
  if (!data || typeof data !== "object") throw new Error("The result file is not a JSON object.");
  if (data.schema_version !== 2) throw new Error("The result file does not use schema version 2.");
  if (!data.dataset || !data.run) throw new Error("The result file is missing dataset or run metadata.");
  if (!Array.isArray(data.solvers) || data.solvers.length === 0) throw new Error("The result file has no solvers.");
  if (!Array.isArray(data.instances)) throw new Error("The result file has no graph instance list.");
  if (data.source_instances !== undefined && !Array.isArray(data.source_instances)) {
    throw new Error("The result file has an invalid source aggregate list.");
  }

  const solverIds = new Set();
  data.solvers.forEach((solver) => {
    if (!solver.id || !solver.name) throw new Error("Every solver needs an id and name.");
    if (solverIds.has(solver.id)) throw new Error(`Duplicate solver id: ${solver.id}`);
    solverIds.add(solver.id);
  });

  const instanceIds = new Set();
  [...data.instances, ...(data.source_instances || [])].forEach((instance) => {
    if (!instance.id || !instance.results) {
      throw new Error("Every graph instance needs an id and result map.");
    }
    if (instanceIds.has(instance.id)) throw new Error(`Duplicate graph instance id: ${instance.id}`);
    instanceIds.add(instance.id);
    for (const field of ["vertices", "edges"]) {
      if (!Number.isInteger(instance[field]) || instance[field] < 0) {
        throw new Error(`${instance.id} has an invalid ${field} count.`);
      }
    }
  });
}

async function fetchJson(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`Could not load ${url.pathname} (HTTP ${response.status}).`);
  return response.json();
}

async function loadResultSet(resultUrl) {
  const document = await fetchJson(resultUrl);
  if (document.schema_version !== 3) return document;
  if (!Array.isArray(document.instance_files) || document.instance_files.length === 0) {
    throw new Error("The result index has no instance files.");
  }
  const chunks = await Promise.all(document.instance_files.map(async (entry) => {
    if (!entry || typeof entry.path !== "string" || !Number.isInteger(entry.instances)) {
      throw new Error("The result index contains an invalid instance-file entry.");
    }
    const url = new URL(entry.path, resultUrl);
    if (url.origin !== window.location.origin) {
      throw new Error("Every instance file must be served from the same site as the page.");
    }
    const chunk = await fetchJson(url);
    if (chunk.schema_version !== 3 || !Array.isArray(chunk.instances)) {
      throw new Error(`${entry.path} is not a schema-version-3 instance file.`);
    }
    if (chunk.instances.length !== entry.instances) {
      throw new Error(`${entry.path} has the wrong instance count.`);
    }
    return chunk.instances;
  }));
  const instances = chunks.flat();
  if (instances.length !== document.instance_count) {
    throw new Error("The result index total does not match its instance files.");
  }
  return { ...document, schema_version: 2, instances };
}

function aliasCount(instance) {
  if (Number.isFinite(instance.alias_count)) return instance.alias_count;
  return Array.isArray(instance.aliases) ? instance.aliases.length : 0;
}

function renderMetadata() {
  const { dataset, run, instances, source_instances: sources = [] } = state.data;
  const aliases = instances.reduce((sum, instance) => sum + aliasCount(instance), 0);
  document.querySelector("#dataset-title").textContent = dataset.title;
  document.querySelector("#run-budget").textContent = `${run.timeout_seconds} s / solver`;
  const graphCount = aliases > 0
    ? `${formatInteger(instances.length)} component views · ${formatInteger(aliases)} aliases`
    : `${formatInteger(instances.length)} component views`;
  document.querySelector("#instance-count").textContent = sources.length > 0
    ? `${graphCount} · ${formatInteger(sources.length)} source views`
    : graphCount;

  const generated = new Date(run.generated_at);
  document.querySelector("#generated-at").textContent = Number.isNaN(generated.getTime())
    ? run.generated_at
    : new Intl.DateTimeFormat("en", { dateStyle: "medium", timeZone: "UTC" }).format(generated);

  document.querySelector("#fixture-notice").hidden = !dataset.synthetic;
}

function populateSelect(selector, values) {
  const select = document.querySelector(selector);
  values.forEach((value) => {
    const option = element("option", "", value);
    option.value = value;
    select.appendChild(option);
  });
}

function populateFilters() {
  const groups = [...new Set(state.data.instances.map((instance) => instance.group).filter(Boolean))]
    .sort((a, b) => a.localeCompare(b));
  const kinds = [...new Set(state.data.instances.map((instance) => instance.kind).filter(Boolean))]
    .sort((a, b) => a.localeCompare(b));
  populateSelect("#group-filter", groups);
  populateSelect("#kind-filter", kinds);
  const sourceOption = document.querySelector('#aggregate-unit option[value="sources"]');
  sourceOption.disabled = !Array.isArray(state.data.source_instances)
    || state.data.source_instances.length === 0;
}

function bindControls() {
  document.querySelector("#search").addEventListener("input", (event) => {
    state.query = event.target.value.trim().toLocaleLowerCase();
    state.page = 1;
    render();
  });

  document.querySelector("#group-filter").addEventListener("change", (event) => {
    state.group = event.target.value;
    state.page = 1;
    render();
  });

  document.querySelector("#kind-filter").addEventListener("change", (event) => {
    state.kind = event.target.value;
    state.page = 1;
    render();
  });

  document.querySelector("#size-filter").addEventListener("change", (event) => {
    state.size = event.target.value;
    state.page = 1;
    render();
  });

  document.querySelector("#status-filter").addEventListener("change", (event) => {
    state.status = event.target.value;
    state.page = 1;
    render();
  });

  document.querySelector("#aggregate-unit").addEventListener("change", (event) => {
    state.aggregateUnit = event.target.value;
    render();
  });

  document.querySelector("#structure-filter").addEventListener("change", (event) => {
    state.structure = event.target.value;
    state.page = 1;
    render();
  });

  document.querySelector("#sort-by").addEventListener("change", (event) => {
    state.sort = event.target.value;
    state.page = 1;
    render();
  });

  document.querySelector("#page-size").addEventListener("change", (event) => {
    state.pageSize = Number(event.target.value);
    state.page = 1;
    render();
  });

  document.querySelector("#sort-direction").addEventListener("click", () => {
    state.direction = state.direction === "asc" ? "desc" : "asc";
    const button = document.querySelector("#sort-direction");
    const ascending = state.direction === "asc";
    button.textContent = `${ascending ? "↑" : "↓"} ${ascending ? "Ascending" : "Descending"}`;
    button.setAttribute("aria-label", `Sort ${ascending ? "ascending" : "descending"}`);
    state.page = 1;
    render();
  });

  document.querySelector("#previous-page").addEventListener("click", () => {
    state.page = Math.max(1, state.page - 1);
    render();
    document.querySelector("#matrix-heading").scrollIntoView({ block: "start" });
  });

  document.querySelector("#next-page").addEventListener("click", () => {
    state.page += 1;
    render();
    document.querySelector("#matrix-heading").scrollIntoView({ block: "start" });
  });
}

function outcomeMatches(instance) {
  const results = state.data.solvers.map((solver) => resultFor(instance, solver.id));
  if (state.status === "complete") return results.every((result) => result.status === "ok");
  if (state.status === "failures") {
    return results.some((result) => ["timeout", "error", "invalid", "incomplete"].includes(result.status));
  }
  if (state.status === "unavailable") {
    return results.some((result) => ["skipped", "unavailable"].includes(result.status));
  }
  return true;
}

function bestWidth(instance) {
  const widths = completedResults(instance).map((result) => result.width);
  return widths.length ? Math.min(...widths) : null;
}

function sizeMatches(vertices) {
  if (state.size === "up-to-100") return vertices <= 100;
  if (state.size === "101-500") return vertices >= 101 && vertices <= 500;
  if (state.size === "501-2000") return vertices >= 501 && vertices <= 2000;
  if (state.size === "2001-10000") return vertices >= 2001 && vertices <= 10000;
  if (state.size === "above-10000") return vertices > 10000;
  return true;
}

function sortValue(instance) {
  const values = {
    instance: instance.label || instance.source || instance.id,
    width: bestWidth(instance),
    vertices: instance.vertices,
    edges: instance.edges,
  };
  return values[state.sort];
}

function visibleInstances(collection = state.data.instances, filterStructure = true) {
  const matches = collection.filter((instance) => {
    const searchable = [
      instance.id,
      instance.label,
      instance.source,
      instance.kind,
      instance.group,
      instance.sha256,
      ...(instance.aliases || []),
    ]
      .filter(Boolean)
      .join(" ")
      .toLocaleLowerCase();
    const queryMatches = !state.query || searchable.includes(state.query);
    const groupMatches = state.group === "all" || instance.group === state.group;
    const kindMatches = state.kind === "all" || instance.kind === state.kind;
    const graphSizeMatches = sizeMatches(instance.vertices);
    const structureMatches = !filterStructure
      || state.structure === "all"
      || !BenchmarkStatistics.isTreeComponent(instance);
    return queryMatches
      && groupMatches
      && kindMatches
      && graphSizeMatches
      && structureMatches
      && outcomeMatches(instance);
  });

  return matches.sort((a, b) => {
    const aValue = sortValue(a);
    const bValue = sortValue(b);
    const aMissing = aValue === null || aValue === undefined;
    const bMissing = bValue === null || bValue === undefined;
    if (aMissing !== bMissing) return aMissing ? 1 : -1;

    let comparison;
    if (typeof aValue === "string") comparison = aValue.localeCompare(bValue);
    else comparison = aValue - bValue;
    return state.direction === "asc" ? comparison : -comparison;
  });
}

function solverLabelCell(solver, index = 0) {
  const cell = element("th", "aggregate-solver");
  cell.scope = "row";
  const label = element("span", "aggregate-solver-label");
  const marker = element("i", "solver-marker");
  marker.style.setProperty("--solver-color", profileStyle(solver, index).color);
  append(label, marker, element("span", "aggregate-solver-name", solver.name));
  append(cell, label);
  if (solver.version) append(cell, element("code", "solver-version", solver.version));
  return cell;
}

function qualityHue(fraction) {
  return fraction <= 0.5
    ? 5 + 86 * fraction
    : 48 + 176 * (fraction - 0.5);
}

function relativeQuality(value, values, higherIsBetter) {
  if (!Number.isFinite(value)) return null;
  const finite = values.filter(Number.isFinite);
  if (finite.length === 0) return null;
  const low = Math.min(...finite);
  const high = Math.max(...finite);
  if (low === high) return 0.5;
  const fraction = (value - low) / (high - low);
  return higherIsBetter ? fraction : 1 - fraction;
}

function aggregateCell(value, className = "") {
  return element("td", className, value);
}

function scoreCell(primary, secondary, quality) {
  const cell = element("td", "aggregate-score");
  if (Number.isFinite(quality)) {
    const hue = qualityHue(quality);
    cell.classList.add("has-relative-quality");
    cell.style.setProperty("--summary-quality", `hsl(${hue} 62% 88%)`);
    cell.style.setProperty("--summary-accent", `hsl(${hue} 58% 38%)`);
  }
  append(cell, element("strong", "aggregate-primary", primary));
  if (secondary) append(cell, element("span", "aggregate-secondary", secondary));
  return cell;
}

function renderAggregateTable(instances) {
  const table = document.querySelector("#aggregate-table");
  const ids = solverIds();
  table.replaceChildren();
  append(
    table,
    element(
      "caption",
      "visually-hidden",
      `Aggregate solver statistics over ${instances.length} selected graphs`,
    ),
  );

  const head = element("thead");
  const groups = element("tr", "aggregate-groups");
  const solverHeader = element("th", "", "Solver");
  solverHeader.scope = "col";
  solverHeader.rowSpan = 2;
  append(groups, solverHeader);
  [
    ["Reliability", 1],
    ["Width quality", 3],
    ["Decomposition", 1],
    ["Run", 1],
  ].forEach(([label, span]) => {
    const cell = element("th", "", label);
    cell.scope = "colgroup";
    cell.colSpan = span;
    append(groups, cell);
  });
  const header = element("tr", "aggregate-metrics");
  [
    "Valid",
    "Best observed",
    "Within +1",
    "Median Δ",
    "Total-size excess",
    "Median time",
  ].forEach((label) => {
    const cell = element("th", "", label);
    cell.scope = "col";
    append(header, cell);
  });
  append(head, groups, header);
  append(table, head);

  const rows = state.data.solvers.map((solver, index) => ({
    solver,
    index,
    aggregate: BenchmarkStatistics.aggregateSolver(instances, ids, solver.id),
  }));
  rows.sort((a, b) => (
    b.aggregate.bestWidths - a.aggregate.bestWidths
    || b.aggregate.valid.length - a.aggregate.valid.length
    || (a.aggregate.medianWidthDelta ?? Infinity) - (b.aggregate.medianWidthDelta ?? Infinity)
    || a.solver.name.localeCompare(b.solver.name)
  ));
  const metricValues = {
    valid: rows.map((row) => row.aggregate.valid.length),
    best: rows.map((row) => row.aggregate.bestWidths),
    withinOne: rows.map((row) => row.aggregate.withinOneWidths),
    delta: rows.map((row) => row.aggregate.medianWidthDelta),
    totalSize: rows.map((row) => row.aggregate.medianTotalSizeExcess),
    elapsed: rows.map((row) => row.aggregate.medianElapsed),
  };

  const body = element("tbody");
  rows.forEach(({ solver, index, aggregate }) => {
    const row = element("tr");
    append(
      row,
      solverLabelCell(solver, index),
      scoreCell(
        formatPercentage(BenchmarkStatistics.share(aggregate.valid.length, instances.length)),
        `${formatInteger(aggregate.valid.length)} / ${formatInteger(instances.length)}`,
        relativeQuality(aggregate.valid.length, metricValues.valid, true),
      ),
      scoreCell(
        formatPercentage(BenchmarkStatistics.share(aggregate.bestWidths, instances.length)),
        `${formatInteger(aggregate.bestWidths)} graphs`,
        relativeQuality(aggregate.bestWidths, metricValues.best, true),
      ),
      scoreCell(
        formatPercentage(BenchmarkStatistics.share(aggregate.withinOneWidths, instances.length)),
        `${formatInteger(aggregate.withinOneWidths)} graphs`,
        relativeQuality(aggregate.withinOneWidths, metricValues.withinOne, true),
      ),
      scoreCell(
        formatNumber(aggregate.medianWidthDelta),
        "above best width",
        relativeQuality(aggregate.medianWidthDelta, metricValues.delta, false),
      ),
      scoreCell(
        formatPercentage(aggregate.medianTotalSizeExcess),
        "median excess",
        relativeQuality(aggregate.medianTotalSizeExcess, metricValues.totalSize, false),
      ),
      scoreCell(
        formatElapsed(aggregate.medianElapsed),
        aggregate.valid.length > 0
          ? `${formatInteger(aggregate.atBudget)} at limit`
          : "no valid runs",
        relativeQuality(aggregate.medianElapsed, metricValues.elapsed, false),
      ),
    );
    append(body, row);
  });
  append(table, body);
}

function renderWidthProfile(instances) {
  renderWidthProfileChart(instances);
  const table = document.querySelector("#width-profile-table");
  const ids = solverIds();
  table.replaceChildren();
  append(
    table,
    element(
      "caption",
      "visually-hidden",
      `Width quality profile over ${instances.length} selected graphs`,
    ),
  );

  const head = element("thead");
  const header = element("tr");
  const solverHeader = element("th", "", "Solver");
  solverHeader.scope = "col";
  append(header, solverHeader);
  profileDeltas.forEach((delta) => {
    const cell = element("th", "", delta === 0 ? "Best" : `Δ ≤ ${delta}`);
    cell.scope = "col";
    append(header, cell);
  });
  append(head, header);
  append(table, head);

  const body = element("tbody");
  state.data.solvers.forEach((solver) => {
    const row = element("tr");
    append(row, solverLabelCell(solver, state.data.solvers.indexOf(solver)));
    profileDeltas.forEach((delta) => {
      append(
        row,
        aggregateCell(
          formatPercentage(
            BenchmarkStatistics.widthProfileShare(
              instances,
              ids,
              solver.id,
              delta,
            ),
          ),
          "profile-value",
        ),
      );
    });
    append(body, row);
  });
  append(table, body);
}

function profileStyle(solver, index) {
  return profileStyles[solver.id] || {
    color: fallbackProfileColors[index % fallbackProfileColors.length],
    dash: index % 2 === 0 ? "" : "6 3",
  };
}

function renderWidthProfileChart(instances) {
  const svg = document.querySelector("#width-profile-chart");
  const legend = document.querySelector("#width-profile-legend");
  const ids = solverIds();
  const width = 940;
  const height = 360;
  const margin = { top: 18, right: 22, bottom: 58, left: 64 };
  const plotWidth = width - margin.left - margin.right;
  const plotHeight = height - margin.top - margin.bottom;
  const xDelta = (delta) => margin.left + (plotWidth * delta) / profileDeltas.at(-1);
  const y = (value) => margin.top + plotHeight * (1 - value / 100);

  svg.replaceChildren(
    svgElement("title", { id: "profile-chart-title" }, "Width quality profile"),
    svgElement(
      "desc",
      { id: "profile-chart-description" },
      `Validated width coverage for ${instances.length} selected graph views at six reported excess thresholds.`,
    ),
  );
  legend.replaceChildren();

  [0, 25, 50, 75, 100].forEach((value) => {
    const ordinate = y(value);
    append(
      svg,
      svgElement("line", {
        class: "profile-grid-line",
        x1: margin.left,
        x2: width - margin.right,
        y1: ordinate,
        y2: ordinate,
      }),
      svgElement(
        "text",
        {
          class: "profile-axis-label",
          x: margin.left - 10,
          y: ordinate + 4,
          "text-anchor": "end",
        },
        `${value}%`,
      ),
    );
  });

  profileDeltas.forEach((delta, index) => {
    const abscissa = xDelta(delta);
    append(
      svg,
      svgElement("line", {
        class: "profile-axis-line",
        x1: abscissa,
        x2: abscissa,
        y1: height - margin.bottom,
        y2: height - margin.bottom + 5,
      }),
      svgElement(
        "text",
        {
          class: "profile-axis-label",
          x: abscissa,
          y: height - margin.bottom + 22,
          "text-anchor": "middle",
        },
        String(delta),
      ),
    );
  });

  append(
    svg,
    svgElement("line", {
      class: "profile-axis-line",
      x1: margin.left,
      x2: margin.left,
      y1: margin.top,
      y2: height - margin.bottom,
    }),
    svgElement("line", {
      class: "profile-axis-line",
      x1: margin.left,
      x2: width - margin.right,
      y1: height - margin.bottom,
      y2: height - margin.bottom,
    }),
    svgElement(
      "text",
      {
        class: "profile-axis-title",
        x: margin.left + plotWidth / 2,
        y: height - 9,
        "text-anchor": "middle",
      },
      "Allowed width excess Δ",
    ),
    svgElement(
      "text",
      {
        class: "profile-axis-title",
        transform: `translate(17 ${margin.top + plotHeight / 2}) rotate(-90)`,
        "text-anchor": "middle",
      },
      "Selected graph views",
    ),
  );

  if (instances.length === 0) {
    append(
      svg,
      svgElement(
        "text",
        {
          class: "profile-empty-label",
          x: margin.left + plotWidth / 2,
          y: margin.top + plotHeight / 2,
          "text-anchor": "middle",
        },
        "No graph views match the current filters.",
      ),
    );
    return;
  }

  state.data.solvers.forEach((solver, solverIndex) => {
    const style = profileStyle(solver, solverIndex);
    const chartDeltas = Array.from({ length: profileDeltas.at(-1) + 1 }, (_, delta) => delta);
    const chartValues = chartDeltas.map((delta) =>
      BenchmarkStatistics.widthProfileShare(instances, ids, solver.id, delta));
    const values = profileDeltas.map((delta) => chartValues[delta]);
    const points = chartDeltas.map((delta) => [xDelta(delta), y(chartValues[delta])]);
    const path = points.reduce((segments, [abscissa, ordinate], index) => {
      if (index === 0) return `M${abscissa},${ordinate}`;
      const previousOrdinate = points[index - 1][1];
      return `${segments} L${abscissa},${previousOrdinate} L${abscissa},${ordinate}`;
    }, "");
    const goatdSolver = solver.id.startsWith("goatd-portfolio");
    const pathNode = svgElement("path", {
      class: `profile-series${goatdSolver ? " is-goatd" : ""}`,
      d: path,
      stroke: style.color,
    });
    if (style.dash) pathNode.setAttribute("stroke-dasharray", style.dash);
    append(svg, pathNode);
    profileDeltas.forEach((delta) => {
      append(
        svg,
        svgElement("circle", {
          class: "profile-point",
          cx: xDelta(delta),
          cy: y(chartValues[delta]),
          r: goatdSolver ? 3.5 : 3,
          stroke: style.color,
        }),
      );
    });

    const item = element("div", "profile-legend-item");
    const swatch = svgElement("svg", { viewBox: "0 0 28 8", "aria-hidden": "true" });
    const line = svgElement("line", {
      x1: 0,
      x2: 28,
      y1: 4,
      y2: 4,
      stroke: style.color,
      "stroke-width": goatdSolver ? 3 : 2,
    });
    if (style.dash) line.setAttribute("stroke-dasharray", style.dash);
    append(swatch, line);
    append(
      item,
      swatch,
      element("span", "profile-legend-name", solver.name),
      element("span", "profile-legend-value", formatPercentage(values.at(-1))),
    );
    item.title = `${solver.name}: ${formatPercentage(values.at(-1))} at Δ ≤ ${profileDeltas.at(-1)}`;
    append(legend, item);
  });
}

function renderAggregate(instances, sourceWeighted) {
  const unit = sourceWeighted ? "source graph views" : "component graphs";
  const structureNote = sourceWeighted
    ? " The component-structure filter does not apply to this aggregate."
    : "";
  document.querySelector("#aggregate-scope").textContent = instances.length === 0
    ? `No ${unit} match the current filters.`
    : `${formatInteger(instances.length)} selected ${unit}. Higher coverage and best-width shares are better; lower excess and elapsed values are better.${structureNote}`;
  renderAggregateTable(instances);
  renderWidthProfile(instances);
}

function isBest(instance, result, metric) {
  if (result.status !== "ok" || !Number.isFinite(result[metric])) return false;
  if (metric === "elapsed_ms" && result.budget_reached) return false;
  const values = completedResults(instance)
    .filter((entry) => metric !== "elapsed_ms" || !entry.budget_reached)
    .map((entry) => entry[metric])
    .filter(Number.isFinite);
  return values.length > 0 && result[metric] === Math.min(...values);
}

function detailMetric(label, value, best) {
  const metric = element("div", best ? "detail-metric is-best" : "detail-metric");
  append(metric, element("dt", "", label), element("dd", "", value));
  return metric;
}

function resultCell(instance, solverId) {
  const result = resultFor(instance, solverId);
  const cell = element("td", `result-cell status-${result.status}`);
  const status = element("div", "result-status");
  const statusLabel = result.status === "ok" && result.budget_reached
    ? "At budget"
    : labelForStatus(result.status);
  append(status, element("i", "status-dot"), element("span", "", statusLabel));
  append(cell, status);

  if (result.status !== "ok") {
    const failure = element("div", "failed-result");
    if (Number.isFinite(result.elapsed_ms)) {
      append(failure, element("span", "failed-time", formatElapsed(result.elapsed_ms)));
    }
    if (result.message) append(failure, element("span", "failed-message", result.message));
    append(cell, failure);
    return cell;
  }

  const quality = BenchmarkStatistics.widthQuality(instance, solverIds(), solverId);
  if (quality) {
    const hue = qualityHue(1 - quality.fraction);
    cell.classList.add("has-width-quality");
    cell.style.setProperty("--quality-background", `hsl(${hue} 62% 93%)`);
    cell.style.setProperty("--quality-accent", `hsl(${hue} 58% 38%)`);
  }
  const width = element("div", "width-result");
  append(width, element("span", "width-label", "width"), element("strong", "", formatInteger(result.width)));
  if (quality) {
    const badge = element("span", "quality-badge", quality.delta === 0 ? "best" : `+${formatInteger(quality.delta)}`);
    badge.setAttribute(
      "aria-label",
      quality.delta === 0 ? "Best observed width" : `${quality.delta} above the best observed width`,
    );
    append(width, badge);
  }
  append(cell, width);

  const details = element("dl", "result-details");
  append(
    details,
    detailMetric("total size", formatInteger(result.total_bag_size), isBest(instance, result, "total_bag_size")),
    detailMetric("bags", formatInteger(result.bag_count), isBest(instance, result, "bag_count")),
    detailMetric("time", formatElapsed(result.elapsed_ms), isBest(instance, result, "elapsed_ms")),
  );
  append(cell, details);
  return cell;
}

function instanceHeading(instance) {
  const cell = element("th", "instance-cell");
  cell.scope = "row";
  append(cell, element("span", "instance-label", instance.label || instance.source || instance.id));

  const source = [instance.source, instance.kind].filter(Boolean).join(" / ");
  if (source && source !== instance.label) {
    append(cell, element("code", "instance-source", source));
  }

  append(
    cell,
    element(
      "span",
      "instance-dimensions",
      `${formatInteger(instance.vertices)} vertices · ${formatInteger(instance.edges)} edges`,
    ),
  );

  const aliases = aliasCount(instance);
  if (aliases > 0) {
    append(cell, element("span", "alias-count", `${formatInteger(aliases)} source ${aliases === 1 ? "alias" : "aliases"}`));
  }

  if (instance.kind || instance.group) {
    const tags = element("span", "instance-tags");
    if (instance.kind) append(tags, element("span", "kind-tag", instance.kind));
    if (instance.group) append(tags, element("span", "group-tag", instance.group));
    append(cell, tags);
  }
  return cell;
}

function renderMatrix(instances, totalMatches, firstIndex) {
  const table = document.querySelector("#result-matrix");
  table.replaceChildren();
  const displayStart = totalMatches === 0 ? 0 : firstIndex + 1;
  append(
    table,
    element(
      "caption",
      "visually-hidden",
      `${instances.length} displayed graphs, starting at ${displayStart}, from ${totalMatches} matches`,
    ),
  );

  const head = element("thead");
  const headerRow = element("tr");
  const instanceHeader = element("th", "instance-column", "Graph instance");
  instanceHeader.scope = "col";
  append(headerRow, instanceHeader);

  state.data.solvers.forEach((solver) => {
    const cell = element("th", "solver-column");
    cell.scope = "col";
    append(cell, element("span", "solver-name", solver.name));
    if (solver.version) append(cell, element("code", "solver-version", solver.version));
    append(headerRow, cell);
  });
  append(head, headerRow);
  append(table, head);

  const body = element("tbody");
  if (instances.length === 0) {
    const row = element("tr");
    const cell = element("td", "empty-result", "No graphs match these filters.");
    cell.colSpan = state.data.solvers.length + 1;
    append(row, cell);
    append(body, row);
  } else {
    instances.forEach((instance) => {
      const row = element("tr", "graph-row");
      append(row, instanceHeading(instance));
      state.data.solvers.forEach((solver) => append(row, resultCell(instance, solver.id)));
      append(body, row);
    });
  }
  append(table, body);
}

function renderPagination(totalMatches) {
  const totalPages = Math.max(1, Math.ceil(totalMatches / state.pageSize));
  state.page = Math.min(state.page, totalPages);
  const start = totalMatches === 0 ? 0 : (state.page - 1) * state.pageSize + 1;
  const end = Math.min(state.page * state.pageSize, totalMatches);
  document.querySelector("#previous-page").disabled = state.page === 1;
  document.querySelector("#next-page").disabled = state.page === totalPages || totalMatches === 0;
  document.querySelector("#page-status").textContent = totalMatches === 0
    ? "No rows"
    : `${formatInteger(start)}–${formatInteger(end)} of ${formatInteger(totalMatches)}`;
}

function render() {
  const matches = visibleInstances();
  const sourceWeighted = state.aggregateUnit === "sources"
    && Array.isArray(state.data.source_instances)
    && state.data.source_instances.length > 0;
  const aggregateMatches = sourceWeighted
    ? visibleInstances(state.data.source_instances, false)
    : matches;
  const totalPages = Math.max(1, Math.ceil(matches.length / state.pageSize));
  state.page = Math.min(state.page, totalPages);
  const firstIndex = (state.page - 1) * state.pageSize;
  const instances = matches.slice(firstIndex, firstIndex + state.pageSize);
  document.querySelector("#visible-count").textContent =
    `${matches.length} of ${state.data.instances.length} graphs`;
  renderAggregate(aggregateMatches, sourceWeighted);
  renderMatrix(instances, matches.length, firstIndex);
  renderPagination(matches.length);
}

async function load() {
  try {
    const requestedPath = new URLSearchParams(window.location.search).get("data") || "results.json";
    const resultUrl = new URL(requestedPath, window.location.href);
    if (resultUrl.origin !== window.location.origin) {
      throw new Error("The result file must be served from the same site as this page.");
    }
    const data = await loadResultSet(resultUrl);
    validateData(data);
    state.data = data;
    renderMetadata();
    populateFilters();
    bindControls();
    render();
  } catch (error) {
    const message = document.querySelector("#load-error");
    message.hidden = false;
    message.textContent =
      `${error.message} Serve this directory through a static web server and check the result path.`;
    document.querySelector("#matrix-scroll").hidden = true;
  }
}

load();
