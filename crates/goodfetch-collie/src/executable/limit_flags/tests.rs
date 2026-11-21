use clap::error::ErrorKind;
use clap::{Args as _, Command, FromArgMatches as _};
use rstest::rstest;
use serde_json::{Value, json};

use super::LimitFlags;

/// The flags a command line gives, or clap's usage error.
fn parse(arguments: &[&str]) -> Result<LimitFlags, clap::Error> {
    let matches = LimitFlags::augment_args(Command::new("command"))
        .try_get_matches_from(std::iter::once("command").chain(arguments.iter().copied()))?;
    LimitFlags::from_arg_matches(&matches)
}

/// The `limits` param one `--limit` gives.
fn limits(flag: &str) -> Option<Value> {
    parse(&["--limit", flag]).unwrap().limits()
}

/// A count is a whole number; a price is a decimal followed by its
/// currency's ISO 4217 code, sent in millionths of it (ADR 0080).
#[rstest]
#[case::a_count("requests=500", json!({ "usage_kind": "requests", "amount": 500 }))]
#[case::zero("requests=0", json!({ "usage_kind": "requests", "amount": 0 }))]
#[case::a_whole_price(
    "paid_fetch=5USD",
    json!({ "usage_kind": "paid_fetch", "currency": "USD", "amount": 5_000_000 })
)]
#[case::a_decimal_price_spaced(
    "paid_solves=0.50 EUR",
    json!({ "usage_kind": "paid_solves", "currency": "EUR", "amount": 500_000 })
)]
fn limit_reads_a_count_or_a_price(#[case] flag: &str, #[case] expected: Value) {
    assert_eq!(limits(flag), Some(json!([expected])));
}

#[test]
fn limit_repeats_into_one_list_in_order() {
    let flags = parse(&["--limit", "requests=5", "--limit", "page_crashes=1"]).unwrap();

    assert_eq!(
        flags.limits(),
        Some(json!([
            { "usage_kind": "requests", "amount": 5 },
            { "usage_kind": "page_crashes", "amount": 1 },
        ]))
    );
}

#[test]
fn no_limit_gives_no_param() {
    assert_eq!(parse(&[]).unwrap().limits(), None);
}

/// A malformed value is a usage error naming the flag, before any engine
/// call.
#[rstest]
#[case::no_equals_sign("requests")]
#[case::no_kind("=5")]
#[case::no_amount("requests=")]
#[case::a_negative_count("requests=-1")]
#[case::a_fractional_count("requests=1.5")]
#[case::a_lowercase_currency("paid_fetch=5usd")]
#[case::finer_than_a_millionth("paid_fetch=0.0000001USD")]
#[case::a_count_past_the_wire("requests=18446744073709551616")]
fn a_malformed_limit_is_a_usage_error_naming_the_flag(#[case] flag: &str) {
    let error = parse(&["--limit", flag]).unwrap_err();

    assert_eq!(
        (
            error.kind(),
            error.exit_code(),
            error.to_string().contains("--limit")
        ),
        (ErrorKind::ValueValidation, 2, true),
        "{error}"
    );
}
