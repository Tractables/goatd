use crate::elimination::refine::*;
use crate::tests::td_fixture::make_td;
use crate::{Graph, TreeDecomposition};

fn trivial_td(num_vertices: u32) -> TreeDecomposition {
    make_td(vec![(0..num_vertices).collect()], Vec::new())
}

#[test]
fn refine_noop_on_tiny_subproblem() {
    // Under the min-side-size threshold: should return unchanged.
    let td = trivial_td(4);
    let vars: Vec<u32> = (0..4).collect();
    let edges: Vec<(u32, u32)> = vec![(0, 1), (1, 2), (2, 3)];
    let out = refine_td_with_flowcutter_cut(td.clone(), &vars, &edges, None);
    assert_eq!(out.bags.len(), td.bags.len());
    assert_eq!(out.treewidth(), td.treewidth());
}

#[test]
fn refine_preserves_coverage_and_rip_on_path() {
    // 32-vertex path graph.  FlowCutter should cut it cleanly.
    let num_vertices = 32u32;
    let vars: Vec<u32> = (0..num_vertices).collect();
    let edges: Vec<(u32, u32)> = (0..num_vertices - 1).map(|i| (i, i + 1)).collect();

    // Start from a deliberately bad TD: one giant bag containing every vertex.
    let td = trivial_td(num_vertices);

    let out = refine_td_with_flowcutter_cut(td.clone(), &vars, &edges, None);

    out.validate(&Graph::new(num_vertices, edges))
        .expect("refinement preserves the decomposition contract");
}
