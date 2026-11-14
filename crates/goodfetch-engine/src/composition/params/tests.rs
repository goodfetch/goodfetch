use std::collections::BTreeMap;

use goodfetch_counted_work::{CountedWork, Row};
use rstest::rstest;
use serde::Deserialize;

use super::Arrived;
use crate::trace::Ids;

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
    let arrived = Arrived::new(
        Some(r#"{"url":"https://a.test/","_meta":{"traceparent":"x"},"wait":{"ms":5}}"#),
        &work,
    );
    let params = arrived.params();

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
    let error = Arrived::new(Some(json), &work)
        .params()
        .read::<Goto>()
        .unwrap_err();

    assert_eq!(error.code(), "invalid_params", "{error:?}");
}

#[test]
fn a_map_reads_only_from_an_object() {
    let work = CountedWork::default();
    let error = Arrived::new(Some("[]"), &work)
        .params()
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

    assert_eq!(
        Arrived::new(Some(json), &work)
            .params()
            .read::<Mode>()
            .unwrap(),
        mode
    );
}

/// What an error says names no line and column: the params are one value.
#[test]
fn an_error_names_no_position() {
    let work = CountedWork::default();
    let error = Arrived::new(Some(r#"{"url":7}"#), &work)
        .params()
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
    let arrived = Arrived::new(Some(r#"{"url":"https://a.test/","wait":{"ms":5}}"#), &work);
    let params = arrived.params();
    let untouched = work.read(Row::MessageDecodes);

    let members = params.read_members::<Addressed<'_>>();
    let after_its_members = work.read(Row::MessageDecodes);
    let read = params.read::<Goto>().map(|goto| goto.url);
    let refused = params.read::<Mode>().is_err();
    let none = Arrived::new(None, &work)
        .params()
        .read::<BTreeMap<String, u64>>();

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

/// The params of a call that names its Run and its Session.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Named {
    run_id: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    count: u32,
}

/// What dispatch takes of `arrived`: its Trace context's `traceparent`
/// and its IDs.
fn taken<'a>(arrived: &Arrived<'a>) -> (Option<String>, Ids<'a>) {
    let preamble = arrived.preamble().unwrap();
    (
        preamble.meta.and_then(|meta| meta.traceparent),
        preamble.ids,
    )
}

/// The read of a request's params into their handler's type finds, in its
/// one pass, what dispatch takes of them: one decode a request (ADR 0140's
/// E1), their `_meta` skipped for the type and their IDs read into it.
#[rstest]
#[case::every_member(
    r#"{"_meta":{"traceparent":"00-0af7"},"run_id":"run-a","session_id":"session-a","count":2}"#,
    Named { run_id: "run-a".to_owned(), session_id: Some("session-a".to_owned()), count: 2 },
    (Some("00-0af7"), [Some("run-a"), Some("session-a")]),
)]
#[case::an_id_written_with_an_escape(
    r#"{"run_id":"run-\u0061","count":2}"#,
    Named { run_id: "run-a".to_owned(), session_id: None, count: 2 },
    (None, [Some("run-a"), None]),
)]
#[case::an_id_that_is_null(
    r#"{"run_id":"run-a","session_id":null}"#,
    Named { run_id: "run-a".to_owned(), session_id: None, count: 0 },
    (None, [Some("run-a"), None]),
)]
fn a_read_into_the_handlers_type_finds_what_dispatch_takes_in_the_same_pass(
    #[case] json: &str,
    #[case] named: Named,
    #[case] dispatch: (Option<&str>, [Option<&str>; 2]),
) {
    let work = CountedWork::default();
    let arrived = Arrived::new(Some(json), &work);

    let read = arrived.params().read::<Named>();
    let (traceparent, ids) = taken(&arrived);

    assert_eq!(read.unwrap(), named);
    assert_eq!(
        (traceparent.as_deref(), ids.each_ref().map(Option::as_deref)),
        dispatch
    );
    assert_eq!(work.read(Row::MessageDecodes), 1);
}

/// Params their handler's type refuses, and params nothing read, are read
/// by dispatch for what it takes of them, which counts one more decode;
/// a request that sent no params holds nothing to read.
#[rstest]
#[case::refused_before_the_ids(r#"{"count":"many","run_id":"run-a"}"#, true, 2)]
#[case::refused_after_the_ids(r#"{"run_id":"run-a","count":"many"}"#, true, 2)]
#[case::never_read(r#"{"run_id":"run-a"}"#, false, 1)]
fn params_no_read_went_through_are_read_by_dispatch(
    #[case] json: &str,
    #[case] read: bool,
    #[case] decodes: u64,
) {
    let work = CountedWork::default();
    let arrived = Arrived::new(Some(json), &work);

    if read {
        arrived.params().read::<Named>().unwrap_err();
    }
    let (_, ids) = taken(&arrived);
    let none = Arrived::new(None, &work).preamble().unwrap();

    assert_eq!(ids.each_ref().map(Option::as_deref), [Some("run-a"), None]);
    assert_eq!((none.meta, none.ids), (None, [None, None]));
    assert_eq!(work.read(Row::MessageDecodes), decodes);
}

/// An ID that is not the type its handler takes is refused as any member
/// is, in the same words.
#[test]
fn an_id_of_another_type_is_refused_in_serdes_words() {
    let work = CountedWork::default();
    let id = Arrived::new(Some(r#"{"run_id":7}"#), &work)
        .params()
        .read::<Named>()
        .unwrap_err();
    let other = Arrived::new(Some(r#"{"url":7}"#), &work)
        .params()
        .read::<Goto>()
        .unwrap_err();

    assert_eq!(id.to_string().replace("run_id", "url"), other.to_string(),);
}

/// A `_meta` the engine refuses fails the check of what dispatch takes,
/// whether the read into the handler's type went through or not.
#[rstest]
#[case::read(r#"{"run_id":"run-a","_meta":{"baggage":"user=a"}}"#)]
#[case::refused(r#"{"run_id":7,"_meta":[]}"#)]
fn a_meta_the_engine_refuses_fails_what_dispatch_takes(#[case] json: &str) {
    let work = CountedWork::default();
    let arrived = Arrived::new(Some(json), &work);
    let params = arrived.params();

    let _ = params.read::<Named>();

    assert!(params.taken().unwrap_err().to_string().contains("`_meta`"));
    assert!(arrived.preamble().is_err());
}

/// A read that went through to the end of the params without passing
/// their members, as one into a type that ignores them, finds nothing for
/// dispatch: dispatch reads them itself, one decode more, and still
/// refuses a `_meta` it does not take and finds their IDs.
#[rstest]
#[case::a_meta_it_refuses(r#"{"run_id":"run-a","_meta":{"baggage":"user=a"}}"#, None)]
#[case::their_ids(r#"{"run_id":"run-a"}"#, Some([Some("run-a"), None]))]
fn a_read_that_passes_none_of_their_members_leaves_dispatch_its_own_read(
    #[case] json: &str,
    #[case] ids: Option<[Option<&str>; 2]>,
) {
    let work = CountedWork::default();
    let arrived = Arrived::new(Some(json), &work);

    arrived.params().read::<serde::de::IgnoredAny>().unwrap();
    let preamble = arrived.preamble().ok();

    assert_eq!(
        preamble
            .as_ref()
            .map(|preamble| preamble.ids.each_ref().map(Option::as_deref)),
        ids
    );
    assert_eq!(work.read(Row::MessageDecodes), 2);
}
