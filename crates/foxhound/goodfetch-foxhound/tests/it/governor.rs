//! The Resource governor on Foxhound's executable over stdio (spec #213's
//! Seam 1; ADR 0040): two crawls of one engine take each attempt's HTTP
//! admission from its governor, whose fixed cap holds both, and what one
//! gives back goes to the other in turn.

use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::crawl::Attached;
use crate::fetch_tiers::settings;

/// The user configuration of a home whose engines keep `in_flight` HTTP
/// admissions at once whatever the machine: each engine is held at
/// pressure (`governor.pressure` at 1 %), where its governor admits only
/// its floor, and the floor and ceiling are fixed at `in_flight`. A test
/// that kills an engine at one of its first `in_flight` handler requests,
/// none answered, needs them all in flight: a governor sized by a loaded
/// machine's memory once admitted one, and the kill waited for a second
/// request that never came.
pub(crate) fn in_flight_whatever_the_machine(in_flight: u32) -> String {
    format!(
        "governor.pressure = 1\ngovernor.http.floor = {in_flight}\ngovernor.http.ceiling = {in_flight}\n"
    )
}

/// Starts the crawl `run_id` of one request on each of `hosts`, up to 3
/// handlers at once, and answers once the engine took it, with each
/// handler request it sent meanwhile kept in `held`.
fn start(engine: &mut Attached, run_id: &str, hosts: &[&str], held: &mut Vec<Value>) {
    let seeds: Vec<Value> = hosts
        .iter()
        .map(|host| json!({ "url": format!("https://{host}/"), "label": "page" }))
        .collect();
    let id = format!("start-{run_id}");
    engine.send(&json!({
        "jsonrpc": "2.0", "id": id, "method": "crawl.start",
        "params": {
            "run_id": run_id,
            "seeds": seeds,
            "settings": { "crawl": { "handler": { "concurrency": 3 } } },
        },
    }));
    loop {
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        } else if message["id"] == id {
            assert!(message.get("result").is_some(), "{message}");
            return;
        }
    }
}

/// Every handler request the engine sends, after those already `held`
/// and with those of `unanswered` not answered yet, until the governor
/// holds the crawls still: each of `crawls`, a Run ID and its number of
/// seeds, has all its requests done, waiting or handled by a request held
/// here, none fetched, and either the governor's `cap` admissions are all
/// held here or nothing waits. Nothing more comes then until one is
/// answered; more than `cap` handled then fails at once. It reads the
/// crawls' progress (`run.progress`) rather than waiting for a quiet
/// stretch, which a loaded machine outlasts (fix-queue 129).
fn turn(
    engine: &mut Attached,
    (crawls, cap): (&[(&str, u64)], u64),
    unanswered: &[Value],
    mut held: Vec<Value>,
) -> Vec<Value> {
    loop {
        let (mut accounted, mut handled, mut waiting) = (true, 0, 0);
        for &(run_id, seeds) in crawls {
            let progress = progress(engine, run_id, &mut held);
            let mine = held
                .iter()
                .chain(unanswered)
                .filter(|request| request["params"]["run_id"] == run_id)
                .count();
            let count = |name: &str| progress["requests"][name].as_u64().unwrap_or(0);
            let done = count("succeeded") + count("failed") + count("skipped") + count("blocked");
            accounted &= count("fetching") == 0
                && count("handling") == u64::try_from(mine).unwrap()
                && done + count("handling") + count("pending") == seeds;
            handled += count("handling");
            waiting += count("pending");
        }
        if !accounted {
            continue;
        }
        // Accounted for, each request handled is one held here, which
        // keeps its admission. Past the cap, nothing would ever end the
        // turn: the governor gave more than its cap, as it once did before
        // its first sample (#1792), and the test waited for the 300 s kill.
        assert!(
            handled <= cap,
            "the governor admits {handled} past its cap of {cap}"
        );
        if handled == cap || waiting == 0 {
            return held;
        }
    }
}

/// The progress of `run_id` at once, each handler request that comes
/// meanwhile kept in `held`.
fn progress(engine: &mut Attached, run_id: &str, held: &mut Vec<Value>) -> Value {
    let id = format!("progress-{run_id}");
    engine.send(&json!({
        "jsonrpc": "2.0", "id": id, "method": "run.progress", "params": { "run_id": run_id },
    }));
    loop {
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        } else if message["id"] == id {
            return message["result"].clone();
        }
    }
}

/// The Runs `held`'s requests are of, in order.
fn runs(held: &[Value]) -> Vec<String> {
    let mut runs: Vec<String> = held
        .iter()
        .map(|request| request["params"]["run_id"].as_str().unwrap().to_owned())
        .collect();
    runs.sort();
    runs
}

fn answer(engine: &mut Attached, held: Vec<Value>) {
    for request in held {
        engine.send(&json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "records": [] } }));
    }
}

