//! PACE text: what a `.gr` and a `.td` parse to, and what a rejection names.

use goatd::{Graph, TreeDecomposition};

#[test]
fn a_gr_file_parses_to_a_canonical_graph_and_writes_back_the_same() {
    // Comment lines, an edge given both ways, a self-loop and a duplicate.
    let text = "c a comment\np tw 4 5\n1 2\n2 1\n3 3\n2 3\n2 3\n";
    let graph = Graph::from_gr(text).expect("a well-formed .gr");
    assert_eq!(graph.num_vertices(), 4);
    assert_eq!(graph.edges(), [(0, 1), (1, 2)]);
    assert_eq!(graph.to_gr(), "p tw 4 2\n1 2\n2 3\n");
    assert_eq!(
        Graph::from_gr(&graph.to_gr()).expect("what to_gr wrote parses"),
        graph
    );
}

#[test]
fn a_rejected_gr_names_what_is_wrong() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "1 2\n",
            "an edge before the problem line",
            &["problem line"],
        ),
        ("p tw 2\n", "a truncated problem line", &["problem line"]),
        ("p tw 2 1\n0 1\n", "vertex 0", &["vertex 0", "1-based"]),
        (
            "p tw 2 1\n1 3\n",
            "a vertex past the count",
            &["vertex 3", "declares 2"],
        ),
        ("p tw 2 1\n1\n", "an edge with one endpoint", &["edge line"]),
        ("p tw x 1\n1 2\n", "a non-numeric vertex count", &[]),
        ("p tw 2 x\n1 2\n", "a non-numeric edge count", &[]),
        (
            "p tw 2 1 extra\n1 2\n",
            "extra fields on a problem line",
            &["problem line"],
        ),
        (
            "p tw 2 1\np tw 2 1\n1 2\n",
            "a second problem line",
            &["more than one problem line"],
        ),
        (
            "p tw 2 0\n1 2\n",
            "more edge lines than the problem line declares",
            &["declares 0 edge lines", "contains 1"],
        ),
        (
            "p tw 2 2\n1 2\n",
            "fewer edge lines than the problem line declares",
            &["declares 2 edge lines", "contains 1"],
        ),
    ];
    for &(text, what, expected) in cases {
        let err = Graph::from_gr(text)
            .map(|_| ())
            .expect_err(what)
            .to_string();
        for want in expected {
            assert!(
                err.contains(want),
                "{what}: the message must name {want:?}, got: {err}",
            );
        }
    }
}

#[test]
fn a_td_file_parses_its_bags_and_tree_edges() {
    // Hand-crafted .td: 2 bags, 1 tree edge
    // Bag 1: vertices 1,2 → stored as 0,1
    // Bag 2: vertices 2,3 → stored as 1,2
    // The undirected tree edge is deliberately written in descending order.
    let td_str = "s td 2 2 3\nb 1 2 1\nb 2 3 2\n2 1\n";
    let td = TreeDecomposition::from_td(td_str).expect("Should parse");
    assert_eq!(td.bags().len(), 2);

    assert_eq!(td.bags()[0].vertices(), [0, 1]);
    assert_eq!(td.bags()[1].vertices(), [1, 2]);

    assert!(
        td.adjacency()[0].contains(&1),
        "Bag 0 should be adjacent to bag 1"
    );
    assert!(
        td.adjacency()[1].contains(&0),
        "Bag 1 should be adjacent to bag 0"
    );
}

#[test]
fn a_td_can_be_written_directly_to_an_io_writer() {
    let expected = "s td 2 2 3\nb 1 1 2\nb 2 2 3\n1 2\n";
    let td = TreeDecomposition::from_td(expected).expect("a well-formed .td");
    let mut written = Vec::new();

    td.write_td(&mut written).expect("write the decomposition");

    assert_eq!(written, expected.as_bytes());
}

#[test]
fn td_comments_are_ignored() {
    let td_str = "c comment line\ns td 1 2 2\nb 1 1 2\n";
    let td = TreeDecomposition::from_td(td_str).expect("Should parse with comments");
    assert_eq!(td.bags().len(), 1);
    assert_eq!(td.bags()[0].vertices(), [0, 1]);
}

#[test]
fn empty_text_is_not_a_tree_decomposition() {
    let result = TreeDecomposition::from_td("");
    assert!(result.is_err(), "empty TD should error");
}

#[test]
fn a_solution_line_without_bags_is_not_a_tree_decomposition() {
    let result = TreeDecomposition::from_td("s td 2 2 3\n");
    assert!(result.is_err(), "TD with header but no bags should error");
}

