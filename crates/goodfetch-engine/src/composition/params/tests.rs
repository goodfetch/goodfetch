use std::borrow::Cow;
use std::collections::BTreeMap;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::page::read::PageReadParams;
use rstest::rstest;
use serde::Deserialize;
use serde_json::value::RawValue;

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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Kept {
    dataset: String,
    records: Vec<Box<RawValue>>,
}

/// Raw JSON values in the params, as a push's Records, keep their fields
/// in the order written through the strict read, at every depth, which a
/// parsed `Value` would sort.
#[test]
fn raw_values_in_params_keep_their_fields_in_the_order_written() {
    let work = CountedWork::default();
    let arrived = Arrived::new(
        Some(r#"{"dataset":"books","records":[{"title":"Dune","price":9}]}"#),
        &work,
    );

    let kept = arrived.params().read::<Kept>().unwrap();

    assert_eq!(
        (kept.dataset.as_str(), kept.records[0].get()),
        ("books", r#"{"title":"Dune","price":9}"#)
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

/// A `page.read`'s fields read only from objects at every depth, and each
/// `kind` only by its name, as the schema gives them (issue 3324).
#[rstest]
#[case::a_field_an_array(r#"{"t":[{"kind":"css","selector":"h1"}]}"#)]
#[case::a_locator_an_array(r#"{"t":{"locator":["css","h1"]}}"#)]
#[case::a_list_an_array(r#"{"t":{"list":[{"kind":"css","selector":"li"}]}}"#)]
#[case::a_rows_field_an_array(
    r#"{"t":{"rows":{"kind":"css","selector":"li"},"fields":{"a":[{"kind":"css","selector":"a"}]}}}"#
)]
#[case::a_match_an_array(r#"{"t":{"locator":{"kind":"css","selector":"h1"},"match":["(a)"]}}"#)]
#[case::a_locator_kind_by_index(r#"{"t":{"locator":{"kind":1,"selector":"//h1"}}}"#)]
#[case::a_transform_kind_by_index(
    r#"{"t":{"locator":{"kind":"css","selector":"h1"},"transform":{"kind":1,"locale":"fr"}}}"#
)]
fn page_read_fields_the_schema_refuses_are_invalid_params(#[case] fields: &str) {
    let work = CountedWork::default();
    let json = format!(r#"{{"page_id":"page-a","fields":{fields}}}"#);
    let error = Arrived::new(Some(&json), &work)
        .params()
        .read::<PageReadParams>()
        .unwrap_err();

    assert_eq!(error.code(), "invalid_params", "{error:?}");
}

/// The same fields as the schema gives them read.
#[test]
fn page_read_fields_as_the_schema_gives_them_read() {
    let work = CountedWork::default();
    let json = r#"{"page_id":"page-a","fields":{
        "title":{"locator":{"kind":"xpath","selector":"//h1"},"match":{"pattern":"(a)"},"transform":{"kind":"number","locale":"fr"}},
        "links":{"list":{"locator":{"kind":"css","selector":"li"}}},
        "rows":{"rows":{"kind":"css","selector":"li"},"fields":{"a":{"locator":{"kind":"css","selector":"a"}}}}
    }}"#;

    let read = Arrived::new(Some(json), &work)
        .params()
        .read::<PageReadParams>()
        .unwrap();

    assert_eq!(read, serde_json::from_str::<PageReadParams>(json).unwrap());
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

/// Each read of a call's params counts one decode of them (ADR 0140's E1),
/// whether it reads them into their type or not; params never read count
/// none, and a call that sent none reads as `{}`.
#[test]
fn each_read_of_a_calls_params_counts_one_decode() {
    let work = CountedWork::default();
    let arrived = Arrived::new(Some(r#"{"url":"https://a.test/","wait":{"ms":5}}"#), &work);
    let params = arrived.params();
    let untouched = work.read(Row::MessageDecodes);

    let read = params.read::<Goto>().map(|goto| goto.url);
    let refused = params.read::<Mode>().is_err();
    let none = Arrived::new(None, &work)
        .params()
        .read::<BTreeMap<String, u64>>();

    assert_eq!((untouched, work.read(Row::MessageDecodes)), (0, 3));
    assert_eq!(read.unwrap(), "https://a.test/");
    assert!(refused);
    assert_eq!(none.unwrap(), BTreeMap::new());
}

/// The params of a page call, which name its page.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnPage {
    #[expect(dead_code, reason = "read for its page only")]
    page_id: String,
    #[expect(dead_code, reason = "read for its page only")]
    url: String,
}

/// The params of a locator call, whose locator names its page.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OnLocator {
    #[expect(dead_code, reason = "read for its page only")]
    locator: Spot,
}

/// A locator.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Spot {
    #[expect(dead_code, reason = "read for its page only")]
    page_id: String,
    #[expect(dead_code, reason = "read for its page only")]
    selector: String,
}

