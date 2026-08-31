"use strict";

// The page in one file: readers for the two PACE formats, a layout for the
// graph, a layout for the decomposition, SVG for both, and the calls into the
// Wasm module. Nothing is loaded from anywhere else.

// A drawing past these sizes is a grey smear, so the panel says so instead.
const MAX_DRAWN_VERTICES = 500;
const MAX_DRAWN_EDGES = 1500;
const MAX_DRAWN_BAGS = 200;
// Each panel offers to draw past those anyway. The graph layout is quadratic
// in the vertices, in time and in memory, so past this many the offer is not
// made: the tab would be gone for minutes.
const MAX_LAID_OUT_VERTICES = 3000;
// A `.td` text past this many bytes is kept for Copy and Save but not put on
// the page, where a construction gone wrong on a large graph can return tens
// of megabytes.
const MAX_SHOWN_OUTPUT = 2 * 1024 * 1024;

// ------------------------------------------------------------------ reading

function isPositiveInteger(value) {
  return Number.isInteger(value) && value >= 1;
}

// A PACE `.gr` graph: `c` comment lines, a `p tw <vertices> <edges>` line, and
// one edge per line as a pair of 1-based vertex numbers.
function parseGr(text) {
  const edges = [];
  const seen = new Set();
  let count = 0;
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === "" || line.startsWith("c")) continue;
    const fields = line.split(/\s+/);
    if (fields[0] === "p") {
      const declared = Number(fields[2]);
      if (!Number.isInteger(declared) || declared < 0) {
        return { error: `line ${i + 1} has no vertex count` };
      }
      count = Math.max(count, declared);
      continue;
    }
    const u = Number(fields[0]);
    const v = Number(fields[1]);
    if (fields.length !== 2 || !isPositiveInteger(u) || !isPositiveInteger(v)) {
      return { error: `line ${i + 1} is not a comment, a p line or an edge` };
    }
    count = Math.max(count, u, v);
    // A self-loop or a repeat draws nothing new.
    const key = u < v ? `${u}:${v}` : `${v}:${u}`;
    if (u !== v && !seen.has(key)) {
      seen.add(key);
      edges.push([u, v]);
    }
  }
  return { count, edges };
}

// A PACE `.td`: `s td <bags> <largest bag> <vertices>`, then `b <i> <v>...` per
// bag, then one tree edge per line as a pair of bag numbers.
function parseTd(text) {
  const bags = new Map();
  const edges = [];
  let header = null;
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === "" || line.startsWith("c")) continue;
    const fields = line.split(/\s+/);
    if (fields[0] === "s") {
      header = {
        bags: Number(fields[2]),
        largest: Number(fields[3]),
        vertices: Number(fields[4]),
      };
      continue;
    }
    if (fields[0] === "b") {
      const id = Number(fields[1]);
      const vertices = fields.slice(2).map(Number);
      if (!isPositiveInteger(id) || !vertices.every(isPositiveInteger)) {
        return { error: `line ${i + 1} is not a bag` };
      }
      bags.set(id, vertices);
      continue;
    }
    const a = Number(fields[0]);
    const b = Number(fields[1]);
    if (fields.length !== 2 || !isPositiveInteger(a) || !isPositiveInteger(b)) {
      return { error: `line ${i + 1} is not a bag or a tree edge` };
    }
    edges.push([a, b]);
  }
  if (header === null) return { error: "there is no s line" };
  return { header, bags, edges };
}

// ------------------------------------------------------------ graph layout

