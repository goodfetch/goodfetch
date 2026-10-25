//! A crawl request's Payload on Foxhound's executable over stdio (spec
//! #581's seam: the protocol, on Foxhound's executable; ADRs 0082 and
//! 0094): a Payload reaches its handler as it was given, in a field of its
//! own that is no part of the request's identity; one that does not fit
//! its Label's JSON Schema, or whose JSON is longer than
//! `crawl.payload.max_size`, is refused at enqueue with `payload_invalid`;
//! and a start under the ID of a Run whose Label's Payload schema changed
//! is refused with `payload_schema_changed` when a pending Payload fails
//! the new schema.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::frontier::{Inside, added, crawl_from, not_added};

/// A book's Payload schema: its ISBN, a text, and nothing else.
fn book_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "isbn": { "type": "string" } },
        "required": ["isbn"],
        "additionalProperties": false,
    })
}

/// `ctx.enqueue(url, label, payload)` from the attempt `handle` gave;
/// answers its `Enqueued`, or its error's code and details.
fn enqueue(
    inside: &mut Inside<'_>,
    handle: &Value,
    url: &str,
    label: &str,
    payload: Option<Value>,
) -> Value {
    let mut params = json!({
        "run_id": handle["run_id"],
        "crawl_request_id": handle["crawl_request"]["id"],
        "lease_epoch": handle["lease_epoch"],
        "url": url,
        "label": label,
    });
    if let Some(payload) = payload {
        params["payload"] = payload;
    }
    answer(&inside.call("crawl_request.enqueue", &params))
}

/// A reply's result, or its error's code and details, each issue's
/// message left out.
fn answer(reply: &Value) -> Value {
    reply.get("result").cloned().unwrap_or_else(|| {
        let mut details = reply["error"]["data"].clone();
        let code = details["code"].take();
        let details = details.as_object_mut().unwrap();
        details.remove("code");
        details.remove("retryable");
        if let Some(issues) = details.get_mut("issues").and_then(Value::as_array_mut) {
            for issue in issues {
                issue.as_object_mut().unwrap().remove("message");
            }
        }
        json!({ "code": code, "details": details })
    })
}

/// Covers: story:6-223, error:payload_invalid, operation:crawl.start, operation:crawl_request.enqueue
#[test]
fn a_payload_reaches_its_handler_as_given_apart_from_its_identity_and_one_that_does_not_fit_its_label_is_refused()
 {
    let env = TestEnv::new();
    let shelf_payload = json!({ "note": "Ünïcødé ✓", "nested": [1, { "a": null }, 2.5, true] });
    let mut engine = Attached::start(&env);

    let mut received = Vec::new();
    let mut answers = Vec::new();
    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "typed",
            "labels": { "shelf": {}, "book": { "payload": book_schema() } },
            "seeds": [{ "url": "https://shop.test/shelf", "label": "shelf", "payload": shelf_payload }],
        }),
        &mut |inside, handle| {
            let request = &handle["crawl_request"];
            received.push((request["label"].clone(), request.get("payload").cloned()));
            if request["label"] == "shelf" {
                for (url, payload) in [
                    ("https://shop.test/book/1", Some(json!({ "isbn": "1" }))),
                    ("https://shop.test/book/1", Some(json!({ "isbn": "2" }))),
                    ("https://shop.test/book/2", Some(json!({ "isbn": 2 }))),
                    ("https://shop.test/book/3", None),
                ] {
                    answers.push(enqueue(inside, handle, url, "book", payload));
                }
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finish());

    let invalid = |path: &str| json!({ "code": "payload_invalid", "details": { "label": "book", "issues": [{ "path": path }] } });
    assert_eq!(
        (&report["result"]["lifecycle"], received, answers),
        (
            &json!("finished"),
            vec![
                (json!("shelf"), Some(shelf_payload)),
                (json!("book"), Some(json!({ "isbn": "1" }))),
            ],
            vec![
                added("2", "https://shop.test/book/1"),
                not_added("duplicate", "https://shop.test/book/1"),
                invalid("/isbn"),
                invalid(""),
            ],
        )
    );
}

/// The params of a crawl `changed` of three books, its Label `book` of
/// the Payload schema `schema`.
fn books(schema: &Value) -> Value {
    json!({
        "run_id": "changed",
        "labels": { "book": { "payload": schema } },
        "seeds": (1..=3)
            .map(|n| json!({ "url": format!("https://shop.test/book/{n}"), "label": "book", "payload": { "isbn": n.to_string() } }))
            .collect::<Vec<_>>(),
        "settings": { "crawl": { "handler": { "concurrency": 1 } } },
    })
}

/// A handler that records each request's ID in `handled` and leaves it
/// unanswered.
fn holding(handled: &mut Vec<Value>) -> impl FnMut(&Value) -> Option<Value> + '_ {
    |params| {
        handled.push(params["crawl_request"]["id"].clone());
        None
    }
}

