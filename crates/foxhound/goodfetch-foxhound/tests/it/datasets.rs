//! Dataset and Record conformance cases on Foxhound's executable over
//! stdio (spec #581's seam: the protocol; ADRs 0082, 0103 and 0105): a
//! crawl declares its Datasets at `crawl.start`, a duplicate name and a key
//! on a field the schema does not require refused before the Run begins; a
//! handler pushes each Record through `crawl_request.push`, checked against
//! its Dataset in the engine and staged with its attempt, or refused with
//! `record_invalid` naming each issue's path and not staged; the Records an
//! attempt staged commit once with it, each Dataset numbering its own from
//! 1 in commit order; and a `succeeded` outcome counts them per Dataset.
//! A start under the ID of a Run, ADR 0102's resume, is refused with
//! `dataset_unknown` when the crawl no longer declares a Dataset that holds
//! Records and with `dataset_key_invalid` when such a Dataset's key
//! changed; a changed schema resumes, each Record keeping the hash of the
//! schema it was committed under.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::frontier::{Inside, crawl_from};
use crate::run_file;
use crate::site::Site;

/// A book's JSON Schema: `title` and `isbn` required, `price` a number.
fn book_schema() -> Value {
    json!({
        "type": "object",
        "required": ["title", "isbn"],
        "properties": {
            "title": { "type": "string" },
            "isbn": { "type": "string" },
            "price": { "type": "number" },
        },
    })
}

/// The reply of a `crawl.start` of one seed declaring `datasets`.
fn start_declaring((engine, site): (&mut Attached, &Site), datasets: &Value) -> Value {
    engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "declared",
            "seeds": [{ "url": site.page("/"), "label": "page" }],
            "datasets": datasets,
        }),
        &mut |_| None,
    )
}

/// The Error code and details of `reply`'s error, the details without
/// their code and retryable flag.
fn refusal(reply: &Value) -> Value {
    let mut details = reply["error"]["data"].clone();
    let code = details["code"].take();
    let object = details.as_object_mut().unwrap();
    object.remove("code");
    object.remove("retryable");
    object.remove("fix");
    json!({ "code": code, "details": details })
}

/// A crawl whose Datasets cannot be declared is refused before its Run
/// begins, naming the Dataset and, for a key, the field.
///
/// Covers: story:6-266, operation:crawl.start
#[rstest]
/// Covers: error:dataset_duplicate
#[case::a_second_dataset_of_one_name(
    json!([{ "name": "books" }, { "name": "books", "schema": book_schema() }]),
    json!({ "code": "dataset_duplicate", "details": { "dataset": "books" } }),
)]
/// Covers: error:dataset_key_invalid
#[case::a_key_on_a_field_the_schema_does_not_require(
    json!([{ "name": "books", "schema": book_schema(), "key": ["isbn", "price"] }]),
    json!({ "code": "dataset_key_invalid", "details": { "dataset": "books", "reason": "not_required", "field": "price" } }),
)]
/// Covers: error:dataset_key_invalid
#[case::a_key_on_a_field_the_schema_does_not_have(
    json!([{ "name": "books", "schema": book_schema(), "key": ["author"] }]),
    json!({ "code": "dataset_key_invalid", "details": { "dataset": "books", "reason": "not_required", "field": "author" } }),
)]
fn a_crawl_whose_datasets_cannot_be_declared_is_refused_before_its_run_begins(
    #[case] datasets: Value,
    #[case] expected: Value,
) {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);

    let reply = start_declaring((&mut engine, &site), &datasets);
    let listed = engine.call("list", "run.list", &json!({}), &mut |_| None);
    assert!(engine.finished());

    assert_eq!(
        (refusal(&reply), &listed["result"]["runs"]),
        (expected, &json!([]))
    );
}

/// The Datasets the pushing crawls declare: `books`, typed and keyed by
/// `isbn`, and `notes`, keyed by `id` with no schema.
fn declared() -> Value {
    json!([
        { "name": "books", "schema": book_schema(), "key": ["isbn"] },
        { "name": "notes", "key": ["id"] },
    ])
}

/// `ctx.push(dataset, record)` from the attempt `handle` gave: its result,
/// else its Error code and details, each issue's message left out.
fn push(inside: &mut Inside<'_>, handle: &Value, dataset: &str, data: &Value) -> Value {
    push_under(inside, handle, &handle["lease_epoch"], dataset, data)
}

