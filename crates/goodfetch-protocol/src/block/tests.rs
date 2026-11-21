use rstest::rstest;
use schemars::generate::SchemaSettings;
use serde_json::{Value, json};

use super::BlockCause;
use crate::catalog::sections::Sections;
use crate::entries::MODULES;

/// The catalog file's sections of every module's entries.
fn catalog() -> Value {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let mut sections = Sections::new(crate::setting_scopes::of);
    for entries in MODULES {
        sections.add(entries, &mut generator);
    }
    sections.into_catalog(&[], &[])
}

/// The catalog's entry of the Block cause `name`, if it lists one.
fn catalog_entry(name: &str) -> Option<Value> {
    catalog()["shared_type_values"]
        .as_array()?
        .iter()
        .find(|value| value["type"] == "block_cause" && value["name"] == name)
        .cloned()
}

/// Covers: block-cause:challenge, block-cause:captcha, block-cause:rate_limit, block-cause:ip_block, block-cause:payment_required, block-cause:consent_required
#[rstest]
#[case::challenge(BlockCause::Challenge, "challenge")]
#[case::captcha(BlockCause::Captcha, "captcha")]
#[case::rate_limit(BlockCause::RateLimit, "rate_limit")]
#[case::ip_block(BlockCause::IpBlock, "ip_block")]
#[case::payment_required(BlockCause::PaymentRequired, "payment_required")]
#[case::consent_required(BlockCause::ConsentRequired, "consent_required")]
fn each_of_collies_block_causes_is_on_the_wire_as_written_and_in_the_catalog_as_collies(
    #[case] cause: BlockCause,
    #[case] wire: &str,
) {
    let written = serde_json::to_value(&cause).unwrap();

    assert_eq!(written, json!(wire));
    assert_eq!(
        serde_json::from_value::<BlockCause>(json!(wire)).unwrap(),
        cause
    );
    let entry = catalog_entry(wire).expect("the catalog lists the Block cause");
    assert_eq!(entry["product"], "collie");
}

/// `decoy` is the one Block cause the catalog marks Foxhound's (ADR 0095).
/// Covers: block-cause:decoy
#[test]
fn decoy_alone_is_foxhounds_block_cause_in_the_catalog() {
    let foxhounds: Vec<Value> = catalog()["shared_type_values"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|value| value["type"] == "block_cause" && value["product"] == "foxhound")
        .map(|value| value["name"].clone())
        .collect();

    assert_eq!(foxhounds, [json!("decoy")]);
}

#[test]
fn the_catalog_lists_no_block_cause_beyond_the_closed_list() {
    let catalog = catalog();

    let mut names: Vec<&str> = catalog["shared_type_values"]
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
