use std::time::Instant;

use goatd::decomposition::{
    FlowCutterConfig, FlowCutterSession,
    polishing::{Advance, Budget, Pause},
    vertex_rebuild,
};
use goatd::{Graph, TreeDecomposition, meter};

fn grid(side: u32) -> Graph {
    Graph::new(
        side * side,
        (0..side * side).flat_map(|v| {
            [
                (v % side + 1 < side).then_some((v, v + 1)),
                (v / side + 1 < side).then_some((v, v + side)),
            ]
            .into_iter()
            .flatten()
        }),
    )
}

fn giant(graph: &Graph) -> TreeDecomposition {
    TreeDecomposition::new(graph, [(0..graph.num_vertices()).collect::<Vec<_>>()], []).unwrap()
}

#[test]
fn zero_and_expired_slices_do_no_lazy_work_even_with_a_work_clock() {
    let graph = grid(4);
    let tree = giant(&graph);
    let _clock = meter::arm(Instant::now());
    let mut vertex = vertex_rebuild::Session::new(&graph, tree.clone()).unwrap();
    let mut flow =
        FlowCutterSession::new(&graph, tree.clone(), FlowCutterConfig::default()).unwrap();
    assert!(matches!(
        vertex.advance(Budget::new(0)),
        Advance::Paused(Pause::Steps)
    ));
    assert!(matches!(
        flow.advance(Budget::new(0)),
        Advance::Paused(Pause::Steps)
    ));
    let expired = Budget::new(100).with_deadline(Instant::now());
    assert!(matches!(
        vertex.advance(expired),
        Advance::Paused(Pause::Deadline)
    ));
    assert!(matches!(
        flow.advance(expired),
        Advance::Paused(Pause::Deadline)
    ));
    assert_eq!(vertex.progress().steps, 0);
    assert_eq!(flow.progress().steps, 0);
    assert_eq!(vertex.current().to_td(), tree.to_td());
    assert_eq!(flow.current().to_td(), tree.to_td());
}

#[test]
fn rejected_vertex_proposals_resume_the_same_sweep() {
    let graph = grid(3);
    let run = |budget| {
        let tree = giant(&graph);
        let original = tree.to_td();
        let mut session = vertex_rebuild::Session::new(&graph, tree).unwrap();
        let mut proposals = Vec::new();
        let mut exhausted = false;
        for _ in 0..100 {
            match session.advance(Budget::new(budget)) {
                Advance::Proposal(proposal) => {
                    proposal.candidate().validate(&graph).unwrap();
                    proposals.push(proposal.candidate().to_td());
                }
                Advance::Paused(Pause::Steps) => {}
                Advance::Exhausted => {
                    exhausted = true;
                    break;
                }
                _ => panic!("unexpected pause"),
            }
        }
        assert!(exhausted, "a rejected sweep must terminate");
        assert_eq!(session.current().to_td(), original);
        assert_eq!(session.progress().attempted, graph.num_vertices() as u64);
        (proposals, session.progress().steps)
    };
    let continuous = run(100);
    assert!(!continuous.0.is_empty());
    assert_eq!(continuous, run(1));
}

#[test]
fn callers_can_accept_a_proposal_without_a_strict_width_improvement() {
    let graph = Graph::new(4, [(0, 1), (1, 2), (2, 3)]);
    let tree = TreeDecomposition::new(
        &graph,
        [vec![0, 1], vec![1, 2], vec![2, 3]],
        [(0, 1), (1, 2)],
    )
    .unwrap();
    let mut session = vertex_rebuild::Session::new(&graph, tree).unwrap();
    let Advance::Proposal(proposal) = session.advance(Budget::new(100)) else {
        panic!("a reconstruction should be offered");
    };
    assert_eq!(
        proposal.candidate().treewidth(),
        proposal.current().treewidth()
    );
    let accepted = proposal.candidate().to_td();
    proposal.accept();
    assert_eq!(session.current().to_td(), accepted);
    assert_eq!(session.progress().accepted, 1);
    let next =
        FlowCutterSession::new(&graph, session.into_tree(), FlowCutterConfig::default()).unwrap();
    assert_eq!(next.current().to_td(), accepted);
    assert_eq!(next.progress().attempted, 0);
}

