use std::collections::BTreeMap;

use goodfetch_counted_work::{CountedWork, Row};
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
    let work = CountedWork::default();
    let params = Params::new(
        r#"{"url":"https://a.test/","_meta":{"traceparent":"x"},"wait":{"ms":5}}"#,
        &work,
    );

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
    let work = CountedWork::default();
    let error = Params::new(json, &work).read::<Goto>().unwrap_err();

    assert_eq!(error.code(), "invalid_params", "{error:?}");
}

#[test]
fn a_map_reads_only_from_an_object() {
    let work = CountedWork::default();
    let error = Params::new("[]", &work)
        .read::<BTreeMap<String, u64>>()
        .unwrap_err();

    assert_eq!(error.code(), "invalid_params");
}

/// An enum reads from its variant's name or a one-member object.
#[rstest]
#[case::unit(r#""Plain""#, Mode::Plain)]
#[case::content(r#"{"Timed":7}"#, Mode::Timed(7))]
fn an_enum_reads_from_its_name_or_one_member(#[case] json: &str, #[case] mode: Mode) {
    let work = CountedWork::default();

    assert_eq!(Params::new(json, &work).read::<Mode>().unwrap(), mode);
}

/// What an error says names no line and column: the params are one value.
#[test]
fn an_error_names_no_position() {
    let work = CountedWork::default();
    let error = Params::new(r#"{"url":7}"#, &work)
        .read::<Goto>()
        .unwrap_err();

    let text = error.to_string();
    assert!(!text.contains("line 1"), "{text}");
}

/// The members a guard reads before the handler, borrowed where they are.
#[derive(Debug, Deserialize, PartialEq)]
struct Addressed<'a> {
    url: Option<&'a str>,
}

/// Each read of a call's params counts one decode of them (ADR 0140's E1),
/// whether it reads them all into their type, holds or not, or only the
/// members a guard needs, where they are; params never read count none, and
/// a call that sent none reads as `{}`.
#[test]
fn each_read_of_a_calls_params_counts_one_decode() {
    let work = CountedWork::default();
    let params = Params::new(r#"{"url":"https://a.test/","wait":{"ms":5}}"#, &work);
    let untouched = work.read(Row::MessageDecodes);

    let members = params.read_members::<Addressed<'_>>();
    let after_its_members = work.read(Row::MessageDecodes);
    let read = params.read::<Goto>().map(|goto| goto.url);
    let refused = params.read::<Mode>().is_err();
    let none = Params::none(&work).read::<BTreeMap<String, u64>>();

    assert_eq!(
        (untouched, after_its_members, work.read(Row::MessageDecodes)),
        (0, 1, 4)
    );
    assert_eq!(
        members,
        Some(Addressed {
            url: Some("https://a.test/")
        })
    );
    assert_eq!(read.unwrap(), "https://a.test/");
    assert!(refused);
    assert_eq!(none.unwrap(), BTreeMap::new());
}
