use std::time::{Duration, Instant};

use goatd::meter::{
    Guard, UNITS_PER_MS, arm, charge, is_armed, milliseconds_for_units, now, units_spent,
};

#[test]
fn an_unarmed_meter_records_nothing() {
    assert!(!is_armed());
    let before = units_spent();
    charge(1_000_000);
    assert_eq!(units_spent(), before);
}

#[test]
fn the_clock_advances_by_charged_work() {
    let epoch = Instant::now();
    let _guard: Guard = arm(epoch);
    assert_eq!(now(), epoch);

    charge(UNITS_PER_MS * 40);
    assert_eq!(now(), epoch + Duration::from_millis(40));
    charge(UNITS_PER_MS * 2);
    assert_eq!(now(), epoch + Duration::from_millis(42));
}

#[test]
fn identical_charges_reach_the_same_reading() {
    let charges = [3_u64, 100_000, 7, UNITS_PER_MS, 1];
    let readings: Vec<Duration> = (0..2)
        .map(|_| {
            let epoch = Instant::now();
            let _guard = arm(epoch);
            for charge_count in charges {
                charge(charge_count * UNITS_PER_MS);
            }
            now() - epoch
        })
        .collect();
    assert_eq!(readings[0], readings[1]);
}

#[test]
fn the_guard_disarms_when_dropped() {
    {
        let _guard = arm(Instant::now());
        assert!(is_armed());
    }
    assert!(!is_armed());
}

#[test]
fn a_nested_guard_restores_the_outer_clock() {
    let outer_epoch = Instant::now();
    let _outer = arm(outer_epoch);
    charge(UNITS_PER_MS * 3);

    {
        let inner_epoch = outer_epoch + Duration::from_secs(10);
        let _inner = arm(inner_epoch);
        charge(UNITS_PER_MS * 2);
        assert_eq!(now(), inner_epoch + Duration::from_millis(2));
    }

    assert_eq!(now(), outer_epoch + Duration::from_millis(5));
}

#[test]
fn units_convert_to_milliseconds() {
    for milliseconds in [0_u64, 1, 90_000, 3_600_000] {
        assert_eq!(
            milliseconds_for_units(milliseconds.saturating_mul(UNITS_PER_MS)),
            milliseconds
        );
    }
}

#[test]
fn an_absurd_charge_does_not_overflow_the_clock() {
    let epoch = Instant::now();
    let _guard = arm(epoch);
    charge(u64::MAX);
    let reading = now();

    assert!(reading >= epoch);
    if let Some(one_year_later) = epoch.checked_add(Duration::from_secs(365 * 24 * 60 * 60)) {
        assert!(reading >= one_year_later);
    }
}
