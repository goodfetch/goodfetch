//! A Payload's checks as pure rules of the Payload, its Label's schema and
//! the cap (spec #581's Testing Decisions: pure rules in process), and the
//! schema hash a resume compares, on a run file in memory.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_counted_work::CountedWork;
use std::collections::BTreeMap;

use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{Payload, PayloadRules, TABLES, store};

/// Rules of the cap `max_size` and the Labels `labels`.
fn rules(max_size: u64, labels: &[(&str, Option<Value>)]) -> PayloadRules {
    PayloadRules::built(
        max_size,
        labels
            .iter()
            .map(|(label, schema)| ((*label).to_owned(), schema.clone()))
            .collect::<BTreeMap<_, _>>(),
    )
}

fn isbn() -> Value {
    json!({ "type": "object", "properties": { "isbn": { "type": "string" } }, "required": ["isbn"] })
}

/// The code of what `rules` answers `payload` enqueued under `label`, and
/// the paths of its issues.
fn checked(rules: &PayloadRules, label: &str, payload: Option<&Value>) -> Option<(String, Value)> {
    let work = CountedWork::default();
    let payload = payload.map(|payload| Payload::of(payload.clone(), &work));
    rules
        .check(label, payload.as_ref(), &work)
        .err()
        .map(|error| {
            let details = error.details().unwrap();
            (
                error.code().to_owned(),
                json!({
                    "limit": details.get("limit"),
                    "paths": details.get("issues").map(|issues| issues
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|issue| issue["path"].clone())
                        .collect::<Vec<_>>()),
                }),
            )
        })
}

fn too_long() -> Option<(String, Value)> {
    refused(&json!({ "limit": "crawl.payload.max_size", "paths": null }))
}

fn not_fitting(paths: &[&str]) -> Option<(String, Value)> {
    refused(&json!({ "limit": null, "paths": paths }))
}

/// `payload_invalid` with `details`, as [`checked`] answers it.
#[expect(
    clippy::unnecessary_wraps,
    reason = "the cases compare it with what `checked` answers, `None` for a Payload added"
)]
fn refused(details: &Value) -> Option<(String, Value)> {
    Some(("payload_invalid".to_owned(), details.clone()))
}

/// The cap counts the Payload's JSON as written compactly, any Label's;
/// a typed Label's Payload fits its schema, a missing one read as `null`;
/// a Label declared without a schema takes any Payload.
///
/// Covers: story:6-223
#[rstest]
/// Covers: setting:crawl.payload.max_size
#[case::exactly_the_cap("untyped", Some(json!("x".repeat(30))), None)]
/// Covers: setting:crawl.payload.max_size, error:payload_invalid
#[case::one_byte_more("untyped", Some(json!("x".repeat(31))), too_long())]
/// Covers: setting:crawl.payload.max_size
#[case::an_object_at_the_cap("untyped", Some(json!({ "a": "x".repeat(24) })), None)]
/// Covers: setting:crawl.payload.max_size, error:payload_invalid
#[case::an_object_one_byte_more("untyped", Some(json!({ "a": "x".repeat(25) })), too_long())]
/// Covers: setting:crawl.payload.max_size, error:payload_invalid
#[case::too_long_for_a_typed_label_too("book", Some(json!({ "isbn": "x".repeat(30) })), too_long())]
#[case::fitting_its_schema("book", Some(json!({ "isbn": "1" })), None)]
/// Covers: error:payload_invalid
#[case::a_wrong_type_by_its_path("book", Some(json!({ "isbn": 1 })), not_fitting(&["/isbn"]))]
/// Covers: error:payload_invalid
#[case::missing_its_field("book", Some(json!({})), not_fitting(&[""]))]
/// Covers: error:payload_invalid
#[case::missing_altogether("book", None, not_fitting(&[""]))]
#[case::a_label_without_a_schema("untyped", Some(json!([1, "two"])), None)]
fn a_payload_is_checked_against_the_cap_and_its_labels_schema(
    #[case] label: &str,
    #[case] payload: Option<Value>,
    #[case] expected: Option<(String, Value)>,
) {
    let rules = rules(32, &[("book", Some(isbn())), ("untyped", None)]);

    assert_eq!(checked(&rules, label, payload.as_ref()), expected);
}

/// A Label the crawl does not declare is refused with `label_unknown`
/// before its Payload is read, with or without one and even past the cap,
/// naming the Label and those the crawl declares.
///
/// Covers: error:label_unknown
#[rstest]
#[case::with_a_payload(Some(json!(3)))]
#[case::without_one(None)]
#[case::past_the_cap(Some(json!("x".repeat(64))))]
fn a_label_the_crawl_does_not_declare_is_refused_naming_those_it_declares(
    #[case] payload: Option<Value>,
) {
    let rules = rules(32, &[("book", Some(isbn())), ("untyped", None)]);
    let work = CountedWork::default();
    let payload = payload.map(|payload| Payload::of(payload, &work));

    let refused = rules.check("free", payload.as_ref(), &work).unwrap_err();

    assert_eq!(
        (
            refused.code(),
            refused.details().cloned().map(Value::Object)
        ),
        (
            "label_unknown",
            Some(json!({ "label": "free", "declared": ["book", "untyped"] }))
        )
    );
}

