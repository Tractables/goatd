"use strict";

// The page in one file: readers for the two PACE formats, a layout for the
// graph, a layout for the decomposition, SVG for both, and the calls into the
// Wasm module. Nothing is loaded from anywhere else.

// A drawing past these sizes is a grey smear, so the panel says so instead
// and offers to draw it anyway.
const MAX_DRAWN_VERTICES = 500;
const MAX_DRAWN_EDGES = 1500;
const MAX_DRAWN_BAGS = 200;
// Up to this many vertices, each is drawn larger with its number on it.
const LABELLED_VERTICES = 90;
// A graph drawn anyway gets this many vertex-pair updates of the stress
// layout, about three seconds' worth; when that buys fewer rounds than the
// minimum, the pivot layout draws it instead.
const LAYOUT_WORK = 4e8;
const MIN_LAYOUT_ROUNDS = 80;
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

// The neighbours of every vertex in two flat arrays: those of `v` are
// `neighbours[start[v]]` up to `neighbours[start[v + 1]]`.
function neighbourLists(count, edges) {
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
  return { start, neighbours };
}

// The number of edges apart every pair of vertices is, by breadth-first search
// from each of them in turn, as one row of `count` numbers per vertex. Two
// vertices in different components are given a distance somewhat past the
// longest path in the graph, which separates the components without letting
// them drift arbitrarily far apart.
function graphDistances(count, edges) {
  const { start, neighbours } = neighbourLists(count, edges);
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
// Both the distances and the rounds are quadratic in the vertex count. Up to
// MAX_DRAWN_VERTICES the layout runs its full rounds; past it, when the
// drawing is asked for anyway, it gets LAYOUT_WORK pair updates in all, and
// when that buys fewer than MIN_LAYOUT_ROUNDS rounds, layoutGraphByPivots.
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

  return squareUp(count, xs, ys);
}

// Neither layout fixes which way up the drawing is, and both usually land
// askew. Turning the drawing so that its bounding box is as small as it can
// be squares up a grid and lays a long graph flat; it is then fitted to the
// unit square, both axes scaled alike, and centred.
function squareUp(count, xs, ys) {
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

  return fitUnit(count, xs, ys);
}

