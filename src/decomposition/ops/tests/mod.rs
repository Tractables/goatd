use crate::Graph;
use crate::tests::td_fixture::{make_td, make_td_for, make_test_td};
use rustc_hash::FxHashSet;

use super::*;

#[test]
fn rooted_forest_walks_each_component_in_breadth_first_order() {
    let adj = vec![vec![1, 2], vec![0], vec![0], vec![4], vec![3]];
    let rooted = rooted_forest_from_adjacency(&adj, [2, 0, 3, 4, 1]);

    assert_eq!(rooted.order, vec![2, 0, 1, 3, 4]);
    assert_eq!(rooted.parent, vec![Some(2), Some(0), None, None, Some(3)]);
    assert_eq!(rooted.depth, vec![1, 2, 0, 0, 1]);
    assert_eq!(rooted.component_roots, vec![2, 3]);
}

#[test]
fn compact_subsumed_bags_keeps_incomparable_branches() {
    let graph = Graph::new(3, [(0, 1), (0, 2)]);
    let td = make_td_for(
        3,
        vec![vec![0], vec![0, 1], vec![0, 2]],
        vec![(0, 1), (0, 2)],
    );

    let compacted = td.compact_subsumed_bags();

    assert_eq!(compacted.bags.len(), 2);
    assert_eq!(compacted.total_bag_size(), 4);
    assert_eq!(compacted.adj, vec![vec![1], vec![0]]);
    compacted
        .validate(&graph)
        .expect("contracting a subsumed branch bag preserves validity");
}

#[test]
fn compact_subsumed_bags_contracts_equal_bags_once() {
    let graph = Graph::new(2, [(0, 1)]);
    let td = make_td_for(2, vec![vec![0, 1], vec![0, 1]], vec![(0, 1)]);

    let compacted = td.compact_subsumed_bags();

    assert_eq!(compacted.bags.len(), 1);
    assert_eq!(compacted.total_bag_size(), 2);
    compacted
        .validate(&graph)
        .expect("contracting duplicate bags preserves validity");
}

#[test]
fn glue_at_separator_preserves_rip_and_covers_all_vertices() {
    // Side A: two bags covering {0, 1, 2, 3}; S = {2, 3}.
    let td_a = make_td_for(6, vec![vec![0, 1, 2], vec![1, 2, 3]], vec![(0, 1)]);
    // Side B: two bags covering {2, 3, 4, 5}; S = {2, 3}.
    let td_b = make_td_for(6, vec![vec![2, 3, 4], vec![3, 4, 5]], vec![(0, 1)]);
    let sep = vec![2u32, 3u32];
    let glued = glue_at_separator(td_a, td_b, &sep).expect("glue should succeed");

    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for bag in &glued.bags {
        for &v in &bag.vertices {
            seen.insert(v);
        }
    }
    for v in 0..6 {
        assert!(seen.contains(&v), "vertex {} missing from glued TD", v);
    }

    glued
        .validate(&Graph::new(6, []))
        .expect("the glued decomposition is valid");

    // Root bag (index 0) is the separator bag.
    assert_eq!(glued.bags[0].vertices, vec![2, 3]);
}

#[test]
fn glue_at_separator_handles_sep_not_in_any_single_bag() {
    // Side A has S spread across two bags: {0,1,2} and {1,3} — no single
    // bag contains {1, 2, 3}.  Augmentation should add vertex 2 to bag 1 (on
    // the path from its src bag 0 to the chosen anchor bag 1).
    let td_a = make_td_for(7, vec![vec![0, 1, 2], vec![1, 3]], vec![(0, 1)]);
    // Side B has a single bag with all of S.
    let td_b = make_td_for(7, vec![vec![1, 2, 3, 4, 5, 6]], Vec::new());
    let sep = vec![1u32, 2u32, 3u32];
    let glued = glue_at_separator(td_a, td_b, &sep).expect("glue should succeed");

    glued
        .validate(&Graph::new(7, []))
        .expect("the glued decomposition is valid");

    // Separator bag at index 0 contains exactly S.
    assert_eq!(glued.bags[0].vertices, vec![1, 2, 3]);
}

#[test]
fn glue_rejects_a_side_that_does_not_contain_the_separator() {
    let td_a = make_td_for(3, vec![vec![0, 1]], Vec::new());
    let td_b = make_td_for(3, vec![vec![0, 1, 2]], Vec::new());

    assert!(glue_at_separator(td_a, td_b, &[1, 2]).is_none());
}

