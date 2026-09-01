# Tree-decomposition comparison

This directory contains a static interface for comparing tree-decomposition
solvers. Each instance is one graph. Optional source, kind and collection
fields record where the graph came from without changing how it is compared.
The interface has no build step or external web dependency. Its stylesheet
takes the solver page's palette, in both colour schemes, and its type
(`bindings/wasm/styles.css` in goatd), so the two pages read as one site;
`logo.png` is the same logo. Published, the page sits under `comparison/` on
goatd's `gh-pages` branch, beside the solver page at the root, and the two
link to each other.

`results.fixture.json` contains invented values for interface development. It
must not be presented as a benchmark result.

## Local preview

Serve this directory with any static file server. For example:

```sh
python3 -m http.server 8000
```

The page loads `results.json` by default. Load the synthetic interface fixture
explicitly when developing the view:

```text
?data=results.fixture.json
```

Filters update both the aggregate comparison and the detailed results. Tree
components are omitted. By default, a graph is excluded only when the pinned
NetworkX min-degree run returned a counted width below 30. A missing, invalid,
timed-out or no-reduction min-degree result is retained. The control can remove
or change that floor. Adding a solver to the comparison therefore does not
change the selected graph population. Sorting applies only to the detailed
records, which are paged so a large corpus does not create every row at once.

For benchmark accounting, a result must pass validation, contain more than one
bag, and have width below the graph's one-bag width `|V| - 1`. A result that
does not improve that bound is counted like a timeout. The raw result remains
available in the detailed view.

## Aggregate statistics

The summary reports, for every displayed solver:

- counted decompositions;
- exact ties with the best width observed for each graph; and
- counted widths within one and four of the best observed width.

Tied minima count as best for every tied solver. The quality--coverage curve
reports the count with a counted width at most 0, 1, 2, 4 or 8 above the best
observed width. Missing, invalid, timed-out and no-reduction results do not
satisfy a profile threshold. Hovering or focusing a metric label explains its
denominator and interpretation. Aggregate metric labels appear once in the
header. Each curve has a wider invisible pointer target; hovering or focusing
it identifies the solver, while its marked thresholds report exact values.

The detailed view lays out every solver in a responsive grid under its graph;
it does not require horizontal scrolling. Connected graphs with
`|E| = |V| - 1`, whose exact width is one, are omitted. Graph views with no
nontrivial component are absent from the export.

Generate the public-README preview from the same solver field and default
filters:

```sh
node export_markdown.js results.json README-table.md
```

`presentation.js` is the shared source for the displayed field, the pinned
min-degree filter and metric definitions. `README-table.md` is generated; do
not edit its numbers by hand.

## Result format

Schema version 2 is independent of a particular graph corpus, solver or
runner. Files use this top-level shape:

```json
{
  "schema_version": 2,
  "dataset": {
    "title": "Display name",
    "description": "Optional description",
    "synthetic": false
  },
  "run": {
    "generated_at": "2026-08-26T18:30:00Z",
    "timeout_seconds": 10
  },
  "solvers": [
    { "id": "stable-id", "name": "Display name", "version": "Optional version" }
  ],
  "source_instances": [],
  "instances": []
}
```

`source_instances` is optional. It has the same result-map shape as
`instances`, plus component counts, and supplies the source-level aggregate
rows described above.

Each instance supplies one graph and one result map:

```json
{
  "id": "graph-pair-sha256:digest/primal",
  "label": "example / primal",
  "source": "collection/example.cnf",
  "kind": "primal",
  "group": "collection",
  "alias_count": 3,
  "aliases": ["optional/example-copy.cnf"],
  "vertices": 100,
  "edges": 450,
  "sha256": "graph-file-digest",
  "results": {
    "stable-id": {
      "status": "ok",
      "budget_reached": false,
      "width": 12,
      "total_bag_size": 700,
      "bag_count": 90,
      "elapsed_ms": 250,
      "wall_elapsed_ms": 250
    }
  }
}
```

Only `id`, `vertices`, `edges` and `results` are required. `label` is the
human-readable graph name. `source`, `kind` and `group` are optional provenance
and filter fields. For the CNF-derived evaluation, a label such as
`example / primal` names the graph while `source` retains the canonical CNF
path. The interface does not pair that graph with an incidence graph or give
either kind special treatment.

Solver IDs in `results` refer to entries in the top-level `solvers` array.
Solver columns are not fixed; the page renders every displayed entry in a
responsive grid. If a solver has no entry for a graph, the page shows that
cell as unavailable.

`aliases` lists other source names represented by the graph. `alias_count` is
the number of additional names and takes precedence in the display when both
fields are present. Listed aliases are included in text search.

A completed result uses status `ok` and supplies all four metrics. Other
statuses may be `timeout`, `error`, `invalid`, `skipped`, `unavailable` or
`incomplete`;
their metrics are `null`. `elapsed_ms` may be present for a non-completed run,
and `message` may give a short reason.

`budget_reached` distinguishes a result emitted in response to the fixed
deadline from a solver that exited early. Deadline results remain eligible for
decomposition-metric comparisons but not elapsed-time rankings.

Width, total bag size, bag count and elapsed time are minimized. Every rank is
computed among counted decompositions for one graph.

Large exports use schema version 3. The named JSON file contains the same
dataset, run, and solver metadata plus `instance_count` and `instance_files`;
each listed file contains an `instances` array. The view loads both forms.
