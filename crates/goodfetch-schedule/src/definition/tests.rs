use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rstest::rstest;

use super::{ZoneUnknown, generated_id, time_zone};

fn wall() -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(1_764_061_234_567)
}

fn counter() -> impl FnMut() -> u64 {
    let mut next = 0x0123_4567_89ab_cdef_u64;
    move || {
        next = next.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        next
    }
}

/// Covers: story:10-7
#[test]
fn a_generated_id_is_s_and_a_lowercase_ulid_of_the_wall_time() {
    let id = generated_id(wall(), counter());

    assert_eq!(id.as_str().len(), 27);
    assert!(id.as_str().starts_with('s'));
    let ulid = ulid::Ulid::from_string(&id.as_str()[1..].to_uppercase()).unwrap();
    assert_eq!(ulid.timestamp_ms(), 1_764_061_234_567);
}

/// Covers: story:10-7
#[test]
fn the_same_draws_generate_the_same_id() {
    assert_eq!(
        generated_id(wall(), counter()),
        generated_id(wall(), counter())
    );
}

/// Covers: story:10-8
#[rstest]
#[case::the_hosts_when_none_is_given(None, "Asia/Kathmandu")]
#[case::the_given_one(Some("Europe/Paris"), "Europe/Paris")]
#[case::utc_given(Some("UTC"), "UTC")]
fn a_schedule_stores_an_explicit_zone(#[case] given: Option<&str>, #[case] stored: &str) {
    assert_eq!(time_zone(given, "Asia/Kathmandu"), Ok(stored.to_owned()));
}

/// Covers: story:10-8
#[rstest]
#[case::local("local")]
#[case::a_posix_rule("CET-1CEST")]
#[case::unknown("Europe/Atlantis")]
fn a_name_the_zone_database_does_not_know_is_refused(#[case] given: &str) {
    assert_eq!(
        time_zone(Some(given), "UTC"),
        Err(ZoneUnknown(given.to_owned()))
    );
}
