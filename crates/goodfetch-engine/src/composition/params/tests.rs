use std::collections::BTreeMap;

use rstest::rstest;
use serde::Deserialize;

use super::Params;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Goto {
    url: String,
    #[serde(default)]
    wait: Option<Wait>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Wait {
    ms: u64,
}

#[derive(Debug, Deserialize, PartialEq)]
enum Mode {
    Plain,
    Timed(u64),
}

/// Params read straight into their type, without their `_meta`.
#[test]
fn params_read_as_their_type_without_their_meta() {
    let params =
        Params::new(r#"{"url":"https://a.test/","_meta":{"traceparent":"x"},"wait":{"ms":5}}"#);

    assert_eq!(
        params.read::<Goto>().unwrap(),
        Goto {
            url: "https://a.test/".to_owned(),
            wait: Some(Wait { ms: 5 }),
        }
    );
}

/// A struct or a map reads only from an object, at every depth, which
/// serde alone would read from an array or `null` too; a `_meta` inside
/// the params is a member like any other.
#[rstest]
#[case::params_an_array(r#"["https://a.test/"]"#)]
#[case::nested_an_array(r#"{"url":"u","wait":[5]}"#)]
#[case::params_null("null")]
#[case::nested_meta(r#"{"url":"u","wait":{"ms":5,"_meta":{}}}"#)]
#[case::unknown_member(r#"{"url":"u","extra":1}"#)]
#[case::trailing_text(r#"{"url":"u"} 1"#)]
fn params_of_another_shape_are_invalid_params(#[case] json: &str) {
    let error = Params::new(json).read::<Goto>().unwrap_err();

    assert_eq!(error.code(), "invalid_params", "{error:?}");
}

#[test]
fn a_map_reads_only_from_an_object() {
    let error = Params::new("[]")
        .read::<BTreeMap<String, u64>>()
        .unwrap_err();

    assert_eq!(error.code(), "invalid_params");
}

/// An enum reads from its variant's name or a one-member object.
#[rstest]
#[case::unit(r#""Plain""#, Mode::Plain)]
#[case::content(r#"{"Timed":7}"#, Mode::Timed(7))]
fn an_enum_reads_from_its_name_or_one_member(#[case] json: &str, #[case] mode: Mode) {
    assert_eq!(Params::new(json).read::<Mode>().unwrap(), mode);
}

/// What an error says names no line and column: the params are one value.
#[test]
fn an_error_names_no_position() {
    let error = Params::new(r#"{"url":7}"#).read::<Goto>().unwrap_err();

    let text = error.to_string();
    assert!(!text.contains("line 1"), "{text}");
}
