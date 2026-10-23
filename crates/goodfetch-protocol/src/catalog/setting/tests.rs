use rstest::rstest;
use serde_json::{Value, json};

use super::ValueKind;

const DOMAINS: ValueKind = ValueKind::List(&ValueKind::Text);
const ADDRESSES: ValueKind = ValueKind::Map(&ValueKind::List(&ValueKind::Address));
const PINS: ValueKind = ValueKind::Map(&ValueKind::Optional(&ValueKind::Text));

#[rstest]
#[case::a_boolean(ValueKind::Boolean, json!(true))]
#[case::a_count(ValueKind::Count, json!(16))]
#[case::a_duration_in_milliseconds(ValueKind::Duration, json!(60_000))]
#[case::a_choice(ValueKind::Choice(&["off", "on"]), json!("on"))]
#[case::a_price(ValueKind::Price, json!("0.05 USD"))]
#[case::a_whole_price(ValueKind::Price, json!("3 EUR"))]
#[case::a_list(DOMAINS, json!(["example.com"]))]
#[case::addresses_by_host(ADDRESSES, json!({"staging.example": ["127.0.0.1", "::1"]}))]
#[case::an_entry_without_a_value(PINS, json!({"example.com": null}))]
fn a_value_of_its_kind_is_accepted(#[case] kind: ValueKind, #[case] value: Value) {
    assert_eq!(kind.check(&value), Ok(()));
}

#[rstest]
#[case::text_for_a_boolean(ValueKind::Boolean, json!("yes"))]
#[case::a_negative_count(ValueKind::Count, json!(-1))]
#[case::a_fraction_of_a_count(ValueKind::Count, json!(1.5))]
#[case::a_duration_as_text(ValueKind::Duration, json!("60s"))]
#[case::an_unknown_choice(ValueKind::Choice(&["off", "on"]), json!("maybe"))]
#[case::a_price_without_currency(ValueKind::Price, json!("0.05"))]
#[case::a_price_with_a_sign(ValueKind::Price, json!("-1 USD"))]
#[case::a_list_of_numbers(DOMAINS, json!([1]))]
#[case::a_host_name_for_an_address(ADDRESSES, json!({"staging.example": ["localhost"]}))]
#[case::null_where_none_is_optional(ADDRESSES, json!({"staging.example": null}))]
fn a_value_of_another_kind_is_refused(#[case] kind: ValueKind, #[case] value: Value) {
    assert!(kind.check(&value).is_err());
}

#[rstest]
#[case::the_same_list(DOMAINS, json!(["a.example", "b.example"]), json!(["a.example", "b.example"]), true)]
#[case::a_list_with_fewer_entries(DOMAINS, json!(["a.example"]), json!(["a.example", "b.example"]), true)]
#[case::a_list_with_another_entry(DOMAINS, json!(["c.example"]), json!(["a.example"]), false)]
#[case::a_lower_count(ValueKind::Count, json!(2), json!(4), true)]
#[case::a_higher_count(ValueKind::Count, json!(5), json!(4), false)]
#[case::a_lower_price(ValueKind::Price, json!("0.01 USD"), json!("0.05 USD"), true)]
#[case::a_price_with_more_digits(ValueKind::Price, json!("0.051 USD"), json!("0.05 USD"), false)]
#[case::a_price_in_another_currency(ValueKind::Price, json!("0.01 EUR"), json!("0.05 USD"), false)]
#[case::a_map_with_fewer_keys(PINS, json!({"a.example": null}), json!({"a.example": null, "b.example": null}), true)]
#[case::a_map_with_another_key(PINS, json!({"c.example": null}), json!({"a.example": null}), false)]
#[case::a_text_that_differs(ValueKind::Text, json!("b"), json!("a"), false)]
fn a_value_narrows_another_when_it_allows_no_more(
    #[case] kind: ValueKind,
    #[case] higher: Value,
    #[case] lower: Value,
    #[case] narrows: bool,
) {
    assert_eq!(kind.narrows(&higher, &lower), narrows);
}

#[test]
fn a_kinds_schema_describes_its_values() {
    assert_eq!(
        ADDRESSES.schema(),
        json!({
            "type": "object",
            "additionalProperties": {
                "type": "array",
                "items": { "type": "string", "format": "ip" },
            },
        })
    );
}

#[rstest]
#[case::cents("0.05 USD", Some((50_000, "USD")))]
#[case::whole_units("3 EUR", Some((3_000_000, "EUR")))]
#[case::six_decimals("0.000001 USD", Some((1, "USD")))]
#[case::past_a_millionth_rounded_up("0.0000001 USD", Some((1, "USD")))]
#[case::trailing_zeros_past_a_millionth("1.50000000 USD", Some((1_500_000, "USD")))]
#[case::no_currency("0.05", None)]
fn a_price_reads_as_millionths_of_its_currency(
    #[case] text: &str,
    #[case] expected: Option<(u64, &str)>,
) {
    let read = super::PriceValue::parse(text);

    assert_eq!(
        read.as_ref()
            .map(|price| (price.millionths, price.currency.as_str())),
        expected
    );
}
