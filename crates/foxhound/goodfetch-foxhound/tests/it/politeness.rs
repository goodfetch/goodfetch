//! Politeness shared on one machine, across processes (spec #213's
//! multi-process tests; ADRs 0053 and 0094: the per-domain part is
//! Foxhound's): two Attached engines of Foxhound's executable under one
//! Goodfetch home crawl one site, and the per-user Politeness file's
//! Permits hold them together under the site's in-flight cap, the waiters
//! woken in turn when a Permit comes back; a Run with
//! `politeness.sharing = run` shares nothing and is marked in its result;
//! and the Permits of an engine killed mid-request are reclaimed once its
//! Run's writer lock is free. The handlers are the test's: a request is in
//! flight from its `crawl_request.handle` until the test answers it.
use std::time::Duration;

use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::crawl::Attached;

/// How long an engine must send nothing for a request to count as held.
const QUIET: Duration = Duration::from_secs(1);

fn one_at_a_time() -> Value {
    json!({ "politeness": { "domain": { "concurrency": 1 } } })
}

/// Starts a crawl of `urls` as `run_id` under `settings`, and answers the
/// `crawl_request.handle` requests it sent before the start's reply,
/// unanswered.
fn start(engine: &mut Attached, run_id: &str, urls: &[&str], settings: &Value) -> Vec<Value> {
    let seeds: Vec<Value> = urls
        .iter()
        .map(|url| json!({ "url": url, "label": "page" }))
        .collect();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": run_id, "seeds": seeds, "settings": settings },
    }));
    let mut held = Vec::new();
    loop {
        let message = engine.next().expect("the engine answers the start");
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        } else if message["id"] == "start" {
            assert!(message.get("result").is_some(), "{message}");
            return held;
        }
    }
}

/// The URL of the next `crawl_request.handle` request `engine` sends.
fn next_request(engine: &mut Attached) -> Value {
    loop {
        let message = engine.next().expect("the engine sends a request");
        if message["method"] == "crawl_request.handle" {
            return message;
        }
    }
}

fn url(request: &Value) -> String {
    request["params"]["crawl_request"]["url"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn answer(engine: &mut Attached, request: &Value) {
    engine.send(&json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "records": [] } }));
}

/// Finishes `run_id`, its work done, and answers its Run report.
fn finish(engine: &mut Attached, run_id: &str) -> Value {
    engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut |_| Some(json!({ "result": { "records": [] } })),
    )
}

/// Two crawls of one site, in two engines, never have more requests in
/// flight on it together than its cap of one. The waiters take turns in
/// the order they began to wait: the first crawl, whose second request
/// waited from its first's admission, goes again, and the second crawl,
/// quiet meanwhile, is woken through its wake endpoint once that ends.
///
/// Covers: story:3-80, story:3-81, setting:politeness.domain.concurrency, operation:crawl.start, format:politeness-file
#[test]
fn two_engines_crawling_one_site_never_have_more_in_flight_on_it_than_its_cap() {
    let env = TestEnv::new();
    let mut first = Attached::start(&env);
    let mut second = Attached::start(&env);

    let mut held = start(
        &mut first,
        "first",
        &["https://shop.test/1", "https://shop.test/2"],
        &one_at_a_time(),
    );
    if held.is_empty() {
        held.push(next_request(&mut first));
    }
    let first_one = held.remove(0);
    let none_yet = start(
        &mut second,
        "second",
        &["https://www.shop.test/1"],
        &one_at_a_time(),
    );
    let quiet_behind_one = none_yet.is_empty() && second.is_quiet_for(QUIET);
    answer(&mut first, &first_one);
    let first_two = next_request(&mut first);
    let quiet_behind_two = second.is_quiet_for(QUIET);
    answer(&mut first, &first_two);
    let second_one = next_request(&mut second);
    answer(&mut second, &second_one);
    let reports = (
        finish(&mut first, "first")["result"]["lifecycle"].clone(),
        finish(&mut second, "second")["result"]["lifecycle"].clone(),
    );
    assert!(first.finish());
    assert!(second.finish());

    assert_eq!(
        (
            [url(&first_one), url(&first_two), url(&second_one)],
            (quiet_behind_one, quiet_behind_two),
            reports,
        ),
        (
            [
                "https://shop.test/1".to_owned(),
                "https://shop.test/2".to_owned(),
                "https://www.shop.test/1".to_owned(),
            ],
            (true, true),
            (json!("finished"), json!("finished")),
        )
    );
}

/// A crawl with `politeness.sharing = run` keeps Politeness of its own: it
/// sends its request while another engine's crawl of the site holds the
/// site's one slot, and its result lists the setting.
///
/// Covers: story:3-85, setting:politeness.sharing
#[test]
fn a_crawl_that_shares_no_politeness_goes_beside_another_and_is_marked_in_its_result() {
    let env = TestEnv::new();
    let mut first = Attached::start(&env);
    let mut second = Attached::start(&env);
    let mut held = start(
        &mut first,
        "shared",
        &["https://shop.test/1"],
        &one_at_a_time(),
    );
    if held.is_empty() {
        held.push(next_request(&mut first));
    }

    let mut own = start(
        &mut second,
        "own",
        &["https://shop.test/2"],
        &json!({ "politeness": { "sharing": "run", "domain": { "concurrency": 1 } } }),
    );
    if own.is_empty() {
        own.push(next_request(&mut second));
    }
    answer(&mut second, &own[0]);
    let report = finish(&mut second, "own");
    answer(&mut first, &held[0]);
    finish(&mut first, "shared");
    assert!(first.finish());
    assert!(second.finish());

    assert_eq!(
        (url(&own[0]), &report["result"]["weakened_by"]),
        (
            "https://shop.test/2".to_owned(),
            &json!([{ "key": "politeness.sharing", "value": "run", "layer": "run" }])
        )
    );
}

/// An engine killed with a request in flight never gives its Permit back:
/// another engine reclaims it, since the dead Run's writer lock is free.
///
/// Covers: story:3-80, setting:politeness.domain.concurrency
#[test]
fn the_permit_of_an_engine_killed_mid_request_is_reclaimed() {
    let env = TestEnv::new();
    let mut victim = Attached::start(&env);
    let held = start(
        &mut victim,
        "killed",
        &["https://shop.test/1"],
        &one_at_a_time(),
    );
    if held.is_empty() {
        next_request(&mut victim);
    }
    victim.kill();

    let mut survivor = Attached::start(&env);
    let mut taken = start(
        &mut survivor,
        "survivor",
        &["https://shop.test/2"],
        &one_at_a_time(),
    );
    if taken.is_empty() {
        taken.push(next_request(&mut survivor));
    }
    answer(&mut survivor, &taken[0]);
    let report = finish(&mut survivor, "survivor");
    assert!(survivor.finish());

    assert_eq!(
        (url(&taken[0]), &report["result"]["lifecycle"]),
        ("https://shop.test/2".to_owned(), &json!("finished"))
    );
}
