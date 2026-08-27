//! Beside the module: these drive the private elimination sink directly.

mod min_fill;

#[test]
fn sampling_mass_prefers_smaller_public_weights() {
    assert_eq!(super::sampling_mass(0), u64::from(u32::MAX) + 1);
    assert_eq!(super::sampling_mass(u32::MAX), 1);
    assert!(super::sampling_mass(7) > super::sampling_mass(8));
}
