//! Trap links on Foxhound's executable over stdio (spec #600's trap link
//! cases; ADR 0035; pitfall S03), against the Proving ground's honeypot
//! links page, `traps.test/hidden-links`: under `fetch.tier: browser`, a
//! handler's `enqueueLinks` answers every link a person cannot see, an
//! invisible `nofollow` one among them, not added with `trap_link`, and the
//! Proving ground receives no request for one, robots.txt not obeyed, so
//! nothing else keeps them away; a visible link, `nofollow` or not, is
//! added, and so is the link in an ad that EasyList's cosmetic rule
//! `##.ad-slot` hides, left out of the judgement (ADR 0047).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 1148;

const HOST: &str = "traps.test";

/// Each element `enqueueLinks` answered, as its kind and its reason or
/// URL's path.
fn answers(answered: &Value) -> Vec<(String, String)> {
    answered["links"]
        .as_array()
        .unwrap_or_else(|| panic!("no links: {answered}"))
        .iter()
        .map(|link| {
            let kind = link["kind"].as_str().unwrap().to_owned();
            let what = match kind.as_str() {
                "added" => {
                    let url = link["url"].as_str().unwrap();
                    let path = url.split_once("://").unwrap().1;
                    path[path.find('/').unwrap()..].to_owned()
                }
                _ => link["reason"].as_str().unwrap_or_default().to_owned(),
            };
            (kind, what)
        })
        .collect()
}

/// On the honeypot links page in the browser, the four honeypots of every
/// page, the transparent one, the clipped one and the two invisible
/// `nofollow` ones are each answered `trap_link` and never fetched; the
/// plain, the `nofollow` and the ad's links are added and fetched, the ad's
/// though live hiding hides it when the links are read.
///
/// Covers: story:8-226, story:8-229, operation:crawl_request.enqueue_links
#[test]
fn the_browser_tier_never_follows_a_link_a_person_cannot_see() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seeds = [json!({ "url": proving_ground.url(HOST, "/hidden-links"), "label": "links" })];

    let mut answered = Value::Null;
    crawl_declaring(
        &mut engine,
        "trap-links",
        &seeds,
        &json!({ "links": {}, "item": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "browser" },
                "site_preferences": { "respect_robots_txt": false },
            }),
        ),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            if handle["crawl_request"]["label"] == "links" {
                // Live hiding hides the ad once the Consent prompt runner's
                // first pass ended: the links are judged after.
                let hidden = inside.call(
                    "locator.wait_for",
                    &json!({
                        "locator": { "page_id": handle["page_id"], "selector": ".ad-slot" },
                        "state": "hidden",
                    }),
                );
                assert!(hidden.get("result").is_some(), "{hidden}");
                let reply = inside.call(
                    "crawl_request.enqueue_links",
                    &json!({
                        "run_id": handle["run_id"],
                        "crawl_request_id": handle["crawl_request"]["id"],
                        "lease_epoch": handle["lease_epoch"],
                        "locator": { "page_id": handle["page_id"], "selector": "a" },
                        "label": "item",
                    }),
                );
                answered = reply.get("result").unwrap_or(&reply).clone();
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finish());

    let trap = || ("not_added".to_owned(), "trap_link".to_owned());
    let added = |path: &str| ("added".to_owned(), path.to_owned());
    assert_eq!(
        answers(&answered),
        [
            trap(),
            trap(),
            trap(),
            trap(),
            trap(),
            trap(),
            trap(),
            trap(),
            added("/items/I-0001"),
            added("/items/I-0002"),
            added("/items/I-0003"),
        ],
        "{answered}"
    );
    let mut fetched: Vec<String> = ReceivedRequests::of(&observations)
        .of_the_case()
        .filter(|request| request["host"] == HOST)
        .map(|request| request["target"].as_str().unwrap().to_owned())
        .filter(|target| target != "/robots.txt")
        .collect();
    fetched.sort();
    fetched.dedup();
    assert_eq!(
        fetched,
        [
            "/hidden-links",
            "/items/I-0001",
            "/items/I-0002",
            "/items/I-0003"
        ]
    );
}
