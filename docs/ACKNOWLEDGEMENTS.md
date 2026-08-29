# Acknowledgements

`goatd` contains a vendored FlowCutter backend and implements algorithms from
the tree-decomposition and graph-partitioning literature.

## Code we build and link

`build.rs` compiles the vendored tree. Licences and copyright are in
[`THIRD-PARTY.md`](THIRD-PARTY.md).

Vendored under `vendor/treedecomp/upstream/`:

| Project | Authors | Role here |
|---|---|---|
| [**FlowCutter**](https://github.com/kit-algo/flow-cutter-pace17) | Michael Hamann, Ben Strasser | Graph bisection; the treewidth solver behind `goatd::flowcutter`, and the separator search `goatd::flowcutter::separator` ports |
| [**sharpSAT-TD**](https://github.com/Laakeri/sharpsat-td) | Tuukka Korhonen, Matti Järvisalo | The graph and bitset layer the FlowCutter driver is built on |
| [**treedecomp**](https://github.com/meelgroup/treedecomp) | Kenji Hashimoto and the treedecomp authors | The C++ tree-decomposition representation used by the vendored backend |

- Michael Hamann, Ben Strasser. *Graph Bisection with Pareto Optimization.* ACM JEA 2018 / ALENEX 2016.
- Tuukka Korhonen, Matti Järvisalo. *Integrating Tree Decompositions into Decision Heuristics of Propositional Model Counters.* CP 2021 (LIPIcs 210:8). (sharpSAT-TD.)

## Decomposition heuristics

- Michael Abseher, Nysret Musliu, Stefan Woltran. *[htd — A Free,
  Open-Source Framework for (Customized) Tree Decompositions and
  Beyond](https://github.com/mabseher/htd).* CPAIOR 2017. goatd's sampled
  elimination orders use the same fill-only minimum-fill criterion and sample
  among tied candidates.
- George Karypis, Vipin Kumar. *[A Fast and High Quality Multilevel Scheme for
  Partitioning Irregular
  Graphs](https://doi.org/10.1137/S1064827595287997).* SIAM Journal on
  Scientific Computing 20(1):359–392, 1998. The graph bisector uses sorted
  heavy-edge matching, coarsening and uncoarsening, FM-style refinement, and
  V-cycles.
- George Karypis, Rajat Aggarwal, Vipin Kumar, Shashi Shekhar. *[Multilevel
  Hypergraph Partitioning: Application in VLSI
  Domain](https://doi.org/10.1145/266021.266273).* DAC 1997. The hypergraph
  bisector adapts the same pair-contraction structure to shared-hyperedge
  connectivity.
- Charles M. Fiduccia, Robert M. Mattheyses. *[A Linear-Time Heuristic for
  Improving Network Partitions](https://doi.org/10.1145/800263.809204).* DAC
  1982. Both bisectors use gain-ordered vertex moves and retain the best prefix
  of each pass.
- Tobias Heuer, Peter Sanders, Sebastian Schlag. *[Network Flow-Based
  Refinement for Multilevel Hypergraph
  Partitioning](https://doi.org/10.4230/LIPIcs.SEA.2018.1).* SEA 2018. The
  hypergraph bisector ends its finest-level refinement with a simplified
  flow-based pass.
- Hans L. Bodlaender, Arie M. C. A. Koster, Frank van den Eijkhof. *[Pre-processing
  Rules for Triangulation of Probabilistic
  Networks](https://doi.org/10.1111/j.1467-8640.2005.00274.x).* Computational
  Intelligence 21(3):286–305, 2005. The shared elimination preprocessor applies
  the islet, twig, series, simplicial, and almost-simplicial rules.

The [**PACE Implementation Challenge**](https://pacechallenge.org/2017/treewidth/)
treewidth tracks supplied the `.gr` and `.td` formats this crate reads and
writes.
