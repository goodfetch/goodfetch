use rstest::rstest;
use serde_json::json;

use super::{HelloParams, ProductVersion, Surface};
use crate::catalog::{Product, ProtocolVersion};
use crate::log::LogLevel;

#[rstest]
#[case::off("off", LogLevel::Off)]
#[case::error("error", LogLevel::Error)]
#[case::warn("warn", LogLevel::Warn)]
#[case::info("info", LogLevel::Info)]
#[case::debug("debug", LogLevel::Debug)]
#[case::trace("trace", LogLevel::Trace)]
fn a_hello_reads_the_log_level_its_client_receives(#[case] word: &str, #[case] level: LogLevel) {
    let hello: HelloParams = serde_json::from_value(json!({
        "protocol_version": "0.1",
        "surface": "sdk_typescript",
        "needs": "collie",
        "product_version": "0.0.0",
        "log_level": word,
    }))
    .unwrap();

    assert_eq!(hello.log_level, Some(level));
}

#[test]
fn a_hello_with_a_log_level_no_engine_knows_is_refused() {
    let hello = serde_json::from_value::<HelloParams>(json!({
        "protocol_version": "0.1",
        "surface": "sdk_typescript",
        "needs": "collie",
        "product_version": "0.0.0",
        "log_level": "verbose",
    }));

    assert!(hello.is_err());
}

#[rstest]
#[case::release("1.4.0", 1)]
#[case::preview("0.3.12", 0)]
#[case::release_candidate("2.0.0-rc.1", 2)]
fn a_product_version_reads_with_its_major(#[case] text: &str, #[case] major: u64) {
    let version = ProductVersion::parse(text).unwrap();

    assert_eq!((version.as_str(), version.major()), (text, major));
}

#[rstest]
#[case::two_numbers("1.4")]
#[case::leading_zero("1.04.0")]
#[case::empty_pre_release("1.4.0-")]
#[case::build_metadata("1.4.0+build")]
#[case::space("1.4.0 ")]
fn a_malformed_product_version_is_refused(#[case] text: &str) {
    assert!(ProductVersion::parse(text).is_err());
}

#[test]
fn a_clients_hello_reads_with_the_product_it_needs() {
    let hello: HelloParams = serde_json::from_value(json!({
        "protocol_version": "0.1",
        "surface": "sdk_python",
        "needs": "foxhound",
        "product_version": "0.0.0",
    }))
    .unwrap();

    assert_eq!(
        hello,
        HelloParams {
            protocol_version: ProtocolVersion { major: 0, minor: 1 },
            surface: Surface::SdkPython,
            form: None,
            token: None,
            needs: Some(Product::Foxhound),
            product_version: ProductVersion::parse("0.0.0").unwrap(),
            log_level: None,
            client_surface: None,
            keep_daemon_alive: false,
        }
    );
}

#[test]
fn a_hello_with_an_unknown_field_is_refused() {
    let hello = serde_json::from_value::<HelloParams>(json!({
        "protocol_version": "0.1",
        "surface": "cli",
        "needs": "collie",
        "product_version": "0.0.0",
        "capabilities": [],
    }));

    assert!(hello.is_err());
}

#[test]
fn every_field_records_the_minor_that_added_it() {
    let schema = schemars::schema_for!(HelloParams);

    let minors: Vec<&serde_json::Value> = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .unwrap()
        .values()
        .map(|field| &field["arrived_in"])
        .collect();
    assert_eq!(minors, vec![&json!("0.1"); 9]);
}
