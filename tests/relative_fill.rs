use goatd::{
    Graph,
    elimination::{self, Order},
};

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
