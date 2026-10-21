use rstest::rstest;
use serde_json::{Value, json};

use super::BlockCause;
use crate::emit::emit;

/// The catalog's entry of the Block cause `name`, if it lists one.
fn catalog_entry(name: &str) -> Option<Value> {
    emit().catalog["shared_type_values"]
        .as_array()?
        .iter()
        .find(|value| value["type"] == "block_cause" && value["name"] == name)
        .cloned()
}

/// Covers: block-cause:challenge, block-cause:captcha, block-cause:rate_limit, block-cause:ip_block, block-cause:decoy, block-cause:payment_required, block-cause:consent_required
#[rstest]
#[case::challenge(BlockCause::Challenge, "challenge", "collie")]
#[case::captcha(BlockCause::Captcha, "captcha", "collie")]
#[case::rate_limit(BlockCause::RateLimit, "rate_limit", "collie")]
#[case::ip_block(BlockCause::IpBlock, "ip_block", "collie")]
#[case::decoy(BlockCause::Decoy, "decoy", "foxhound")]
#[case::payment_required(BlockCause::PaymentRequired, "payment_required", "collie")]
#[case::consent_required(BlockCause::ConsentRequired, "consent_required", "collie")]
fn each_block_cause_is_on_the_wire_as_written_and_in_the_catalog_under_its_products_mark(
    #[case] cause: BlockCause,
    #[case] wire: &str,
    #[case] product: &str,
) {
    let written = serde_json::to_value(cause).unwrap();

    assert_eq!(written, json!(wire));
    assert_eq!(
        serde_json::from_value::<BlockCause>(json!(wire)).unwrap(),
        cause
    );
    let entry = catalog_entry(wire).expect("the catalog lists the Block cause");
    assert_eq!(entry["product"], product);
}

#[test]
fn the_catalog_lists_no_block_cause_beyond_the_closed_list() {
    let emitted = emit();

    let mut names: Vec<&str> = emitted.catalog["shared_type_values"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| value["type"] == "block_cause")
        .map(|value| value["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();

    assert_eq!(
        names,
        [
            "captcha",
            "challenge",
            "consent_required",
            "decoy",
            "ip_block",
            "payment_required",
            "rate_limit"
        ]
    );
}

#[test]
fn the_bundle_holds_the_block_cause_type_with_every_cause() {
    let emitted = emit();

    assert_eq!(
        emitted.bundle["shared_types"]["block_cause"],
        json!({ "$ref": "#/$defs/BlockCause" })
    );
    let mut values: Vec<&str> = emitted.bundle["$defs"]["BlockCause"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value["const"].as_str().unwrap())
        .collect();
    values.sort_unstable();
    assert_eq!(
        values,
        [
            "captcha",
            "challenge",
            "consent_required",
            "decoy",
            "ip_block",
            "payment_required",
            "rate_limit"
        ]
    );
}