/// [`push`] naming the lease epoch `lease_epoch`.
fn push_under(
    inside: &mut Inside<'_>,
    handle: &Value,
    lease_epoch: &Value,
    dataset: &str,
    data: &Value,
) -> Value {
    let reply = inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": lease_epoch,
            "dataset": dataset,
            "data": data,
        }),
    );
    if let Some(result) = reply.get("result") {
        return result.clone();
    }
    let mut refused = refusal(&reply);
    if let Some(issues) = refused["details"]
        .get_mut("issues")
        .and_then(Value::as_array_mut)
    {
        for issue in issues {
            issue.as_object_mut().unwrap().remove("message");
        }
    }
    refused
}

/// Each committed Record of the run file of `run_id`, in Dataset and
/// number order: its Dataset, its number, its crawl request and its data.
fn committed(env: &TestEnv, run_id: &str) -> Vec<(String, i64, String, Value)> {
    let connection = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    let mut select = connection
        .prepare("SELECT dataset, number, work_item, data FROM records ORDER BY dataset, number")
        .unwrap();
    select
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                serde_json::from_str(&row.get::<_, String>(3)?).unwrap(),
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The outcomes of the Run `run_id`, in commit order, as `run.outcomes`
/// reads them: each one's request ID, kind, `records` and
/// `nothing_committed`, or its error's code.
fn outcomes(engine: &mut Attached, run_id: &str) -> Vec<Value> {
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
                "kind": outcome["kind"],
                "records": outcome.get("records"),
                "nothing_committed": outcome.get("nothing_committed"),
                "error": outcome["error"].get("code"),
            })
        })
        .collect()
}

/// A Record pushed through `crawl_request.push` is checked in the engine
/// against the Dataset it names and staged, or refused and not staged:
/// `record_invalid` names each issue's path (a missing required field at
/// the Record, a field of the wrong type at its own path, a key's field
/// missing from a Dataset with no schema, a Record that is no JSON
/// object), `dataset_unknown` names the Datasets the crawl declares. The
/// attempt keeps what it pushed before, so only the two valid Records
/// commit, each Dataset numbering from 1; the outcome counts them per
/// Dataset, with the rejections of the refused Records of a declared
/// Dataset, and a request that pushed nothing says so.
///
/// Covers: story:6-267, story:6-268, story:6-269, operation:crawl_request.push, error:record_invalid, error:dataset_unknown, operation:run.outcomes
#[test]
fn a_pushed_record_is_checked_against_its_dataset_and_only_one_that_holds_is_staged() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let book = json!({ "title": "Dune", "isbn": "0441013597", "price": 9.99 });

    let mut answers = Vec::new();
    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "pushed",
            "seeds": [
                { "url": site.page("/book"), "label": "page" },
                { "url": site.page("/empty"), "label": "page" },
            ],
            "datasets": declared(),
        }),
        &mut |inside, handle| {
            if handle["crawl_request"]["url"] == site.page("/book") {
                answers = vec![
                    push(inside, handle, "books", &book),
                    push(
                        inside,
                        handle,
                        "books",
                        &json!({ "title": "Emma", "price": "cheap" }),
                    ),
                    push(inside, handle, "notes", &json!({ "text": "no id" })),
                    push(inside, handle, "notes", &json!(5)),
                    push(inside, handle, "bokks", &book),
                    push(inside, handle, "notes", &json!({ "id": "n1" })),
                ];
            }
            json!({ "result": { "records": [] } })
        },
    );
    let read = outcomes(&mut engine, "pushed");
    assert!(engine.finished());

    let invalid = |dataset: &str, paths: &[&str]| {
        json!({
            "code": "record_invalid",
            "details": {
                "dataset": dataset,
                "issues": paths.iter().map(|path| json!({ "path": path })).collect::<Vec<_>>(),
            },
        })
    };
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            answers,
            committed(&env, "pushed"),
            read
        ),
        (
            &json!("finished"),
            vec![
                json!({}),
                invalid("books", &["", "/price"]),
                invalid("notes", &["/id"]),
                invalid("notes", &[""]),
                json!({
                    "code": "dataset_unknown",
                    "details": { "dataset": "bokks", "declared": ["books", "notes"] },
                }),
                json!({}),
            ],
            vec![
                ("books".to_owned(), 1, "1".to_owned(), book),
                ("notes".to_owned(), 1, "1".to_owned(), json!({ "id": "n1" })),
            ],
            vec![
                json!({
                    "request": "1", "kind": "succeeded", "nothing_committed": null, "error": null,
                    "records": [
                        { "dataset": "books", "committed": 1, "dropped": 0, "rejected": 1 },
                        { "dataset": "notes", "committed": 1, "dropped": 0, "rejected": 2 },
                    ],
                }),
                json!({
                    "request": "2", "kind": "succeeded", "records": [], "error": null,
                    "nothing_committed": "none_pushed",
                }),
            ],
        )
    );
}

