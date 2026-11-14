//! Calls that no page will make again, killed right after their answer
//! (spec #2138's Seam 1, ADR 0142): at `batch`, the default, an answer
//! comes once the call's change is in a written batch, so a `SIGKILL` that
//! follows it takes none of them.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::cell::{Cell, RefCell};

use goodfetch_test_client::Attached;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::seeds::{Came, STREAM, next_for_stream, seed, send_seeds};
use crate::site::Site;

/// The Run ID of the crawl the case kills.
const RUN_ID: &str = "answered";

/// The `crawl.start` of the keep-alive crawl from `failing`, every other
/// setting at its default.
fn start(engine: &mut Attached, failing: &str) -> Value {
    engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": RUN_ID,
            "seeds": [seed(failing)],
            "settings": { "crawl": { "keep_alive": true } },
        }),
        &mut |_| None,
    )
}

/// `run.retry_failed` on `engine`, asked until it re-admits the request
/// the crawl's first attempt failed for good, and its last answer: every
/// later request stays unanswered, so a kill finds it in flight.
fn retry_the_failed_request(engine: &mut Attached) -> Value {
    let failed = Cell::new(false);
    let mut fail_once = |_: &Value| {
        (!failed.replace(true)).then(|| {
            json!({ "error": { "code": -32003, "message": "handler_failed",
                "data": { "code": "handler_failed", "retryable": false } } })
        })
    };
    let mut retried = json!(null);
    for _ in 0..600 {
        retried = engine.call(
            "retry",
            "run.retry_failed",
            &json!({ "run_id": RUN_ID }),
            &mut fail_once,
        );
        if retried["result"]["readmitted"] == 1 {
            break;
        }
    }
    retried
}

/// At `batch`, a `SIGKILL` right after the answers of `run.retry_failed`,
/// `key_value_store.set` made outside a handler and `run.add_seeds` loses
/// none of them: the resumed Run holds the streamed seed and the value
/// set, and lists none of the requests `run.retry_failed` re-admitted as
/// failed.
///
/// Covers: story:3-25, setting:run_state.durability, operation:run.add_seeds, operation:run.retry_failed, operation:key_value_store.set
#[test]
fn at_batch_a_kill_right_after_an_answer_loses_no_call_that_no_page_will_make_again() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let (failing, streamed) = (site.page("/failing"), site.page("/streamed"));

    let mut killed = site.engine(&env);
    start(&mut killed, &failing);
    let retried = retry_the_failed_request(&mut killed);
    let set = killed.call(
        "set",
        "key_value_store.set",
        &json!({ "run_id": RUN_ID, "key": "cursor", "value": 3 }),
        &mut |_| None,
    );
    killed.send(&json!({
        "jsonrpc": "2.0", "id": STREAM, "method": "run.add_seeds",
        "params": { "run_id": RUN_ID },
    }));
    let mut leave = |_: &Value| None;
    let credit = next_for_stream(&mut killed, &mut leave);
    send_seeds(&mut killed, &[seed(&streamed)], true);
    let added = [
        next_for_stream(&mut killed, &mut leave),
        next_for_stream(&mut killed, &mut leave),
    ];
    killed.kill();

    let mut resuming = site.engine(&env);
    let handled = RefCell::new(Vec::new());
    let mut answer = |params: &Value| {
        let url = params["crawl_request"]["url"].clone();
        handled.borrow_mut().push(url.clone());
        Some(json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": url } }] } }))
    };
    let again = start(&mut resuming, &failing);
    let resumed = resuming.call(
        "resume",
        "run.resume",
        &json!({ "run_id": RUN_ID, "settings": { "crawl": { "keep_alive": true } } }),
        &mut answer,
    );
    let value = resuming.call(
        "get",
        "key_value_store.get",
        &json!({ "run_id": RUN_ID, "key": "cursor" }),
        &mut answer,
    );
    let listed = resuming.call(
        "failed",
        "run.failed_requests",
        &json!({ "run_id": RUN_ID }),
        &mut answer,
    );
    let finished = resuming.call(
        "finish",
        "run.finish",
        &json!({ "run_id": RUN_ID }),
        &mut answer,
    );
    assert!(resuming.finished());

    let mut handled = handled.into_inner();
    handled.sort_by_key(ToString::to_string);
    assert_eq!(
        (
            &retried["result"],
            &set["result"],
            matches!(credit, Came::Credit(_)),
            matches!(&added[1], Came::Reply(reply) if reply["result"]["added"] == 1),
        ),
        (&json!({ "readmitted": 1 }), &json!({}), true, true),
        "{added:?}"
    );
    assert_eq!(
        (
            &again["result"]["started_as"],
            &resumed["result"]["started_as"],
            &value["result"]["value"],
            &listed["result"]["requests"],
            handled,
            &finished["result"]["lifecycle"],
        ),
        (
            &json!("existing"),
            &json!("resumed"),
            &json!(3),
            &json!([]),
            vec![json!(failing), json!(streamed)],
            &json!("finished"),
        )
    );
}
