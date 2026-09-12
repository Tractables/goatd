use crate::elimination::execution::{ElimSink, ElimStop};
use crate::elimination::graph::EliminationGraph;
use crate::elimination::greedy::min_fill::eliminate_min_fill;

fn check_order<const RELATIVE: bool>(edges: &[(u32, u32)], salt: &[u32]) -> Vec<u32> {
    let n = salt.len();
    let mut graph = EliminationGraph::from_edges(n as u32, edges);
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    eliminate_min_fill::<RELATIVE>(
        &mut graph,
        salt,
        ElimSink::new(&mut bags, &mut rank, 0),
        ElimStop::default(),
    );
    assert_eq!(bags.len(), n);
    let mut adjacent = vec![vec![false; n]; n];
    let mut active = vec![true; n];
    for &(u, v) in edges {
        adjacent[u as usize][v as usize] = true;
        adjacent[v as usize][u as usize] = true;
    }
    for bag in &bags {
        let scores: Vec<_> = (0..n)
            .filter(|&v| active[v])
            .map(|v| {
                let neighbours: Vec<_> = (0..n).filter(|&u| active[u] && adjacent[v][u]).collect();
                let mut fill = 0u128;
                for (i, &u) in neighbours.iter().enumerate() {
                    for &w in &neighbours[i + 1..] {
                        fill += u128::from(!adjacent[u][w]);
                    }
                }
                (v, fill, neighbours.len())
            })
            .collect();
        let &(expected, _, _) = scores
            .iter()
            .min_by(|&&(v, f, d), &&(w, g, e)| {
                let primary = if RELATIVE {
                    (f * e.max(1) as u128).cmp(&(g * d.max(1) as u128))
                } else {
                    f.cmp(&g)
                };
                primary.then_with(|| (d, salt[v], v).cmp(&(e, salt[w], w)))
            })
            .unwrap();
        assert_eq!(
            bag[0] as usize, expected,
            "relative={RELATIVE}, edges={edges:?}"
        );
        let v = expected;
        let neighbours: Vec<_> = (0..n).filter(|&u| active[u] && adjacent[v][u]).collect();
        assert_eq!(bag.len(), neighbours.len() + 1);
        for &u in &neighbours {
            assert!(bag.contains(&(u as u32)));
            for &w in &neighbours {
                if u != w {
                    adjacent[u][w] = true;
                }
            }
        }
        active[v] = false;
    }
    bags.iter().map(|bag| bag[0]).collect()
}

#[test]
fn relative_fill_and_ordinary_fill_match_exact_six_vertex_oracles() {
    let all: Vec<_> = (0..6)
        .flat_map(|u| (u + 1..6).map(move |v| (u, v)))
        .collect();
    let mut different = 0;
    for mask in 0..1u32 << all.len() {
        let edges: Vec<_> = all
            .iter()
            .enumerate()
            .filter_map(|(i, &edge)| (mask & (1 << i) != 0).then_some(edge))
            .collect();
        let salt = [7, 2, 2, 9, 0, 3];
        let ordinary = check_order::<false>(&edges, &salt);
        let relative = check_order::<true>(&edges, &salt);
        different += usize::from(ordinary != relative);
    }
    assert!(
        different > 0,
        "relative ranking must make different choices"
    );
    println!("relative fill changes {different} of 32768 exact six-vertex orders");
}

#[test]
fn relative_fill_handles_empty_and_expired_searches() {
    assert!(check_order::<true>(&[], &[]).is_empty());
    let mut graph = EliminationGraph::from_edges(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]);
    let mut bags = Vec::new();
    let mut rank = Vec::new();
    let exit = eliminate_min_fill::<true>(
        &mut graph,
        &[0; 4],
        ElimSink::new(&mut bags, &mut rank, 0),
        ElimStop {
            hard_deadline: Some(crate::meter::now()),
            ..ElimStop::default()
        },
    );
    assert!(matches!(
        exit,
        crate::elimination::execution::ElimExit::DeadlineReached(_)
    ));
    assert!(bags.is_empty());
}
