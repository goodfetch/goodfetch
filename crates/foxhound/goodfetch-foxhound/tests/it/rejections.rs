//! Rejection table conformance cases on Foxhound's executable over stdio
//! (spec #581's seam: the protocol; ADR 0103): a Record the engine's check
//! refuses at `crawl_request.push` is raised to the handler and never
//! staged as a Record, and its rejection is staged with the attempt
//! instead, so it enters the Dataset's rejection table once, when its
//! attempt commits, numbered from 1 per Dataset in commit order, with the
//! Record as JSON, the reason, the request's ID and its URL. A `succeeded`
//! outcome counts the request's rejections per Dataset, and
//! `run.rejected_records` reads the table as a cursor with `after`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::Attached;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::attached;
use crate::frontier::{Inside, crawl_from};

/// The Datasets the crawls declare: `books`, with a schema requiring
/// `title` and `isbn` and keyed by `isbn`, and `notes`, keyed by `id` with
/// no schema.
fn declared() -> Value {
    json!([
        {
            "name": "books",
            "schema": {
                "type": "object",
                "required": ["title", "isbn"],
                "properties": {
                    "title": { "type": "string" },
                    "isbn": { "type": "string" },
                    "price": { "type": "number" },
                },
            },
            "key": ["isbn"],
        },
        { "name": "notes", "key": ["id"] },
    ])
}

/// `ctx.push(dataset, record)` from the attempt `handle` gave: its reply.
fn push(inside: &mut Inside<'_>, handle: &Value, dataset: &str, data: &Value) -> Value {
    inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "dataset": dataset,
            "data": data,
        }),
    )
}

/// The entry a rejection table should hold for a push `refused` with
/// `record_invalid`: numbered `number`, the Record `data` as pushed, the
/// reason the push's error gave, each issue's path and message, and the
/// request `request` of `url`.
fn entry(number: u64, data: &Value, refused: &Value, request: &str, url: &str) -> Value {
    assert_eq!(
        refused["error"]["data"]["code"], "record_invalid",
        "{refused}"
    );
    json!({
        "number": number,
        "data": data,
        "reason": refused["error"]["message"],
        "issues": refused["error"]["data"]["issues"],
        "request_id": request,
        "url": url,
    })
}

/// The reply of `run.rejected_records` for `dataset` of the Run `run_id`,
/// after `after` when given.
fn rejected(engine: &mut Attached, run_id: &str, dataset: &str, after: Option<u64>) -> Value {
    let mut params = json!({ "run_id": run_id, "dataset": dataset });
    if let Some(after) = after {
        params["after"] = json!(after);
    }
    engine.call("rejected", "run.rejected_records", &params, &mut |_| None)
}

/// Each outcome of the Run `run_id` in commit order: its request ID and
/// its `records` counts.
fn counts(engine: &mut Attached, run_id: &str) -> Vec<Value> {
    let read = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    read["result"]["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| {
            json!({
                "request": outcome["request"]["id"],
                "records": outcome["records"],
                "nothing_committed": outcome.get("nothing_committed"),
            })
        })
        .collect()
}