/// A Record in a handler's answer that does not hold for its Dataset, as
/// an escaped `record_invalid` would leave it, fails the request with
/// `record_invalid`, not retried, and nothing the attempt staged commits:
/// neither the answer's valid Record nor the one it pushed.
///
/// Covers: story:6-267, error:record_invalid, operation:crawl_request.push
#[test]
fn an_answer_holding_a_record_that_does_not_hold_fails_its_request_unretried_and_commits_nothing() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);

    let mut attempts = 0;
    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "escaped",
            "seeds": [{ "url": site.page("/book"), "label": "page" }],
            "datasets": declared(),
        }),
        &mut |inside, handle| {
            attempts += 1;
            push(inside, handle, "notes", &json!({ "id": "n1" }));
            json!({ "result": { "records": [
                { "dataset": "notes", "data": { "id": "n2" } },
                { "dataset": "books", "data": { "title": "Emma" } },
            ] } })
        },
    );
    let read = outcomes(&mut engine, "escaped");
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            attempts,
            committed(&env, "escaped"),
            read
        ),
        (
            &json!("finished"),
            1,
            Vec::new(),
            vec![json!({
                "request": "1", "kind": "failed", "records": null, "nothing_committed": null,
                "error": "record_invalid",
            })],
        )
    );
}

/// The Records two attempts of one request pushed, the first failing
/// with a retryable error, commit once: only the second's; a push naming
/// the first attempt once it ended fails with `attempt_ended`. Each
/// Dataset numbers its committed Records from 1, in commit order, with
/// no gaps, across the requests that committed.
///
/// Covers: story:6-268, operation:crawl_request.push, error:attempt_ended
#[test]
fn records_of_a_retried_request_commit_once_numbered_from_one_in_commit_order() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let note = |handle: &Value, n: u32| {
        let request = handle["crawl_request"]["id"].as_str().unwrap();
        json!({ "id": format!("{request}-{}-{n}", handle["attempt"]) })
    };

    let mut first_epoch = Value::Null;
    let mut late = Value::Null;
    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "retried",
            "seeds": [
                { "url": site.page("/a"), "label": "page" },
                { "url": site.page("/b"), "label": "page" },
            ],
            "datasets": declared(),
        }),
        &mut |inside, handle| {
            for n in 1..=2 {
                push(inside, handle, "notes", &note(handle, n));
            }
            let a = handle["crawl_request"]["url"] == site.page("/a");
            if a && handle["attempt"] == 1 {
                first_epoch = handle["lease_epoch"].clone();
                return json!({ "error": {
                    "code": -32000, "message": "the site failed for now",
                    "data": { "code": "response_unavailable", "retryable": true, "status": 503 },
                } });
            }
            if a {
                late = push_under(
                    inside,
                    handle,
                    &first_epoch,
                    "notes",
                    &json!({ "id": "late" }),
                );
            }
            json!({ "result": { "records": [] } })
        },
    );
    let read = outcomes(&mut engine, "retried");
    assert!(engine.finished());

    let committed = committed(&env, "retried");
    let numbers: Vec<i64> = committed.iter().map(|(_, number, _, _)| *number).collect();
    let mut by_number: Vec<Value> = committed
        .iter()
        .map(|(_, _, request, _)| json!(request))
        .collect();
    by_number.dedup();
    let commit_order: Vec<Value> = read
        .iter()
        .filter(|outcome| outcome["kind"] == "succeeded")
        .map(|outcome| outcome["request"].clone())
        .collect();
    let mut ids: Vec<Value> = committed
        .iter()
        .map(|(_, _, _, data)| data["id"].clone())
        .collect();
    ids.sort_by_key(Value::to_string);
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &late["code"],
            numbers,
            by_number,
            ids
        ),
        (
            &json!("finished"),
            &json!("attempt_ended"),
            vec![1, 2, 3, 4],
            commit_order,
            vec![
                json!("1-2-1"),
                json!("1-2-2"),
                json!("2-1-1"),
                json!("2-1-2")
            ],
        )
    );
}

