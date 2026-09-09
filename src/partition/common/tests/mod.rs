use crate::partition::common::{
    FmBalance, GainBuckets, Stall, balance_bounds, commit_best_prefix, fm_balance, index_split,
    lift_to_fine, matching_order, max_vcycles, project_to_coarse, random_bisection,
    repair_bisection, select_move, shrank_enough, tiny_bisection,
};
use crate::rng::Xorshift64;

#[test]
fn the_index_and_tiny_fallbacks_cover_their_exact_domains() {
    assert_eq!(index_split(0), Vec::<u8>::new());
    assert_eq!(index_split(1), vec![1]);
    assert_eq!(index_split(5), vec![0, 0, 1, 1, 1]);

    assert_eq!(tiny_bisection(0), Some(vec![]));
    assert_eq!(tiny_bisection(1), Some(vec![0]));
    assert_eq!(tiny_bisection(2), Some(vec![0, 1]));
    assert_eq!(tiny_bisection(3), None);
}

#[test]
fn bisections_project_and_lift_through_a_coarsening() {
    let mut counts = Vec::new();
    let mut coarse = Vec::new();
    project_to_coarse(
        &[0, 1, 1, 1, 0],
        &[0, 0, 1, 1, 1],
        2,
        &mut counts,
        &mut coarse,
    );
    assert_eq!(coarse, [0, 1]);

    let mut fine = Vec::new();
    lift_to_fine(&coarse, &[0, 0, 1, 1, 1], &mut fine);
    assert_eq!(fine, [0, 0, 1, 1, 1]);
}

#[test]
fn bisection_repair_enforces_nonempty_sides() {
    assert_eq!(repair_bisection(vec![0, 0, 0, 0], 0.5), [0, 0, 0, 1]);
    assert_eq!(repair_bisection(vec![0, 1, 1], 0.5), [0, 1, 1]);
}

#[test]
fn bisection_repair_moves_only_the_excess_assignments() {
    assert_eq!(
        repair_bisection(vec![0, 0, 0, 0, 0, 1], 0.0),
        [0, 0, 0, 1, 1, 1],
    );
    assert_eq!(repair_bisection(vec![0, 1, 1, 1, 1], 0.1), [0, 1, 1, 1, 0],);
}

#[test]
fn balance_bounds_and_current_weights_use_vertex_weight() {
    assert_eq!(balance_bounds(&[3, 2, 1, 2], 0.25), (2, 6));
    assert_eq!(balance_bounds(&[1, 1, 1], 0.0), (1, 2));
    assert!(fm_balance(2, &[1, 1], &[0, 1], 0.0).is_none());

    let balance =
        fm_balance(4, &[3, 2, 1, 2], &[0, 1, 1, 0], 0.25).expect("four vertices can be refined");
    assert_eq!(balance.weight, [5, 3]);
    assert_eq!(balance.min_part_weight, 2);
    assert_eq!(balance.max_part_weight, 6);
}

#[test]
fn a_random_bisection_repeats_and_never_overfills_its_first_side() {
    let weights = [5, 3, 2, 1, 1];
    let run = || random_bisection(&weights, &mut Xorshift64::from_state(17));

    let part = run();
    assert_eq!(part, run());
    let weight0: u32 = weights
        .iter()
        .zip(&part)
        .map(|(&weight, &side)| if side == 0 { weight } else { 0 })
        .sum();
    assert!(weight0 <= weights.iter().sum::<u32>() / 2);
}

#[test]
fn the_matching_order_is_degree_ascending_and_shuffles_only_inside_a_run() {
    let degrees = [3u32, 1, 2, 1, 3, 1];
    let weights = [1u32; 6];
    let order = matching_order(6, |v| degrees[v], &weights, &mut Xorshift64::from_state(17));

    let visited: Vec<u32> = order.iter().map(|&v| degrees[v]).collect();
    assert_eq!(visited, [1, 1, 1, 2, 3, 3]);
    let mut vertices = order.clone();
    vertices.sort_unstable();
    assert_eq!(vertices, [0, 1, 2, 3, 4, 5]);

    // One stream, one order.
    let repeat = matching_order(6, |v| degrees[v], &weights, &mut Xorshift64::from_state(17));
    assert_eq!(order, repeat);
}

