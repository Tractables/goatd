use std::time::{Duration, Instant};

use super::{expired, remaining};

#[test]
fn an_absent_deadline_never_expires() {
    assert!(!expired(None));
}

#[test]
fn a_past_deadline_is_expired_and_has_no_time_remaining() {
    let past = Instant::now() - Duration::from_secs(1);

    assert!(expired(Some(past)));
    assert_eq!(remaining(past), Duration::ZERO);
}

#[test]
fn a_future_deadline_is_live_and_has_time_remaining() {
    let future = Instant::now() + Duration::from_secs(60);

    assert!(!expired(Some(future)));
    let left = remaining(future);
    assert!(left > Duration::ZERO && left <= Duration::from_secs(60));
}

#[test]
fn an_armed_deadline_advances_only_with_charged_work() {
    let epoch = Instant::now();
    let _guard = crate::meter::arm(epoch);
    let deadline = epoch + Duration::from_millis(2);

    assert!(!expired(Some(deadline)));
    crate::meter::charge(crate::meter::UNITS_PER_MS);
    assert!(!expired(Some(deadline)));
    crate::meter::charge(crate::meter::UNITS_PER_MS);
    assert!(expired(Some(deadline)));
    assert_eq!(remaining(deadline), Duration::ZERO);
}

#[test]
fn nested_wall_cutoffs_restore_the_enclosing_limit_with_an_armed_meter() {
    let epoch = Instant::now();
    let _meter = crate::meter::arm(epoch);
    let future = epoch + Duration::from_secs(60);
    let _outer = super::WallGuard::new(Some(future));
    assert!(!expired(None));
    {
        let _inner = super::WallGuard::new(Some(epoch));
        assert!(expired(None));
        let _cannot_extend = super::WallGuard::new(Some(future));
        assert!(expired(None));
    }
    assert!(!expired(None));
    assert_eq!(crate::meter::now(), epoch);
}

#[test]
fn a_wall_cutoff_that_has_run_out_leaves_no_time_on_a_later_deadline() {
    let epoch = Instant::now();
    let _meter = crate::meter::arm(epoch);
    let deadline = epoch + Duration::from_secs(60);

    assert!(remaining(deadline) > Duration::ZERO);
    let _guard = super::WallGuard::new(Some(epoch));
    assert!(expired(None));
    assert_eq!(remaining(deadline), Duration::ZERO);
}
