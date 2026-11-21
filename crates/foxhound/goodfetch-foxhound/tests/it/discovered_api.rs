//! The discovered-API tier (spec #579; ADR 0020; pitfall B09), on
//! Foxhound's executable against the Proving ground's infinite lists and
//! Chrome for Testing: under `fetch.tier: auto`, a sample's browser run
//! captures the JSON calls its page makes, and a run on `discovered_api`
//! replays them for the same request, its handler's
//! `page.wait_for_response` and `page.wait_for_request_finished` answered
//! from the replayed calls; a Label whose
//! samples matched there is learned onto `discovered_api`, whose later
//! requests replay the plan for their own lists, by page number or by
//! cursor, at a fraction of the browser's requests. A list whose call
//! carries a token the site takes once stays on the browser.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::ObservedRequest;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, provenance, settings, tiers};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 770;

const HOST: &str = "tiers.test";

/// The lists of each crawl: the first three are its Label's samples.
const LISTS: u64 = 6;

/// The waits a handler arms before it reloads its page, by list kind: the
/// first two calls of its list, as URL patterns.
fn waits(kind: &str) -> [String; 2] {
    match kind {
        "scroll" | "guarded" => [
            format!(r"/api/{kind}/\d+\?page=1$"),
            format!(r"/api/{kind}/\d+\?page=2$"),
        ],
        _ => [
            format!(r"/api/{kind}/\d+(\?token=\d+)?$"),
            format!(r"/api/{kind}/\d+\?after="),
        ],
    }
}

/// A handler that reads its list's first two pages from the calls its
/// page makes: it arms a `page.wait_for_request_finished` for each, whose
/// bodies the engine keeps, and a `page.wait_for_response` for the first,
/// reloads the page, reads each finished call's body and pushes one Record
/// of its items and the first response's URL and status. A call its tier
/// cannot serve leaves it with nothing to push; a call answered without
/// items adds none.
fn read_the_list(kind: &'static str) -> impl FnMut(&mut Inside<'_>, &Value) -> Value {
    move |inside, handle| {
        let page_id = handle["page_id"].clone();
        let engine = inside.engine();
        let patterns = if kind == "signed" {
            vec![waits(kind)[0].clone()]
        } else {
            waits(kind).to_vec()
        };
        let mut armed = vec![(
            "response".to_owned(),
            "page.wait_for_response",
            patterns[0].clone(),
        )];
        for (index, pattern) in patterns.iter().enumerate() {
            armed.push((
                format!("finished-{index}"),
                "page.wait_for_request_finished",
                pattern.clone(),
            ));
        }
        for (id, method, pattern) in &armed {
            engine.send(&json!({
                "jsonrpc": "2.0", "id": id, "method": method,
                "params": {
                    "page_id": page_id,
                    "url": { "kind": "regex", "pattern": pattern },
                    "timeout": 15_000,
                },
            }));
        }
        engine.send(&json!({
            "jsonrpc": "2.0", "id": "reload", "method": "page.reload",
            "params": { "page_id": page_id },
        }));
        let mut replies = std::collections::BTreeMap::new();
        while replies.len() < armed.len() + 1 {
            let reply = engine.next().unwrap();
            let id = reply["id"].as_str().unwrap_or_default().to_owned();
            replies.insert(id, reply);
        }
        let Some(first) = replies["response"]
            .get("result")
            .map(|result| result["response"].clone())
        else {
            return json!({ "result": { "records": [] } });
        };
        let mut items = Vec::new();
        for (id, _, _) in armed.iter().skip(1) {
            let Some(response) = replies[id].get("result").map(|result| &result["response"]) else {
                return json!({ "result": { "records": [] } });
            };
            let read = inside.call(
                "response.body",
                &json!({ "page_id": page_id, "request_id": response["request_id"] }),
            );
            let path = read["result"]["path"]
                .as_str()
                .unwrap_or_else(|| panic!("{} {read}", handle["tier"]));
            let body: Value = serde_json::from_slice(&body_of(path)).unwrap();
            items.extend(body["items"].as_array().into_iter().flatten().cloned());
        }
        let reply = inside.call(
            "crawl_request.push",
            &json!({
                "run_id": handle["run_id"],
                "crawl_request_id": handle["crawl_request"]["id"],
                "lease_epoch": handle["lease_epoch"],
                "dataset": "pages",
                "records": [{
                    "list": handle["crawl_request"]["url"],
                    "items": items,
                    "first": [first["url"], first["status"]],
                }],
            }),
        );
        assert!(reply.get("result").is_some(), "{reply}");
        json!({ "result": { "records": [] } })
    }
}