// Fit a drawing to the unit square, scaling both axes by the same factor so
// it is not stretched, and centre what is left over.
function fitUnit(count, xs, ys) {
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

// A graph in several pieces is laid out a piece at a time. Laid out
// together, the pieces would sit at one made-up distance from each other,
// which puts the small ones on an arc that takes half the drawing or, in the
// pivot layout, on top of each other far from the large one, which is then
// squeezed into a corner. Here the largest piece fills the drawing, and the
// rest, single vertices included, are packed in rows beneath it with their
// edges drawn at the same length as its.
function layoutByComponents(count, edges, lay) {
  const { start, neighbours } = neighbourLists(count, edges);
  const seen = new Uint8Array(count + 1);
  const pieces = [];
  for (let v = 1; v <= count; v++) {
    if (seen[v]) continue;
    const members = [v];
    seen[v] = 1;
    for (let i = 0; i < members.length; i++) {
      const u = members[i];
      for (let k = start[u]; k < start[u + 1]; k++) {
        const w = neighbours[k];
        if (!seen[w]) {
          seen[w] = 1;
          members.push(w);
        }
      }
    }
    pieces.push(members);
  }
  if (pieces.length === 1) return lay(count, edges);
  pieces.sort((a, b) => b.length - a.length);

  // Each piece is renumbered and laid out on its own; a small piece takes the
  // full stress layout whatever the large one gets.
  const piece = new Uint32Array(count + 1);
  const index = new Uint32Array(count + 1);
  pieces.forEach((members, p) => members.forEach((v, i) => {
    piece[v] = p;
    index[v] = i + 1;
  }));
  const own = pieces.map(() => []);
  for (const [u, v] of edges) own[piece[u]].push([index[u], index[v]]);
  const laid = pieces.map((members, p) => {
    if (members.length === 1) return null;
    const inner = members.length <= MAX_DRAWN_VERTICES ? layoutGraph(members.length, own[p]) : lay(members.length, own[p]);
    let minX = Infinity;
    let maxX = -Infinity;
    let minY = Infinity;
    let maxY = -Infinity;
    for (let i = 1; i <= members.length; i++) {
      minX = Math.min(minX, inner.xs[i]);
      maxX = Math.max(maxX, inner.xs[i]);
      minY = Math.min(minY, inner.ys[i]);
      maxY = Math.max(maxY, inner.ys[i]);
    }
    const lengths = own[p].map(([u, v]) => Math.hypot(inner.xs[u] - inner.xs[v], inner.ys[u] - inner.ys[v]));
    lengths.sort((a, b) => a - b);
    return { ...inner, minX, minY, width: maxX - minX, height: maxY - minY, edge: lengths[lengths.length >> 1] };
  });

  // The largest piece fills the unit square; every other piece is scaled so
  // that its edges match, never larger than the square, and the pieces go in
  // rows beneath, an edge length apart (a tenth of the square at most, for
  // when the largest piece is a single edge). When even the largest piece is
  // a single vertex, the rows start with it.
  const xs = new Float64Array(count + 1);
  const ys = new Float64Array(count + 1);
  const unit = laid[0] === null ? 0.05 : laid[0].edge;
  const gap = Math.min(Math.max(unit, 0.01), 0.1);
  const place = (p, x, y, scale) => {
    const members = pieces[p];
    const inner = laid[p];
    for (let i = 0; i < members.length; i++) {
      xs[members[i]] = x + (inner === null ? 0 : (inner.xs[i + 1] - inner.minX) * scale);
      ys[members[i]] = y + (inner === null ? 0 : (inner.ys[i + 1] - inner.minY) * scale);
    }
  };
  place(0, 0, 0, 1);
  let x = laid[0] === null ? gap : 0;
  let y = laid[0] === null ? 0 : 1 + 2 * gap;
  let rowHeight = 0;
  for (let p = 1; p < pieces.length; p++) {
    const inner = laid[p];
    const scale = inner === null ? 0 : Math.min(1, unit / Math.max(inner.edge, 1e-9));
    const width = inner === null ? 0 : inner.width * scale;
    const height = inner === null ? 0 : inner.height * scale;
    if (x > 0 && x + width > 1) {
      x = 0;
      y += rowHeight + gap;
      rowHeight = 0;
    }
    place(p, x, y, scale);
    x += width + gap;
    rowHeight = Math.max(rowHeight, height);
  }
  return fitUnit(count, xs, ys);
}

// Pivot MDS, for a graph too large for the stress layout: breadth-first
// search from a few dozen pivots spread through the graph, each the vertex
// farthest from the pivots before it, then the two leading directions of the
// squared distances to them, which is classical scaling restricted to the
// pivots. A grid comes out as a grid. A graph with less shape comes out
// with vertices that have the same distances to every pivot on top of each
// other; when that has happened to more than a few, some rounds of stress
// majorization follow in which each vertex answers only to its neighbours
// and to the pivots, which pulls them apart (and would only roughen a grid,
// so a grid gets none). The cost is a search per pivot and a few passes over
// the vertices, so ten thousand vertices take well under a second.
function layoutGraphByPivots(count, edges, pivots = 60, rounds = 50) {
  const xs = new Float64Array(count + 1);
  const ys = new Float64Array(count + 1);
  if (count === 0) return { xs, ys };
  if (count === 1) {
    xs[1] = 0.5;
    ys[1] = 0.5;
    return { xs, ys };
  }
  const { start, neighbours } = neighbourLists(count, edges);
  const k = Math.min(pivots, count);
  // Row per vertex, column per pivot.
  const distance = new Float64Array(count * k).fill(-1);
  const nearest = new Float64Array(count + 1).fill(Infinity);
  const queue = new Int32Array(count);
  const pivotAt = new Int32Array(k);
  let longest = 1;
  let pivot = 1;
  for (let p = 0; p < k; p++) {
    pivotAt[p] = pivot;
    distance[(pivot - 1) * k + p] = 0;
    queue[0] = pivot;
    for (let head = 0, tail = 1; head < tail; head++) {
      const at = queue[head];
      const d = distance[(at - 1) * k + p] + 1;
      for (let i = start[at]; i < start[at + 1]; i++) {
        const to = neighbours[i];
        if (distance[(to - 1) * k + p] >= 0) continue;
        distance[(to - 1) * k + p] = d;
        longest = Math.max(longest, d);
        queue[tail++] = to;
      }
    }
    // The next pivot is the vertex farthest from all pivots so far; a vertex
    // no pivot has reached, in another component, comes first.
    let farthest = 0;
    for (let v = 1; v <= count; v++) {
      const d = distance[(v - 1) * k + p];
      if (d >= 0) nearest[v] = Math.min(nearest[v], d);
      if (farthest === 0 || nearest[v] > nearest[farthest]) farthest = v;
    }
    pivot = farthest;
  }
  for (let i = 0; i < distance.length; i++) {
    if (distance[i] < 0) distance[i] = longest * 1.5;
  }
  const raw = distance.slice();

  // Double-centre the squared distances, in place.
  const rowMean = new Float64Array(count + 1);
  const columnMean = new Float64Array(k);
  let mean = 0;
  for (let v = 1; v <= count; v++) {
    const row = (v - 1) * k;
    let sum = 0;
    for (let p = 0; p < k; p++) {
      const square = distance[row + p] * distance[row + p];
      distance[row + p] = square;
      sum += square;
      columnMean[p] += square;
    }
    rowMean[v] = sum / k;
    mean += sum;
  }
  mean /= count * k;
  for (let p = 0; p < k; p++) columnMean[p] /= count;
  for (let v = 1; v <= count; v++) {
    const row = (v - 1) * k;
    for (let p = 0; p < k; p++) {
      distance[row + p] = -0.5 * (distance[row + p] - rowMean[v] - columnMean[p] + mean);
    }
  }

  // The k-by-k product of the centred matrix with itself, then its two
  // leading eigenvectors by power iteration, the second kept orthogonal to
  // the first.
  const product = new Float64Array(k * k);
  for (let v = 1; v <= count; v++) {
    const row = (v - 1) * k;
    for (let a = 0; a < k; a++) {
      const left = distance[row + a];
      if (left === 0) continue;
      for (let b = a; b < k; b++) product[a * k + b] += left * distance[row + b];
    }
  }
  for (let a = 0; a < k; a++) {
    for (let b = a + 1; b < k; b++) product[b * k + a] = product[a * k + b];
  }
  const random = mulberry32(0x2545f491);
  const eigenvector = (avoid) => {
    let vector = Float64Array.from({ length: k }, () => random() - 0.5);
    const next = new Float64Array(k);
    for (let round = 0; round < 300; round++) {
      if (avoid !== null) {
        let dot = 0;
        for (let i = 0; i < k; i++) dot += vector[i] * avoid[i];
        for (let i = 0; i < k; i++) vector[i] -= dot * avoid[i];
      }
      let norm = 0;
      for (let a = 0; a < k; a++) {
        let sum = 0;
        for (let b = 0; b < k; b++) sum += product[a * k + b] * vector[b];
        next[a] = sum;
        norm += sum * sum;
      }
      norm = Math.sqrt(norm) || 1;
      for (let i = 0; i < k; i++) vector[i] = next[i] / norm;
    }
    return vector;
  };
  const first = eigenvector(null);
  const second = eigenvector(first);
  for (let v = 1; v <= count; v++) {
    const row = (v - 1) * k;
    let x = 0;
    let y = 0;
    for (let p = 0; p < k; p++) {
      x += distance[row + p] * first[p];
      y += distance[row + p] * second[p];
    }
    xs[v] = x;
    ys[v] = y;
  }

  // The coordinates are in edge lengths. The rounds are needed when more
  // than one vertex in twenty shares a spot, to a quarter of an edge.
  const spots = new Set();
  for (let v = 1; v <= count; v++) spots.add(`${Math.round(xs[v] * 4)},${Math.round(ys[v] * 4)}`);
  if (spots.size >= 0.95 * count) return squareUp(count, xs, ys);

  // The stress rounds, over neighbours (one edge apart) and pivots (their
  // searched distance apart), from a start jittered by a fixed seed so that
  // vertices on one spot can come apart.
  for (let v = 1; v <= count; v++) {
    xs[v] += 0.1 * (random() - 0.5);
    ys[v] += 0.1 * (random() - 0.5);
  }
  const nextX = new Float64Array(count + 1);
  const nextY = new Float64Array(count + 1);
  const pull = (i, j, target, weight, sums) => {
    const ex = xs[i] - xs[j];
    const ey = ys[i] - ys[j];
    let length = Math.sqrt(ex * ex + ey * ey);
    if (length < 1e-9) length = 1e-9;
    const stretch = target / length;
    sums[0] += weight * (xs[j] + ex * stretch);
    sums[1] += weight * (ys[j] + ey * stretch);
    sums[2] += weight;
  };
  const sums = [0, 0, 0];
  for (let round = 0; round < rounds; round++) {
    for (let i = 1; i <= count; i++) {
      sums[0] = sums[1] = sums[2] = 0;
      for (let n = start[i]; n < start[i + 1]; n++) pull(i, neighbours[n], 1, 1, sums);
      const row = (i - 1) * k;
      for (let p = 0; p < k; p++) {
        const target = raw[row + p];
        if (target === 0) continue;
        pull(i, pivotAt[p], target, 1 / (target * target), sums);
      }
      if (sums[2] === 0) {
        nextX[i] = xs[i];
        nextY[i] = ys[i];
      } else {
        nextX[i] = sums[0] / sums[2];
        nextY[i] = sums[1] / sums[2];
      }
    }
    xs.set(nextX);
    ys.set(nextY);
  }
  return squareUp(count, xs, ys);
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
  const labelled = count <= LABELLED_VERTICES;
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

// The row of example graphs: each has a key, which is how the address bar
// names it, the name on its button, and its maker. All but one are generated
// when chosen, so the page carries the recipe and not the text; the
// competition graph is a file beside the page. The first is the graph the
// page opens with unless the address names another. The larger ones are
// past what the page draws unasked; they show the solver at work.
const EXAMPLES = [
  ["grid6", "6×6 grid", () => grid(6, 6, "a 6x6 grid; its treewidth is 6")],
  ["petersen", "Petersen graph", petersen],
  ["3-tree", "random 3-tree, 40 vertices", () => kTree(3, 40, 1)],
  ["hypercube5", "5-dimensional hypercube", () => hypercube(5)],
  ["grid20", "20×20 grid", () => grid(20, 20, "a 20x20 grid; its treewidth is 20")],
  ["cube7", "7×7×7 grid", () => cubeGrid(7)],
  ["mcc2025-093", "Model Counting Competition CNF, 1,843 vertices", () => fetched("mcc2025-track1-093.gr")],
  ["4-tree", "random 4-tree, 2,000 vertices", () => kTree(4, 2000, 1)],
  ["sparse3000", "random sparse graph, 3,000 vertices", () => randomGraph(3000, 4500, 1)],
  ["grid100", "100×100 grid, 10,000 vertices", () => grid(100, 100, "a 100x100 grid; its treewidth is 100")],
];

// The host keeps the page's files in caches for a while, so the workflow
// stamps the references in index.html with the commit (app.js?v=<sha>), and
// the stamp goes on to every file the page loads: a new page never runs with
// an older file a browser still holds. Opened by hand there is no stamp and
// the names are plain.
const stamp = typeof document === "undefined" || document.currentScript === null
  ? ""
  : new URL(document.currentScript.src).search;

// A graph shipped as a file beside the page.
function fetched(file) {
  return fetch(file + stamp).then((response) => {
    if (!response.ok) throw new Error(`${response.status} for ${file}`);
    return response.text();
  });
}

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
    element("zoom").hidden = true;
    shown = null;
    parsedTree = null;
    element("output").textContent = "";
    result = "";
    element("result-summary").textContent = "";
    element("result-summary").classList.remove("failed");
    element("raw").open = false;
    element("copy").disabled = true;
    element("save").disabled = true;
    if (ready && !running) element("status").textContent = "ready";

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
      graphView.innerHTML = note(
        `Not drawn: ${graph.count} vertices and ${graph.edges.length} edges is` +
          " past what is readable here.",
        true,
      );
      return;
    }
    renderGraph(false);
  }

  function renderGraph(large) {
    const { count, edges } = parsedGraph;
    const rounds = large ? Math.min(300, Math.floor(LAYOUT_WORK / (count * count))) : 300;
    const layout = layoutByComponents(count, edges, (n, e) =>
      rounds >= MIN_LAYOUT_ROUNDS ? layoutGraph(n, e, rounds) : layoutGraphByPivots(n, e),
    );
    graphView.innerHTML = graphSvg(count, edges, layout, large);
    vertexElements = new Map();
    for (const vertex of graphView.querySelectorAll(".vertex")) {
      vertexElements.set(vertex.dataset.vertex, vertex);
    }
  }

  function drawTree() {
    element("legend").hidden = true;
    element("zoom").hidden = true;
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
    element("zoom").hidden = false;
    zoom = "auto";
    applyZoom();
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

  // The tree's zoom: a factor, "fit" for whatever fills the panel's width,
  // or "auto", which fits when that leaves the labels at least half their
  // size and otherwise draws at full size to scroll. The room is the panel's
  // box less border and padding, which a scrollbar coming or going does not
  // move, so the decision does not feed back on itself. The scroll starts
  // with the root, which sits over the middle of the drawing, in view.
  let zoom = "auto";
  function applyZoom() {
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
    const fit = Math.min(1, room / natural);
    const factor = zoom === "fit" ? fit : zoom === "auto" ? (fit >= 0.5 ? fit : 1) : zoom;
    svg.style.width = `${natural * factor}px`;
    svg.style.height = `${Number(svg.getAttribute("height")) * factor}px`;
    element("zoom-level").textContent = `${Math.round(factor * 100)}%`;
    treeView.scrollLeft = (treeView.scrollWidth - treeView.clientWidth) / 2;
  }
  element("zoom").addEventListener("click", (event) => {
    const button = event.target.closest("button");
    const svg = treeView.querySelector("svg.tree");
    if (button === null || svg === null) return;
    const current = parseFloat(svg.style.width) / Number(svg.getAttribute("width"));
    const asked = button.dataset.zoom;
    if (asked === "fit") zoom = "fit";
    else if (asked === "one") zoom = 1;
    else if (asked === "in") zoom = Math.min(4, current * 1.25);
    else zoom = Math.max(0.02, current / 1.25);
    applyZoom();
  });

  // The panel's width follows the window and the layout; a fit is decided
  // again whenever it changes.
  new ResizeObserver(applyZoom).observe(treeView);

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

  // A button per example; the one whose text is in the box is marked. (Not
  // with "on", which clearHighlight takes off everything on the page.)
  const examples = element("examples");
  EXAMPLES.forEach(([, name], i) => {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.textContent = name;
    chip.dataset.example = i;
    examples.append(chip);
  });
  const markExample = (i) => {
    for (const chip of examples.children) {
      chip.classList.toggle("chosen", chip.dataset.example === String(i));
    }
  };

  // Choosing an example runs it, once the solver is there: the point of the
  // buttons is to see what comes out. An example may come from a file next
  // to the page rather than a recipe, so the text may take a moment.
  let loading = 0;
  async function loadExample(i) {
    markExample(i);
    const ticket = ++loading;
    let text;
    try {
      text = await EXAMPLES[i][2]();
    } catch (failure) {
      element("status").textContent = `the example did not load: ${failure}`;
      return;
    }
    if (ticket !== loading) return;
    element("graph").value = text;
    clearTimeout(pending);
    drawGraph();
    requestRun();
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
    element("link").hidden = true;
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

  // The address carries the choices, so a result can be linked to:
  // ?graph=<example>&order=<construction>&seed=<n>&budget=<ms>. It is read
  // once when the page opens, before the first run, and written at every
  // run. A graph typed in is not in the address, so a run on one clears it
  // and hides the button that copies it.
  const orderKey = (option) => option.textContent.toLowerCase().replace(/\s+/g, "-");
  function readAddress() {
    const params = new URLSearchParams(location.search);
    const order = [...element("order").options].find((option) => orderKey(option) === params.get("order"));
    if (order !== undefined) element("order").value = order.value;
    for (const id of ["seed", "budget"]) {
      const value = params.get(id);
      if (value !== null && /^\d{1,9}$/.test(value)) element(id).value = value;
    }
    return EXAMPLES.findIndex(([key]) => key === params.get("graph"));
  }
  function writeAddress() {
    const chosen = examples.querySelector(".chosen");
    element("link").hidden = chosen === null;
    if (chosen === null) {
      history.replaceState(null, "", location.pathname);
      return;
    }
    const params = new URLSearchParams();
    params.set("graph", EXAMPLES[Number(chosen.dataset.example)][0]);
    params.set("order", orderKey(element("order").selectedOptions[0]));
    params.set("seed", String(setting("seed")));
    params.set("budget", String(setting("budget")));
    history.replaceState(null, "", `${location.pathname}?${params}`);
  }
  element("link").addEventListener("click", () => {
    navigator.clipboard.writeText(location.href).then(
      () => flash(element("link"), "Copied"),
      () => flash(element("link"), "Not copied"),
    );
  });

  // The solver runs in a worker, so the page stays live during a run, the
  // status can count the seconds, and a run can be stopped: Cancel ends the
  // worker and starts another, which takes a moment to load the module. A
  // run asked for before the worker is ready, or while another runs, starts
  // once the worker is.
  let worker = null;
  let ready = false;
  let running = false;
  let runWhenReady = false;
  let ticking = 0;

  function startSolver() {
    ready = false;
    element("run").disabled = true;
    element("status").textContent = "loading the solver";
    worker = new Worker("worker.js" + stamp);
    worker.onmessage = (event) => {
      if (event.data.ready === true) {
        ready = true;
        element("run").disabled = false;
        element("status").textContent = "ready";
        if (runWhenReady) {
          runWhenReady = false;
          run();
        }
      } else if (event.data.error !== undefined) {
        settle();
        element("status").textContent = `the solver failed: ${event.data.error}`;
      } else {
        finish(event.data.td, event.data.elapsed);
      }
    };
    worker.onerror = (event) => {
      element("status").textContent = `the solver did not load: ${event.message}`;
    };
  }

  function requestRun() {
    if (running) {
      stop();
      runWhenReady = true;
    } else if (ready) {
      run();
    } else {
      runWhenReady = true;
    }
  }

  function stop() {
    worker.terminate();
    settle();
    startSolver();
  }

  function settle() {
    running = false;
    clearInterval(ticking);
    document.body.classList.remove("busy");
    element("cancel").hidden = true;
    element("run").hidden = false;
  }

  // The other side of the call takes unsigned numbers, where a blank or
  // negative field would arrive as something enormous.
  const setting = (id) => Math.max(0, Math.trunc(Number(element(id).value)) || 0);

  function run() {
    clearTimeout(pending);
    drawGraph();
    writeAddress();
    running = true;
    const started = performance.now();
    element("status").textContent = "running";
    ticking = setInterval(() => {
      element("status").textContent = `running, ${Math.round((performance.now() - started) / 1000)} s`;
    }, 1000);
    element("run").hidden = true;
    element("cancel").hidden = false;
    document.body.classList.add("busy");
    worker.postMessage({
      graph: element("graph").value,
      order: setting("order"),
      seed: setting("seed"),
      budget: setting("budget"),
    });
  }

  element("run").addEventListener("click", requestRun);
  element("cancel").addEventListener("click", () => {
    if (!running) return;
    stop();
    element("status").textContent = "stopped; loading the solver";
  });

  function finish(text, elapsed) {
    settle();
    result = text;
    element("output").textContent =
      text.length <= MAX_SHOWN_OUTPUT
        ? text
        : `${text.split("\n", 1)[0]}\n... ${Math.round(text.length / 1048576)} MB of text; Save writes it to a file.`;

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
  startSolver();
  const opening = readAddress();
  if (element("graph").value === "") loadExample(opening === -1 ? 0 : opening);
  else drawGraph();
}
