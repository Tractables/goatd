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