/// The reply of a start under the Run ID `run_id` of the seed
/// `https://shop.test/book`, declaring `datasets`: its error's code and
/// details, else how it started.
fn start_again((engine, site): (&mut Attached, &Site), run_id: &str, datasets: &Value) -> Value {
    let reply = engine.call(
        "again",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": [{ "url": site.page("/book"), "label": "page" }],
            "datasets": datasets,
        }),
        &mut |_| None,
    );
    match reply.get("result") {
        Some(result) => json!({ "started_as": result["started_as"] }),
        None => refusal(&reply),
    }
}

/// A start under the ID of a Run whose `books` holds a Record and whose
/// `notes` holds none is checked against the Datasets the Run began with
/// before any work: `books` dropped or renamed fails with
/// `dataset_unknown`, naming the Datasets now declared, and its key
/// changed with `dataset_key_invalid`, naming both keys; `notes`, which
/// holds no Record, may be dropped or keyed anew.
///
/// Covers: story:6-272, operation:crawl.start
#[rstest]
/// Covers: error:dataset_unknown
#[case::a_dataset_holding_records_dropped(
    json!([{ "name": "notes", "key": ["id"] }]),
    json!({ "code": "dataset_unknown", "details": { "dataset": "books", "declared": ["notes"] } }),
)]
/// Covers: error:dataset_unknown
#[case::a_dataset_holding_records_renamed(
    json!([
        { "name": "titles", "schema": book_schema(), "key": ["isbn"] },
        { "name": "notes", "key": ["id"] },
    ]),
    json!({ "code": "dataset_unknown", "details": { "dataset": "books", "declared": ["titles", "notes"] } }),
)]
/// Covers: error:dataset_key_invalid
#[case::the_key_of_a_dataset_holding_records_changed(
    json!([
        { "name": "books", "schema": book_schema(), "key": ["title"] },
        { "name": "notes", "key": ["id"] },
    ]),
    json!({
        "code": "dataset_key_invalid",
        "details": { "dataset": "books", "reason": "changed", "key": ["title"], "began_with": ["isbn"] },
    }),
)]
#[case::a_dataset_holding_no_record_dropped(
    json!([{ "name": "books", "schema": book_schema(), "key": ["isbn"] }]),
    json!({ "started_as": "existing" }),
)]
#[case::the_key_of_a_dataset_holding_no_record_changed(
    json!([
        { "name": "books", "schema": book_schema(), "key": ["isbn"] },
        { "name": "notes", "key": ["text"] },
    ]),
    json!({ "started_as": "existing" }),
)]
fn a_start_under_its_run_id_refuses_a_dataset_holding_records_dropped_or_keyed_anew(
    #[case] datasets: Value,
    #[case] expected: Value,
) {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let book = json!({ "title": "Dune", "isbn": "0441013597" });

    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "kept",
            "seeds": [{ "url": site.page("/book"), "label": "page" }],
            "datasets": declared(),
        }),
        &mut |_, _| json!({ "result": { "records": [{ "dataset": "books", "data": book }] } }),
    );
    let answer = start_again((&mut engine, &site), "kept", &datasets);
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            committed(&env, "kept").len(),
            answer
        ),
        (&json!("finished"), 1, expected)
    );
}

/// A start under the ID of a Run whose `books` holds only rejected
/// Records, every pushed Record having failed its schema, refuses `books`
/// renamed with `dataset_unknown`, as it refuses a Dataset holding Records:
/// `run.rejected_records` still reads its rejections under its name.
///
/// Covers: story:6-272, operation:crawl.start, operation:run.rejected_records, error:dataset_unknown
#[test]
fn a_start_under_its_run_id_refuses_a_dataset_holding_only_rejected_records_renamed() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let untitled = json!({ "isbn": "0441013597" });

    let mut refused = Value::Null;
    let report = crawl_from(
        &mut engine,
        &json!({
            "run_id": "rejecting",
            "seeds": [{ "url": site.page("/book"), "label": "page" }],
            "datasets": declared(),
        }),
        &mut |inside, handle| {
            refused = push(inside, handle, "books", &untitled);
            json!({ "result": { "records": [] } })
        },
    );
    let answer = start_again(
        (&mut engine, &site),
        "rejecting",
        &json!([
            { "name": "titles", "schema": book_schema(), "key": ["isbn"] },
            { "name": "notes", "key": ["id"] },
        ]),
    );
    let rejections = engine.call(
        "rejected",
        "run.rejected_records",
        &json!({ "run_id": "rejecting", "dataset": "books" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &refused["code"],
            committed(&env, "rejecting").len(),
            answer,
            rejections["result"]["records"].as_array().map(Vec::len),
        ),
        (
            &json!("finished"),
            &json!("record_invalid"),
            0,
            json!({ "code": "dataset_unknown", "details": { "dataset": "books", "declared": ["titles", "notes"] } }),
            Some(1),
        )
    );
}

