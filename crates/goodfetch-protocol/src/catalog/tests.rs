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
