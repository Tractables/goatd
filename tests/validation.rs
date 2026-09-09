use goatd::{Graph, TreeDecomposition};

// An independent check on small inputs: cover edges directly and walk the
// induced bag graph for each vertex, including disconnected bag forests.
fn is_decomposition(bags: &[Vec<u32>], tree: &[(usize, usize)], edges: &[(u32, u32)]) -> bool {
    for vertex in 0..3 {
        let Some(start) = bags.iter().position(|bag| bag.contains(&vertex)) else {
            return false;
        };
        let mut seen = vec![false; bags.len()];
        seen[start] = true;
        let mut stack = vec![start];
        while let Some(bag) = stack.pop() {
            for &(u, v) in tree {
                let next = if u == bag {
                    v
                } else if v == bag {
                    u
                } else {
                    continue;
                };
                if !seen[next] && bags[next].contains(&vertex) {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        if bags
            .iter()
            .enumerate()
            .any(|(i, bag)| bag.contains(&vertex) && !seen[i])
        {
            return false;
        }
    }
    edges
        .iter()
        .all(|(u, v)| bags.iter().any(|bag| bag.contains(u) && bag.contains(v)))
}

#[test]
fn validation_agrees_with_direct_coverage_and_connectivity_on_small_forests() {
    let trees: &[&[(usize, usize)]] = &[&[], &[(0, 1)], &[(0, 1), (1, 2)], &[(0, 2), (2, 1)]];
    for bits in 0..512 {
        let bags: Vec<Vec<u32>> = (0..3)
            .map(|bag| {
                (0..3)
                    .filter(|vertex| bits & (1 << (3 * bag + vertex)) != 0)
                    .collect()
            })
            .collect();
        for &tree in trees {
            for edge_bits in 0..8 {
                let edges: Vec<_> = [(0, 1), (0, 2), (1, 2)]
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, edge)| (edge_bits & (1 << i) != 0).then_some(edge))
                    .collect();
                let graph = Graph::new(3, edges.iter().copied());
                assert_eq!(
                    TreeDecomposition::new(&graph, bags.clone(), tree.iter().copied()).is_ok(),
                    is_decomposition(&bags, tree, &edges),
                    "bags={bags:?} tree={tree:?} edges={edges:?}"
                );
            }
        }
    }
}

#[test]
fn validation_handles_a_high_degree_graph_vertex_in_a_branching_bag_tree() {
    let graph = Graph::new(9, (1..9).map(|v| (0, v)));
    let mut bags = vec![vec![0]];
    bags.extend((1..9).map(|v| vec![v, 0]));
    let tree: Vec<_> = (1..9).map(|bag| (0, bag)).collect();
    let td = TreeDecomposition::new(&graph, bags, tree).unwrap();
    td.validate(&graph).unwrap();
    assert_eq!(td.treewidth(), 1);
}

#[test]
fn trusted_construction_matches_checked_construction_for_valid_input() {
    let graph = Graph::new(3, [(0, 1), (1, 2)]);
    let bags = [vec![1, 0], vec![2, 1]];
    let edges = [(1, 0)];
    let checked = TreeDecomposition::new(&graph, bags.clone(), edges).unwrap();
    let trusted = TreeDecomposition::new_trusted(&graph, bags, edges).unwrap();
    assert_eq!(trusted, checked);
    trusted.validate(&graph).unwrap();
}

#[test]
#[cfg_attr(
    debug_assertions,
    should_panic(expected = "invalid trusted decomposition")
)]
fn trusted_construction_checks_the_graph_contract_only_in_debug_builds() {
    let graph = Graph::new(3, [(0, 2)]);
    let td = TreeDecomposition::new_trusted(&graph, [vec![0, 1], vec![1, 2]], [(0, 1)])
        .expect("bag-tree endpoints are in range");
    assert!(td.validate(&graph).is_err());
}

#[test]
fn trusted_construction_rejects_out_of_range_bag_tree_endpoints() {
    let graph = Graph::new(1, []);
    let error = TreeDecomposition::new_trusted(&graph, [vec![0]], [(0, 1)]).unwrap_err();
    assert!(error.to_string().contains("outside 0..1"));
}
