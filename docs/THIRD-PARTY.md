# Third-party components

`goatd` is Apache-2.0 (`LICENSE`). This file records the third-party components
that ship inside the crate, the Python wheels and the C libraries, or are
linked into a build. The required BSD-2-Clause and MIT notices are reproduced
below.

## Rust dependencies

`rand`, `rustc-hash`, and the build-dependency `cc`. Each is dual-licensed
`MIT OR Apache-2.0` and is used here under Apache-2.0.

## FlowCutter tree decomposition — vendored C++, statically linked

The FlowCutter tree-decomposition backend is compiled from source by `build.rs`
and linked in. Third-party sources live in `vendor/treedecomp/upstream/`; paths
in the table below are relative to it.

| Files | Upstream | Licence | Copyright |
|---|---|---|---|
| `flow-cutter-pace17/**`, `IFlowCutter.{cpp,hpp}` | [FlowCutter, PACE 2017](https://github.com/kit-algo/flow-cutter-pace17) | BSD-2-Clause | © 2016 Ben Strasser |
| `graph.{cpp,hpp}`, `bitset.hpp`, `utils.hpp` | [sharpSAT-TD](https://github.com/Laakeri/sharpsat-td) | MIT | © 2021 Tuukka Korhonen and Matti Järvisalo |
| `TreeDecomposition.{cpp,hpp}` | [treedecomp](https://github.com/meelgroup/treedecomp) | MIT | © 2023 Kenji Hashimoto |
| `treedecomp_defs.hpp` | [treedecomp](https://github.com/meelgroup/treedecomp) | MIT | © 2023 Authors of treedecomp |
| `time_mem.hpp` | [MiniSat](https://github.com/niklasso/minisat) / [CryptoMiniSat](https://github.com/msoos/cryptominisat) | MIT | © 2003–2006 Niklas Eén, Niklas Sörensson; © 2009–2020 the CryptoMiniSat authors |

The files beside `upstream/` are goatd's own and carry goatd's licence:
`ffi.cpp`, `ffi.h`, and `heap_selftest.cpp`.

**These files are modified.** The BSD-2-Clause and MIT licences above permit
modification and require the notice be kept, which it is; this states the
changes for a reader who expects upstream source. Most are marked in place with
a `// goatd:` comment naming the reason:

- missing `<cstdint>` / `<cstddef>` includes added, for types the upstream files
  use but only received transitively on the standard libraries they were written
  against;
- a budget on the bag-adjacency arc count in `IFlowCutter.cpp`, reported to the
  Rust caller as a missing backend result;
- 64-bit child-index arithmetic in the k-ary heap in
  `flow-cutter-pace17/src/heap.hpp`, avoiding signed integer overflow on large
  heaps;
- a null check on the `sspp::Bitset` allocation in `bitset.hpp`, which prints
  the requested byte count before aborting if allocation still fails after the
  Rust-side size guard;
- a density gate on FlowCutter's min-shortcut ordering heuristic, in
  `IFlowCutter.cpp`, so a clique-dominated graph does not spend the whole
  construction budget in one heuristic;
- a `tight_gates` parameter on `IFlowCutter::constructTD_timed_patience`, in
  `IFlowCutter.{cpp,hpp}`, so a deadline that is only an outer bound leaves the
  pre-loop heuristic node gates where the untimed search has them;
- an abandonment deadline on the two greedy elimination passes, in
  `flow-cutter-pace17/src/greedy_order.{cpp,hpp}`, so a pass that reaches it is
  dropped whole rather than running past the deadline bounding the search around
  it;
- a work-unit budget and a reported restart-iteration count on
  `IFlowCutter::constructTD`, `constructTD_timed` and
  `constructTD_timed_patience`, in `IFlowCutter.{cpp,hpp}`, so a caller metering
  construction can bound and charge the search by the work it does rather than
  by the clock; the timed entry also measures patience on that work budget;
- a per-thread touch counter and a touch budget on the two greedy elimination
  passes, in `flow-cutter-pace17/src/greedy_order.{cpp,hpp}`, which is what lets
  those passes be bounded and charged the same way;
- the GCC `__builtin_ctzll`/`__builtin_popcountll` intrinsics in `bitset.hpp`
  replaced with their C++20 `<bit>` equivalents, and an unused `<sys/time.h>`
  include dropped from `IFlowCutter.cpp`, so the sources compile under MSVC.

The goatd-owned C ABI shim catches C++ exceptions before they reach Rust and
reports them as a missing backend result.

No GPL-licensed component is included in or linked by this crate.

---

## BSD-2-Clause — FlowCutter

```
Copyright (c) 2016, Ben Strasser
All rights reserved.

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

Redistributions of source code must retain the above copyright notice, this list
of conditions and the following disclaimer.
Redistributions in binary form must reproduce the above copyright notice, this
list of conditions and the following disclaimer in the documentation and/or
other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR
ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES
(INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON
ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
(INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
```

## MIT

Applies to the MIT-licensed components above, each under its own copyright line
as listed:

```
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```
