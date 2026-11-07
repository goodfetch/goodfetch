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

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings, tiers};
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
        "scroll" => [
            r"/api/scroll/\d+\?page=1$".to_owned(),
            r"/api/scroll/\d+\?page=2$".to_owned(),
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
/// cannot serve leaves it with nothing to push.
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
            items.extend(body["items"].as_array().unwrap().iter().cloned());
        }
        let reply = inside.call(
            "crawl_request.push",
            &json!({
                "run_id": handle["run_id"],
                "crawl_request_id": handle["crawl_request"]["id"],
                "lease_epoch": handle["lease_epoch"],
                "dataset": "pages",
                "data": {
                    "list": handle["crawl_request"]["url"],
                    "items": items,
                    "first": [first["url"], first["status"]],
                },
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
    let all: Vec<&Value> = received.every().collect();
    let count = all[skip..]
        .iter()
        .filter(|request| {
            let target = request["target"].as_str().unwrap_or_default();
            (4..=LISTS).any(|n| {
                target == format!("/{kind}/{n}")
                    || target.starts_with(&format!("/api/{kind}/{n}?"))
                    || target == format!("/api/{kind}/{n}")
            })
        })
        .count();
    (count, all.len())
}

/// A crawl of the lists of `kind` under `fetch.tier`, `tier`, as
/// `run_id`, read by [`read_the_list`].
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
        &settings(proving_ground, &json!({ "fetch": { "tier": tier } })),
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

    let sample = ["http sample", "browser sample", "discovered_api sample"];
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

    let sample = ["http sample", "browser sample", "discovered_api sample"];
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