// A small deterministic generator, so the same graph text always draws the
// same picture.
function mulberry32(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// The number of edges apart every pair of vertices is, by breadth-first search
// from each of them in turn, as one row of `count` numbers per vertex. Two
// vertices in different components are given a distance somewhat past the
// longest path in the graph, which separates the components without letting
// them drift arbitrarily far apart.
function graphDistances(count, edges) {
  const degree = new Int32Array(count + 2);
  for (const [u, v] of edges) {
    degree[u]++;
    degree[v]++;
  }
  const start = new Int32Array(count + 2);
  let total = 0;
  for (let v = 1; v <= count; v++) {
    start[v] = total;
    total += degree[v];
  }
  start[count + 1] = total;
  const next = start.slice();
  const neighbours = new Int32Array(total);
  for (const [u, v] of edges) {
    neighbours[next[u]++] = v;
    neighbours[next[v]++] = u;
  }

  const distance = new Float64Array(count * count);
  const queue = new Int32Array(count);
  let longest = 1;
  for (let source = 1; source <= count; source++) {
    const row = (source - 1) * count;
    distance.fill(-1, row, row + count);
    distance[row + source - 1] = 0;
    queue[0] = source;
    for (let head = 0, tail = 1; head < tail; head++) {
      const at = queue[head];
      for (let i = start[at]; i < start[at + 1]; i++) {
        const to = neighbours[i];
        if (distance[row + to - 1] >= 0) continue;
        distance[row + to - 1] = distance[row + at - 1] + 1;
        longest = Math.max(longest, distance[row + to - 1]);
        queue[tail++] = to;
      }
    }
  }
  for (let i = 0; i < distance.length; i++) {
    if (distance[i] < 0) distance[i] = longest * 1.5;
  }
  return distance;
}

// Stress majorization: move each vertex to the place that best agrees with
// where all the others would like it to be, given the number of edges between
// them, weighting a pair by 1/distance^2 so short distances matter most. Every
// round lowers the disagreement, and a few hundred rounds settle it.
//
// Vertices start on a circle, jittered so that a symmetric graph does not sit
// in a perfectly balanced position that the update cannot leave. The jitter
// comes from a fixed seed, so the same graph always draws the same way.
//
// Both the distances and the rounds are quadratic in the vertex count, which
// is what MAX_DRAWN_VERTICES keeps in hand.
function layoutGraph(count, edges, rounds = 300) {
  const xs = new Float64Array(count + 1);
  const ys = new Float64Array(count + 1);
  if (count === 0) return { xs, ys };

  const random = mulberry32(0x9e3779b9);
  for (let v = 1; v <= count; v++) {
    const angle = (2 * Math.PI * (v - 1)) / count;
    xs[v] = Math.cos(angle) + 0.05 * (random() - 0.5);
    ys[v] = Math.sin(angle) + 0.05 * (random() - 0.5);
  }
  if (count === 1) {
    xs[1] = 0.5;
    ys[1] = 0.5;
    return { xs, ys };
  }

  const distance = graphDistances(count, edges);
  const weight = new Float64Array(count * count);
  const rowWeight = new Float64Array(count + 1);
  for (let i = 1; i <= count; i++) {
    const row = (i - 1) * count;
    let sum = 0;
    for (let j = 1; j <= count; j++) {
      if (i === j) continue;
      const target = distance[row + j - 1];
      weight[row + j - 1] = 1 / (target * target);
      sum += weight[row + j - 1];
    }
    rowWeight[i] = sum;
  }

  const nextX = new Float64Array(count + 1);
  const nextY = new Float64Array(count + 1);
  for (let round = 0; round < rounds; round++) {
    for (let i = 1; i <= count; i++) {
      const row = (i - 1) * count;
      const x = xs[i];
      const y = ys[i];
      let sumX = 0;
      let sumY = 0;
      for (let j = 1; j <= count; j++) {
        if (i === j) continue;
        const ex = x - xs[j];
        const ey = y - ys[j];
        let length = Math.sqrt(ex * ex + ey * ey);
        if (length < 1e-9) length = 1e-9;
        const pull = distance[row + j - 1] / length;
        sumX += weight[row + j - 1] * (xs[j] + ex * pull);
        sumY += weight[row + j - 1] * (ys[j] + ey * pull);
      }
      nextX[i] = sumX / rowWeight[i];
      nextY[i] = sumY / rowWeight[i];
    }
    xs.set(nextX);
    ys.set(nextY);
  }

  // Stress majorization fixes the shape but not which way up it is, and it
  // usually lands askew. Turning it so that its bounding box is as small as it
  // can be squares up a grid and lays a long graph flat.
  let turn = 0;
  let smallest = Infinity;
  for (let degrees = 0; degrees < 90; degrees++) {
    const angle = (degrees * Math.PI) / 180;
    const cos = Math.cos(angle);
    const sin = Math.sin(angle);
    let lowX = Infinity;
    let highX = -Infinity;
    let lowY = Infinity;
    let highY = -Infinity;
    for (let v = 1; v <= count; v++) {
      const x = xs[v] * cos - ys[v] * sin;
      const y = xs[v] * sin + ys[v] * cos;
      lowX = Math.min(lowX, x);
      highX = Math.max(highX, x);
      lowY = Math.min(lowY, y);
      highY = Math.max(highY, y);
    }
    const area = (highX - lowX) * (highY - lowY);
    if (area < smallest - 1e-12) {
      smallest = area;
      turn = angle;
    }
  }
  const cos = Math.cos(turn);
  const sin = Math.sin(turn);
  for (let v = 1; v <= count; v++) {
    const x = xs[v] * cos - ys[v] * sin;
    ys[v] = xs[v] * sin + ys[v] * cos;
    xs[v] = x;
  }

  // Fit the drawing to the unit square, scaling both axes by the same factor
  // so it is not stretched, and centre what is left over.
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  for (let v = 1; v <= count; v++) {
    minX = Math.min(minX, xs[v]);
    maxX = Math.max(maxX, xs[v]);
    minY = Math.min(minY, ys[v]);
    maxY = Math.max(maxY, ys[v]);
  }
  const extent = Math.max(maxX - minX, maxY - minY, 1e-9);
  const shiftX = (extent - (maxX - minX)) / 2;
  const shiftY = (extent - (maxY - minY)) / 2;
  for (let v = 1; v <= count; v++) {
    xs[v] = (xs[v] - minX + shiftX) / extent;
    ys[v] = (ys[v] - minY + shiftY) / extent;
  }
  return { xs, ys };
}

// ------------------------------------------------------------- tree layout

const BAG_HEIGHT = 24;
const BAG_PADDING = 9;
const BAG_GAP = 14;
const ROW_HEIGHT = 48;
// One character of the 11px monospace the bag labels are set in.
const CHARACTER_WIDTH = 6.7;

// Each bag to the bags joined to it. A tree edge naming a bag the file does
// not list is dropped.
function adjacency(bags, edges) {
  const neighbours = new Map();
  for (const id of bags.keys()) neighbours.set(id, []);
  for (const [a, b] of edges) {
    if (!neighbours.has(a) || !neighbours.has(b)) continue;
    neighbours.get(a).push(b);
    neighbours.get(b).push(a);
  }
  return neighbours;
}

// Bags become rows by depth. Each subtree is given a horizontal span wide
// enough for its children side by side, and a bag is centred over its own
// span, which puts it over the middle of its children.
function layoutTree(bags, edges) {
  const neighbours = adjacency(bags, edges);
  const nodes = new Map();
  for (const [id, vertices] of bags) {
    const label = vertices.join(" ");
    nodes.set(id, {
      id,
      vertices,
      label,
      width: Math.max(30, label.length * CHARACTER_WIDTH + 2 * BAG_PADDING),
      children: [],
      x: 0,
      y: 0,
    });
  }

  // The largest bag roots the drawing. Anything the walk from it does not
  // reach gets its own root, so a `.td` that is not connected still draws.
  const bySize = [...nodes.keys()].sort(
    (a, b) => nodes.get(b).vertices.length - nodes.get(a).vertices.length || a - b,
  );
  const largest = bySize.length === 0 ? 0 : nodes.get(bySize[0]).vertices.length;
  const reached = new Set();
  const roots = [];
  for (const root of bySize) {
    if (reached.has(root)) continue;
    roots.push(root);
    reached.add(root);
    // Breadth-first, so every bag hangs off the neighbour nearest the root.
    for (let queue = [root], at = 0; at < queue.length; at++) {
      for (const next of neighbours.get(queue[at])) {
        if (reached.has(next)) continue;
        reached.add(next);
        nodes.get(queue[at]).children.push(next);
        queue.push(next);
      }
    }
  }

  const spans = new Map();
  const measure = (id) => {
    const node = nodes.get(id);
    let children = 0;
    for (const child of node.children) children += measure(child) + BAG_GAP;
    const span = Math.max(node.width, Math.max(0, children - BAG_GAP));
    spans.set(id, span);
    return span;
  };

  let depth = 0;
  const place = (id, left, row) => {
    const node = nodes.get(id);
    node.y = row * ROW_HEIGHT;
    node.x = left + (spans.get(id) - node.width) / 2;
    depth = Math.max(depth, row);
    let children = 0;
    for (const child of node.children) children += spans.get(child) + BAG_GAP;
    let cursor = left + (spans.get(id) - Math.max(0, children - BAG_GAP)) / 2;
    for (const child of node.children) {
      place(child, cursor, row + 1);
      cursor += spans.get(child) + BAG_GAP;
    }
  };

  let cursor = 0;
  for (const root of roots) {
    measure(root);
    place(root, cursor, 0);
    cursor += spans.get(root) + 3 * BAG_GAP;
  }

  return {
    nodes,
    neighbours,
    largest,
    width: Math.max(0, cursor - 3 * BAG_GAP),
    height: roots.length === 0 ? 0 : depth * ROW_HEIGHT + BAG_HEIGHT,
  };
}

// ------------------------------------------------------------------ drawing

// Two decimals is finer than a pixel at these sizes and keeps the markup short.
function round(value) {
  return Math.round(value * 100) / 100;
}

function graphSvg(count, edges, layout, large = false) {
  // The viewBox is about the width the panel gives the drawing, so the labels
  // come out near the size the stylesheet asks for. A large drawing, one past
  // the cap and asked for anyway, takes its natural size instead, about
  // twenty pixels a vertex, and scrolls.
  const size = large ? Math.round(Math.sqrt(count) * 21) : 420;
  const labelled = count <= 90;
  const radius = labelled ? 8 : 4;
  const inset = radius + 6;
  const scale = size - 2 * inset;
  const at = (values, v) => round(inset + values[v] * scale);

  const parts = [
    `<svg class="drawing graph${large ? " large" : ""}" viewBox="0 0 ${size} ${size}"`,
    large ? ` width="${size}" height="${size}"` : "",
    ` role="img" aria-label="the input graph"><g class="edges">`,
  ];
  for (const [u, v] of edges) {
    const ends =
      ` x1="${at(layout.xs, u)}" y1="${at(layout.ys, u)}"` +
      ` x2="${at(layout.xs, v)}" y2="${at(layout.ys, v)}"`;
    // The drawn line has a wide transparent twin, so the edge can be hovered
    // without aiming at a hair line.
    parts.push(`<g class="edge" data-u="${u}" data-v="${v}">`);
    parts.push(`<title>edge ${u} ${v}</title>`);
    parts.push(`<line class="ink"${ends}/><line class="hit"${ends}/></g>`);
  }
  parts.push("</g>");
  for (let v = 1; v <= count; v++) {
    const x = at(layout.xs, v);
    const y = at(layout.ys, v);
    parts.push(`<g class="vertex" data-vertex="${v}">`);
    parts.push(`<title>vertex ${v}</title>`);
    parts.push(`<circle cx="${x}" cy="${y}" r="${radius}"/>`);
    if (labelled) parts.push(`<text x="${x}" y="${y}" dy=".33em">${v}</text>`);
    parts.push("</g>");
  }
  parts.push("</svg>");
  return parts.join("");
}

function treeSvg(tree) {
  const inset = 10;
  const width = round(tree.width + 2 * inset);
  const height = round(tree.height + 2 * inset);

  const parts = [
    `<svg class="drawing tree" width="${width}" height="${height}"`,
    ` viewBox="0 0 ${width} ${height}"`,
    ` role="img" aria-label="the tree decomposition"><g class="tree-edges">`,
  ];
  for (const node of tree.nodes.values()) {
    const x1 = round(inset + node.x + node.width / 2);
    const y1 = round(inset + node.y + BAG_HEIGHT);
    for (const id of node.children) {
      const child = tree.nodes.get(id);
      parts.push(
        `<line data-a="${node.id}" data-b="${id}" x1="${x1}" y1="${y1}"`,
        ` x2="${round(inset + child.x + child.width / 2)}"`,
        ` y2="${round(inset + child.y)}"/>`,
      );
    }
  }
  parts.push("</g>");
  // The bags of the largest size are the ones that set the width. When every
  // bag is that size there is nothing to single out.
  const singled = [...tree.nodes.values()].some((node) => node.vertices.length < tree.largest);
  for (const node of tree.nodes.values()) {
    const x = round(inset + node.x);
    const y = round(inset + node.y);
    const widest = singled && node.vertices.length === tree.largest;
    parts.push(`<g class="bag${widest ? " widest" : ""}" data-bag="${node.id}">`);
    parts.push(`<title>bag ${node.id}</title>`);
    parts.push(
      `<rect x="${x}" y="${y}" width="${round(node.width)}"`,
      ` height="${BAG_HEIGHT}" rx="5"/>`,
    );
    parts.push(
      `<text x="${round(inset + node.x + node.width / 2)}"`,
      ` y="${round(y + BAG_HEIGHT / 2)}" dy=".33em">${node.label}</text>`,
    );
    parts.push("</g>");
  }
  parts.push("</svg>");
  return parts.join("");
}

// "s td <bags> <largest bag> <vertices>"; the width is one less than the
// largest bag.
// The three figures of a result, each a number with its word beside it.
function summarise(header, elapsed) {
  const stat = (figure, word) => `<span class="stat"><b>${figure}</b>${word}</span>`;
  const ms = elapsed < 1 ? "&lt;1" : String(Math.round(elapsed));
  return stat(header.largest - 1, "width") + stat(header.bags, "bags") + stat(ms, "ms");
}

// ----------------------------------------------------------------- examples

// The menu of example graphs, each generated when chosen, so the page carries
// the recipe and not the text. The first is the graph the page opens with.
// The last three are past what the page draws; they show the solver at work.
const EXAMPLES = [
  ["6×6 grid", () => grid(6, 6, "a 6x6 grid; its treewidth is 6")],
  ["Petersen graph", petersen],
  ["random 3-tree, 40 vertices", () => kTree(3, 40, 1)],
  ["5-dimensional hypercube", () => hypercube(5)],
  ["20×20 grid", () => grid(20, 20, "a 20x20 grid; its treewidth is 20")],
  ["7×7×7 grid", () => cubeGrid(7)],
  ["random 4-tree, 2,000 vertices", () => kTree(4, 2000, 1)],
  ["random sparse graph, 3,000 vertices", () => randomGraph(3000, 4500, 1)],
  ["100×100 grid, 10,000 vertices", () => grid(100, 100, "a 100x100 grid; its treewidth is 100")],
];

// The `.gr` text of a graph: a comment naming it, the problem line, then an
// edge per line with the smaller vertex first.
function gr(comment, count, edges) {
  const lines = [`c ${comment}`, `p tw ${count} ${edges.length}`];
  for (const [u, v] of edges) lines.push(u < v ? `${u} ${v}` : `${v} ${u}`);
  return lines.join("\n") + "\n";
}

// Vertices row by row, each joined to the one on its right and the one below.
function grid(rows, columns, comment) {
  const edges = [];
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < columns; c++) {
      const v = r * columns + c + 1;
      if (c + 1 < columns) edges.push([v, v + 1]);
      if (r + 1 < rows) edges.push([v, v + columns]);
    }
  }
  return gr(comment, rows * columns, edges);
}