/// A Record the engine's check refuses at push enters its Dataset's
/// rejection table when its attempt commits: numbered from 1 per Dataset
/// in commit order, with the Record as JSON, the reason the push raised,
/// each issue, the request's ID and its URL; a push into an undeclared
/// Dataset enters no table. Each `succeeded` outcome counts its own
/// request's rejections per Dataset, so the counts equal the table's
/// entries for each request, and a request that kept nothing says so.
///
/// Covers: story:6-270, story:6-269, operation:run.rejected_records, operation:crawl_request.push, error:record_invalid
#[test]
fn a_record_the_engine_rejects_enters_its_datasets_rejection_table_numbered_with_its_request() {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let book = json!({ "title": "Dune", "isbn": "0441013597" });
    let cheap = json!({ "title": "Emma", "price": "cheap" });
    let untitled = json!({ "isbn": "0141439518" });
    let unkeyed = json!({ "text": "no id" });

    let mut refusals = Vec::new();
    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "rejecting",
            "seeds": [
                { "url": "https://shop.test/book", "label": "page" },
                { "url": "https://shop.test/other", "label": "page" },
            ],
            "datasets": declared(),
        }),
        &mut |inside, handle| {
            if handle["crawl_request"]["url"] == "https://shop.test/book" {
                push(inside, handle, "books", &book);
                refusals.push(push(inside, handle, "books", &cheap));
                refusals.push(push(inside, handle, "notes", &unkeyed));
                push(inside, handle, "bokks", &cheap);
            } else {
                refusals.push(push(inside, handle, "books", &untitled));
            }
            json!({ "result": { "records": [] } })
        },
    );
    let books = rejected(&mut engine, "rejecting", "books", None);
    let notes = rejected(&mut engine, "rejecting", "notes", None);
    let read = counts(&mut engine, "rejecting");
    assert!(engine.finished());

    let book_url = "https://shop.test/book";
    let other_url = "https://shop.test/other";
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &books["result"],
            &notes["result"],
            read
        ),
        (
            &json!("finished"),
            &json!({
                "records": [
                    entry(1, &cheap, &refusals[0], "1", book_url),
                    entry(2, &untitled, &refusals[2], "2", other_url),
                ],
                "next": 2,
                "ended": true,
            }),
            &json!({
                "records": [entry(1, &unkeyed, &refusals[1], "1", book_url)],
                "next": 1,
                "ended": true,
            }),
            vec![
                json!({
                    "request": "1", "nothing_committed": null,
                    "records": [
                        { "dataset": "books", "committed": 1, "dropped": 0, "rejected": 1 },
                        { "dataset": "notes", "committed": 0, "dropped": 0, "rejected": 1 },
                    ],
                }),
                json!({
                    "request": "2", "nothing_committed": "none_kept",
                    "records": [
                        { "dataset": "books", "committed": 0, "dropped": 0, "rejected": 1 },
                    ],
                }),
            ],
        )
    );
}

/// The rejection table is read as a cursor: `after` resumes a reader after
/// the last number it handled, and a page past the last entry of an ended
/// Run is empty and says so. A rejection staged by an attempt that failed
/// and was retried never enters the table, so a retried request's
/// rejections enter once; a Dataset the crawl does not declare is refused
/// with `dataset_unknown`.
///
/// Covers: story:6-270, operation:run.rejected_records, error:dataset_unknown
#[test]
fn the_rejection_table_is_read_after_a_number_and_holds_only_the_committing_attempts_rejections() {
    let env = TestEnv::new();
    let mut engine = attached(&env);
    let unkeyed = |handle: &Value, n: u32| {
        json!({
            "request": handle["crawl_request"]["id"],
            "attempt": handle["attempt"],
            "n": n,
        })
    };

    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "resumed",
            "seeds": [
                { "url": "https://shop.test/a", "label": "page" },
                { "url": "https://shop.test/b", "label": "page" },
            ],
            "datasets": declared(),
        }),
        &mut |inside, handle| {
            let a = handle["crawl_request"]["url"] == "https://shop.test/a";
            if a && handle["attempt"] == 1 {
                push(inside, handle, "notes", &unkeyed(handle, 1));
                return json!({ "error": {
                    "code": -32000, "message": "the site failed for now",
                    "data": { "code": "response_unavailable", "retryable": true, "status": 503 },
                } });
            }
            for n in 1..=2 {
                push(inside, handle, "notes", &unkeyed(handle, n));
            }
            json!({ "result": { "records": [] } })
        },
    );
    let all = rejected(&mut engine, "resumed", "notes", None);
    let later = rejected(&mut engine, "resumed", "notes", Some(2));
    let past = rejected(&mut engine, "resumed", "notes", Some(4));
    let unknown = rejected(&mut engine, "resumed", "bokks", None);
    assert!(engine.finished());

    let numbered = |page: &Value| -> Vec<(Value, Value)> {
        page["result"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| (entry["number"].clone(), entry["data"].clone()))
            .collect()
    };
    let mut attempts: Vec<Value> = numbered(&all)
        .into_iter()
        .map(|(_, data)| json!([data["request"], data["attempt"]]))
        .collect();
    attempts.sort_by_key(Value::to_string);
    let later_numbers: Vec<Value> = numbered(&later).into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            attempts,
            later_numbers,
            (&later["result"]["next"], &later["result"]["ended"]),
            &past["result"],
            (
                &unknown["error"]["data"]["code"],
                &unknown["error"]["data"]["declared"]
            ),
        ),
        (
            &json!("finished"),
            vec![
                json!(["1", 2]),
                json!(["1", 2]),
                json!(["2", 1]),
                json!(["2", 1])
            ],
            vec![json!(3), json!(4)],
            (&json!(4), &json!(true)),
            &json!({ "records": [], "next": 4, "ended": true }),
            (&json!("dataset_unknown"), &json!(["books", "notes"])),
        )
    );
}