/// A start under the ID of a Run, ADR 0102's resume, after the Label
/// `book`'s Payload schema gained a required field its three pending
/// Payloads lack is refused before any work, and one whose new schema
/// they fit goes on with the Run: the run file kept each Label's schema
/// hash. The first book's attempt is left unanswered meanwhile, so all
/// three are pending, one of them leased: the engine may send that
/// attempt after `crawl.start` replies, so the later starts wait for its
/// `crawl_request.handle` (ADR 0064).
///
/// Covers: story:6-223, error:payload_schema_changed, operation:crawl.start
#[test]
fn a_start_under_its_run_id_after_a_payload_schema_changed_is_refused_when_a_pending_payload_fails_it()
 {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);
    let mut handled = Vec::new();

    let started = engine.call(
        "start",
        "crawl.start",
        &books(&book_schema()),
        &mut holding(&mut handled),
    );
    while handled.is_empty() {
        let message = engine
            .next()
            .expect("the engine sends the first book's attempt");
        if message["method"] == "crawl_request.handle" {
            handled.push(message["params"]["crawl_request"]["id"].clone());
        }
    }
    let mut titled = book_schema();
    titled["properties"]["title"] = json!({ "type": "string" });
    let mut required = titled.clone();
    required["required"] = json!(["isbn", "title"]);
    let refused = engine.call(
        "again",
        "crawl.start",
        &books(&required),
        &mut holding(&mut handled),
    );
    let mut more = books(&titled);
    more["labels"]["author"] = json!({ "payload": { "type": "object" } });
    let compatible = engine.call(
        "compatible",
        "crawl.start",
        &more,
        &mut holding(&mut handled),
    );
    engine.call(
        "abort",
        "run.abort",
        &json!({ "run_id": "changed" }),
        &mut holding(&mut handled),
    );
    assert!(engine.finish());

    assert_eq!(
        (
            &started["result"]["started_as"],
            answer(&refused),
            &compatible["result"]["started_as"],
            handled,
        ),
        (
            &json!("new"),
            json!({
                "code": "payload_schema_changed",
                "details": {
                    "label": "book", "failing": 3, "pending": 3, "crawl_request_id": "1",
                    "issues": [{ "path": "" }],
                },
            }),
            &json!("existing"),
            vec![json!("1")],
        )
    );
}

/// A Payload whose JSON is `size` bytes long: a string of `size - 2`
/// characters and its quotes.
fn of_size(size: usize) -> Value {
    Value::String("x".repeat(size - 2))
}

/// `payload_invalid` naming `crawl.payload.max_size` for a `page`
/// Payload of `size` bytes over `max_size`.
fn too_long(size: u64, max_size: u64) -> Value {
    json!({
        "code": "payload_invalid",
        "details": { "label": "page", "limit": "crawl.payload.max_size", "size": size, "max_size": max_size },
    })
}

/// Covers: story:6-223, error:payload_invalid, setting:crawl.payload.max_size, operation:crawl_request.enqueue, operation:run.add_seed
#[test]
fn a_payload_longer_than_crawl_payload_max_size_is_refused_at_enqueue_naming_the_setting() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);

    let mut answers = Vec::new();
    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "capped",
            "settings": { "crawl": { "payload": { "max_size": 64 } } },
            "seeds": [{ "url": "https://shop.test/", "label": "page", "payload": of_size(64) }],
        }),
        &mut |inside, handle| {
            if handle["crawl_request"]["url"] == "https://shop.test/" {
                answers.push(enqueue(inside, handle, "/a", "page", Some(of_size(64))));
                answers.push(enqueue(inside, handle, "/b", "page", Some(of_size(65))));
                answers.push(answer(&inside.call(
                    "run.add_seed",
                    &json!({
                        "run_id": handle["run_id"],
                        "seed": { "url": "https://shop.test/c", "label": "page", "payload": of_size(65) },
                    }),
                )));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finish());

    assert_eq!(
        (&report["result"]["lifecycle"], answers),
        (
            &json!("finished"),
            vec![
                added("2", "https://shop.test/a"),
                too_long(65, 64),
                too_long(65, 64),
            ],
        )
    );
}

/// Covers: story:6-223, error:payload_invalid, setting:crawl.payload.max_size, operation:crawl.start
#[test]
fn a_seed_past_the_default_64_kib_is_refused_before_its_run_begins() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env);

    let refused = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "large",
            "seeds": [
                { "url": "https://shop.test/a", "label": "page", "payload": of_size(65_536) },
                { "url": "https://shop.test/b", "label": "page", "payload": of_size(65_537) },
            ],
        }),
        &mut |_| None,
    );
    let listed = engine.call("list", "run.list", &json!({}), &mut |_| None);
    assert!(engine.finish());

    assert_eq!(
        (answer(&refused), &listed["result"]["runs"]),
        (too_long(65_537, 65_536), &json!([]))
    );
}
