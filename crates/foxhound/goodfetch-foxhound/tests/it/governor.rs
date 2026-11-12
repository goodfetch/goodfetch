//! The Resource governor on Foxhound's executable over stdio (spec #213's
//! Seam 1; ADR 0040): two crawls of one engine take each attempt's HTTP
//! admission from its governor, whose fixed cap holds both, and what one
//! gives back goes to the other in turn.

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::settings;
use crate::site::Site;

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
/// handlers at once, and answers once the engine took it. Each request is
/// for its host's held page, `/held/<host>`, which the site sends only
/// once [`release`] asked for it: until then its attempt is in flight and
/// keeps its HTTP admission.
fn start(engine: &mut Attached, site: &Site, run_id: &str, hosts: &[&str]) {
    let seeds: Vec<Value> = hosts
        .iter()
        .map(|host| json!({ "url": site.url(host, &format!("/held/{host}")), "label": "page" }))
        .collect();
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": run_id,
            "seeds": seeds,
            "settings": { "crawl": { "handler": { "concurrency": 3 } } },
        }),
        &mut |_| Some(json!({ "result": { "records": [] } })),
    );
    assert!(started.get("result").is_some(), "{started}");
}

/// The progress of `run_id` at once, each handler request that comes
/// meanwhile answered with no Record.
fn progress(engine: &mut Attached, run_id: &str) -> Value {
    engine.call(
        "progress",
        "run.progress",
        &json!({ "run_id": run_id }),
        &mut |_| Some(json!({ "result": { "records": [] } })),
    )["result"]
        .clone()
}

/// The requests in flight, each as its Run ID and its URL, once the
/// governor holds the crawls still: `done` requests are done over all of
/// `crawls`, each a Run ID and its number of seeds, every other is being
/// fetched or waits, none is with its handler, and either the governor's
/// `cap` admissions are all taken or nothing waits. Nothing more moves
/// then until a held page is released; more than `cap` in flight fails at
/// once. It reads the crawls' progress (`run.progress`) rather than
/// waiting for a quiet stretch, which a loaded machine outlasts (fix-queue
/// 129).
fn in_flight(
    engine: &mut Attached,
    (crawls, cap): (&[(&str, u64)], u64),
    done: u64,
) -> Vec<(String, String)> {
    loop {
        let (mut accounted, mut finished, mut waiting) = (true, 0, 0);
        let mut flying = Vec::new();
        for &(run_id, seeds) in crawls {
            let progress = progress(engine, run_id);
            let count = |name: &str| progress["requests"][name].as_u64().unwrap_or(0);
            let ended = count("succeeded") + count("failed") + count("skipped") + count("blocked");
            let entries = progress["in_flight"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            accounted &= count("handling") == 0
                && count("fetching") == u64::try_from(entries.len()).unwrap()
                && ended + count("fetching") + count("pending") == seeds;
            finished += ended;
            waiting += count("pending");
            flying.extend(entries.iter().map(|entry| {
                (
                    run_id.to_owned(),
                    entry["url"].as_str().unwrap_or_default().to_owned(),
                )
            }));
        }
        if !accounted || finished != done {
            continue;
        }
        // Past the cap, nothing would ever end the turn: the governor gave
        // more than its cap, as it once did before its first sample
        // (#1792), and the test waited for the 300 s kill.
        let taken = u64::try_from(flying.len()).unwrap();
        assert!(
            taken <= cap,
            "the governor admits {taken} past its cap of {cap}"
        );
        if taken == cap || waiting == 0 {
            return flying;
        }
    }
}

/// Has the site send the held page of each request of `flying`.
fn release(site: &Site, flying: &[(String, String)]) {
    for (_, url) in flying {
        let (_, key) = url.rsplit_once("/held/").expect("a held page");
        let released = site.fetch(&site.url("shop.test", &format!("/release/{key}")));
        assert_eq!(released.status, 200, "{url}");
    }
}

/// The Runs `flying`'s requests are of, in order.
fn runs(flying: &[(String, String)]) -> Vec<String> {
    let mut runs: Vec<String> = flying.iter().map(|(run_id, _)| run_id.clone()).collect();
    runs.sort();
    runs
}

fn answer(engine: &mut Attached, held: Vec<Value>) {
    for request in held {
        engine.send(&json!({ "jsonrpc": "2.0", "id": request["id"], "result": { "records": [] } }));
    }
}

/// With `governor.http` fixed at 2, the crawl that started first takes
/// both admissions though it could fetch 3 pages at once; the second
/// waits; once the first gives them back, they go to each crawl in turn,
/// and once the first is done, both to the second.
///
/// Covers: story:3-55
#[test]
fn two_crawls_take_the_governors_http_admissions_in_turn() {
    let env = TestEnv::new();
    env.user_configuration("governor.http.floor = 2\ngovernor.http.ceiling = 2\n");
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let crawls: &[(&str, u64)] = &[("first", 3), ("second", 3)];

    start(
        &mut engine,
        &site,
        "first",
        &["a.test", "b.test", "api.test"],
    );
    let alone = in_flight(&mut engine, (&crawls[..1], 2), 0);
    start(
        &mut engine,
        &site,
        "second",
        &["other.test", "many.test", "capped.test"],
    );
    let mut flying = in_flight(&mut engine, (crawls, 2), 0);
    let mut turns = vec![runs(&alone), runs(&flying)];
    let mut done = 0;
    while !flying.is_empty() {
        release(&site, &flying);
        done += u64::try_from(flying.len()).unwrap();
        flying = in_flight(&mut engine, (crawls, 2), done);
        turns.push(runs(&flying));
    }
    for run_id in ["first", "second"] {
        engine.send(&json!({ "jsonrpc": "2.0", "id": run_id, "method": "run.finish", "params": { "run_id": run_id } }));
    }
    assert!(engine.finished());

    assert_eq!(
        turns,
        [
            vec!["first".to_owned(), "first".to_owned()],
            vec!["first".to_owned(), "first".to_owned()],
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
    let site = Site::start(&env);
    let mut engine = site.engine(&env);
    let crawl: &[(&str, u64)] = &[("held", 2)];

    start(&mut engine, &site, "held", &["a.test", "b.test"]);
    let first = in_flight(&mut engine, (crawl, 1), 0);
    let group = wait_group(&mut engine, "held", "governor");
    release(&site, &first);
    let second = in_flight(&mut engine, (crawl, 1), 1);
    release(&site, &second);
    let last = in_flight(&mut engine, (crawl, 1), 2);
    assert!(engine.finished());

    // Whichever request took the one admission, the other waits: the
    // page in flight and the domain that waits are the crawl's two hosts.
    let mut hosts = vec![
        first[0]
            .1
            .rsplit_once("/held/")
            .map(|(_, host)| host.to_owned()),
        group["domains"][0]["domain"].as_str().map(str::to_owned),
    ];
    hosts.sort();
    assert_eq!(
        (
            &group["reason"],
            &group["count"],
            group["domains"].as_array().map(Vec::len),
            hosts,
            second.len(),
            last.len()
        ),
        (
            &json!("governor"),
            &json!(1),
            Some(1),
            vec![Some("a.test".to_owned()), Some("b.test".to_owned())],
            1,
            0
        )
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
    let mut engine = crate::site::engine_on(&env, &proving_ground);
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
    // would keep the second from ever coming, and the engine's silence
    // would fail the read at its 10 s deadline. The verdict reads the requests themselves, not
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
    let flying = in_flight(&mut engine, (&[("analysed", 2)], 2), 2);
    assert!(engine.finished());
    assert!(flying.is_empty(), "{flying:?}");
}
