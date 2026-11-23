use rstest::rstest;
use serde_json::json;

use super::{
    CallCancelledDetails, CancelledBy, ClosedValue, HelloParams, ProductVersion, StoppedBy,
    Surface, VersionLine,
};
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
#[case::a_release("1.4.0", "1")]
#[case::a_later_major("2.0.3", "2")]
#[case::a_preview("0.4.2", "0.4")]
#[case::the_first_preview("0.0.1", "0.0")]
#[case::a_release_candidate("1.5.0-rc.1", "1")]
fn each_major_has_its_own_line_and_each_preview_minor_counts_as_one(
    #[case] version: &str,
    #[case] line: &str,
) {
    let version = ProductVersion::parse(version).unwrap();

    assert_eq!(version.line().to_string(), line);
}

#[rstest]
#[case::a_release("1")]
#[case::a_preview("0.4")]
#[case::the_first_preview("0.0")]
fn a_lines_folder_name_reads_back_as_that_line(#[case] name: &str) {
    let line = VersionLine::parse(name);

    assert_eq!(line.map(|line| line.to_string()).as_deref(), Some(name));
}

#[rstest]
#[case::zero("0")]
#[case::leading_zero("01")]
#[case::a_preview_with_a_leading_zero("0.04")]
#[case::a_release_minor("1.2")]
#[case::a_patch("0.4.1")]
#[case::a_word("runs")]
#[case::empty("")]
fn a_name_that_is_no_lines_folder_is_refused(#[case] name: &str) {
    assert_eq!(VersionLine::parse(name), None);
}

#[test]
fn every_preview_line_comes_before_the_first_release_line() {
    let lines = ["0.2", "0.10", "1", "2"].map(|name| VersionLine::parse(name).unwrap());

    assert!(lines.windows(2).all(|pair| pair[0] < pair[1]));
}

#[rstest]
#[case::a_preview("0.4", "0.5")]
#[case::a_release("1", "2")]
fn the_line_after_a_preview_is_the_next_preview_and_after_a_release_the_next_major(
    #[case] line: &str,
    #[case] next: &str,
) {
    let line = VersionLine::parse(line).unwrap();

    assert_eq!(line.next(), VersionLine::parse(next));
}

#[rstest]
#[case::by_patch("1.4.2", "1.4.10")]
#[case::by_minor("1.4.10", "1.5.0")]
#[case::a_pre_release_below_its_release("1.5.0-rc.1", "1.5.0")]
#[case::numeric_pre_release_identifiers_numerically("1.5.0-rc.2", "1.5.0-rc.10")]
#[case::numbers_below_words("1.5.0-1", "1.5.0-alpha")]
#[case::a_shorter_pre_release_below_a_longer_one("1.5.0-alpha", "1.5.0-alpha.1")]
#[case::a_patch_past_u64("1.0.18446744073709551615", "1.0.18446744073709551616")]
fn product_versions_order_by_semantic_versioning_precedence(
    #[case] older: &str,
    #[case] newer: &str,
) {
    let (older, newer) = (
        ProductVersion::parse(older).unwrap(),
        ProductVersion::parse(newer).unwrap(),
    );

    assert!(older < newer);
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
            settings: serde_json::Map::new(),
            handlers: std::collections::BTreeMap::new(),
        }
    );
}

/// A `hello` names its handlers by Plugin kind (ADR 0104).
#[test]
fn a_hello_reads_its_handlers_by_plugin_kind() {
    let hello: HelloParams = serde_json::from_value(json!({
        "protocol_version": "0.1",
        "surface": "sdk_typescript",
        "product_version": "0.0.0",
        "handlers": { "proxy_provider": ["my_pool"] },
    }))
    .unwrap();

    assert_eq!(
        hello.handlers,
        std::collections::BTreeMap::from([(
            "proxy_provider".to_owned(),
            std::collections::BTreeSet::from(["my_pool".to_owned()])
        )])
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
fn call_cancelled_details_read_who_stopped_the_call_what_closed_and_for_whom() {
    let sent = json!({ "by": "person", "closed": "run", "person": "ada" });

    let read: CallCancelledDetails = serde_json::from_value(sent.clone()).unwrap();

    assert_eq!(
        (&read.by, &read.closed, read.person.as_deref()),
        (&CancelledBy::Person, &Some(ClosedValue::Run), Some("ada"))
    );
    assert_eq!(serde_json::to_value(&read).unwrap(), sent);
}

#[test]
fn call_cancelled_details_keep_who_and_what_a_newer_engine_names() {
    let sent = json!({ "by": "a_newer_stopper", "closed": "a_newer_value" });

    let read: CallCancelledDetails = serde_json::from_value(sent.clone()).unwrap();

    assert_eq!(
        (
            read.by.as_str(),
            read.closed.as_ref().map(ClosedValue::as_str),
            read.person.as_deref(),
        ),
        ("a_newer_stopper", Some("a_newer_value"), None)
    );
    assert_eq!(serde_json::to_value(&read).unwrap(), sent);
}

#[rstest]
#[case::person(StoppedBy::Person, "person")]
#[case::client(StoppedBy::Client, "client")]
#[case::program(StoppedBy::Program, "program")]
#[case::browser(StoppedBy::Browser, "browser")]
fn whoever_stops_a_run_stops_its_calls_under_the_same_name(
    #[case] stopped: StoppedBy,
    #[case] wire: &str,
) {
    let by = CancelledBy::from(stopped);

    assert_eq!(
        (by.as_str(), serde_json::to_value(&by).unwrap()),
        (wire, json!(wire))
    );
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
    assert_eq!(minors, vec![&json!("0.1"); 11]);
}
