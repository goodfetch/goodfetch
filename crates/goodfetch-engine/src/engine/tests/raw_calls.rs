//! A raw call in its Run's trace (ADR 0099, G8), over the in-memory
//! network: a call whose `_meta` marks it raw is a line of its Run's
//! `trace/calls.jsonl`, marked raw, with its operation, when it arrived and
//! how it ended, whether it names its Run by its Run ID or by a page; a
//! generated call, the same operation without the mark, writes nothing
//! there; and a `_meta` the engine refuses is refused as before, the call
//! never reaching its handler or the trace.

use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::home::RUN_TRACE_CALLS;
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::run::{call, run_folder};
use super::{Client, attached_on, engine_with, hello};
use crate::call_error::CallError;
use crate::composition::{CallContext, Contribution, Handler, RunOfCall};

/// A call that names its page, and answers.
const ON_PAGE: Operation = Operation {
    name: "test.on_page",
    description: "Answers a call that names its page.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// The page every Run of [`Pages`] opened.
const PAGE: &str = "the-page";

/// The Run the tests start.
const RUN: &str = "marked";

/// Pages as their contribution finds their Run: [`PAGE`] is [`RUN`]'s.
struct Pages;

impl RunOfCall for Pages {
    fn page_run<'a>(
        &'a self,
        page_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<RunId>> + Send + 'a>> {
        Box::pin(ready(
            (page_id == PAGE).then(|| RunId::parse(RUN).expect("the test's Run ID parses")),
        ))
    }

    fn session_run(&self, _session_id: &str) -> Option<RunId> {
        None
    }
}

/// `test.on_page`, and [`Pages`].
fn pages() -> Contribution {
    Contribution::new("test-pages")
        .with_handler(Handler::new(&ON_PAGE, |_: Value, _: CallContext| {
            ready(Ok::<Value, CallError>(json!({ "on_page": true })))
        }))
        .with_run_of_call(Arc::new(Pages))
}

/// Starts [`RUN`], makes `calls` and finishes it, then answers each call's
/// reply and the lines of the Run's raw call log, without when each call
/// arrived, which the test asserts is there.
fn traced(calls: &[(&str, Value)]) -> (Vec<Value>, Vec<Value>) {
    let (mut replies, lines) = ended_by(calls, &("run.finish", json!({ "run_id": RUN })));
    replies.pop();
    (replies, lines)
}

/// [`traced`], [`RUN`] ended by `ending`, whose reply is the last.
fn ended_by(calls: &[(&str, Value)], ending: &(&str, Value)) -> (Vec<Value>, Vec<Value>) {
    let env = TestEnv::new();
    let engine = engine_with(Product::Collie, Some(pages()));
    let calls: Vec<(String, Value)> = calls
        .iter()
        .chain([ending])
        .map(|(method, params)| ((*method).to_owned(), params.clone()))
        .collect();
    let replies = attached_on(&engine, &env, |mut client: Client| async move {
        client.send(&[hello("collie", "0.1")]).await;
        client.next().await;
        call(&mut client, 1, "run.start", json!({ "run_id": RUN })).await;
        let mut replies = Vec::new();
        for (id, (method, params)) in (2..).zip(calls) {
            replies.push(call(&mut client, id, &method, params).await);
        }
        replies
    });
    #[expect(
        clippy::disallowed_methods,
        reason = "the test reads the Run trace the engine wrote in its own Goodfetch home"
    )]
    let log =
        std::fs::read_to_string(run_folder(&env, RUN).join(RUN_TRACE_CALLS)).unwrap_or_default();
    let lines = log
        .lines()
        .map(|line| {
            let mut line: Value = serde_json::from_str(line).unwrap();
            let arrived_at = line
                .as_object_mut()
                .and_then(|line| line.remove("arrived_at"));
            assert!(
                arrived_at.as_ref().is_some_and(Value::is_string),
                "a raw call's line says when it arrived: {line}"
            );
            line
        })
        .collect();
    (replies, lines)
}

/// `params`, marked raw.
fn raw(params: &Value) -> Value {
    let mut params = params.clone();
    params["_meta"] = json!({ "raw": true });
    params
}

/// Covers: story:11-181
#[test]
fn a_raw_call_is_a_line_of_its_runs_trace_marked_raw_with_how_it_ended() {
    let by_run = json!({ "run_id": RUN });
    let by_page = json!({ "page_id": PAGE });

    let (replies, lines) = traced(&[
        ("run.usage", raw(&by_run)),
        ("test.crawl_setting", raw(&by_run)),
        ("test.on_page", raw(&by_page)),
    ]);

    assert_eq!(
        replies[1]["error"]["data"]["code"],
        json!("not_in_product"),
        "{}",
        replies[1]
    );
    assert_eq!(
        lines,
        [
            json!({ "operation": "run.usage", "raw": true, "ended": "answered" }),
            json!({
                "operation": "test.crawl_setting",
                "raw": true,
                "ended": { "failed": "not_in_product" },
            }),
            json!({ "operation": "test.on_page", "raw": true, "ended": "answered" }),
        ]
    );
}

/// A raw call that ends its own Run, or waits for its end, is a line of
/// that Run's trace, written before the Run's end closes it.
/// Covers: story:11-181
#[test]
fn a_raw_call_that_ends_its_own_run_is_a_line_of_its_runs_trace() {
    let by_run = json!({ "run_id": RUN });

    let ended: Vec<(Value, Vec<Value>)> = ["run.finish", "run.stop", "run.abort"]
        .into_iter()
        .map(|ending| {
            let (replies, lines) = ended_by(&[], &(ending, raw(&by_run)));
            (replies[0]["result"]["lifecycle"].clone(), lines)
        })
        .collect();

    assert_eq!(
        ended,
        ["run.finish", "run.stop", "run.abort"].map(|operation| {
            let lifecycle = if operation == "run.finish" {
                "finished"
            } else {
                "stopped"
            };
            (
                json!(lifecycle),
                vec![json!({ "operation": operation, "raw": true, "ended": "answered" })],
            )
        })
    );
}

/// A generated call adds nothing to the Run trace, by Run ID or by page.
/// Covers: story:11-181
#[test]
fn a_generated_call_writes_nothing_to_the_runs_trace() {
    let (replies, lines) = traced(&[
        ("run.usage", json!({ "run_id": RUN })),
        ("test.on_page", json!({ "page_id": PAGE })),
    ]);

    assert_eq!(
        (
            replies[0]["result"].is_object(),
            replies[1]["result"].clone()
        ),
        (true, json!({ "on_page": true }))
    );
    assert_eq!(lines, Vec::<Value>::new());
}

/// A raw call's `_meta` holding more than the mark is refused as a
/// generated call's is, before its handler, and so is in no Run trace.
/// Covers: story:11-181
#[test]
fn a_raw_call_whose_meta_is_refused_writes_nothing_to_the_runs_trace() {
    let mut refused = raw(&json!({ "run_id": RUN }));
    refused["_meta"]["baggage"] = json!("forged");

    let (replies, lines) = traced(&[("run.usage", refused)]);

    assert_eq!(replies[0]["error"]["data"]["code"], json!("invalid_params"));
    assert_eq!(lines, Vec::<Value>::new());
}