/// With `governor.http` fixed at 2, the crawl that started first takes
/// both admissions though it could run 3 handlers; the second waits; once
/// the first gives them back, they go to each crawl in turn, and once the
/// first is done, both to the second.
///
/// Covers: story:3-55
#[test]
fn two_crawls_take_the_governors_http_admissions_in_turn() {
    let env = TestEnv::new();
    env.user_configuration("governor.http.floor = 2\ngovernor.http.ceiling = 2\n");
    let mut engine = Attached::start(&env);

    let mut alone = Vec::new();
    start(
        &mut engine,
        "first",
        &["a.test", "b.test", "c.test"],
        &mut alone,
    );
    let alone = turn(&mut engine, (&[("first", 3)], 2), &[], alone);
    let mut waiting = Vec::new();
    start(
        &mut engine,
        "second",
        &["d.test", "e.test", "f.test"],
        &mut waiting,
    );
    let waiting = turn(
        &mut engine,
        (&[("first", 3), ("second", 3)], 2),
        &alone,
        waiting,
    );
    let mut turns = vec![runs(&alone), runs(&waiting)];
    answer(&mut engine, alone);
    for _ in 0..3 {
        let held = turn(
            &mut engine,
            (&[("first", 3), ("second", 3)], 2),
            &[],
            Vec::new(),
        );
        turns.push(runs(&held));
        answer(&mut engine, held);
    }
    for run_id in ["first", "second"] {
        engine.send(&json!({ "jsonrpc": "2.0", "id": run_id, "method": "run.finish", "params": { "run_id": run_id } }));
    }
    assert!(engine.finish());

    assert_eq!(
        turns,
        [
            vec!["first".to_owned(), "first".to_owned()],
            Vec::new(),
            vec!["first".to_owned(), "second".to_owned()],
            vec!["second".to_owned(), "second".to_owned()],
            Vec::new(),
        ]
    );
}

/// Follows the progress of `run_id` from its first value to the first that
/// holds a wait group of `reason`, and answers that group.
fn wait_group(engine: &mut Attached, run_id: &str, reason: &str) -> Value {
    let mut params = json!({ "run_id": run_id });
    loop {
        engine.send(
            &json!({ "jsonrpc": "2.0", "id": "progress", "method": "run.progress", "params": params }),
        );
        let progress = loop {
            let message = engine.next().unwrap();
            if message["id"] == "progress" {
                break message["result"].clone();
            }
        };
        if let Some(group) = progress["waiting"]
            .as_array()
            .and_then(|groups| groups.iter().find(|group| group["reason"] == reason))
        {
            return group.clone();
        }
        assert_eq!(progress["last"], false, "the crawl ended: {progress}");
        params["after"] = progress["position"].clone();
    }
}

/// A crawl whose governor admits nothing more, `governor.http` fixed at 1
/// with one request in flight, says that its due request waits under
/// `governor`.
///
/// Covers: story:3-162, operation:run.progress
#[test]
fn a_crawl_the_governor_admits_nothing_more_for_says_its_due_requests_wait_under_governor() {
    let env = TestEnv::new();
    env.user_configuration("governor.http.floor = 1\ngovernor.http.ceiling = 1\n");
    let mut engine = Attached::start(&env);

    let mut held = Vec::new();
    start(&mut engine, "held", &["a.test", "b.test"], &mut held);
    let held = turn(&mut engine, (&[("held", 2)], 1), &[], held);
    let group = wait_group(&mut engine, "held", "governor");
    answer(&mut engine, held);
    let _ = turn(&mut engine, (&[("held", 2)], 1), &[], Vec::new());
    assert!(engine.finish());

    assert_eq!(
        group,
        json!({ "reason": "governor", "count": 1, "domains": [{ "domain": "b.test" }] })
    );
}

/// With `governor.http` fixed at 1, a crawl on the `http` tier hands its
/// two requests, on two domains, to its handler at once: each attempt
/// gives its HTTP admission back once its document is fetched, so a page
/// being analysed or handled holds no place among the requests in flight
/// (ADRs 0004 and 0138), and the second request is fetched meanwhile.
///
/// Covers: setting:fetch.tier, operation:crawl.start, operation:run.progress
#[test]
fn a_fetched_page_holds_no_http_admission_while_it_is_handled() {
    let env = TestEnv::new();
    env.user_configuration(&in_flight_whatever_the_machine(1));
    let proving_ground = ProvingGround::start(&env, 1905).unwrap();
    let mut engine = Attached::start(&env);
    let seeds = [
        proving_ground.url("extraction.test", "/values"),
        proving_ground.url("catalog.test", "/catalog/1"),
    ];
    let mut held = Vec::new();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": {
            "run_id": "analysed",
            "seeds": seeds.iter().map(|url| json!({ "url": url, "label": "page" })).collect::<Vec<_>>(),
            "settings": settings(&proving_ground, &json!({
                "fetch": { "tier": "http" },
                "crawl": { "handler": { "concurrency": 2 } },
            })),
        },
    }));
    loop {
        let message = engine.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        } else if message["id"] == "start" {
            assert!(message.get("result").is_some(), "{message}");
            break;
        }
    }

    // Both handler requests come, neither answered: the first page gave
    // its admission back once its document was fetched, and the second was
    // fetched meanwhile. Held until the first's answer, the admission
    // would keep the second from ever coming, and nextest's kill would end
    // the test. The verdict reads the requests themselves, not
    // `run.progress`: its `waiting` is the delivery's view at its last
    // admission, which still shows the second under `governor` for a
    // moment after the admission came back (4 of 20 runs, #1905's flake).
    while held.len() < 2 {
        let message = engine
            .next()
            .expect("the engine closed before the second handler request");
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        }
    }
    answer(&mut engine, held);
    let _ = turn(&mut engine, (&[("analysed", 2)], 2), &[], Vec::new());
    assert!(engine.finish());
}
