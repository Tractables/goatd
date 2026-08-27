use crate::Xorshift64;

#[test]
fn equal_rng_states_produce_equal_streams() {
    let draw = || {
        let mut rng = Xorshift64::from_state(0x1234_5678_9abc_def0);
        (0..16).map(|_| rng.next_u64()).collect::<Vec<_>>()
    };

    assert_eq!(draw(), draw());
}

#[test]
fn next_u32_is_the_low_half_of_the_same_next_u64_draw() {
    let mut wide = Xorshift64::from_state(17);
    let mut narrow = Xorshift64::from_state(17);

    assert_eq!(narrow.next_u32(), wide.next_u64() as u32);
}

#[test]
fn zero_is_the_documented_fixed_point() {
    let mut rng = Xorshift64::from_state(0);

    assert!((0..8).all(|_| rng.next_u64() == 0));
}
