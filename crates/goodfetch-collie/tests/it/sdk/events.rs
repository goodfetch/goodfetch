//! A Rust program's page events read by meaning (issue 2933, ADR 0081), on
//! the linked engine, which launches the browser of the Browser cache in
//! the test's Goodfetch home: the
//! `listeners-in-order-and-a-thrower-raises-listener-failed` client
//! conformance case in Rust's `linked` mode. Rust reads its own
//! `EventStream`, so no listener of its throws and the case's
//! `listener_failed` has no Rust form; the client crate's `events` tests
//! cover the stream's credits and its cancel on the wire.

use goodfetch_collie::{Collie, blocking, events};
use goodfetch_test_support::{TestEnv, browser_cache_in_home, real_time_runtime};
use serde_json::json;

use super::options;

/// The `listeners-in-order-and-a-thrower-raises-listener-failed` case,
/// linked: `page.events(events::Console)` hands the page's console
/// messages in order, and once the stream is dropped the page stays open
/// and answers the call after it.
/// Covers: story:11-61, story:11-62, story:11-63
#[test]
fn page_events_hands_the_console_in_order_and_a_dropped_stream_leaves_the_page_open() {
    let env = TestEnv::new();
    browser_cache_in_home(&env).unwrap();
    let (texts, after) = real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();
        let page = run.new_page().await.unwrap();

        let mut console = page.events(events::Console);
        page.evaluate("() => { console.log('first'); console.log('second'); }")
            .await
            .unwrap();
        let mut texts = Vec::new();
        for _ in 0..2 {
            texts.push(console.next().await.unwrap().unwrap().text);
        }
        drop(console);
        let after = page.evaluate("() => 1 + 1").await.unwrap();
        run.finish().await.unwrap();
        collie.close().await.unwrap();
        (texts, after)
    });

    assert_eq!(texts, ["first", "second"]);
    assert_eq!(after, Some(json!(2)));
}

/// The same case through `blocking`: the page's `events` iterates the same
/// console messages, in order.
/// Covers: story:11-61, story:11-62, story:11-63
#[test]
fn page_events_iterates_the_console_in_order_through_blocking() {
    let env = TestEnv::new();
    browser_cache_in_home(&env).unwrap();
    let collie = blocking::Collie::connect(options(&env)).unwrap();
    let run = collie.start().send().unwrap();
    let page = run.new_page().send().unwrap();

    let console = page.events(events::Console);
    page.evaluate("() => { console.log('first'); console.log('second'); }")
        .send()
        .unwrap();
    let texts: Vec<String> = console
        .take(2)
        .map(|message| message.unwrap().text)
        .collect();
    let after = page.evaluate("() => 1 + 1").send().unwrap();
    run.finish().send().unwrap();
    collie.close().unwrap();

    assert_eq!(texts, ["first", "second"]);
    assert_eq!(after, Some(json!(2)));
}
