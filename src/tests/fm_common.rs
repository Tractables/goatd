use crate::Xorshift64;
use crate::fm_common::{
    GainBuckets, Stall, balance_bounds, commit_best_prefix, fm_balance, index_split,
    random_bisection, tiny_bisection,
};

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
fn balance_bounds_and_current_weights_use_vertex_weight() {
    assert_eq!(balance_bounds(&[3, 2, 1, 2], 0.25), (2, 6));
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
fn committing_moves_keeps_only_the_best_positive_prefix() {
    let moves = [(0, -1), (1, 3), (2, -1)];
    let mut part = vec![1, 1, 1];

    assert!(commit_best_prefix(&moves, &[-1, 2, 1], &mut part));
    assert_eq!(part, vec![1, 1, 0]);

    let mut non_improving = vec![1, 1];
    assert!(!commit_best_prefix(
        &[(0, -1), (1, 1)],
        &[-1, 0],
        &mut non_improving,
    ));
    assert_eq!(non_improving, vec![0, 0]);

    let mut untouched = vec![0, 1];
    assert!(!commit_best_prefix(&[], &[], &mut untouched));
    assert_eq!(untouched, vec![0, 1]);
}

#[test]
fn gain_buckets_track_the_best_gain_and_most_recent_tie() {
    let mut queue = GainBuckets::new(3, 3);
    queue.insert(0, -1);
    queue.insert(1, 2);
    queue.insert(2, 2);

    assert_eq!(queue.top(), Some(2));
    assert!(queue.contains(0) && queue.contains(1) && queue.contains(2));

    queue.update(0, 3);
    assert_eq!(queue.top(), Some(0));
    queue.remove(0);
    assert_eq!(queue.top(), Some(2));
    queue.remove(2);
    assert_eq!(queue.top(), Some(1));
    queue.remove(1);
    assert_eq!(queue.top(), None);
    queue.remove(1);
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