/// A resume whose pending requests name a Label the crawl no longer
/// declares is refused with `label_unknown`, naming the first such Label
/// and those the crawl declares; one whose pending requests all name
/// declared Labels goes on, a new Label among them.
///
/// Covers: error:label_unknown
#[test]
fn a_resume_with_pending_requests_of_a_label_no_longer_declared_is_refused() {
    let resumed = rules(64, &[("book", None), ("review", None)]);

    let answers: Vec<Option<(String, Value)>> = [
        vec!["book".to_owned()],
        Vec::new(),
        vec!["author".to_owned(), "book".to_owned(), "shelf".to_owned()],
    ]
    .iter()
    .map(|pending| {
        resumed.resumes(pending, "run-1").err().map(|refused| {
            (
                refused.code().to_owned(),
                Value::Object(refused.details().cloned().unwrap()),
            )
        })
    })
    .collect();

    assert_eq!(
        answers,
        [
            None,
            None,
            Some((
                "label_unknown".to_owned(),
                json!({ "label": "author", "declared": ["book", "review"] })
            )),
        ]
    );
}

/// A run file in memory with the payload tables, `recorded`'s Labels
/// recorded, and each of `payloads` kept by its request's ID, unchecked.
fn run_file(recorded: &PayloadRules, payloads: &[(&str, Value)]) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    recorded.record(&connection).unwrap();
    for (request, payload) in payloads {
        let payload = Payload::of(payload.clone(), &CountedWork::default());
        store(&connection, request, Some(&payload)).unwrap();
    }
    connection
}

/// A resume's Labels change a recorded schema only when its hash differs:
/// the same schema with its keys in another order is not checked again,
/// even over Payloads that would fail it, while a changed one checks every
/// pending Payload of its Label, counting those that fail and naming the
/// first, and one they all fit, or a Label the Run did not begin with whose
/// Label has nothing pending, goes on.
///
/// Covers: story:6-223
#[test]
fn a_resume_checks_pending_payloads_only_against_a_schema_whose_hash_changed() {
    let began = rules(64, &[("book", Some(isbn()))]);
    let file = run_file(
        &began,
        &[
            ("1", json!({ "isbn": "1" })),
            ("2", json!({ "isbn": 2 })),
            ("3", json!({ "isbn": "3" })),
        ],
    );
    let pending = |label: &str| {
        Ok(if label == "book" {
            vec!["1".to_owned(), "2".to_owned(), "3".to_owned()]
        } else {
            Vec::new()
        })
    };
    let reordered: Value = serde_json::from_str(
        r#"{ "required": ["isbn"], "properties": { "isbn": { "type": "string" } }, "type": "object" }"#,
    )
    .unwrap();
    let mut titled = isbn();
    titled["properties"]["title"] = json!({ "type": "string" });
    titled["required"] = json!(["isbn", "title"]);
    let any_isbn = json!({ "type": "object", "required": ["isbn"] });

    let answers: Vec<Option<Value>> = [
        rules(64, &[("book", Some(reordered))]),
        rules(64, &[("book", Some(titled))]),
        rules(
            64,
            &[
                ("book", Some(any_isbn)),
                ("author", Some(json!({ "type": "string" }))),
            ],
        ),
        rules(64, &[("book", None)]),
    ]
    .iter()
    .map(|resumed| {
        resumed
            .schema_changed(&file, pending)
            .unwrap()
            .map(|changed| {
                json!({
                    "label": changed.label, "failing": changed.failing, "pending": changed.pending,
                    "first": changed.crawl_request_id,
                })
            })
    })
    .collect();

    assert_eq!(
        answers,
        [
            None,
            Some(json!({ "label": "book", "failing": 3, "pending": 3, "first": "1" })),
            None,
            None,
        ]
    );
}

/// A resume records its Labels again, in place of those the Run began
/// with, so a new Label is declared from then on and one no longer
/// declared is not, while the cap stays the one recorded.
///
/// Covers: story:6-223
#[test]
fn a_resume_records_its_labels_in_place_of_those_the_run_began_with() {
    let file = run_file(&rules(64, &[("book", Some(isbn())), ("old", None)]), &[]);

    rules(
        32,
        &[("book", None), ("new", Some(json!({ "type": "string" })))],
    )
    .record_labels_again(&file)
    .unwrap();

    assert_eq!(
        PayloadRules::read(&file).unwrap(),
        rules(
            64,
            &[("book", None), ("new", Some(json!({ "type": "string" })))]
        )
    );
}