/// Params read for their `url` alone, every other member skipped.
#[derive(Debug, Deserialize)]
struct Loose {
    #[expect(dead_code, reason = "read for its page only")]
    url: String,
}

/// The page `json` names once read as `P` and taken by dispatch, and the
/// decodes that counted.
fn page_after<P: serde::de::DeserializeOwned>(json: &str) -> (Option<String>, u64) {
    let work = CountedWork::default();
    let arrived = Arrived::new(Some(json), &work);
    let params = arrived.params();
    params.read::<P>().unwrap();
    params.taken().unwrap();
    (
        params.page().map(Cow::into_owned),
        work.read(Row::MessageDecodes),
    )
}

/// The page a call names, by its own `page_id` or its locator's, is found
/// in the one pass that reads the params into the handler's type, whether
/// the type reads those members or skips them; a call's own `page_id` wins
/// over its locator's, and a locator that is no object names none.
#[rstest]
#[case::by_its_page_id(page_after::<OnPage>, r#"{"url":"u","page_id":"page-a"}"#, Some("page-a"))]
#[case::by_its_locator(
    page_after::<OnLocator>,
    r#"{"locator":{"selector":"a","page_id":"page-a"}}"#,
    Some("page-a")
)]
#[case::written_with_an_escape(page_after::<OnPage>, r#"{"page_id":"page-\u0061","url":"u"}"#, Some("page-a"))]
#[case::by_a_page_id_its_type_skips(page_after::<Loose>, r#"{"page_id":"page-a","url":"u"}"#, Some("page-a"))]
#[case::by_a_locator_its_type_skips(
    page_after::<Loose>,
    r#"{"url":"u","locator":{"page_id":"page-a","near":[{"page_id":"page-b"}]}}"#,
    Some("page-a")
)]
#[case::by_a_locator_holding_a_meta(
    page_after::<Loose>,
    r#"{"url":"u","locator":{"_meta":{},"page_id":"page-a"}}"#,
    Some("page-a")
)]
#[case::its_own_before_its_locators(
    page_after::<Loose>,
    r#"{"locator":{"page_id":"page-b"},"page_id":"page-a","url":"u"}"#,
    Some("page-a")
)]
#[case::by_a_locator_that_is_no_object(page_after::<Loose>, r##"{"url":"u","locator":"#go"}"##, None)]
#[case::by_none(page_after::<Loose>, r#"{"url":"u","near":{"page_id":"page-a"}}"#, None)]
fn the_page_a_call_names_is_found_in_the_read_into_its_handlers_type(
    #[case] read: fn(&str) -> (Option<String>, u64),
    #[case] json: &str,
    #[case] page: Option<&str>,
) {
    let (found, decodes) = read(json);

    assert_eq!((found.as_deref(), decodes), (page, 1));
}

/// Params no read went through whole, as those read into a type that
/// passes none of their members, have their page found by dispatch's own
/// read, one decode more; params never read, by that read alone.
#[rstest]
#[case::by_its_page_id(r#"{"page_id":"page-a","url":"u"}"#)]
#[case::by_its_locator(r#"{"url":"u","locator":{"page_id":"page-a"}}"#)]
fn the_page_params_no_read_went_through_name_is_found_by_dispatch(#[case] json: &str) {
    let work = CountedWork::default();
    let skipped = Arrived::new(Some(json), &work);
    skipped.params().read::<serde::de::IgnoredAny>().unwrap();
    let skipped = (
        skipped.params().page().map(Cow::into_owned),
        work.read(Row::MessageDecodes),
    );
    let unread = Arrived::new(Some(json), &work);
    let unread = (
        unread.params().page().map(Cow::into_owned),
        work.read(Row::MessageDecodes),
    );

    assert_eq!(
        (skipped, unread),
        (
            (Some("page-a".to_owned()), 2),
            (Some("page-a".to_owned()), 3)
        )
    );
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
#[case::a_raw_calls_mark(
    r#"{"_meta":{"traceparent":"00-0af7","raw":true},"run_id":"run-a"}"#,
    Named { run_id: "run-a".to_owned(), session_id: None, count: 0 },
    (Some("00-0af7"), [Some("run-a"), None]),
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
#[case::a_raw_mark_that_is_no_boolean(r#"{"run_id":"run-a","_meta":{"raw":"yes"}}"#)]
fn a_meta_the_engine_refuses_fails_what_dispatch_takes(#[case] json: &str) {
    let work = CountedWork::default();
    let arrived = Arrived::new(Some(json), &work);
    let params = arrived.params();

    let _ = params.read::<Named>();

    let refusal = params.taken().unwrap_err();
    assert_eq!(
        (refusal.code(), refusal.to_string().contains("`_meta`")),
        ("invalid_params", true)
    );
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
