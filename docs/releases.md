# Release notes

## Unreleased

### Migrating from 0.2.1

The command-line tool and Python `decompose` now default to `portfolio`;
pass `minfill` explicitly to retain the previous default. With no budget,
the portfolio uses its finite, count-bounded schedule.

Replace `TreeDecomposition::shape` with `bag_mass` and `max_separator`.
Invalid or inert options now return errors naming the option and the orders that accept it.
Seeded output is consistent across 32-bit and 64-bit targets, but may differ
from 0.2.1.

### Additions and fixes

- `Order::MinFillRanked`, `Order::Ranked` and `Prepared::unreduced` support
  caller-selected vertex ordering.
- Prepared width bounds cover reductions and deadline completion; standalone
  merge fallback uses only its remaining budget.
- Polishing sessions keep wall deadlines separate from construction-work
  deadlines.
- Fix elimination bookkeeping, stop reporting and refinement dispatch.
- Browser input uses heap storage for large graphs, validates unsigned
  settings and reports module-load failures.
- Release builds check version consistency across crates and citation metadata.
