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

/// A learned rule the run file holds under a kind no rule has, or a form
/// no rule merges, cannot be read: the report's read fails rather than
/// name a rule that was never learned.
#[rstest]
#[case::an_unknown_kind("mystery", "")]
#[case::an_unknown_form("equivalent_form", "mystery")]
fn a_learned_rule_of_an_unknown_kind_or_form_is_not_read(#[case] kind: &str, #[case] form: &str) {
    let connection = rusqlite::Connection::open_in_memory().unwrap();
    for statement in super::TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    connection
        .execute(
            "INSERT INTO learned_rules (kind, host, path, parameter, form, evidence)
             VALUES (?1, 'shop.test', '', '', ?2, 'two pairs')",
            (kind, form),
        )
        .unwrap();

    let read = super::learned_rules(&connection);

    assert!(read.is_err(), "{read:?}");
}
