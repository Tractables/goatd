use crate::Graph;
use crate::td_ops::*;
use crate::tests::td_fixture::{make_td, make_test_td};
use rustc_hash::FxHashSet;

#[test]
fn glue_at_separator_preserves_rip_and_covers_all_vertices() {
    // Side A: two bags covering {0, 1, 2, 3}; S = {2, 3}.
    let td_a = make_td(vec![vec![0, 1, 2], vec![1, 2, 3]], vec![(0, 1)]);
    // Side B: two bags covering {2, 3, 4, 5}; S = {2, 3}.
    let td_b = make_td(vec![vec![2, 3, 4], vec![3, 4, 5]], vec![(0, 1)]);
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
    let td_a = make_td(vec![vec![0, 1, 2], vec![1, 3]], vec![(0, 1)]);
    // Side B has a single bag with all of S.
    let td_b = make_td(vec![vec![1, 2, 3, 4, 5, 6]], Vec::new());
    let sep = vec![1u32, 2u32, 3u32];
    let glued = glue_at_separator(td_a, td_b, &sep).expect("glue should succeed");

    glued
        .validate(&Graph::new(7, []))
        .expect("the glued decomposition is valid");

    // Separator bag at index 0 contains exactly S.
    assert_eq!(glued.bags[0].vertices, vec![1, 2, 3]);
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
    let keep: FxHashSet<u32> = [0, 1, 2, 3].iter().copied().collect();
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
    proj.validate(&Graph::new(4, []))
        .expect("the projected decomposition is valid");
}

#[test]
fn project_td_full_set() {
    let td = make_test_td();
    let all: FxHashSet<u32> = (0..6).collect();
    let proj = project_td(&td, &all).unwrap();
    assert_eq!(proj.td.bags.len(), 3);
    assert_eq!(proj.local_to_global, vec![0, 1, 2, 3, 4, 5]);
}

#[test]
fn project_td_subset_removes_empty_bags() {
    let td = make_test_td();
    // Keep only vertices {0, 1, 2} — bag2 ({3,4,5}) becomes empty.
    let keep: FxHashSet<u32> = [0, 1, 2].iter().copied().collect();
    let proj = project_td(&td, &keep).unwrap();
    // bag0 and bag1 survive (bag1 has {1,2} after projection).
    assert_eq!(proj.td.bags.len(), 2);
    assert_eq!(proj.local_to_global, vec![0, 1, 2]);
}

#[test]
fn project_td_contracts_through_empty() {
    // Keep {0, 1, 4, 5} — bag1 becomes empty, bag0 and bag2 should be connected.
    let td = make_td(
        vec![vec![0, 1], vec![2, 3], vec![4, 5]],
        vec![(0, 1), (1, 2)],
    );
    let keep: FxHashSet<u32> = [0, 1, 4, 5].iter().copied().collect();
    let proj = project_td(&td, &keep).unwrap();
    assert_eq!(proj.td.bags.len(), 2);
    assert!(proj.td.adj[0].contains(&1));
    assert!(proj.td.adj[1].contains(&0));
}

#[test]
fn project_td_single_vertex() {
    let td = make_test_td();
    let keep: FxHashSet<u32> = [3].iter().copied().collect();
    let proj = project_td(&td, &keep).unwrap();
    // Vertex 3 is in bag1 and bag2, so both survive (each with just {0} after
    // renumbering).
    assert!(!proj.td.bags.is_empty());
    assert_eq!(proj.local_to_global, vec![3]);
}
