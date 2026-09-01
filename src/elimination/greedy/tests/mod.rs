//! Beside the module: these drive the private elimination sink directly.

mod min_fill;

#[test]
fn sampling_mass_prefers_smaller_public_weights() {
    assert_eq!(super::sampling_mass(0), u64::from(u32::MAX) + 1);
    assert_eq!(super::sampling_mass(u32::MAX), 1);
    assert!(super::sampling_mass(7) > super::sampling_mass(8));
}

#[test]
fn uniform_sampling_repeats_the_generic_weighted_choices() {
    let weights = vec![7; 64];
    let vertices: Vec<u32> = (0..64).collect();
    let uniform_mass = super::uniform_sampling_mass(&weights).expect("equal weights");
    let mut fast = crate::rng::Xorshift64::from_state(17);
    let mut generic = fast;

    for len in 2..=vertices.len() {
        assert_eq!(
            super::sample_tie_set(&vertices[..len], &weights, &mut fast, Some(uniform_mass),),
            super::sample_tie_set(&vertices[..len], &weights, &mut generic, None),
            "tie-set length {len}",
        );
    }
}

#[test]
fn unequal_sampling_weights_do_not_enable_the_uniform_path() {
    assert_eq!(super::uniform_sampling_mass(&[1, 1, 2, 1]), None);
}