// An n×n×n grid: each vertex joined to the next one along each axis.
function cubeGrid(n) {
  const at = (x, y, z) => (x * n + y) * n + z + 1;
  const edges = [];
  for (let x = 0; x < n; x++) {
    for (let y = 0; y < n; y++) {
      for (let z = 0; z < n; z++) {
        if (x + 1 < n) edges.push([at(x, y, z), at(x + 1, y, z)]);
        if (y + 1 < n) edges.push([at(x, y, z), at(x, y + 1, z)]);
        if (z + 1 < n) edges.push([at(x, y, z), at(x, y, z + 1)]);
      }
    }
  }
  return gr(`a ${n}x${n}x${n} grid, ${n * n * n} vertices`, n * n * n, edges);
}

// The d-dimensional hypercube: the vertices are the d-bit numbers, joined
// when they differ in one bit.
function hypercube(d) {
  const count = 1 << d;
  const edges = [];
  for (let u = 0; u < count; u++) {
    for (let b = 0; b < d; b++) {
      const v = u ^ (1 << b);
      if (u < v) edges.push([u + 1, v + 1]);
    }
  }
  return gr(`the ${d}-dimensional hypercube, ${count} vertices`, count, edges);
}

// The outer 5-cycle, the inner pentagram, and a spoke between each pair.
function petersen() {
  const edges = [];
  for (let i = 0; i < 5; i++) {
    edges.push([i + 1, ((i + 1) % 5) + 1]);
    edges.push([i + 6, ((i + 2) % 5) + 6]);
    edges.push([i + 1, i + 6]);
  }
  return gr("the Petersen graph; its treewidth is 4", 10, edges);
}

