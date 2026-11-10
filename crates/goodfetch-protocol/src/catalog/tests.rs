use rstest::rstest;
use serde_json::json;

use super::{Product, ProtocolVersion};

#[rstest]
#[case::collie_contains_collie(Product::Collie, Product::Collie, true)]
#[case::collie_lacks_foxhound(Product::Collie, Product::Foxhound, false)]
#[case::foxhound_contains_collie(Product::Foxhound, Product::Collie, true)]
#[case::foxhound_contains_foxhound(Product::Foxhound, Product::Foxhound, true)]
fn a_product_contains_itself_and_foxhound_contains_collie(
    #[case] product: Product,
    #[case] other: Product,
    #[case] contains: bool,
) {
    assert_eq!(product.contains(other), contains);
}

#[rstest]
#[case::collie(Product::Collie, "collie")]
#[case::foxhound(Product::Foxhound, "foxhound")]
fn product_reads_and_writes_the_mark_the_catalog_writes(
    #[case] product: Product,
    #[case] mark: &str,
) {
    let written = serde_json::to_value(product).unwrap();

    assert_eq!(written, json!(mark));
    assert_eq!(product.as_str(), mark);
    assert_eq!(serde_json::from_value::<Product>(written).unwrap(), product);
}

#[rstest]
#[case::zero("0.1", ProtocolVersion { major: 0, minor: 1 })]
#[case::several_digits("12.304", ProtocolVersion { major: 12, minor: 304 })]
fn a_protocol_version_reads_as_major_dot_minor(
    #[case] text: &str,
    #[case] version: ProtocolVersion,
) {
    let read: ProtocolVersion = serde_json::from_value(json!(text)).unwrap();

    assert_eq!(read, version);
    assert_eq!(serde_json::to_value(read).unwrap(), json!(text));
}

#[rstest]
#[case::empty("")]
#[case::major_only("1")]
#[case::patch("1.2.3")]
#[case::leading_zero("01.2")]
#[case::sign("+1.2")]
#[case::space(" 1.2")]
#[case::too_large("70000.1")]
#[case::not_a_number("a.b")]
fn a_malformed_protocol_version_is_refused(#[case] text: &str) {
    let read = serde_json::from_value::<ProtocolVersion>(json!(text));

    assert!(read.is_err(), "{text:?} was read as {read:?}");
}

/// Covers: story:11-218
#[test]
fn the_catalog_marks_every_entry_it_keeps_to_the_sdks() {
    let unmarked: Vec<&str> = crate::entries::MODULES
        .iter()
        .flat_map(|entries| entries.operations)
        .filter(|operation| {
            super::SDK_ONLY.contains(&operation.name) && operation.reach != super::Reach::SdkOnly
        })
        .map(|operation| operation.name)
        .collect();

    assert_eq!(unmarked, Vec::<&str>::new());
}
