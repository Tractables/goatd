use goatd::{
    Graph,
    elimination::{self, Order},
};

#[test]
fn relative_fill_builds_tie_breaks_for_whole_and_disconnected_residuals() {
    for components in [1, 2] {
        let edges = (0..components).flat_map(|component| {
            let offset = 8 * component;
            (offset..offset + 4)
                .flat_map(move |left| (offset + 4..offset + 8).map(move |right| (left, right)))
        });
        let graph = Graph::new(8 * components, edges);
        for seed in [0, 7] {
            let tree = elimination::decompose(&graph, Order::RelativeFill, seed, None).unwrap();
            tree.validate(&graph).unwrap();
            assert_eq!(tree.treewidth(), 4);
        }
    }
}

#[test]
fn relative_fill_validates_every_five_vertex_graph() {
    let all: Vec<_> = (0..5)
        .flat_map(|u| (u + 1..5).map(move |v| (u, v)))
        .collect();
    for mask in 0..1u32 << all.len() {
        let edges: Vec<_> = all
            .iter()
            .enumerate()
            .filter_map(|(i, &edge)| (mask & (1 << i) != 0).then_some(edge))
            .collect();
        let graph = Graph::new(5, edges);
        let tree = elimination::decompose(&graph, Order::RelativeFill, 0, None).unwrap();
        tree.validate(&graph).unwrap();
    }
}
