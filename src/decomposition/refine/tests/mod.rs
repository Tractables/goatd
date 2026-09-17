use super::*;
use crate::tests::td_fixture::make_td;
use crate::{Graph, TreeDecomposition};

fn trivial_td(num_vertices: u32) -> TreeDecomposition {
    make_td(vec![(0..num_vertices).collect()], Vec::new())
}

#[test]
fn refine_noop_on_tiny_subproblem() {
    // Under the min-side-size threshold: should return unchanged.
    let td = trivial_td(4);
    let edges: Vec<(u32, u32)> = vec![(0, 1), (1, 2), (2, 3)];
    let graph = Graph::new(4, edges);
    let out = refine_with_flowcutter(td.clone(), &graph, None).unwrap();
    assert_eq!(out.bags.len(), td.bags.len());
    assert_eq!(out.treewidth(), td.treewidth());
}

#[test]
fn refine_preserves_coverage_and_rip_on_path() {
    // 32-vertex path graph.  FlowCutter should cut it cleanly.
    let num_vertices = 32u32;
    let edges: Vec<(u32, u32)> = (0..num_vertices - 1).map(|i| (i, i + 1)).collect();
    let graph = Graph::new(num_vertices, edges);

    // Start from a deliberately bad TD: one giant bag containing every vertex.
    let td = trivial_td(num_vertices);

    let out = refine_with_flowcutter(td.clone(), &graph, None).unwrap();

    out.validate(&graph)
        .expect("refinement preserves the decomposition contract");
}

#[test]
fn refine_splits_a_disconnected_region_at_its_components() {
    // A 32-vertex path and one vertex joined to nothing. The root region is
    // the whole graph, so the separator search declines it and the pass used
    // to return the input untouched.
    let num_vertices = 33u32;
    let edges: Vec<(u32, u32)> = (0..31).map(|i| (i, i + 1)).collect();
    let graph = Graph::new(num_vertices, edges);
    let td = trivial_td(num_vertices);

    let out = refine_with_flowcutter(td.clone(), &graph, None).unwrap();

    out.validate(&graph)
        .expect("refinement preserves the decomposition contract");
    assert!(
        out.quality_key() <= td.quality_key(),
        "the pass returned something worse than its input"
    );
    assert!(
        out.treewidth() < td.treewidth(),
        "one isolated vertex left the path unrefined at width {}",
        out.treewidth()
    );
}

#[test]
fn a_component_split_moves_no_vertex_between_bags() {
    // Two 8-vertex paths with nothing between them. Both components are under
    // the minimum region size, so the root split at the components is all the
    // pass does, and that split only divides the one bag it started from.
    let num_vertices = 16u32;
    let edges: Vec<(u32, u32)> = (0..7).chain(8..15).map(|i| (i, i + 1)).collect();
    let graph = Graph::new(num_vertices, edges);
    let td = trivial_td(num_vertices);

    let out = refine_with_flowcutter(td.clone(), &graph, None).unwrap();

    out.validate(&graph)
        .expect("refinement preserves the decomposition contract");
    assert_eq!(out.bags.len(), 2);
    assert_eq!(out.total_bag_size(), td.total_bag_size());
    assert_eq!(out.treewidth(), 7);
}

#[test]
fn a_connected_graph_has_no_component_split() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);

    assert!(largest_component_split(&graph).is_none());
}

#[test]
fn a_component_split_takes_the_largest_component_first() {
    // Vertex 0 on its own, a triangle on 1, 2, 3, and a pair on 4, 5.
    let graph = Graph::new(6, [(1, 2), (2, 3), (3, 1), (4, 5)]);

    assert_eq!(
        largest_component_split(&graph),
        Some((vec![1, 2, 3], vec![0, 4, 5]))
    );
}