/// The bytes of the body file at `path`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's body file from outside the engine, on no async worker"
)]
fn body_of(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// The Records the Run `run_id` committed, sorted.
fn committed(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let connection = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    let mut statement = connection.prepare("SELECT data FROM records").unwrap();
    let mut records: Vec<Value> = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|data| serde_json::from_str(&data.unwrap()).unwrap())
        .collect();
    records.sort_by_key(ToString::to_string);
    records
}

/// Each `fetch_tier` Provenance entry of `run_id`, as `<tier> <chosen>`.
fn chosen(env: &TestEnv, run_id: &str) -> Vec<String> {
    tiers(env, run_id)
        .iter()
        .map(|entry| {
            format!(
                "{} {}",
                entry[1].as_str().unwrap(),
                entry[2].as_str().unwrap()
            )
        })
        .collect()
}

/// How many requests the Proving ground received for the lists after the
/// samples, `/<kind>/<n>` and `/api/<kind>/<n>` for `n` past 3, since the
/// `skip` first it received.
fn later_requests(proving_ground: &ProvingGround, kind: &str, skip: usize) -> (usize, usize) {
    let received = proving_ground.received_requests().unwrap();
    let all: Vec<&ObservedRequest> = received.every().collect();
    let count = all[skip..]
        .iter()
        .filter(|request| {
            let target = request.target.as_str();
            (4..=LISTS).any(|n| {
                target == format!("/{kind}/{n}")
                    || target.starts_with(&format!("/api/{kind}/{n}?"))
                    || target == format!("/api/{kind}/{n}")
            })
        })
        .count();
    (count, all.len())
}

/// A crawl of the lists of `kind` under `fetch.tier`, `tier`, its HTTP
/// sent from the browser (`fetch.http_from: browser`), as `run_id`, read
/// by [`read_the_list`].
fn crawl(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    (run_id, kind, tier): (&str, &'static str, &str),
) {
    let mut engine = engine(env);
    let seeds: Vec<Value> = (1..=LISTS)
        .map(|n| json!({ "url": proving_ground.url(HOST, &format!("/{kind}/{n}")), "label": kind }))
        .collect();
    let finished = crawl_declaring(
        &mut engine,
        run_id,
        &seeds,
        &json!({ kind: {} }),
        &settings(
            proving_ground,
            &json!({ "fetch": { "tier": tier, "http_from": "browser" } }),
        ),
        &mut read_the_list(kind),
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
}

/// A list a script loads page by page, by page number or by cursor, is
/// learned onto `discovered_api` from its samples: each sample's run there
/// replays its browser's calls, its handler's waits answered from them,
/// and stages what the browser's run staged; the later lists replay the
/// plan for their own pages. Every list commits the Records a crawl on
/// the browser commits, and the later ones cost the site a fraction of
/// the browser's requests: their document and their two calls, twice the
/// document with the reload, where the browser loads every page of the
/// list each time.
///
/// Covers: story:5-119, story:5-114, setting:fetch.tier, operation:page.wait_for_response, operation:page.wait_for_request_finished, operation:page.reload, operation:response.body, operation:crawl_request.push
#[rstest::rstest]
#[case::page_number("scroll")]
#[case::cursor("cursor")]
fn an_infinite_list_compared_against_the_browser_is_served_by_its_replayed_calls(
    #[case] kind: &'static str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let auto = format!("api-{kind}");
    crawl(&env, &proving_ground, (&auto, kind, "auto"));
    let (replayed, seen) = later_requests(&proving_ground, kind, 0);
    let on_browser = format!("browser-{kind}");
    crawl(&env, &proving_ground, (&on_browser, kind, "browser"));
    let (browsed, _) = later_requests(&proving_ground, kind, seen);

    let sample = [
        "browser_fetch sample",
        "browser sample",
        "discovered_api sample",
    ];
    let expected_chosen: Vec<String> = sample
        .iter()
        .chain(&sample)
        .chain(&sample)
        .map(|entry| (*entry).to_owned())
        .chain((4..=LISTS).map(|_| "discovered_api compared".to_owned()))
        .collect();
    assert_eq!(chosen(&env, &auto), expected_chosen);
    let records = committed(&env, &auto);
    assert_eq!(records.len(), 6, "{records:?}");
    assert!(
        records
            .iter()
            .all(|record| record["items"].as_array().unwrap().len() == 4),
        "{records:?}"
    );
    assert_eq!(records, committed(&env, &on_browser));
    // Three lists, each its document twice and two calls.
    assert_eq!(replayed, 3 * (2 + 2));
    assert!(browsed >= 3 * (2 + 2 * 3), "{browsed}");
}

/// A handler that waits for its list's second call and pushes what it was
/// told: its status, or the code and setting of the refusal.
fn read_the_second_call(inside: &mut Inside<'_>, handle: &Value) -> Value {
    let page_id = handle["page_id"].clone();
    let engine = inside.engine();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "second", "method": "page.wait_for_response",
        "params": {
            "page_id": page_id,
            "url": { "kind": "regex", "pattern": r"/api/deep/\d+\?page=2$" },
            "timeout": 15_000,
        },
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "reload", "method": "page.reload",
        "params": { "page_id": page_id },
    }));
    let mut second = Value::Null;
    for _ in 0..2 {
        let reply = engine.next().unwrap();
        if reply["id"] == "second" {
            second = reply;
        }
    }
    let told = match second.get("result") {
        Some(result) => json!({ "status": result["response"]["status"] }),
        None => json!({
            "code": second["error"]["data"]["code"],
            "setting": second["error"]["data"]["setting"],
        }),
    };
    let reply = inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "dataset": "pages",
            "records": [{ "list": handle["crawl_request"]["url"], "told": told }],
        }),
    );
    assert!(reply.get("result").is_some(), "{reply}");
    json!({ "result": { "records": [] } })
}

