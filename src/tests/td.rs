//! The tree-decomposition interchange type and the reader that fills it.

use crate::tests::td_fixture::make_td;
use crate::{Graph, TreeDecomposition};

/// The bag list and the adjacency are indexed alike, and a consumer holding a
/// bag's position reads its neighbours at that same position. The solution line
/// sizes one and the bag lines fill the other, so a file whose two disagree
/// would slide a bag's neighbours onto a different bag.
#[test]
fn a_declared_bag_count_the_file_does_not_match_leaves_bags_and_adj_co_indexed() {
    // Three bags declared, two written.
    let short = "s td 3 2 3\nb 1 1 2\nb 2 2 3\n1 2\n";
    let err = TreeDecomposition::from_td(short)
        .map(|_| ())
        .expect_err("a file defining fewer bags than it declares is not a decomposition")
        .to_string();
    assert!(
        err.contains('3') && err.contains('2'),
        "the message must name both counts, got: {err}",
    );

    // The same disagreement reached the other way: as many bag lines as
    // declared, but one bag defined twice and another not at all.
    let repeated = "s td 3 2 3\nb 1 1 2\nb 1 2 3\nb 3 3\n";
    let err = TreeDecomposition::from_td(repeated)
        .map(|_| ())
        .expect_err("a bag defined twice leaves another undefined")
        .to_string();
    assert!(
        err.contains('1'),
        "the message must name the repeated bag, got: {err}",
    );

    // The well-formed file: one line per declared bag, and the two lists agree.
    let whole = TreeDecomposition::from_td("s td 3 2 3\nb 1 1 2\nb 2 2 3\nb 3 3\n1 2\n2 3\n")
        .expect("a file defining each declared bag once");
    assert_eq!(whole.bags.len(), whole.adj.len());
    for (position, bag) in whole.bags.iter().enumerate() {
        assert_eq!(bag.id, position, "bag ids index the adjacency");
    }
    assert!(whole.adj[1].contains(&0) && whole.adj[1].contains(&2));
}

/// The treewidth is one less than the largest bag, and a decomposition with
/// nothing in it is width zero rather than an underflow: no bags, one empty bag
/// and one single-vertex bag all decompose something that needs no separator at
/// all.
#[test]
fn treewidth_is_the_largest_bag_less_one_and_zero_when_there_is_nothing_to_decompose() {
    let cases = [
        (Vec::new(), 0, "no bags at all"),
        (vec![Vec::new()], 0, "one bag with nothing in it"),
        (vec![vec![0]], 0, "one bag holding one vertex"),
        (
            vec![vec![0, 1], vec![1, 2], vec![2, 3]],
            1,
            "a path of two-vertex bags",
        ),
        (vec![vec![0, 1], vec![1, 2, 3]], 2, "a widest bag of three"),
    ];
    for (bags, expected, what) in cases {
        let decomposition = make_td(bags, Vec::new());
        assert_eq!(
            decomposition.treewidth(),
            expected,
            "{what} has width {expected}",
        );
    }
}

/// What `to_td` writes, `from_td` reads back as the same decomposition, and
/// the solution line carries the largest bag and the vertex count it was
/// given.
#[test]
fn a_decomposition_written_as_td_reads_back_as_itself() {
    let td = make_td(
        vec![vec![0, 1, 2], vec![1, 2, 3], vec![3, 4, 5]],
        vec![(0, 1), (1, 2)],
    );
    let text = td.to_td(6);
    assert!(
        text.starts_with("s td 3 3 6\n"),
        "the solution line names the bag count, the largest bag and the vertex count: {text}",
    );
    let back = TreeDecomposition::from_td(&text).expect("what to_td wrote parses");
    assert_eq!(back, td);
}

#[test]
fn validation_accepts_a_decomposition_of_the_graph() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);
    let td = make_td(
        vec![vec![0, 1], vec![1, 2], vec![2, 3]],
        vec![(0, 1), (1, 2)],
    );

    td.validate(&graph).expect("a path decomposition");
}

#[test]
fn validation_accepts_one_bag_tree_per_graph_component() {
    let td = make_td(vec![vec![0, 1], vec![2, 3], vec![4]], Vec::new());

    td.validate(&Graph::new(5, [(0, 1), (2, 3)]))
        .expect("a decomposition forest of a disconnected graph");
}

#[test]
fn an_empty_decomposition_validates_for_an_empty_graph() {
    let td = TreeDecomposition {
        bags: Vec::new(),
        adj: Vec::new(),
    };

    td.validate(&Graph::new(0, [])).expect("the empty graph");
}

#[test]
fn validation_requires_bag_ids_to_index_the_bag_list() {
    let mut td = make_td(vec![vec![0]], Vec::new());
    td.bags[0].id = 7;

    let error = td
        .validate(&Graph::new(1, []))
        .expect_err("the bag id is not its position")
        .to_string();
    assert!(error.contains("bag 7") && error.contains("position 0"));
}

#[test]
fn validation_requires_the_bag_adjacency_to_be_acyclic() {
    let td = make_td(
        vec![vec![0], vec![0], vec![0]],
        vec![(0, 1), (1, 2), (2, 0)],
    );

    let error = td
        .validate(&Graph::new(1, []))
        .expect_err("a cycle is not a decomposition tree")
        .to_string();
    assert!(error.contains("3 edges") && error.contains("forest") && error.contains("2"));
}

#[test]
fn validation_requires_every_graph_vertex_to_be_in_a_bag() {
    let td = make_td(vec![vec![0, 1], vec![1, 2]], vec![(0, 1)]);

    let error = td
        .validate(&Graph::new(4, [(0, 1), (1, 2)]))
        .expect_err("vertex 3 is missing")
        .to_string();
    assert!(error.contains("vertex 3") && error.contains("no bag"));
}

#[test]
fn validation_requires_every_graph_edge_to_be_in_a_bag() {
    let td = make_td(vec![vec![0, 1], vec![1, 2]], vec![(0, 1)]);

    let error = td
        .validate(&Graph::new(3, [(0, 2)]))
        .expect_err("the endpoints never share a bag")
        .to_string();
    assert!(error.contains("edge (0, 2)") && error.contains("no bag"));
}

#[test]
fn validation_requires_the_running_intersection_property() {
    let td = make_td(
        vec![vec![0, 1], vec![1, 2], vec![0, 2]],
        vec![(0, 1), (1, 2)],
    );

    let error = td
        .validate(&Graph::new(3, [(0, 1), (1, 2), (0, 2)]))
        .expect_err("the two bags holding vertex 0 are separated")
        .to_string();
    assert!(error.contains("vertex 0") && error.contains("not connected"));
}