#[test]
fn explicit_vertex_order_counts_attempts_without_candidates() {
    let graph = Graph::new(4, [(1, 2), (2, 3)]);
    let mut session = vertex_rebuild::Session::new(&graph, giant(&graph)).unwrap();
    assert!(session.set_vertex_order(vec![0, 1, 1, 3]).is_err());
    session.set_vertex_order(vec![0, 3, 2, 1]).unwrap();
    assert!(matches!(
        session.advance(Budget::new(1)),
        Advance::Paused(Pause::Steps)
    ));
    assert!(matches!(
        session.advance(Budget::new(1)),
        Advance::Paused(Pause::Steps)
    ));
    assert_eq!(session.progress().attempted, 1);
    assert_eq!(session.progress().proposed, 0);
    assert!(matches!(
        session.advance(Budget::new(1)),
        Advance::Proposal(_)
    ));
    assert_eq!(session.progress().attempted, 2);
}

#[test]
fn sliced_separator_search_preserves_recursive_proposals_and_acceptance() {
    let graph = grid(4);
    let config = FlowCutterConfig::default()
        .with_iterations(4)
        .with_vertex_limits(4, None)
        .with_max_depth(3);
    let run = |budget| {
        let mut session = FlowCutterSession::new(&graph, giant(&graph), config).unwrap();
        let mut proposals = Vec::new();
        let mut recursive = false;
        let mut exhausted = false;
        for _ in 0..10_000 {
            let subregion = session
                .region_vertices()
                .is_some_and(|vertices| vertices.len() < graph.num_vertices() as usize);
            match session.advance(Budget::new(budget)) {
                Advance::Proposal(proposal) => {
                    recursive |= subregion;
                    proposal.current().validate(&graph).unwrap();
                    proposal.candidate().validate(&graph).unwrap();
                    let key = |tree: &TreeDecomposition| (tree.treewidth(), tree.total_bag_size());
                    let accept = key(proposal.candidate()) < key(proposal.current());
                    proposals.push((proposal.candidate().to_td(), accept));
                    if accept {
                        proposal.accept();
                    }
                }
                Advance::Paused(Pause::Steps) => {}
                Advance::Exhausted => {
                    exhausted = true;
                    break;
                }
                _ => panic!("unexpected pause"),
            }
        }
        assert!(
            exhausted,
            "bounded recursion and restart counts must terminate"
        );
        assert!(
            recursive,
            "the fixture must exercise a recursive replacement"
        );
        let progress = session.progress();
        (
            proposals,
            session.into_tree().to_td(),
            progress.steps,
            progress.attempted,
        )
    };
    assert_eq!(run(10_000), run(1));
}

#[test]
fn inspecting_the_current_separator_does_not_restart_it() {
    let graph = grid(4);
    let mut session = FlowCutterSession::new(
        &graph,
        giant(&graph),
        FlowCutterConfig::default().with_iterations(4),
    )
    .unwrap();
    let original = session.current().to_td();
    let Advance::Proposal(proposal) = session.advance(Budget::new(10_000)) else {
        panic!("a restart should offer a separator");
    };
    proposal.reject();
    let attempts = session.progress().attempted;
    assert!(session.set_config(FlowCutterConfig::default()).is_err());
    let proposal = session
        .offer_current()
        .expect("a completed restart has a current separator");
    proposal.candidate().validate(&graph).unwrap();
    proposal.reject();
    assert_eq!(session.progress().attempted, attempts);
    assert_eq!(session.current().to_td(), original);
}

#[test]
fn recursive_acceptance_commits_the_complete_offered_tree() {
    let graph = grid(5);
    for accept_every in [1, 2] {
        let config = FlowCutterConfig::default()
            .with_iterations(3)
            .with_vertex_limits(3, None)
            .with_max_depth(4);
        let mut session = FlowCutterSession::new(&graph, giant(&graph), config).unwrap();
        let mut proposals = 0;
        let mut exhausted = false;
        for _ in 0..20_000 {
            let mut accepted = None;
            match session.advance(Budget::new(1)) {
                Advance::Proposal(proposal) => {
                    proposal.candidate().validate(&graph).unwrap();
                    proposals += 1;
                    if proposals % accept_every == 0 {
                        accepted = Some(proposal.candidate().to_td());
                        proposal.accept();
                    }
                }
                Advance::Paused(Pause::Steps) => {}
                Advance::Exhausted => {
                    exhausted = true;
                    break;
                }
                _ => panic!("unexpected pause"),
            }
            if let Some(tree) = accepted {
                assert_eq!(session.current().to_td(), tree);
            }
            session.current().validate(&graph).unwrap();
        }
        assert!(exhausted);
        assert!(proposals > 2);
    }
}
