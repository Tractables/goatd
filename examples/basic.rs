use goatd::Graph;
use goatd::elimination::{Config, elimination_td};

fn main() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)]);
    let td = elimination_td(&graph, Config::MinFill, 0, None);

    assert_eq!(td.treewidth(), 2);
    print!("{}", td.to_td(graph.num_vertices));
}
