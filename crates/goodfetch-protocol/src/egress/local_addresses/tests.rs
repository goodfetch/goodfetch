#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::IpAddr;

use rstest::rstest;
use serde_json::{Value, json};

use super::{
    Ipv6Prefix, LOCAL_ADDRESSES_SHAPE, LocalAddressFailure, LocalAddressUnavailableDetails,
    LocalAddresses,
};
use crate::catalog::ValueKind;

fn addresses(texts: &[&str]) -> LocalAddresses {
    LocalAddresses::Addresses(texts.iter().map(|text| text.parse().unwrap()).collect())
}

/// The three forms ADR 0044 gives the setting.
/// Covers: story:4-44, setting:egress.local_addresses
#[rstest]
#[case::a_list_of_addresses(json!(["192.0.2.10", "2001:db8::10"]), addresses(&["192.0.2.10", "2001:db8::10"]))]
#[case::an_interface(json!("eth0"), LocalAddresses::Interface("eth0".to_owned()))]
#[case::an_interface_with_a_windows_name(json!("Ethernet 2"), LocalAddresses::Interface("Ethernet 2".to_owned()))]
#[case::a_prefix(
    json!("2001:db8:1::/48"),
    LocalAddresses::Prefix(Ipv6Prefix { network: "2001:db8:1::".parse().unwrap(), length: 48 })
)]
#[case::a_prefix_whose_host_bits_are_set(
    json!("2001:db8:1:2::5/64"),
    LocalAddresses::Prefix(Ipv6Prefix { network: "2001:db8:1:2::".parse().unwrap(), length: 64 })
)]
fn local_addresses_take_a_list_an_interface_or_an_ipv6_prefix(
    #[case] value: Value,
    #[case] expected: LocalAddresses,
) {
    assert_eq!(LocalAddresses::parse(&value), Ok(expected));
}

/// Covers: setting:egress.local_addresses
#[rstest]
#[case::an_empty_list(json!([]))]
#[case::a_list_with_a_name(json!(["192.0.2.10", "eth0"]))]
#[case::an_ipv4_prefix(json!("192.0.2.0/24"))]
#[case::a_prefix_too_long(json!("2001:db8::/129"))]
#[case::a_prefix_of_no_address(json!("nowhere/64"))]
#[case::a_bare_address(json!("192.0.2.10"))]
#[case::an_empty_name(json!(""))]
#[case::a_name_with_a_control_character(json!("eth\n0"))]
#[case::a_number(json!(10))]
fn a_value_of_no_form_is_invalid(#[case] value: Value) {
    assert!(LocalAddresses::parse(&value).is_err());
    assert!(
        ValueKind::Shape(&LOCAL_ADDRESSES_SHAPE)
            .check(&value)
            .is_err()
    );
}

#[rstest]
#[case::its_first_address("2001:db8:1::", true)]
#[case::an_address_inside("2001:db8:1:ffff::1", true)]
#[case::the_next_prefix("2001:db8:2::", false)]
fn a_prefix_contains_the_addresses_of_its_length(#[case] address: &str, #[case] inside: bool) {
    let LocalAddresses::Prefix(prefix) = LocalAddresses::parse(&json!("2001:db8:1::/48")).unwrap()
    else {
        panic!("a prefix");
    };
    let IpAddr::V6(address) = address.parse().unwrap() else {
        panic!("an IPv6 address");
    };
    assert_eq!(prefix.contains(address), inside);
}

#[test]
fn local_address_unavailable_names_the_entry_and_why_on_the_wire() {
    let details = LocalAddressUnavailableDetails {
        entry: "2001:db8:ffff::/64".to_owned(),
        failure: LocalAddressFailure::PrefixUnusable,
        setting: "egress.local_addresses".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({
            "entry": "2001:db8:ffff::/64",
            "failure": "prefix_unusable",
            "setting": "egress.local_addresses",
        })
    );
}
