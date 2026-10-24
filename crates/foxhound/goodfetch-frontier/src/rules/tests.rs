//! A trap's path pattern as a pure function of a URL's path (spec #581):
//! numeric, date-like and hexadecimal segments generalized, every other
//! segment kept as written.

use rstest::rstest;

use super::pattern;

#[rstest]
#[case::numbers("/calendar/2026/10", "/calendar/{number}/{number}")]
#[case::a_year_and_a_month("/calendar/2026-01", "/calendar/{date}")]
#[case::a_day("/events/2026_10_24/", "/events/{date}/")]
#[case::day_first("/events/24.10.2026", "/events/{date}")]
#[case::a_hash("/sessions/5f3a9c0e/view", "/sessions/{hex}/view")]
#[case::a_word_of_hexadecimal_letters("/cafe/facade", "/cafe/facade")]
#[case::too_short_for_a_token("/a1b2c", "/a1b2c")]
#[case::mixed("/items/I-0001", "/items/I-0001")]
#[case::not_a_date("/v/1-2-3-4", "/v/1-2-3-4")]
#[case::home("/", "/")]
fn a_path_pattern_generalizes_numbers_dates_and_tokens(#[case] path: &str, #[case] expected: &str) {
    assert_eq!(pattern(path), expected);
}