/// Each committed Record of the run file of `run_id`, in number order:
/// its number, its data and the hash of the schema it was committed under.
fn hashed(env: &TestEnv, run_id: &str) -> Vec<(i64, Value, Option<String>)> {
    let connection = run_file(env, run_id);
    let mut select = connection
        .prepare("SELECT number, data, schema_hash FROM records ORDER BY number")
        .unwrap();
    select
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                serde_json::from_str(&row.get::<_, String>(1)?).unwrap(),
                row.get(2)?,
            ))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// The schema hash the run file of `run_id` holds for its Dataset
/// `books`.
fn books_hash(env: &TestEnv, run_id: &str) -> Option<String> {
    run_file(env, run_id)
        .query_row(
            "SELECT schema_hash FROM datasets WHERE name = 'books'",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

/// A crawl killed while its second request is handled resumes, after a
/// start under its Run ID whose `books` schema changed (`price` now a
/// string), under that schema: the resumed attempt's Record, which only the
/// new schema takes, commits, and each Record keeps the hash of the schema
/// it was committed under.
///
/// Covers: story:6-272, operation:crawl.start, operation:run.resume
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test's own thread polls the run file until the first attempt committed, before it kills the engine"
)]
fn a_crawl_resumed_after_its_schema_changed_checks_new_records_against_it_and_each_keeps_its_schemas_hash()
 {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let schema = |price: &str| {
        let mut schema = book_schema();
        schema["properties"]["price"] = json!({ "type": price });
        json!([{ "name": "books", "schema": schema, "key": ["isbn"] }])
    };
    let one_at_a_time = json!({ "crawl": { "handler": { "concurrency": 1 } } });
    let start = |datasets: Value| {
        json!({
            "run_id": "evolved",
            "seeds": [
                { "url": site.page("/a"), "label": "page" },
                { "url": site.page("/b"), "label": "page" },
            ],
            "datasets": datasets,
            "settings": one_at_a_time,
        })
    };
    let first = json!({ "title": "Dune", "isbn": "1", "price": 9.99 });
    let second = json!({ "title": "Emma", "isbn": "2", "price": "cheap" });

    let mut killed = site.engine(&env);
    killed.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": start(schema("number")),
    }));
    let (mut started, mut held) = (false, false);
    while !(started && held) {
        let message = killed.next().expect("the engine runs the crawl");
        if message["id"] == "start" {
            started = true;
        } else if message["method"] == "crawl_request.handle" {
            if message["params"]["crawl_request"]["url"] == site.page("/a") {
                killed.send(&json!({
                    "jsonrpc": "2.0", "id": message["id"],
                    "result": { "records": [{ "dataset": "books", "data": first }] },
                }));
            } else {
                held = true;
            }
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while hashed(&env, "evolved").is_empty() {
        assert!(
            std::time::Instant::now() < deadline,
            "the first seed's Record is committed"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let began = books_hash(&env, "evolved");
    killed.kill();

    let mut resuming = site.engine(&env);
    let mut answer = |params: &Value| {
        assert_eq!(params["crawl_request"]["url"], site.page("/b"));
        Some(json!({ "result": { "records": [{ "dataset": "books", "data": second }] } }))
    };
    let again = resuming.call(
        "again",
        "crawl.start",
        &start(schema("string")),
        &mut answer,
    );
    let resumed = resuming.call(
        "resume",
        "run.resume",
        &json!({ "run_id": "evolved", "settings": one_at_a_time }),
        &mut answer,
    );
    let finished = resuming.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "evolved" }),
        &mut answer,
    );
    assert!(resuming.finished());

    let now = books_hash(&env, "evolved");
    assert_ne!(began, now, "the changed schema has another hash");
    assert_eq!(
        (
            &again["result"]["started_as"],
            &resumed["result"]["started_as"],
            &finished["result"]["lifecycle"],
            hashed(&env, "evolved"),
        ),
        (
            &json!("existing"),
            &json!("resumed"),
            &json!("finished"),
            vec![(1, first, began), (2, second, now)],
        )
    );
}