/// A side whose bags fall into two components has no path of bags between
/// them, so a separator vertex living in the far one cannot be carried to the
/// anchor along one. Written into both ends regardless, its bags are
/// disconnected and the glued decomposition is not one.
#[test]
fn augmenting_a_disconnected_side_for_a_separator_keeps_the_running_intersection() {
    // Side A in two components, with one separator vertex in each: whichever
    // bag is anchored, the other separator vertex is across the gap.
    let td_a = make_td(vec![vec![0, 1], vec![2, 5]], Vec::new());
    // Side B holds the whole separator in one bag.
    let td_b = make_td(vec![vec![0, 3, 4, 5]], Vec::new());

    let glued = glue_at_separator(td_a, td_b, &[0u32, 5u32]).expect("glue should succeed");
    glued
        .validate(&Graph::new(6, []))
        .expect("the glued decomposition is valid");

    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for bag in &glued.bags {
        for &v in &bag.vertices {
            seen.insert(v);
        }
    }
    for v in 0..6 {
        assert!(seen.contains(&v), "vertex {v} missing from the glued tree");
    }
}

#[test]
fn project_td_keeping_global_ids_preserves_ids() {
    let td = make_test_td();
    let keep = [0, 1, 2, 3];
    let proj = project_td_keeping_global_ids(&td, &keep).unwrap();

    let mut seen: FxHashSet<u32> = FxHashSet::default();
    for bag in &proj.bags {
        for &v in &bag.vertices {
            assert!(
                keep.contains(&v),
                "projected bag contains non-kept vertex {}",
                v
            );
            seen.insert(v);
        }
    }
    for v in [0, 1, 2, 3] {
        assert!(seen.contains(&v), "vertex {} missing after projection", v);
    }
}

#[test]
fn project_td_full_set() {
    let td = make_test_td();
    let all: Vec<u32> = (0..6).collect();
    let proj = td.project(&all).unwrap();
    assert_eq!(proj.decomposition().bags.len(), 3);
    assert_eq!(proj.local_to_original(), [0, 1, 2, 3, 4, 5]);
}

#[test]
fn project_td_subset_removes_empty_bags() {
    let td = make_test_td();
    // Keep only vertices {0, 1, 2} — bag2 ({3,4,5}) becomes empty.
    let keep = [0, 1, 2];
    let proj = td.project(&keep).unwrap();
    // bag0 and bag1 survive (bag1 has {1,2} after projection).
    assert_eq!(proj.decomposition().bags.len(), 2);
    assert_eq!(proj.local_to_original(), [0, 1, 2]);
}

#[test]
fn project_td_contracts_through_empty() {
    // Keep {0, 1, 4, 5} — bag1 becomes empty, bag0 and bag2 should be connected.
    let td = make_td(
        vec![vec![0, 1], vec![2, 3], vec![4, 5]],
        vec![(0, 1), (1, 2)],
    );
    let keep = [0, 1, 4, 5];
    let proj = td.project(&keep).unwrap();
    assert_eq!(proj.decomposition().bags.len(), 2);
    assert!(proj.decomposition().adj[0].contains(&1));
    assert!(proj.decomposition().adj[1].contains(&0));
}

#[test]
fn project_td_single_vertex() {
    let td = make_test_td();
    let keep = [3];
    let proj = td.project(&keep).unwrap();
    // Vertex 3 is in bag1 and bag2, so both survive (each with just {0} after
    // renumbering).
    assert!(!proj.decomposition().bags.is_empty());
    assert_eq!(proj.local_to_original(), [3]);
}

#[test]
fn empty_projection_is_an_empty_decomposition() {
    let td = make_test_td();
    let keep = [];

    assert!(project_td_keeping_global_ids(&td, &keep).is_none());
    let projection = td.project(&keep).unwrap();
    assert!(projection.decomposition().bags().is_empty());
    assert!(projection.local_to_original().is_empty());
}

#[test]
fn gluing_requires_a_bag_on_each_side() {
    let empty = make_td_for(1, Vec::new(), Vec::new());
    let one = make_td_for(1, vec![vec![0]], Vec::new());

    assert!(glue_at_separator(empty.clone(), one.clone(), &[]).is_none());
    assert!(glue_at_separator(one, empty, &[]).is_none());
}