/// A replayed call whose answer nests past `network.max_json_depth` answers
/// the wait for it `response_limit_exceeded` naming the setting: a list
/// learned onto `discovered_api` from samples that nest within it meets the
/// depth on a later list, and its handler is told so.
///
/// Covers: story:5-119, row:SC-05, error:response_limit_exceeded, setting:network.max_json_depth, operation:page.wait_for_response
#[test]
fn a_replayed_call_past_the_json_depth_answers_its_wait_response_limit_exceeded() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seeds: Vec<Value> = (1..=LISTS)
        .map(|n| json!({ "url": proving_ground.url(HOST, &format!("/deep/{n}")), "label": "deep" }))
        .collect();

    let finished = crawl_declaring(
        &mut engine,
        "api-deep",
        &seeds,
        &json!({ "deep": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut read_the_second_call,
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");

    let refused = json!({
        "code": "response_limit_exceeded",
        "setting": "network.max_json_depth",
    });
    let mut told: Vec<(String, Value)> = committed(&env, "api-deep")
        .into_iter()
        .map(|record| {
            let list = record["list"].as_str().unwrap().to_owned();
            (list, record["told"].clone())
        })
        .collect();
    told.sort_by(|left, right| left.0.cmp(&right.0));
    let expected: Vec<(String, Value)> = (1..=LISTS)
        .map(|n| {
            let told = if n < 4 {
                json!({ "status": 200 })
            } else {
                refused.clone()
            };
            (proving_ground.url(HOST, &format!("/deep/{n}")), told)
        })
        .collect();
    assert_eq!(told, expected);
}

/// A list whose call carries a token its document issued, which the site
/// takes once, cannot be replayed: each sample's run on `discovered_api`
/// is refused the call, its wait escalates, and the Label stays on the
/// browser (ADR 0020).
///
/// Covers: story:5-119, setting:fetch.tier, operation:page.wait_for_response, operation:page.wait_for_request_finished
#[test]
fn a_list_whose_call_carries_a_token_taken_once_stays_on_the_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    crawl(&env, &proving_ground, ("api-signed", "signed", "auto"));

    let sample = [
        "browser_fetch sample",
        "browser sample",
        "discovered_api sample",
    ];
    let expected_chosen: Vec<String> = sample
        .iter()
        .chain(&sample)
        .chain(&sample)
        .map(|entry| (*entry).to_owned())
        .chain((4..=LISTS).map(|_| "browser compared".to_owned()))
        .collect();
    assert_eq!(chosen(&env, "api-signed"), expected_chosen);
    let records = committed(&env, "api-signed");
    assert_eq!(records.len(), 6, "{records:?}");
}

/// The lists of `kinds` on `origin`, each a Label of its kind's name.
fn lists_of(origin: &str, kinds: &[&str]) -> Vec<Value> {
    kinds
        .iter()
        .flat_map(|kind| {
            (1..=LISTS)
                .map(move |n| json!({ "url": format!("{origin}/{kind}/{n}"), "label": kind }))
        })
        .collect()
}

/// A crawl as `run_id` of the scroll and guarded lists on `origin`, under
/// the `fetch` settings `fetch`, each read by [`read_the_list`].
fn crawl_scroll_and_guarded(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    (run_id, origin): (&str, &str),
    fetch: &Value,
) {
    let mut engine = engine(env);
    let mut scroll = read_the_list("scroll");
    let mut guarded = read_the_list("guarded");
    let finished = crawl_declaring(
        &mut engine,
        run_id,
        &lists_of(origin, &["scroll", "guarded"]),
        &json!({ "scroll": {}, "guarded": {} }),
        &settings(proving_ground, &json!({ "fetch": fetch })),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap_or_default();
            if url.contains("/guarded/") {
                guarded(inside, handle)
            } else {
                scroll(inside, handle)
            }
        },
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
}

/// The TLS fingerprints, without what a resumed handshake adds, of the
/// requests for the lists of `kind` past the samples, their documents and
/// their calls apart, among `requests`.
fn later_fingerprints(
    requests: &[&ObservedRequest],
    kind: &str,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let ja4 = |request: &&&ObservedRequest| {
        request
            .tls
            .as_ref()
            .and_then(|hello| hello["ja4_base"].as_str())
            .unwrap_or("none")
            .to_owned()
    };
    let documents = requests
        .iter()
        .filter(|request| (4..=LISTS).any(|n| request.target == format!("/{kind}/{n}")))
        .map(|request| ja4(&request))
        .collect();
    let calls = requests
        .iter()
        .filter(|request| {
            (4..=LISTS).any(|n| request.target.starts_with(&format!("/api/{kind}/{n}?")))
        })
        .map(|request| ja4(&request))
        .collect();
    (documents, calls)
}

/// Under `fetch.http_from: learned` (#2420), a sample's replay of its
/// browser's calls leaves from a Session's own transport first, and from
/// the browser where that replay differed: the infinite-scroll list, whose
/// API answers any client alike, is learned onto `discovered_api` with its
/// calls sent from the transport, its later lists' documents and calls
/// arriving with the transport's TLS fingerprint; the guarded list, whose
/// API answers only the pinned browser, keeps `discovered_api` with its
/// calls sent from the browser, arriving with the browser's. Provenance's
/// tier entry names where each replay's calls left from, and every list
/// commits the Records a crawl on the browser commits.
///
/// Covers: story:5-119, setting:fetch.http_from, setting:fetch.tier
#[test]
fn under_learned_a_list_replays_its_calls_from_the_cheapest_sender_that_matches_the_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&format!(
        "tls.trusted_cas.\"{HOST}\" = ['{}']\n",
        proving_ground.description().ca.display()
    ));
    let origin = format!("https://{HOST}:{}", proving_ground.address().port());

    crawl_scroll_and_guarded(
        &env,
        &proving_ground,
        ("learned", &origin),
        &json!({ "tier": "auto", "http_from": "learned" }),
    );
    let received = proving_ground.received_requests().unwrap();
    let requests: Vec<&ObservedRequest> = received.every().collect();
    crawl_scroll_and_guarded(
        &env,
        &proving_ground,
        ("on-browser", &origin),
        &json!({ "tier": "browser" }),
    );

    let mut learned: Vec<Value> = provenance(&env, "learned", "fetch_tier")
        .iter()
        .filter(|entry| entry["chosen"] != "sample")
        .map(|entry| {
            json!([
                entry["label"],
                entry["tier"],
                entry["chosen"],
                entry["calls_sent_on"]
            ])
        })
        .collect();
    learned.sort_by_key(ToString::to_string);
    let mut expected: Vec<Value> = [("guarded", "browser_fetch"), ("scroll", "http")]
        .into_iter()
        .flat_map(|(label, sent_on)| {
            (4..=LISTS).map(move |_| json!([label, "discovered_api", "compared", sent_on]))
        })
        .collect();
    expected.sort_by_key(ToString::to_string);
    assert_eq!(learned, expected);

    let (scroll_documents, scroll_calls) = later_fingerprints(&requests, "scroll");
    let (guarded_documents, guarded_calls) = later_fingerprints(&requests, "guarded");
    assert_eq!(scroll_documents.len(), 1, "{scroll_documents:?}");
    assert_eq!(scroll_calls, scroll_documents);
    assert_eq!(guarded_documents.len(), 1, "{guarded_documents:?}");
    assert_eq!(guarded_calls, guarded_documents);
    assert_ne!(scroll_calls, guarded_calls);

    let records = committed(&env, "learned");
    assert_eq!(records.len(), 2 * 6, "{records:?}");
    assert!(
        records
            .iter()
            .all(|record| record["items"].as_array().unwrap().len() == 4),
        "{records:?}"
    );
    assert_eq!(records, committed(&env, "on-browser"));
}
