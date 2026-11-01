//! Independent quotient/remainder rounding reference, separate from socket fixtures.
use super::pixel_extent;

#[test]
fn fractional_pixel_extents_match_integer_reference() {
    for logical in [1_u32, 2, 3, 50, 101, 480, 640, 2160, 3840] {
        for numerator in [60_u32, 120, 150, 180, 240, 300] {
            let product = u64::from(logical) * u64::from(numerator);
            let quotient = product / 120;
            let remainder = product % 120;
            let reference = quotient + u64::from(remainder >= 60);
            assert_eq!(
                u64::from(pixel_extent(logical, numerator).unwrap()),
                reference
            );
        }
    }
    assert_eq!(pixel_extent(101, 150).unwrap(), 126);
    assert_eq!(pixel_extent(101, 180).unwrap(), 152);
    assert_eq!(pixel_extent(1, 60).unwrap(), 1);
}

#[test]
fn invalid_physical_extents_are_rejected_without_clamping() {
    for (logical, scale) in [
        (0, 120),
        (1, 0),
        (1, 1),
        (i32::MAX as u32, 121),
        (u32::MAX, u32::MAX),
    ] {
        assert!(pixel_extent(logical, scale).is_err());
    }
    assert_eq!(pixel_extent(i32::MAX as u32, 120).unwrap(), i32::MAX as u32);
}

#[test]
fn output_text_is_bounded_and_nul_free() {
    use super::identity::text;
    assert_eq!(text("DP-3".into()).as_deref(), Some("DP-3"));
    assert_eq!(text("x".repeat(256)).map(|value| value.len()), Some(256));
    assert_eq!(text("x".repeat(257)), None);
    assert_eq!(text("DP\0-3".into()), None);
}
