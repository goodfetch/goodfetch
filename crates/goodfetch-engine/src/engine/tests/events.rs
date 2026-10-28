//! The event stream and page bodies, through the engine's public interface
//! over the in-memory network (spec #213's Seam 3): a subscriber that grants
//! no credits falls behind to an `events.resync` and holds back no Run and
//! no other subscriber (ADR 0024), telemetry coalesces and counts what it dropped, a
//! subscription resumes after the position its Client last read or starts
//! with `events.resync` (ADR 0031), and a page body is a file in its Run's
//! folder that `page.body` sends one chunk per credit. Credits are granted,
//! never timed (ADR 0064).

use std::time::Duration;

use goodfetch_protocol::catalog::{
    Event, EventClass, Operation, OperationClass, OperationKind, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::run::{RunId, RunKind, Tags};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Client, attached_on, attached_with, dependencies, engine_with, hello};
use crate::composition::{CallContext, CallError, Contribution, Handler};
use crate::engine::{Dependencies, Engine};
use crate::entropy::Entropy;
use crate::events::EventsOptions;
use crate::runs::{EndedBy, Start, Started};

/// A telemetry event of the tests, scoped by its `scope` field.
const TEST_PROGRESS: Event = Event {
    name: "test.progress",
    description: "A test's progress.",
    class: EventClass::Telemetry,
    payload: schema_of::<Value>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Publishes `test.progress` for each of `steps`, `{scope, step}`, and
/// answers once it has.
const PROGRESS: Operation = Operation {
    name: "test.progress",
    description: "Publishes progress.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[&TEST_PROGRESS],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: goodfetch_protocol::catalog::PROTOCOL_VERSION,
};

/// Starts the Run `run_id` and stores `size` bytes as one of its page
/// bodies, answering the body's handle.
const BODY: Operation = Operation {
    name: "test.body",
    description: "Stores a page body.",
    ..PROGRESS
};

/// The `size` bytes a test body holds.
pub(super) fn body_bytes(size: usize) -> Vec<u8> {
    (0..size)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect()
}

fn test_streams() -> Contribution {
    Contribution::new("test-streams")
        .with_handler(Handler::new(
            &PROGRESS,
            |params: Value, context: CallContext| async move {
                for step in params["steps"].as_array().unwrap() {
                    let scope = step["scope"].as_str().unwrap();
                    context
                        .events()
                        .publish(&TEST_PROGRESS, Some(scope), step)
                        .await;
                }
                Ok::<Value, CallError>(json!({ "published": true }))
            },
        ))
        .with_handler(Handler::new(
            &BODY,
            |params: Value, context: CallContext| async move {
                let started = context
                    .runs()
                    .start(Start {
                        run_id: Some(RunId::parse(params["run_id"].as_str().unwrap()).unwrap()),
                        tags: Tags::default(),
                        kind: RunKind::Collie,
                        needs: Product::Collie,
                        inputs: std::collections::BTreeMap::new(),
                        ended_by: EndedBy::Finish,
                        initialize: None,
                        policy_version: None,
                        weakened: Vec::new(),
                        binding: None,
                        clean_up_digest: None,
                        checkpoint_interval: None,
                        timeout: None,
                        settings: None,
                    })
                    .await
                    .unwrap();
                let Started::New(run) = started else {
                    panic!("the Run began before");
                };
                let size = usize::try_from(params["size"].as_u64().unwrap()).unwrap();
                let handle = run.store_body(body_bytes(size).into()).await.unwrap();
                Ok::<Value, CallError>(serde_json::to_value(handle).unwrap())
            },
        ))
}

pub(super) fn engine(options: EventsOptions) -> Engine {
    engine_with(Product::Collie, Some(test_streams())).with_events(options)
}

fn options(queue: usize, retained: usize) -> EventsOptions {
    EventsOptions { queue, retained }
}

fn call(id: i64, method: &str, params: Value) -> Value {
    let mut call = json!({ "jsonrpc": "2.0", "id": id, "method": method });
    call["params"] = params;
    call
}

pub(super) fn credit(id: i64, credits: u32) -> Value {
    json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": id, "credits": credits } })
}

fn cancel(id: i64) -> Value {
    json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": id } })
}

/// Opens the Connection: `hello`, then `events.subscribe` as call `id`
/// with `params`; answers the engine instance the `hello` named.
async fn open(client: &mut Client, id: i64, params: Value) -> Value {
    client.send(&[hello("collie", "0.1")]).await;
    let opened = client.next().await.unwrap();
    assert!(opened.get("result").is_some(), "{opened}");
    client.send(&[call(id, "events.subscribe", params)]).await;
    opened["result"]["engine_instance"].clone()
}

/// What `events.subscribe` resumes after: `position` of `engine_instance`.
fn after(engine_instance: &Value, position: u64) -> Value {
    json!({ "after": { "engine_instance": engine_instance, "position": position } })
}

/// The events of a `$/events` batch of the call `id`, if `message` is one.
fn batch(message: &Value, id: i64) -> Option<Vec<Value>> {
    (message["method"] == "$/events" && message["params"]["id"] == json!(id))
        .then(|| message["params"]["events"].as_array().unwrap().clone())
}

/// Reads until the subscription `id` sent `count` events, and answers them,
/// with every other message read on the way.
async fn read_events(client: &mut Client, id: i64, count: usize) -> (Vec<Value>, Vec<Value>) {
    let (mut events, mut others) = (Vec::new(), Vec::new());
    while events.len() < count {
        let message = client.next().await.expect("the engine closed");
        match batch(&message, id) {
            Some(batch) => events.extend(batch),
            None => others.push(message),
        }
    }
    (events, others)
}

fn positions(events: &[Value]) -> Vec<u64> {
    events
        .iter()
        .map(|event| event["position"].as_u64().unwrap())
        .collect()
}

/// The value at `pointer` in the `index`th of `events`, or `null`.
fn field(events: &[Value], index: usize, pointer: &str) -> Value {
    events
        .get(index)
        .and_then(|event| event.pointer(pointer))
        .cloned()
        .unwrap_or(Value::Null)
}

fn strictly_increasing(positions: &[u64]) -> bool {
    positions.windows(2).all(|pair| pair[0] < pair[1])
}

/// Reads the next message, or `None` when none comes within a minute of the
/// paused clock, which moves on only once the engine has nothing left to
/// do: a message the engine holds back never comes.
async fn next_unheld(client: &mut Client) -> Option<Value> {
    tokio::time::timeout(Duration::from_secs(60), client.next())
        .await
        .ok()
        .flatten()
}

/// Covers: story:3-8, operation:events.subscribe, event:run.axes_changed, event:events.resync
#[test]
fn a_subscriber_that_grants_no_credits_falls_behind_to_a_resync_and_holds_back_no_run_or_subscriber()
 {
    const QUEUE: usize = 2;
    const RUNS: usize = 5;
    let env = TestEnv::new();
    let engine = engine(options(QUEUE, 64));

    let (unheld, read, behind) = attached_on(&engine, &env, |mut client| async move {
        // Subscription 1 grants nothing; subscription 2 reads everything.
        open(&mut client, 1, json!({})).await;
        client
            .send(&[call(2, "events.subscribe", json!({})), credit(2, 100)])
            .await;
        let starts: Vec<Value> = (0..RUNS)
            .map(|index| {
                call(
                    10 + i64::try_from(index).unwrap(),
                    "run.start",
                    json!({ "run_id": format!("run-{index}") }),
                )
            })
            .collect();
        client.send(&starts).await;
        let (mut answered, mut read) = (0, Vec::new());
        while answered < RUNS || read.len() < RUNS {
            let Some(message) = next_unheld(&mut client).await else {
                return (false, read, Vec::new());
            };
            if let Some(batch) = batch(&message, 2) {
                read.extend(batch);
            } else if message["result"]["lifecycle"] == "running" {
                answered += 1;
            } else {
                panic!("an unexpected message: {message}");
            }
        }
        client.send(&[credit(1, 100)]).await;
        let (mut behind, _) = read_events(&mut client, 1, 1).await;
        client
            .send(&[call(30, "run.start", json!({ "run_id": "after" }))])
            .await;
        let (after, _) = read_events(&mut client, 1, 1).await;
        behind.extend(after);
        (true, read, behind)
    });

    let last = positions(&read).last().copied().unwrap_or(0);
    let runs: Vec<Value> = (0..RUNS)
        .map(|index| json!({ "run_id": format!("run-{index}"), "lifecycle": "running" }))
        .collect();
    assert_eq!(
        (
            unheld,
            read.len(),
            strictly_increasing(&positions(&read)),
            field(&behind, 0, "/event"),
            field(&behind, 0, "/data/runs"),
            field(&behind, 0, "/position").as_u64().unwrap_or(0) > last,
            field(&behind, 1, "/event"),
            field(&behind, 1, "/data/run_id"),
            strictly_increasing(&positions(&behind)),
        ),
        (
            true,
            RUNS,
            true,
            json!("events.resync"),
            Value::Array(runs),
            true,
            json!("run.axes_changed"),
            json!("after"),
            true,
        ),
        "{behind:#?}"
    );
}

/// Covers: story:3-8
#[test]
fn telemetry_never_waits_and_coalesces_per_scope_counting_what_it_dropped() {
    let env = TestEnv::new();
    let engine = engine(options(1, 64));

    let (published, events) = attached_on(&engine, &env, |mut client| async move {
        open(&mut client, 1, json!({})).await;
        let steps: Vec<Value> = (1..=5)
            .map(|step| json!({ "scope": "a", "step": step }))
            .chain([json!({ "scope": "b", "step": 1 })])
            .collect();
        client
            .send(&[call(2, "test.progress", json!({ "steps": steps }))])
            .await;
        // With no credit granted, the producer still answers.
        let published = client.next().await.unwrap();
        client.send(&[credit(1, 10)]).await;
        let (events, _) = read_events(&mut client, 1, 2).await;
        (published, events)
    });

    let summary: Vec<(Value, Value, Value)> = events
        .iter()
        .map(|event| {
            (
                event["data"]["scope"].clone(),
                event["data"]["step"].clone(),
                event.get("dropped").cloned().unwrap_or(json!(0)),
            )
        })
        .collect();
    assert_eq!(
        (published["id"].clone(), summary),
        (
            json!(2),
            vec![
                (json!("a"), json!(5), json!(4)),
                (json!("b"), json!(1), json!(0)),
            ]
        )
    );
}

/// Starts each Run of `run_ids` on a subscription `id` granted ample
/// credits, and answers the engine instance and the events it read.
async fn start_runs(client: &mut Client, id: i64, run_ids: &[&str]) -> (Value, Vec<Value>) {
    let instance = open(client, id, json!({})).await;
    client.send(&[credit(id, 100)]).await;
    for (index, run_id) in run_ids.iter().enumerate() {
        client
            .send(&[call(
                20 + i64::try_from(index).unwrap(),
                "run.start",
                json!({ "run_id": run_id }),
            )])
            .await;
    }
    let (events, _) = read_events(client, id, run_ids.len()).await;
    (instance, events)
}

/// Covers: story:3-9
#[test]
fn a_subscription_resumes_after_the_position_its_client_last_read() {
    let env = TestEnv::new();
    let engine = engine(options(64, 3));

    let (first, resumed) = attached_on(&engine, &env, |mut client| async move {
        let (instance, first) = start_runs(&mut client, 1, &["one", "two", "three"]).await;
        client.send(&[cancel(1)]).await;
        let position = first[0]["position"].as_u64().unwrap();
        client
            .send(&[
                call(2, "events.subscribe", after(&instance, position)),
                credit(2, 100),
            ])
            .await;
        let (resumed, _) = read_events(&mut client, 2, 2).await;
        (first, resumed)
    });

    assert_eq!(
        positions(&resumed),
        positions(&first)[1..].to_vec(),
        "{resumed:#?}"
    );
}

/// Covers: story:3-9, event:events.resync
#[test]
fn a_position_the_engine_no_longer_holds_starts_with_a_resync_then_the_stream() {
    let env = TestEnv::new();
    let engine = engine(options(64, 1));

    let (first, gone, future) = attached_on(&engine, &env, |mut client| async move {
        let (instance, first) = start_runs(&mut client, 1, &["one", "two"]).await;
        let last = first[1]["position"].as_u64().unwrap();
        client
            .send(&[
                call(2, "events.subscribe", after(&instance, 0)),
                credit(2, 100),
                call(3, "events.subscribe", after(&instance, last + 100)),
                credit(3, 100),
            ])
            .await;
        let (gone, _) = read_events(&mut client, 2, 1).await;
        let (future, _) = read_events(&mut client, 3, 1).await;
        client
            .send(&[call(30, "run.start", json!({ "run_id": "three" }))])
            .await;
        let (after_gone, _) = read_events(&mut client, 2, 1).await;
        (first, [gone, after_gone].concat(), future)
    });

    let last = first[1]["position"].as_u64().unwrap();
    let runs = json!([
        { "run_id": "one", "lifecycle": "running" },
        { "run_id": "two", "lifecycle": "running" },
    ]);
    assert_eq!(
        (
            gone[0]["event"].clone(),
            gone[0]["data"]["runs"].clone(),
            gone[0]["position"].as_u64().unwrap() > last,
            gone[1]["event"].clone(),
            gone[1]["data"]["run_id"].clone(),
            strictly_increasing(&positions(&gone)),
            future[0]["event"].clone(),
            future[0]["data"]["runs"].clone(),
        ),
        (
            json!("events.resync"),
            runs.clone(),
            true,
            json!("run.axes_changed"),
            json!("three"),
            true,
            json!("events.resync"),
            runs,
        )
    );
}

/// Covers: story:3-9, event:events.resync
#[test]
fn after_a_restart_a_position_the_previous_engine_instance_gave_starts_with_a_resync() {
    let env = TestEnv::new();
    let engine = engine(options(64, 64));
    let read = |client: Client| async move {
        let mut client = client;
        start_runs(&mut client, 1, &["one", "two"]).await
    };

    let (before, first) = attached_on(&engine, &env, read);
    // The same home and engine, in a new process: a Daemon restarted.
    let last_read = after(&before, first[0]["position"].as_u64().unwrap());
    let (restarted, resumed) = attached_with(
        &engine,
        || Dependencies {
            environment: std::collections::BTreeMap::new(),
            keychain: crate::secrets::Keychain::absent(),
            entropy: Entropy::seeded(env.seed().wrapping_add(1)),
            ..dependencies(&env)
        },
        |mut client| async move {
            let (restarted, _) = start_runs(&mut client, 1, &["three", "four"]).await;
            client
                .send(&[call(2, "events.subscribe", last_read), credit(2, 100)])
                .await;
            let (resumed, _) = read_events(&mut client, 2, 1).await;
            (restarted, resumed)
        },
    );

    assert_eq!(
        (
            before.is_string(),
            before == restarted,
            field(&resumed, 0, "/event"),
            field(&resumed, 0, "/data/runs"),
        ),
        (
            true,
            false,
            json!("events.resync"),
            json!([
                { "run_id": "four", "lifecycle": "running" },
                { "run_id": "three", "lifecycle": "running" },
            ]),
        ),
        "{resumed:#?}"
    );
}

/// Covers: story:3-10, operation:page.body
#[test]
fn a_page_body_is_a_file_in_its_runs_folder_and_page_body_sends_one_chunk_per_credit() {
    const SIZE: usize = 150 * 1024;
    let env = TestEnv::new();
    let engine = engine(EventsOptions::default());

    let (handle, chunks_beyond, chunks, read) =
        attached_on(&engine, &env, |mut client| async move {
            client.send(&[hello("collie", "0.1")]).await;
            let _ = client.next().await;
            client
                .send(&[call(
                    1,
                    "test.body",
                    json!({ "run_id": "bodies", "size": SIZE }),
                )])
                .await;
            let handle = client.next().await.unwrap()["result"].clone();
            client
                .send(&[call(
                    2,
                    "page.body",
                    json!({ "run_id": "bodies", "body": handle["body"] }),
                )])
                .await;
            let (mut granted, mut chunks, mut chunks_beyond) = (0, Vec::new(), Vec::new());
            loop {
                if chunks.len() == granted {
                    client.send(&[credit(2, 1)]).await;
                    granted += 1;
                }
                let message = client.next().await.expect("the engine closed");
                if message["method"] == "$/chunk" {
                    chunks.push(message["params"].clone());
                    if chunks.len() > granted {
                        chunks_beyond.push(chunks.len());
                    }
                } else {
                    return (handle, chunks_beyond, chunks, message);
                }
            }
        });

    let bytes: Vec<u8> = chunks
        .iter()
        .flat_map(|chunk| {
            data_encoding::BASE64
                .decode(chunk["data"].as_str().unwrap().as_bytes())
                .unwrap()
        })
        .collect();
    let offsets: Vec<u64> = chunks
        .iter()
        .map(|chunk| chunk["offset"].as_u64().unwrap())
        .collect();
    let path = std::path::PathBuf::from(handle["path"].as_str().unwrap());
    assert_eq!(
        (
            path.starts_with(env.home().join("1").join("runs").join("bodies")),
            std::fs::read(&path).unwrap() == body_bytes(SIZE),
            handle["size"].clone(),
            chunks_beyond,
            offsets,
            bytes == body_bytes(SIZE),
            read["result"].clone(),
        ),
        (
            true,
            true,
            json!(SIZE),
            Vec::<usize>::new(),
            vec![0, 64 * 1024, 128 * 1024],
            true,
            json!({ "size": SIZE }),
        )
    );
}

/// Covers: error:body_not_found
#[test]
fn page_body_of_an_unknown_body_or_run_fails_with_body_not_found() {
    let env = TestEnv::new();
    let engine = engine(EventsOptions::default());
    let unknown = "0".repeat(64);

    let replies = attached_on(&engine, &env, |mut client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        let _ = client.next().await;
        client
            .send(&[
                call(1, "test.body", json!({ "run_id": "bodies", "size": 10 })),
                call(
                    2,
                    "page.body",
                    json!({ "run_id": "bodies", "body": unknown }),
                ),
                call(
                    3,
                    "page.body",
                    json!({ "run_id": "nowhere", "body": unknown }),
                ),
                call(
                    4,
                    "page.body",
                    json!({ "run_id": "bodies", "body": "../run.sqlite" }),
                ),
            ])
            .await;
        let mut replies = Vec::new();
        while replies.len() < 4 {
            replies.push(client.next().await.unwrap());
        }
        replies.sort_by_key(|reply| reply["id"].as_i64());
        replies
    });

    let codes: Vec<Value> = replies[1..]
        .iter()
        .map(|reply| reply["error"]["data"]["code"].clone())
        .collect();
    assert_eq!(codes, vec![json!("body_not_found"); 3], "{replies:#?}");
}

/// Covers: event:run.axes_changed
#[test]
fn dispatch_records_each_event_it_sends_with_its_name() {
    let env = TestEnv::new();
    let engine = engine(EventsOptions::default());

    let records = super::dispatch_records(|| {
        attached_on(&engine, &env, |mut client| async move {
            start_runs(&mut client, 1, &["recorded"]).await.1
        })
    });

    let events: Vec<&String> = records
        .iter()
        .filter(|record| record.starts_with(": event sent"))
        .collect();
    assert_eq!(events, [": event sent event=\"run.axes_changed\""]);
}
