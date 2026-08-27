use goatd::Graph;
use goatd::elimination::{Config, elimination_td};

fn main() {
    let graph = Graph::new(
        8,
        [
            (0, 1),
            (1, 2),
            (3, 4),
            (4, 5),
            (0, 3),
            (1, 4),
            (2, 5),
            (1, 3),
            (2, 4),
            (4, 6),
            (5, 6),
            (5, 7),
            (6, 7),
        ],
    );
    let td = elimination_td(&graph, Config::MinFill, 0, None);

    td.validate(&graph).expect("a valid decomposition");
    print!("{}", td.to_td(graph.num_vertices));
}
