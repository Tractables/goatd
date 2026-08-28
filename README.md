# Tree-decomposition comparison

This directory contains a static interface for comparing tree-decomposition
solvers. Each instance is one graph. Optional source, kind and collection
fields record where the graph came from without changing how it is compared.
The interface has no build step or external web dependency.

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

Filters update both aggregate tables and the detailed results. Sorting applies
only to the detailed table, which is paged so a large corpus does not create
every row at once.

## Aggregate statistics

The first table reports, for every solver:

- valid decompositions and best-observed widths as counts and shares of all
  selected graphs;
- median absolute width excess over the best width observed for each graph;
- median percentage excess in total bag size and bag count;
- median elapsed time for valid decompositions; and
- how many valid decompositions were returned at the time budget.

Excess medians use graphs on which that solver returned a valid decomposition;
coverage is shown separately. Tied minima count as best for every tied solver.
The width quality profile uses every selected graph as its denominator and
reports the share with a valid width at most 0, 1, 2, 4, 8 or 16 above the
best observed width. Missing, invalid and timed-out results do not satisfy a
profile threshold.

When an export includes `source_instances`, the aggregate tables use those
rows and the detailed matrix continues to show the deduplicated component
graphs. This gives each source CNF the same aggregate weight. For a disconnected
source, width is the maximum component width; total bag size, bag count and
elapsed time are summed. A missing component result makes the source result
incomplete. Graph views with no nontrivial component are omitted from the
solver comparison.

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
Solver columns are not fixed; the page renders every entry and scrolls the
matrix horizontally when needed. If a solver has no entry for a graph, the
page shows that cell as unavailable.

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
computed among validated decompositions for one graph.

Large exports use schema version 3. The named JSON file contains the same
dataset, run, and solver metadata plus `instance_count` and `instance_files`;
each listed file contains an `instances` array. The view loads both forms.