#[test]
fn move_selection_takes_the_best_gain_and_skips_a_side_the_window_blocks() {
    let mut bq = [GainBuckets::new(4), GainBuckets::new(4)];
    bq[0].insert(0, 1);
    bq[0].insert(1, 5);
    bq[1].insert(2, 3);
    bq[1].insert(3, 5);
    let gain = [1i64, 5, 3, 5];
    let locked = [false; 4];
    let vertex_weights = [1u32; 4];

    // Both sides can move, and side 0 keeps a gain tie.
    let open = FmBalance {
        weight: [2, 2],
        min_part_weight: 1,
        max_part_weight: 3,
    };
    assert_eq!(
        select_move(&bq, &gain, &locked, &vertex_weights, &open),
        Some((1, 0, 5))
    );

    // Side 0 is at the floor, so only side 1 is searched.
    let floor = FmBalance {
        weight: [1, 3],
        min_part_weight: 1,
        max_part_weight: 3,
    };
    assert_eq!(
        select_move(&bq, &gain, &locked, &vertex_weights, &floor),
        Some((3, 1, 5))
    );

    // Both sides are at the floor and the ceiling at once: no legal move.
    let pinned = FmBalance {
        weight: [1, 1],
        min_part_weight: 1,
        max_part_weight: 1,
    };
    assert_eq!(
        select_move(&bq, &gain, &locked, &vertex_weights, &pinned),
        None
    );
}

#[test]
fn the_coarsening_floor_and_vcycle_counts_are_read_from_here() {
    assert!(shrank_enough(100, 89));
    assert!(!shrank_enough(100, 90));
    assert!(!shrank_enough(20, 20));

    assert_eq!(max_vcycles(99), 1);
    assert_eq!(max_vcycles(100), 2);
    assert_eq!(max_vcycles(399), 2);
    assert_eq!(max_vcycles(400), 4);
}

#[test]
fn committing_moves_keeps_only_the_best_positive_prefix() {
    let moves = [0, 1, 2];
    let mut part = vec![1, 1, 1];

    assert!(commit_best_prefix(&moves, &[-1, 2, 1], &mut part));
    assert_eq!(part, vec![1, 1, 0]);

    let mut non_improving = vec![1, 1];
    assert!(!commit_best_prefix(&[0, 1], &[-1, 0], &mut non_improving,));
    assert_eq!(non_improving, vec![0, 0]);

    let mut untouched = vec![0, 1];
    assert!(!commit_best_prefix(&[], &[], &mut untouched));
    assert_eq!(untouched, vec![0, 1]);
}

#[test]
fn gain_buckets_track_the_best_gain_and_most_recent_tie() {
    let mut queue = GainBuckets::new(3);
    queue.insert(0, -1);
    queue.insert(1, 2);
    queue.insert(2, 2);

    assert_eq!(queue.best_satisfying(|_| true), Some(2));
    assert!(queue.contains(0) && queue.contains(1) && queue.contains(2));

    queue.update(0, 3);
    assert_eq!(queue.best_satisfying(|_| true), Some(0));
    queue.remove(0);
    assert_eq!(queue.best_satisfying(|_| true), Some(2));
    queue.remove(2);
    assert_eq!(queue.best_satisfying(|_| true), Some(1));
    queue.remove(1);
    assert_eq!(queue.best_satisfying(|_| true), None);
    queue.remove(1);
}

#[test]
fn a_gain_bucket_reused_after_it_emptied_holds_only_its_new_vertex() {
    let mut queue = GainBuckets::new(3);
    queue.insert(0, 5);
    queue.insert(1, 5);
    queue.remove(0);
    // Emptying the gain-5 bucket puts it on the free list, and the next vertex
    // filed at that gain gets it back.
    queue.remove(1);
    queue.insert(2, 5);

    assert_eq!(queue.best_satisfying(|_| true), Some(2));
    queue.remove(2);
    assert_eq!(queue.best_satisfying(|_| true), None);
}

#[test]
fn a_reset_queue_keeps_nothing_of_the_pass_before_it() {
    let mut queue = GainBuckets::new(2);
    queue.insert(0, 7);
    queue.reset(2);

    assert_eq!(queue.best_satisfying(|_| true), None);
    assert!(!queue.contains(0));
    queue.insert(1, 7);
    assert_eq!(queue.best_satisfying(|_| true), Some(1));
}

#[test]
fn gain_buckets_do_not_allocate_the_numeric_range_between_gains() {
    let mut queue = GainBuckets::new(2);
    queue.insert(0, i64::MIN);
    queue.insert(1, i64::MAX);

    assert_eq!(queue.best_satisfying(|_| true), Some(1));
    queue.remove(1);
    assert_eq!(queue.best_satisfying(|_| true), Some(0));
}

#[test]
fn gain_buckets_skip_an_ineligible_vertex_without_removing_it() {
    let mut queue = GainBuckets::new(3);
    queue.insert(0, 5);
    queue.insert(1, 4);
    queue.insert(2, 3);

    assert_eq!(queue.best_satisfying(|vertex| vertex != 0), Some(1));
    assert_eq!(queue.best_satisfying(|_| true), Some(0));
}

#[test]
fn a_stall_resets_only_for_a_strictly_better_running_gain() {
    let mut stall = Stall::new(2);

    assert!(!stall.record(1));
    assert!(!stall.record(1));
    assert!(stall.record(0));

    let mut reset = Stall::new(2);
    assert!(!reset.record(0));
    assert!(!reset.record(1));
    assert!(!reset.record(1));
    assert!(reset.record(1));
}
