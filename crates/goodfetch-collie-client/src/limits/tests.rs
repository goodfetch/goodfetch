//! Limits in code (ADR 0080): a `Money` turns exactly into millionths of its
//! currency or fails at `Money::new`, and `Limits` lists each Limit in the
//! wire's form, one per Usage kind and currency.
//! Covers: story:11-51

use goodfetch_conformance::{ClientCaseRun, RustMode};
use goodfetch_protocol::run::RunLimit;
use goodfetch_test_support::case_logs;
use proptest::prelude::*;
use rstest::rstest;

use super::{Limits, Money, MoneyError, UsageKind};

#[rstest]
#[case::whole("3", 3_000_000)]
#[case::six_decimals("0.123456", 123_456)]
#[case::zeros_past_the_sixth_decimal("1.1234560000", 1_123_456)]
#[case::one_millionth("0.000001", 1)]
#[case::the_largest_the_wire_carries("18446744073709.551615", u64::MAX)]
fn money_turns_an_amount_exactly_into_millionths(#[case] amount: &str, #[case] millionths: u64) {
    let money = Money::new(amount, "USD").unwrap();

    assert_eq!((money.millionths(), money.currency()), (millionths, "USD"));
}

/// The `money-inexact-refused-before-sending` case: in Rust an amount
/// that does not turn exactly into millionths fails at `Money::new`, so no
/// call can carry it.
#[rstest]
#[case::finer_than_a_millionth("0.0000001", "USD", MoneyError::FinerThanAMillionth)]
#[case::past_what_the_wire_carries("18446744073709.551616", "USD", MoneyError::TooLarge)]
#[case::comma("1,50", "USD", MoneyError::NotADecimal)]
#[case::negative("-1", "USD", MoneyError::NotADecimal)]
#[case::exponent("1e3", "USD", MoneyError::NotADecimal)]
#[case::no_digit_before_the_point(".5", "USD", MoneyError::NotADecimal)]
#[case::no_digit_after_the_point("5.", "USD", MoneyError::NotADecimal)]
#[case::empty("", "USD", MoneyError::NotADecimal)]
#[case::lowercase_currency("1", "usd", MoneyError::NotACurrency)]
#[case::long_currency("1", "USDT", MoneyError::NotACurrency)]
fn money_refuses_an_amount_it_cannot_turn_exactly(
    #[case] amount: &str,
    #[case] currency: &str,
    #[case] expected: MoneyError,
) {
    let _case = ClientCaseRun::start(
        case_logs().as_deref(),
        "money-inexact-refused-before-sending",
        RustMode::Linked,
    )
    .unwrap();
    assert_eq!(Money::new(amount, currency), Err(expected));
}

#[test]
fn limits_list_each_count_and_price_in_the_order_given() {
    let limits = Limits::new()
        .count(UsageKind::MODEL_CALLS, 100)
        .price(UsageKind::PAID_SOLVES, Money::new("2.5", "USD").unwrap())
        .count(UsageKind::BROWSER_TIME, 60_000)
        .price(
            UsageKind::PROXY_BYTES,
            Money::new("0.000001", "EUR").unwrap(),
        );

    assert_eq!(
        Vec::from(limits),
        [
            limit("model_calls", None, 100),
            limit("paid_solves", Some("USD"), 2_500_000),
            limit("browser_time", None, 60_000),
            limit("proxy_bytes", Some("EUR"), 1),
        ]
    );
}

#[test]
fn a_limit_set_again_on_a_kind_and_currency_replaces_it_in_place() {
    let limits = Limits::new()
        .count(UsageKind::REQUESTS, 5)
        .price(UsageKind::PAID_SOLVES, Money::new("2", "USD").unwrap())
        .price(UsageKind::PAID_SOLVES, Money::new("3", "EUR").unwrap())
        .count(UsageKind::REQUESTS, 3)
        .price(UsageKind::PAID_SOLVES, Money::new("1", "USD").unwrap());

    assert_eq!(
        Vec::from(limits),
        [
            limit("requests", None, 3),
            limit("paid_solves", Some("USD"), 1_000_000),
            limit("paid_solves", Some("EUR"), 3_000_000),
        ]
    );
}

#[test]
fn a_usage_kind_a_product_adds_is_named_by_its_catalog_name() {
    let limits = Limits::new().count(UsageKind::named("paid_fetch"), 3);

    assert_eq!(Vec::from(limits), [limit("paid_fetch", None, 3)]);
}

proptest! {
    /// Any number of millionths the wire carries, written with its six
    /// decimals and any trailing zeros, turns back into the same number.
    #[test]
    fn an_amount_written_from_millionths_turns_back_into_them(
        millionths in any::<u64>(),
        zeros in 0_usize..4,
    ) {
        let amount = format!(
            "{}.{:06}{}",
            millionths / 1_000_000,
            millionths % 1_000_000,
            "0".repeat(zeros),
        );

        prop_assert_eq!(Money::new(&amount, "USD").map(|money| money.millionths()), Ok(millionths));
    }

    /// A non-zero digit past the sixth decimal is refused, whatever comes
    /// before it.
    #[test]
    fn a_non_zero_digit_past_the_sixth_decimal_is_refused(
        units in 0_u64..1_000_000,
        decimals in "[0-9]{6}[0-9]*",
        past in 1_u8..10,
    ) {
        let amount = format!("{units}.{decimals}{past}");

        prop_assert_eq!(Money::new(&amount, "USD"), Err(MoneyError::FinerThanAMillionth));
    }
}

fn limit(usage_kind: &str, currency: Option<&str>, amount: u64) -> RunLimit {
    RunLimit {
        usage_kind: usage_kind.to_owned(),
        currency: currency.map(str::to_owned),
        amount,
    }
}