// A random k-tree: a clique of k + 1 vertices, then each further vertex
// joined to every vertex of a k-clique already there, chosen at random. Its
// treewidth is exactly k, so the answer is known however large it gets.
function kTree(k, count, seed) {
  const random = mulberry32(seed);
  const edges = [];
  const cliques = [];
  const first = Array.from({ length: k + 1 }, (_, i) => i + 1);
  for (let i = 0; i <= k; i++) {
    for (let j = i + 1; j <= k; j++) edges.push([first[i], first[j]]);
    cliques.push(first.filter((_, at) => at !== i));
  }
  for (let v = k + 2; v <= count; v++) {
    const clique = cliques[Math.floor(random() * cliques.length)];
    for (const u of clique) {
      edges.push([u, v]);
      cliques.push([...clique.filter((w) => w !== u), v]);
    }
  }
  const comment = `a random ${k}-tree on ${count} vertices; its treewidth is exactly ${k}`;
  return gr(comment, count, edges);
}

// A random graph with the given number of edges, no two alike and none a
// loop. A vertex may be left with no edge at all.
function randomGraph(count, size, seed) {
  const random = mulberry32(seed);
  const taken = new Set();
  const edges = [];
  while (edges.length < size) {
    const u = 1 + Math.floor(random() * count);
    const v = 1 + Math.floor(random() * count);
    if (u === v) continue;
    const key = Math.min(u, v) * (count + 1) + Math.max(u, v);
    if (taken.has(key)) continue;
    taken.add(key);
    edges.push([u, v]);
  }
  const comment = `a random graph on ${count} vertices with ${size} edges, seed ${seed}`;
  return gr(comment, count, edges);
}

