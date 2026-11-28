use std::net::{Ipv4Addr, SocketAddr};

use rstest::rstest;

use crate::catalog::{Authority, Product};
use crate::connection::{Form, ProductVersion};
use crate::http_listener::{
    ADDRESS, ENABLED, PORT, default_address, default_port, parse_form, parse_product,
};

/// Each major release and each `0.y` preview gets a distinct loopback port
/// of its own, separate from `daemon.address`'s 7900/7600 block (ADR 0030).
///
/// Covers: story:11-258
#[rstest]
#[case::release_1("1.0.0", 7801)]
#[case::release_2("2.3.1", 7802)]
#[case::preview_0_1("0.1.0", 7701)]
#[case::preview_0_4("0.4.2", 7704)]
fn default_port_follows_the_major_or_preview_minor(#[case] version: &str, #[case] expected: u16) {
    let version = ProductVersion::parse(version).unwrap();
    assert_eq!(default_port(&version), expected);
    assert_eq!(
        default_address(&version),
        SocketAddr::from((Ipv4Addr::LOCALHOST, expected))
    );
}

/// All three HTTP listener settings are configuration-only (`Authority::Machine`),
/// off by default on loopback.
///
/// Covers: story:11-258
#[test]
fn settings_are_machine_only_and_off_on_loopback_by_default() {
    assert!(matches!(
        (ENABLED.authority, ADDRESS.authority, PORT.authority),
        (Authority::Machine, Authority::Machine, Authority::Machine)
    ));
    assert_eq!((ENABLED.default)(), serde_json::json!(false));
    assert_eq!((ADDRESS.default)(), serde_json::json!("127.0.0.1"));
}

#[test]
fn header_parsers_accept_canonical_wire_names_only() {
    assert_eq!(parse_product("collie"), Some(Product::Collie));
    assert_eq!(parse_product("foxhound"), Some(Product::Foxhound));
    assert_eq!(parse_product("Collie"), None);
    assert_eq!(parse_product("other"), None);

    assert_eq!(parse_form("catalog"), Some(Form::Catalog));
    assert_eq!(parse_form("tool"), Some(Form::Tool));
    assert_eq!(parse_form("Catalog"), None);
    assert_eq!(parse_form("other"), None);
}
