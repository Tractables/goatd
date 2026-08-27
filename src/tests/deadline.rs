use std::time::{Duration, Instant};

use crate::deadline::{expired, remaining};

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
