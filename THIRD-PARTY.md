# Third-party components

`goatd` is Apache-2.0 (`LICENSE`). This file records every third-party
component that ships inside the crate or is linked into a build of it, and the
licence each is used under. For the BSD-2-Clause and MIT components below,
reproducing the notice is a **licence condition**, not a courtesy — the
required texts are at the end of this file.

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

One directory up, outside `upstream/`, sit the three files that are goatd's own
and carry goatd's licence: `ffi.cpp` / `ffi.h`, the C ABI shim over the above,
and `heap_selftest.cpp`, which drives upstream's k-way id-heap from a unit test.
The split is the boundary itself — everything under `upstream/` is somebody
else's source, and nothing else is.

**These files are modified.** The BSD-2-Clause and MIT licences above permit
modification and require the notice be kept, which it is; this states the
changes for a reader who expects upstream source. Most are marked in place with
a `// goatd:` comment naming the reason:

- missing `<cstdint>` / `<cstddef>` includes added, for types the upstream files
  use but only received transitively on the standard libraries they were written
  against;
- two memory-safety guards, in `IFlowCutter.cpp` and
  `flow-cutter-pace17/src/heap.hpp`: a budget on the bag-adjacency arc count and
  64-bit child-index arithmetic in the k-ary heap. Both convert an
  out-of-memory or integer-overflow crash on a pathological graph into a clean
  error the caller can handle;
- a null check on the `sspp::Bitset` allocation, in `bitset.hpp`, for the same
  reason;
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
  by the clock;
- a per-thread touch counter and a touch budget on the two greedy elimination
  passes, in `flow-cutter-pace17/src/greedy_order.{cpp,hpp}`, which is what lets
  those passes be bounded and charged the same way.

Upstream's heap arithmetic is also exercised from a unit test, through
`heap_selftest.cpp`; that file is goatd's own and adds nothing to the upstream
sources.

Diffing `vendor/treedecomp/upstream/` against the upstream projects named in the
table shows each of the modifications above.

No third-party test data ships with this crate: every test fixture is generated
in test code from a construction written here.

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
