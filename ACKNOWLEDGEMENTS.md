# Acknowledgements

`goatd` builds tree decompositions. Everything it does rests on prior work —
the vendored FlowCutter solver and the graph layer it runs on, and the
tree-decomposition and graph-partitioning literature its own heuristics come
from. This document credits that work.

## Code we build and link

`build.rs` compiles the vendored tree. Licences and copyright are in
[`THIRD-PARTY.md`](THIRD-PARTY.md).

Vendored under `vendor/treedecomp/upstream/`:

| Project | Authors | Role here |
|---|---|---|
| [**FlowCutter**](https://github.com/kit-algo/flow-cutter-pace17) | Michael Hamann, Ben Strasser | Graph bisection; the treewidth solver behind `goatd::flowcutter`, and the separator search `goatd::flowcutter_rs` ports |
| [**sharpSAT-TD**](https://github.com/Laakeri/sharpsat-td) | Tuukka Korhonen, Matti Järvisalo | The graph and bitset layer the FlowCutter driver is built on |
| [**treedecomp**](https://github.com/meelgroup/treedecomp) | Kenji Hashimoto and the treedecomp authors | The `TreeDecomposition` representation and its definitions |

- Michael Hamann, Ben Strasser. *Graph Bisection with Pareto Optimization.* ACM JEA 2018 / ALENEX 2016.
- Tuukka Korhonen, Matti Järvisalo. *Integrating Tree Decompositions into Decision Heuristics of Propositional Model Counters.* CP 2021 (LIPIcs 210:8). (sharpSAT-TD.)

## Decomposition heuristics

- Michael Abseher, Nysret Musliu, Stefan Woltran. *[htd — A Free, Open-Source Framework for (Customized) Tree Decompositions and Beyond](https://github.com/mabseher/htd).* CPAIOR 2017. The fill-only min-fill with sampled tie-breaking that the sampled orders follow.
- George Karypis, Rajat Aggarwal, Vipin Kumar, Shashi Shekhar. *Multilevel Hypergraph Partitioning: Application in VLSI Domain.* DAC 1997 (extended: IEEE Transactions on VLSI Systems 7(1):69–79, 1999). ([hMETIS](https://karypis.github.io/glaros/software/metis/overview.html).)
- Sebastian Schlag, Vitali Henne, Tobias Heuer, Henning Meyerhenke, Peter Sanders, Christian Schulz. *k-way Hypergraph Partitioning via n-Level Recursive Bisection.* ALENEX 2016. [KaHyPar](https://github.com/kahypar/kahypar)'s n-level scheme, which the multilevel bisectors follow.
- Tobias Heuer, Peter Sanders, Sebastian Schlag. *Network Flow-Based Refinement for Multilevel Hypergraph Partitioning.* SEA 2018 (LIPIcs 103:1); ACM Journal of Experimental Algorithmics 24(2), article 2.3, 2019. The flow-based refinement the hypergraph bisector runs, simplified, as its final polishing pass.
- Henning Meyerhenke, Peter Sanders, Christian Schulz. *Partitioning Complex Networks via Size-Constrained Clustering.* SEA 2014 (journal version: Journal of Heuristics 22(5):759–782, 2016). Size-constrained label propagation, the coarsening the bisectors use.
- The **minimum-fill-in** and **minimum-degree** heuristics, the classical baselines all of the above are measured against, and the safe reductions (simplicial and almost-simplicial vertices, and the rest) of Bodlaender, Koster and van den Eijkhof that precede every order here.

We are also indebted to the [**PACE Implementation Challenge**](https://pacechallenge.org/2017/treewidth/) treewidth tracks
(2016, 2017) for making this landscape comparable and its implementations
public, and for the `.gr` / `.td` formats this crate reads and writes.

---

If we have misattributed a technique or missed a debt, please open an issue.
Corrections are welcome and will be applied.
