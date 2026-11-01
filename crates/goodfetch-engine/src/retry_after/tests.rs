//! A site's `Retry-After` read in both its forms (ADR 0118; RFC 9110
//! §10.2.3), each against the response's own `Date`, at a reading of the
//! engine's clock the tests give.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rstest::rstest;

use super::{RetryAfter, detail, of_detail, read};

/// 2026-10-21T07:27:00Z, the response's `Date` in the cases below.
fn date() -> SystemTime {
    UNIX_EPOCH + Duration::from_mins(29_876_127)
}

/// The engine's clock, an hour ahead of the site's: a date is measured
/// against the response's `Date`, so the skew changes nothing.
fn skewed() -> SystemTime {
    date() + Duration::from_hours(1)
}

/// Covers: story:3-159
#[rstest]
#[case::seconds("120", None, RetryAfter::Until(skewed() + Duration::from_secs(120)))]
#[case::seconds_with_spaces(" 30 ", None, RetryAfter::Until(skewed() + Duration::from_secs(30)))]
#[case::imf_fixdate(
    "Wed, 21 Oct 2026 07:28:00 GMT",
    Some("Wed, 21 Oct 2026 07:27:00 GMT"),
    RetryAfter::Until(skewed() + Duration::from_mins(1))
)]
#[case::rfc_850(
    "Wednesday, 21-Oct-26 07:28:00 GMT",
    Some("Wed, 21 Oct 2026 07:27:00 GMT"),
    RetryAfter::Until(skewed() + Duration::from_mins(1))
)]
#[case::asctime(
    "Wed Oct 21 07:28:00 2026",
    Some("Wed, 21 Oct 2026 07:27:00 GMT"),
    RetryAfter::Until(skewed() + Duration::from_mins(1))
)]
#[case::date_without_the_responses_date(
    "Wed, 21 Oct 2026 09:27:30 GMT",
    None,
    RetryAfter::Until(skewed() + Duration::from_mins(60) + Duration::from_secs(30))
)]
fn a_retry_after_in_either_form_asks_for_its_time(
    #[case] value: &str,
    #[case] response_date: Option<&str>,
    #[case] asked: RetryAfter,
) {
    assert_eq!(read(value, response_date, skewed()), asked);
}

/// A value that does not parse, `0`, or a date already past adds no wait;
/// one that does not parse is told apart, so the Run trace notes it.
///
/// Covers: story:3-159
#[rstest]
#[case::zero("0", None, RetryAfter::NoWait)]
#[case::date_already_past(
    "Wed, 21 Oct 2026 07:26:00 GMT",
    Some("Wed, 21 Oct 2026 07:27:00 GMT"),
    RetryAfter::NoWait
)]
#[case::date_equal_to_the_responses(
    "Wed, 21 Oct 2026 07:27:00 GMT",
    Some("Wed, 21 Oct 2026 07:27:00 GMT"),
    RetryAfter::NoWait
)]
#[case::words("soon", None, RetryAfter::Unparsable)]
#[case::negative("-5", None, RetryAfter::Unparsable)]
#[case::fraction("1.5", None, RetryAfter::Unparsable)]
#[case::empty("", None, RetryAfter::Unparsable)]
#[case::date_not_in_gmt("Wed, 21 Oct 2026 07:28:00 CET", None, RetryAfter::Unparsable)]
fn a_retry_after_that_asks_for_nothing_adds_no_wait(
    #[case] value: &str,
    #[case] response_date: Option<&str>,
    #[case] asked: RetryAfter,
) {
    assert_eq!(read(value, response_date, skewed()), asked);
}

/// A `Date` that does not parse leaves a date measured against the
/// engine's clock.
///
/// Covers: story:3-159
#[test]
fn an_unreadable_response_date_measures_against_the_engines_clock() {
    let asked = read("Wed, 21 Oct 2026 08:28:00 GMT", Some("yesterday"), skewed());

    assert_eq!(asked, RetryAfter::Until(skewed() + Duration::from_mins(1)));
}

/// A date is measured against the response's own `Date`, whose one-second
/// resolution it shares, so a response received just before the engine's
/// clock ticks a second keeps its whole wait from that moment: nothing is
/// lost or gained to the second boundary.
///
/// Covers: story:3-159
#[test]
fn a_date_received_just_before_a_second_ticks_keeps_its_whole_wait() {
    let received = skewed() + Duration::from_millis(999);

    let asked = read(
        "Wed, 21 Oct 2026 07:27:02 GMT",
        Some("Wed, 21 Oct 2026 07:27:00 GMT"),
        received,
    );

    assert_eq!(asked, RetryAfter::Until(received + Duration::from_secs(2)));
}

/// An absurd number of seconds saturates rather than overflow.
///
/// Covers: story:3-159
#[test]
fn an_absurd_number_of_seconds_is_still_a_time() {
    let asked = read("99999999999999999999999999", None, skewed());

    assert!(
        matches!(asked, RetryAfter::Until(until) if until > skewed() + Duration::from_hours(876_000)),
        "{asked:?}"
    );
}

/// The time a site asked for reads back from the `retry_after` detail it is
/// written as, and a detail that names no time names none.
///
/// Covers: story:3-160
#[test]
fn a_retry_after_detail_names_the_time_the_site_asked_for() {
    let asked = date() + Duration::from_secs(30);

    assert_eq!(
        (
            detail(asked),
            of_detail(&serde_json::json!(detail(asked))),
            of_detail(&serde_json::Value::Null),
        ),
        ("2026-10-21T07:27:30Z".to_owned(), Some(asked), None)
    );
}
