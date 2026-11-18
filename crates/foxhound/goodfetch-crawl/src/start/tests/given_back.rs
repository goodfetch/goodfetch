//! A Crawl request the program gives back (ADRs 0093 and 0102), through
//! the engine's public interface over the in-memory network: its reply
//! `call_cancelled` by the program returns it to pending with no attempt
//! counted, and the Run goes on and delivers it again.

use std::collections::BTreeSet;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};

use super::{
    Reply, answer_all, crawl, delivered, dependencies, records, run_file, seeds, work_items,
};

/// The reply the program gives a Crawl request it drops without handling
/// it: `call_cancelled` by the program, nothing closed under it.
fn given_back() -> Reply {
    Reply::FailsWith("call_cancelled", false, json!({ "by": "program" }))
}

#[test]
fn a_crawl_request_the_program_gives_back_is_delivered_again_in_the_same_run_uncounted() {
    let env = TestEnv::new();

    let (handled, finished) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut first = true;
            let mut handler = |params: &Value| {
                if std::mem::take(&mut first) {
                    given_back()
                } else {
                    answer_all(params)
                }
            };
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "given", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut handler,
                )
                .await;
            let finished = client
                .call("run.finish", json!({ "run_id": "given" }), &mut handler)
                .await;
            (delivered(client), finished)
        },
    );

    let file = run_file(&env, "given");
    assert_eq!(
        (
            handled,
            finished["result"]["completeness"].clone(),
            records(&file).len(),
            work_items(&file),
        ),
        (
            vec![
                ("https://shop.test/page/1".to_owned(), 1),
                ("https://shop.test/page/1".to_owned(), 1),
            ],
            json!("complete"),
            1,
            vec![("1".to_owned(), "succeeded".to_owned(), 1, None)],
        )
    );
}

/// The program gives back every Crawl request leased to it, as dropping
/// `serve` does, until the Run is paused, so each lies given back at the
/// pause; the Run resumed delivers each again with its attempt count
/// unchanged.
///
/// Covers: operation:run.resume
#[test]
fn every_crawl_request_given_back_then_resumed_keeps_its_attempt_count() {
    let env = TestEnv::new();

    let (before, after, finished) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut giving_back = |_: &Value| given_back();
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "stopped", "seeds": seeds(2), "labels": { "page": {} } }),
                    &mut giving_back,
                )
                .await;
            // Until each page was given back and delivered again.
            while delivered(client).len() < 4
                || delivered(client)
                    .iter()
                    .map(|(page, _)| page)
                    .collect::<BTreeSet<_>>()
                    .len()
                    < 2
            {
                client
                    .call(
                        "run.progress",
                        json!({ "run_id": "stopped" }),
                        &mut giving_back,
                    )
                    .await;
            }
            client
                .call(
                    "run.pause",
                    json!({ "run_id": "stopped" }),
                    &mut giving_back,
                )
                .await;
            let before = delivered(client);
            client.handled.clear();
            client
                .call(
                    "run.resume",
                    json!({ "run_id": "stopped" }),
                    &mut answer_all,
                )
                .await;
            let finished = client
                .call(
                    "run.finish",
                    json!({ "run_id": "stopped" }),
                    &mut answer_all,
                )
                .await;
            let mut after = delivered(client);
            after.sort();
            (before, after, finished)
        },
    );

    let attempts: BTreeSet<u64> = before.iter().map(|(_, attempt)| *attempt).collect();
    assert_eq!(
        (
            attempts,
            after,
            finished["result"]["completeness"].clone(),
            work_items(&run_file(&env, "stopped")),
        ),
        (
            BTreeSet::from([1]),
            vec![
                ("https://shop.test/page/1".to_owned(), 1),
                ("https://shop.test/page/2".to_owned(), 1),
            ],
            json!("complete"),
            vec![
                ("1".to_owned(), "succeeded".to_owned(), 1, None),
                ("2".to_owned(), "succeeded".to_owned(), 1, None),
            ],
        )
    );
}

/// A `call_cancelled` the program did not give back its Crawl request
/// with, the browser stopping the call or a page closed under it, ends the
/// attempt a counted handler failure.
///
/// Covers: error:call_cancelled
#[rstest]
#[case::by_browser(json!({ "by": "browser" }))]
#[case::page_closed(json!({ "by": "program", "closed": "page" }))]
fn a_call_cancelled_that_is_no_give_back_counts_its_attempt(#[case] details: Value) {
    let env = TestEnv::new();

    crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut handler =
                |_: &Value| Reply::FailsWith("call_cancelled", false, details.clone());
            client
                .call(
                    "crawl.start",
                    json!({ "run_id": "cancelled", "seeds": seeds(1), "labels": { "page": {} } }),
                    &mut handler,
                )
                .await;
            client
                .call("run.finish", json!({ "run_id": "cancelled" }), &mut handler)
                .await
        },
    );

    assert_eq!(
        work_items(&run_file(&env, "cancelled")),
        vec![(
            "1".to_owned(),
            "failed".to_owned(),
            1,
            Some("call_cancelled".to_owned())
        )],
    );
}