// --------------------------------------------------------------- the page

if (typeof document !== "undefined") {
  const element = (id) => document.getElementById(id);
  const graphView = element("graph-view");
  const treeView = element("tree-view");
  let solver = null;
  let drawnGraph = "";
  // What hovering needs of the decomposition on show: the bags, which bags
  // are joined, and which bags hold each vertex. Null while none is drawn.
  let shown = null;
  // The `.td` text of the last run, whole, whether or not it is on the page.
  let result = "";
  // The graph in the box and the decomposition of the last run, read; either
  // is null when there is nothing to draw.
  let parsedGraph = null;
  let parsedTree = null;

  createGoatd()
    .then((module) => {
      solver = module;
      element("run").disabled = false;
      element("status").textContent = "ready";
      // The page opens on an example; show what it gives without a click.
      if (examples.querySelector(".on") !== null && shown === null) element("run").click();
    })
    .catch((failure) => {
      element("status").textContent = `the solver did not load: ${failure}`;
    });

  // A note in a panel, with a button to draw what was held back when there
  // is something to draw.
  const note = (text, anyway = false) =>
    `<p class="note">${text}${anyway ? ' <button type="button" class="anyway">Draw it anyway</button>' : ""}</p>`;

  function drawGraph() {
    const text = element("graph").value;
    if (text === drawnGraph) return;
    drawnGraph = text;
    clearHighlight();
    // Whatever is on show belongs to the graph that was there before.
    treeView.innerHTML = note("Press Decompose.");
    element("legend").hidden = true;
    shown = null;
    parsedTree = null;
    element("output").textContent = "";
    result = "";
    element("result-summary").textContent = "";
    element("result-summary").classList.remove("failed");
    element("raw").open = false;
    element("copy").disabled = true;
    element("save").disabled = true;
    if (solver !== null) element("status").textContent = "ready";

    parsedGraph = null;
    vertexElements = new Map();
    const graph = parseGr(text);
    if (graph.error !== undefined) {
      graphView.innerHTML = note(`Not drawn: ${graph.error}.`);
      return;
    }
    if (graph.count === 0) {
      graphView.innerHTML = note("Nothing to draw yet.");
      return;
    }
    parsedGraph = graph;
    if (graph.count > MAX_DRAWN_VERTICES || graph.edges.length > MAX_DRAWN_EDGES) {
      const offered = graph.count <= MAX_LAID_OUT_VERTICES;
      graphView.innerHTML = note(
        `Not drawn: ${graph.count} vertices and ${graph.edges.length} edges is` +
          " past what is readable here." +
          (offered ? " The layout takes a while at this size." : ""),
        offered,
      );
      return;
    }
    renderGraph(false);
  }

  function renderGraph(large) {
    const { count, edges } = parsedGraph;
    graphView.innerHTML = graphSvg(count, edges, layoutGraph(count, edges), large);
    vertexElements = new Map();
    for (const vertex of graphView.querySelectorAll(".vertex")) {
      vertexElements.set(vertex.dataset.vertex, vertex);
    }
  }

  function drawTree() {
    element("legend").hidden = true;
    shown = null;
    if (parsedTree.bags.size > MAX_DRAWN_BAGS) {
      treeView.innerHTML = note(
        `Not drawn: ${parsedTree.bags.size} bags is past what is readable here.`,
        true,
      );
      return;
    }
    renderTree();
  }

  function renderTree() {
    const tree = layoutTree(parsedTree.bags, parsedTree.edges);
    treeView.innerHTML = treeSvg(tree);
    element("legend").hidden = treeView.querySelector(".bag.widest") === null;
    fitTree();
    const holders = new Map();
    for (const [id, vertices] of parsedTree.bags) {
      for (const v of vertices) {
        if (!holders.has(v)) holders.set(v, []);
        holders.get(v).push(id);
      }
    }
    const bagElements = new Map();
    for (const bag of treeView.querySelectorAll(".bag")) bagElements.set(bag.dataset.bag, bag);
    shown = { bags: parsedTree.bags, neighbours: tree.neighbours, holders, bagElements };
  }

  // Runs `work` after the browser has had a frame to paint what was just put
  // on the page, since the work holds the tab. A tab in the background paints
  // no frame, so a timer starts it there instead; whichever comes first, once.
  function afterPaint(work) {
    let started = false;
    const start = () => {
      if (started) return;
      started = true;
      work();
    };
    requestAnimationFrame(() => setTimeout(start, 0));
    setTimeout(start, 250);
  }

  graphView.addEventListener("click", (event) => {
    if (event.target.closest(".anyway") === null) return;
    graphView.innerHTML = note("Laying out…");
    afterPaint(() => renderGraph(true));
  });
  treeView.addEventListener("click", (event) => {
    if (event.target.closest(".anyway") !== null) renderTree();
  });

  // A tree a little wider than its panel is scaled down to fit, since labels
  // at four fifths of their size still read. One much wider keeps its size
  // and scrolls, starting with the root, which sits over the middle of the
  // drawing, in view. The room is the panel's box less border and padding,
  // which a scrollbar coming or going does not move, so the decision does
  // not feed back on itself.
  function fitTree() {
    const svg = treeView.querySelector("svg.tree");
    if (svg === null) return;
    const style = getComputedStyle(treeView);
    const room =
      treeView.getBoundingClientRect().width -
      parseFloat(style.borderLeftWidth) -
      parseFloat(style.borderRightWidth) -
      parseFloat(style.paddingLeft) -
      parseFloat(style.paddingRight);
    const natural = Number(svg.getAttribute("width"));
    svg.classList.toggle("fit", natural > room && natural * 0.8 <= room);
    treeView.scrollLeft = (treeView.scrollWidth - treeView.clientWidth) / 2;
  }

  // The panel's width follows the window, the zoom level and the layout;
  // the fit is decided again whenever it changes.
  new ResizeObserver(fitTree).observe(treeView);

  // The stylesheet has this many branch colours; a bag with more neighbours
  // reuses them.
  const SIDES = 6;
  const SIDE_CLASSES = Array.from({ length: SIDES }, (_, i) => `side-${i + 1}`);
  // Elements by vertex and by bag, mapped once per drawing: a lookup in the
  // DOM per bag would make a hover quadratic in the size of the tree.
  let vertexElements = new Map();
  const vertexElement = (v) => vertexElements.get(String(v));
  const bagElement = (id) => shown.bagElements.get(String(id));

  function clearHighlight() {
    for (const marked of document.querySelectorAll(".on, .side")) {
      marked.classList.remove("on", "side", ...SIDE_CLASSES);
    }
  }

  // Hovering a bag marks the vertices it holds, and the edges among them, in
  // the graph. Taking the bag out of the tree leaves one branch per
  // neighbour; the bags of each branch, and the vertices they hold beyond
  // the bag, get a colour of their own. No edge of the graph joins two
  // branches, so an edge is coloured only when both ends share a branch.
  function highlightBag(id) {
    clearHighlight();
    bagElement(id).classList.add("on");
    const held = new Set(shown.bags.get(id).map(String));
    for (const v of held) vertexElement(v)?.classList.add("on");
    const sideOf = new Map();
    shown.neighbours.get(id).forEach((branch, i) => {
      const side = SIDE_CLASSES[i % SIDES];
      const seen = new Set([id, branch]);
      for (let queue = [branch], at = 0; at < queue.length; at++) {
        const bag = queue[at];
        bagElement(bag).classList.add("side", side);
        for (const v of shown.bags.get(bag)) {
          if (!held.has(String(v))) sideOf.set(String(v), side);
        }
        for (const next of shown.neighbours.get(bag)) {
          if (seen.has(next)) continue;
          seen.add(next);
          queue.push(next);
        }
      }
    });
    for (const [v, side] of sideOf) vertexElement(v)?.classList.add("side", side);
    for (const edge of graphView.querySelectorAll(".edge")) {
      const { u, v } = edge.dataset;
      if (held.has(u) && held.has(v)) {
        edge.classList.add("on");
      } else if (sideOf.has(u) && sideOf.get(u) === sideOf.get(v)) {
        edge.classList.add("side", sideOf.get(u));
      }
    }
  }

  const holdersOf = (v) => shown.holders.get(Number(v)) ?? [];

  // Marks some bags and the tree edges among them.
  function markBags(ids) {
    const marked = new Set(ids.map(String));
    for (const id of marked) bagElement(id).classList.add("on");
    for (const edge of treeView.querySelectorAll(".tree-edges line")) {
      if (marked.has(edge.dataset.a) && marked.has(edge.dataset.b)) edge.classList.add("on");
    }
  }

  // Hovering a vertex marks the bags that hold it: a decomposition keeps them
  // a connected piece of the tree.
  function highlightVertex(v) {
    clearHighlight();
    vertexElement(v).classList.add("on");
    markBags(holdersOf(v));
  }

  // Hovering an edge marks the bags that hold both its ends: a decomposition
  // gives every edge at least one.
  function highlightEdge(edge) {
    clearHighlight();
    edge.classList.add("on");
    const { u, v } = edge.dataset;
    vertexElement(u).classList.add("on");
    vertexElement(v).classList.add("on");
    const ofV = new Set(holdersOf(v));
    markBags(holdersOf(u).filter((id) => ofV.has(id)));
  }

  treeView.addEventListener("pointerover", (event) => {
    const bag = event.target.closest(".bag");
    if (bag === null) clearHighlight();
    else highlightBag(Number(bag.dataset.bag));
  });
  treeView.addEventListener("pointerleave", clearHighlight);

  graphView.addEventListener("pointerover", (event) => {
    if (shown === null) return;
    const vertex = event.target.closest(".vertex");
    const edge = event.target.closest(".edge");
    if (vertex !== null) highlightVertex(vertex.dataset.vertex);
    else if (edge !== null) highlightEdge(edge);
    else clearHighlight();
  });
  graphView.addEventListener("pointerleave", clearHighlight);

  // A button per example; the one whose text is in the box is marked.
  const examples = element("examples");
  EXAMPLES.forEach(([name], i) => {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.textContent = name;
    chip.dataset.example = i;
    examples.append(chip);
  });
  const markExample = (i) => {
    for (const chip of examples.children) {
      chip.classList.toggle("on", chip.dataset.example === String(i));
    }
  };

  // Choosing an example runs it, once the solver is there: the point of the
  // buttons is to see what comes out.
  function loadExample(i) {
    element("graph").value = EXAMPLES[i][1]();
    markExample(i);
    clearTimeout(pending);
    drawGraph();
    if (!element("run").disabled) element("run").click();
  }
  examples.addEventListener("click", (event) => {
    const chip = event.target.closest("button");
    if (chip !== null) loadExample(Number(chip.dataset.example));
  });

  // Redrawing while someone types would be a layout run per keystroke. Once
  // edited, the text is no longer the example it started from.
  let pending = 0;
  element("graph").addEventListener("input", () => {
    markExample(-1);
    clearTimeout(pending);
    pending = setTimeout(drawGraph, 300);
  });

  // The `.td` text to the clipboard or to a file. Ctrl+A in the output would
  // select the whole page.
  function flash(button, text) {
    const label = button.textContent;
    button.textContent = text;
    setTimeout(() => {
      button.textContent = label;
    }, 1500);
  }
  element("copy").addEventListener("click", (event) => {
    event.preventDefault();
    navigator.clipboard.writeText(result).then(
      () => flash(element("copy"), "Copied"),
      () => flash(element("copy"), "Not copied"),
    );
  });
  element("save").addEventListener("click", (event) => {
    event.preventDefault();
    const link = document.createElement("a");
    link.href = URL.createObjectURL(new Blob([result], { type: "text/plain" }));
    link.download = "decomposition.td";
    link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 1000);
  });

  // The call holds this tab for as long as the construction runs, so the
  // status is painted before it starts.
  element("run").addEventListener("click", () => {
    clearTimeout(pending);
    drawGraph();
    element("status").textContent = "running";
    element("run").disabled = true;
    afterPaint(decompose);
  });

  // The other side of the call takes unsigned numbers, where a blank or
  // negative field would arrive as something enormous.
  const setting = (id) => Math.max(0, Math.trunc(Number(element(id).value)) || 0);

  function decompose() {
    const started = performance.now();
    const pointer = solver.ccall(
      "goatd_decompose",
      "number",
      ["string", "number", "number", "number"],
      [element("graph").value, setting("order"), setting("seed"), setting("budget")],
    );
    const text = solver.UTF8ToString(pointer);
    solver.ccall("goatd_string_free", null, ["number"], [pointer]);
    const elapsed = performance.now() - started;

    result = text;
    element("output").textContent =
      text.length <= MAX_SHOWN_OUTPUT
        ? text
        : `${text.split("\n", 1)[0]}\n... ${Math.round(text.length / 1048576)} MB of text; Save writes it to a file.`;
    element("run").disabled = false;

    const decomposition = parseTd(text);
    const failed = decomposition.error !== undefined;
    parsedTree = failed ? null : decomposition;
    element("copy").disabled = failed;
    element("save").disabled = failed;
    element("status").textContent = failed ? "failed" : "ready";
    const summary = element("result-summary");
    if (failed) summary.textContent = text.split("\n", 1)[0];
    else summary.innerHTML = summarise(decomposition.header, elapsed);
    summary.classList.toggle("failed", failed);
    element("raw").open = failed;
    if (failed) {
      treeView.innerHTML = note("No decomposition to draw.");
      element("legend").hidden = true;
      shown = null;
    } else {
      drawTree();
    }
  }

  // A browser that brings the text back on reload keeps it; otherwise the
  // page opens on the first example.
  if (element("graph").value === "") loadExample(0);
  else drawGraph();
}