#[test]
fn blank_lines_are_not_a_tree_decomposition() {
    let result = TreeDecomposition::from_td("\n\n\n");
    assert!(result.is_err(), "blank lines should give no bags");
}

/// Malformed output is rejected, never a panic, and the rejection names the id
/// it read and the count that id had to fit inside, which is the pair a reader
/// of the file needs to find the line at fault. Bag and vertex ids are written
/// 1-based, so `0` is not an id and the solution line's counts bound the rest.
///
/// The two cases whose expectation is empty are rejected by the number parser,
/// whose wording belongs to it.
#[test]
fn a_rejected_td_names_the_offending_id_and_the_count_it_was_checked_against() {
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "s td 2 2 3\nb 0 1 2\nb 2 2 3\n",
            "bag id 0",
            &["bag id 0", "1-based"],
        ),
        (
            "s td 1 2 2\nb 1 0 2\n",
            "vertex 0",
            &["vertex 0", "1-based"],
        ),
        (
            "s td 2 2 3\nb 1 1 2\nb 2 2 3\n0 2\n",
            "bag id 0 on a tree edge",
            &["bag id 0", "1-based"],
        ),
        (
            "s td 1 2 2\nb 5 1 2\n",
            "a bag id past the declared bag count",
            &["bag id 5", "declares 1"],
        ),
        (
            "s td 1 2 2\nb 1 1 9\n",
            "a vertex past the declared vertex count",
            &["vertex 9", "declares 2"],
        ),
        (
            "s td 2 2 3\nb 1 1 2\nb 2 2 3\n1 9\n",
            "a tree edge past the declared bag count",
            &["bag id 9", "declares 2"],
        ),
        (
            "s td 1 1 1\nb 1 1\n1 1\n",
            "a self-loop in the bag tree",
            &["adjacent to itself", "1 1"],
        ),
        (
            "b 1 1 2\n",
            "a bag line before the solution line",
            &["before the solution line"],
        ),
        ("s td x 2 3\nb 1 1 2\n", "a non-numeric bag count", &[]),
        (
            "s td 1\nb 1 1 2\n",
            "a truncated solution line",
            &["malformed solution line"],
        ),
        ("s td 1 2 2\nb 1 1 x\n", "a non-numeric vertex", &[]),
        (
            "s not-td 1 2 2\nb 1 1 2\n",
            "the wrong solution kind",
            &["malformed solution line"],
        ),
        (
            "s td 1 x 2\nb 1 1 2\n",
            "a non-numeric maximum bag size",
            &[],
        ),
        (
            "s td 1 1 2\nb 1 1 2\n",
            "a bag larger than the declared maximum",
            &["maximum bag size 1", "contains 2 vertices"],
        ),
        (
            "s td 1 2 1\nb 1 1 1\n",
            "a repeated vertex within one bag",
            &["bag 1", "vertex 1", "more than once"],
        ),
        (
            "s td 1 3 2\nb 1 1 2\n",
            "a declared maximum larger than every bag",
            &["maximum bag size 3", "contains 2 vertices"],
        ),
        (
            "s td 1 1 1\ns td 1 1 1\nb 1 1\n",
            "a second solution line",
            &["more than one solution line"],
        ),
        (
            "s td 1 1 1\nb 1 1\nnot a tree edge\n",
            "a malformed tree edge",
            &["malformed tree edge"],
        ),
        (
            "s td 1 1 1\nb 1 1\none two\n",
            "a non-numeric tree edge",
            &[],
        ),
        (
            "s td 2 1 2\nb 1 1\nb 2 2\n",
            "a disconnected bag graph",
            &["bag tree has 0 edges", "tree on 2 bags has 1"],
        ),
        (
            "s td 3 1 3\nb 1 1\nb 2 2\nb 3 3\n1 2\n1 3\n2 3\n",
            "a cycle in the bag graph",
            &["bag tree has 3 edges", "tree on 3 bags has 2"],
        ),
        (
            "s td 2 1 3\nb 1 1\nb 2 2\n1 2\n",
            "a declared vertex absent from every bag",
            &["vertex 2", "no bag"],
        ),
        (
            "s td 3 2 3\nb 1 1 2\nb 2 2 3\nb 3 1 3\n1 2\n2 3\n",
            "a vertex whose bags are disconnected",
            &["vertex 0", "not connected"],
        ),
    ];
    for &(td_str, what, expected) in cases {
        let err = TreeDecomposition::from_td(td_str)
            .map(|_| ())
            .expect_err(what)
            .to_string();
        for want in expected {
            assert!(
                err.contains(want),
                "{what}: the message must name {want:?}, got: {err}",
            );
        }
    }
}
