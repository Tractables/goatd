use goatd::decomposition::vertex_rebuild::improve;
use goatd::{Graph, TreeDecomposition};
use std::time::Instant;

#[test]
fn the_checked_entry_rejects_a_tree_for_a_different_graph() {
    let graph = Graph::new(2, []);
    let tree = TreeDecomposition::new(&graph, [vec![0], vec![1]], [(0, 1)]).unwrap();
    assert!(improve(&Graph::new(2, [(0, 1)]), &tree, Instant::now()).is_err());
}
