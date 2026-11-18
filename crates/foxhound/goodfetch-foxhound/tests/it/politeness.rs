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
use goodfetch_test_client::Attached;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::site::Site;

fn one_at_a_time() -> Value {
    json!({ "politeness": { "domain": { "concurrency": 1 } } })
}

/// Starts a crawl of `urls` as `run_id` under `settings`, and answers the
/// `crawl_request.handle` requests it sent before the start's reply,
/// unanswered.
fn start(engine: &mut Attached, run_id: &str, urls: &[String], settings: &Value) -> Vec<Value> {
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

/// The first of `held`, the requests `engine` sent that are not taken
/// yet, or else the next it sends.
fn taken(engine: &mut Attached, held: &mut Vec<Value>) -> Value {
    if held.is_empty() {
        next_request(engine)
    } else {
        held.remove(0)
    }
}

/// Follows the progress (`run.progress`) of `run_id` on `engine` until its
/// due request settles: it waits for the site under `domain_concurrency`,
/// the Politeness file having refused its Permit and listed it as a waiter
/// (ADR 0053), or nothing is pending any more, each handler request that
/// comes meanwhile kept in `held`. Answers how many requests that value
/// shows waiting for the site, and how many handled.
fn settled(engine: &mut Attached, run_id: &str, held: &mut Vec<Value>) -> (u64, u64) {
    settled_under(engine, run_id, held, &["domain_concurrency"])
}

/// Follows the progress of `run_id` on `engine` as [`settled`] does, until
/// its due request has waited under each of `reasons` in turn, the last
/// with nothing being fetched, or nothing is pending any more. Answers how
/// many requests the last value shows waiting under the last reason, and
/// how many handled.
fn settled_under(
    engine: &mut Attached,
    run_id: &str,
    held: &mut Vec<Value>,
    reasons: &[&str],
) -> (u64, u64) {
    let mut params = json!({ "run_id": run_id });
    let mut seen = 0;
    loop {
        engine.send(&json!({
            "jsonrpc": "2.0", "id": "progress", "method": "run.progress", "params": params,
        }));
        let progress = loop {
            let message = engine.next().expect("the engine answers its progress");
            if message["method"] == "crawl_request.handle" {
                held.push(message);
            } else if message["id"] == "progress" {
                break message["result"].clone();
            }
        };
        let count = |reason: &str| {
            progress["waiting"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|group| group["reason"] == reason)
                .and_then(|group| group["count"].as_u64())
        };
        while seen + 1 < reasons.len() && count(reasons[seen]).is_some() {
            seen += 1;
        }
        let last = reasons.last().copied().unwrap_or_default();
        let waiting = count(last).filter(|_| seen + 1 == reasons.len());
        let requests = &progress["requests"];
        // A request still being fetched is not handled yet: the count of
        // those handled is read once none is.
        let settled = waiting.is_some() && requests["fetching"] == 0;
        if settled || requests["pending"] == 0 || progress["last"] == true {
            return (
                waiting.unwrap_or(0),
                requests["handling"].as_u64().unwrap_or(0),
            );
        }
        params["after"] = progress["position"].clone();
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

/// Aborts `run_id`, and answers its Run report.
fn abort(engine: &mut Attached, run_id: &str) -> Value {
    engine.call(
        "abort",
        "run.abort",
        &json!({ "run_id": run_id }),
        &mut |_| Some(json!({ "result": { "records": [] } })),
    )
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
/// waiting meanwhile, is woken through its wake endpoint once that ends.
/// Each crawl's progress says when its request waits for the site.
///
/// Covers: story:3-80, story:3-81, setting:politeness.domain.concurrency, operation:crawl.start, format:politeness-file
#[test]
fn two_engines_crawling_one_site_never_have_more_in_flight_on_it_than_its_cap() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    // The first crawl's second request waits for the site from its first's
    // admission only if the governor admits both at once: a governor under
    // pressure lowers its HTTP target towards its floor, one by default
    // (ADR 0040), and the second request would wait for the governor.
    env.user_configuration("governor.http.floor = 2\ngovernor.http.ceiling = 2\n");
    let mut first = site.engine(&env);
    let mut second = site.engine(&env);

    let mut held = start(
        &mut first,
        "first",
        &[site.page("/1"), site.page("/2")],
        &one_at_a_time(),
    );
    let first_waits = settled(&mut first, "first", &mut held);
    let first_one = taken(&mut first, &mut held);
    let mut none_yet = start(
        &mut second,
        "second",
        &[site.url("www.shop.test", "/1")],
        &one_at_a_time(),
    );
    let behind_one = settled(&mut second, "second", &mut none_yet);
    answer(&mut first, &first_one);
    let first_two = taken(&mut first, &mut held);
    let behind_two = settled(&mut second, "second", &mut none_yet);
    let sent_early = none_yet.len();
    answer(&mut first, &first_two);
    let second_one = taken(&mut second, &mut none_yet);
    answer(&mut second, &second_one);
    let reports = (
        finish(&mut first, "first")["result"]["lifecycle"].clone(),
        finish(&mut second, "second")["result"]["lifecycle"].clone(),
    );
    assert!(first.finished());
    assert!(second.finished());

    assert_eq!(
        (
            [url(&first_one), url(&first_two), url(&second_one)],
            (first_waits, behind_one, behind_two, sent_early),
            reports,
        ),
        (
            [
                site.page("/1"),
                site.page("/2"),
                site.url("www.shop.test", "/1"),
            ],
            ((1, 1), (1, 0), (1, 0), 0),
            (json!("finished"), json!("finished")),
        )
    );
}

/// Two crawls of one site, in two engines, keep its delay together: the
/// first engine sends its page and holds the site's Delegation, and the
/// second crawl's request, which recalls it (ADR 0142), waits under
/// `domain_delay` for the hour since that send, none sent. The crawls
/// fetch no robots.txt, whose request would start the hour before the
/// first page's.
///
/// Covers: story:3-80, setting:politeness.domain.delay, setting:politeness.delegation, format:politeness-file
#[test]
fn two_engines_crawling_one_site_keep_its_delay_together() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let an_hour_apart = json!({
        "politeness": { "domain": { "delay": 3_600_000 } },
        "site_preferences": { "respect_robots_txt": false },
    });
    let mut first = site.engine(&env);
    let mut second = site.engine(&env);

    let mut held = start(&mut first, "first", &[site.page("/1")], &an_hour_apart);
    let sent = taken(&mut first, &mut held);
    answer(&mut first, &sent);
    let mut none_yet = start(&mut second, "second", &[site.page("/2")], &an_hour_apart);
    let behind = settled_under(&mut second, "second", &mut none_yet, &["domain_delay"]);
    let sent_early = none_yet.len();
    let reports = (
        finish(&mut first, "first")["result"]["lifecycle"].clone(),
        abort(&mut second, "second")["result"]["lifecycle"].clone(),
    );
    assert!(first.finished());
    assert!(second.finished());

    assert_eq!(
        (url(&sent), behind, sent_early, reports),
        (
            site.page("/1"),
            (1, 0),
            0,
            (json!("finished"), json!("stopped"))
        )
    );
}

/// Two crawls of one site, in two engines, keep its 429's halving
/// together: with the first crawl's four requests in flight, the one the
/// site answers 429 halves the site's concurrency from four to two (ADR
/// 0021) and backs it off. The second crawl's request waits under
/// `backoff`, then, the backoff over, under `domain_concurrency` while the
/// first's three others are in flight, which the cap of four would let it
/// pass; it is sent once they ended.
///
/// Covers: story:3-80, setting:politeness.domain.concurrency, setting:politeness.backoff.initial, setting:politeness.delegation, format:politeness-file
#[test]
fn two_engines_crawling_one_site_keep_its_429_halving_together() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    env.user_configuration("governor.http.floor = 4\ngovernor.http.ceiling = 4\n");
    let four = json!({
        "crawl": { "handler": { "concurrency": 4 } },
        "politeness": {
            "domain": { "concurrency": 4 },
            "backoff": { "initial": 1_000, "max": 1_000 },
        },
    });
    let quiet = |mark: &str| site.url("scope.test", &format!("/quiet?mark={mark}"));
    let refused = site.url("scope.test", "/quiet?mark=refused&status=429");
    let mut first = site.engine(&env);
    let mut second = site.engine(&env);

    let seeds = [quiet("1"), quiet("2"), quiet("3"), refused.clone()];
    let mut held = start(&mut first, "first", &seeds, &four);
    let mut in_flight: Vec<Value> = (0..4).map(|_| taken(&mut first, &mut held)).collect();
    let at = in_flight
        .iter()
        .position(|request| url(request) == refused)
        .expect("the page answered 429 is in flight");
    answer(&mut first, &in_flight.remove(at));
    let mut none_yet = start(&mut second, "second", &[quiet("second")], &four);
    let behind = settled_under(
        &mut second,
        "second",
        &mut none_yet,
        &["backoff", "domain_concurrency"],
    );
    let sent_early = none_yet.len();
    for request in &in_flight {
        answer(&mut first, request);
    }
    let second_one = taken(&mut second, &mut none_yet);
    answer(&mut second, &second_one);
    let reports = (
        finish(&mut first, "first")["result"]["lifecycle"].clone(),
        finish(&mut second, "second")["result"]["lifecycle"].clone(),
    );
    assert!(first.finished());
    assert!(second.finished());

    assert_eq!(
        (behind, sent_early, url(&second_one), reports),
        (
            (1, 0),
            0,
            quiet("second"),
            (json!("finished"), json!("finished"))
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
    let site = Site::start(&env);
    let mut first = site.engine(&env);
    let mut second = site.engine(&env);
    let mut held = start(&mut first, "shared", &[site.page("/1")], &one_at_a_time());
    if held.is_empty() {
        held.push(next_request(&mut first));
    }

    let mut own = start(
        &mut second,
        "own",
        &[site.page("/2")],
        &json!({ "politeness": { "sharing": "run", "domain": { "concurrency": 1 } } }),
    );
    if own.is_empty() {
        own.push(next_request(&mut second));
    }
    answer(&mut second, &own[0]);
    let report = finish(&mut second, "own");
    answer(&mut first, &held[0]);
    finish(&mut first, "shared");
    assert!(first.finished());
    assert!(second.finished());

    // Beside what reaching the test's site weakens.
    let sharing: Vec<&Value> = report["result"]["weakened"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|weakened| weakened["key"] == "politeness.sharing")
        .collect();
    assert_eq!(
        (url(&own[0]), sharing),
        (
            site.page("/2"),
            vec![
                &json!({ "key": "politeness.sharing", "value": "run", "layer": "run", "origin": "settings" })
            ]
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
    let site = Site::start(&env);
    let mut victim = site.engine(&env);
    let held = start(&mut victim, "killed", &[site.page("/1")], &one_at_a_time());
    if held.is_empty() {
        next_request(&mut victim);
    }
    victim.kill();

    let mut survivor = site.engine(&env);
    let mut taken = start(
        &mut survivor,
        "survivor",
        &[site.page("/2")],
        &one_at_a_time(),
    );
    if taken.is_empty() {
        taken.push(next_request(&mut survivor));
    }
    answer(&mut survivor, &taken[0]);
    let report = finish(&mut survivor, "survivor");
    assert!(survivor.finished());

    assert_eq!(
        (url(&taken[0]), &report["result"]["lifecycle"]),
        (site.page("/2"), &json!("finished"))
    );
}
