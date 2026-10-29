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

use crate::crawl::Attached;
use crate::fetch_tiers::{engine, expecting, provenance, settings, tiers};
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

/// Each request of the Run `run_id` as its path, how it ended, its Block
/// cause or Error code, and its count of attempts, by path.
fn endings(engine: &mut Attached, run_id: &str) -> Vec<(String, Value)> {
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let mut endings: Vec<(String, Value)> = answer["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {answer}"))
        .iter()
        .map(|outcome| {
            let url = outcome["request"]["url"].as_str().unwrap();
            let path = url.split_once("://").unwrap().1;
            let what = if outcome["kind"] == "blocked" {
                outcome["cause"].clone()
            } else {
                outcome["error"]["code"].clone()
            };
            (
                path[path.find('/').unwrap()..].to_owned(),
                json!([outcome["kind"], what, outcome["attempts"]]),
            )
        })
        .collect();
    endings.sort_by(|left, right| left.0.cmp(&right.0));
    endings
}

/// A handler that, on a page of the Label `from`, enqueues every link of
/// it under the Label `to`, keeping each answer, and answers no Record.
fn enqueuing<'a>(
    from: &'a str,
    to: &'a str,
    answered: &'a mut Value,
) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + 'a {
    move |inside: &mut Inside<'_>, handle: &Value| {
        if handle["crawl_request"]["label"] == from {
            let reply = inside.call(
                "crawl_request.enqueue_links",
                &json!({
                    "run_id": handle["run_id"],
                    "crawl_request_id": handle["crawl_request"]["id"],
                    "lease_epoch": handle["lease_epoch"],
                    "locator": { "page_id": handle["page_id"], "selector": "a" },
                    "label": to,
                }),
            );
            *answered = reply.get("result").unwrap_or(&reply).clone();
        }
        json!({ "result": { "records": [] } })
    }
}

/// The requests for the honeypots under `/hp/` the Proving ground saw.
fn honeypots_fetched(observations: &Value) -> usize {
    ReceivedRequests::of(observations)
        .of_the_case()
        .filter(|request| request["host"] == HOST)
        .filter(|request| request["target"].as_str().unwrap().starts_with("/hp/"))
        .count()
}

/// An HTTP-tier crawl, which has no layout, cannot judge what a person
/// sees: on the honeypot links page it adds every link, and marks in
/// Provenance, `trap_link_suspected`, each one whose markup hides it (the
/// `hidden`, `display:none`, `visibility:hidden`, `aria-hidden` holder,
/// `opacity:0` and invisible `display:none` `nofollow` honeypots); a link
/// only layout hides (placed off the page, clipped) and every link a
/// person sees go unmarked. robots.txt, obeyed, keeps the honeypots from
/// being fetched.
///
/// Covers: story:8-228, operation:crawl_request.enqueue_links
#[test]
fn the_http_tier_adds_the_links_it_cannot_judge_and_marks_the_suspected_ones() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seeds = [json!({ "url": proving_ground.url(HOST, "/hidden-links"), "label": "links" })];

    let mut answered = Value::Null;
    crawl_declaring(
        &mut engine,
        "trap-links-http",
        &seeds,
        &json!({ "links": {}, "item": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "http" } })),
        &mut enqueuing("links", "item", &mut answered),
    );
    let suspected = provenance(&env, "trap-links-http", "trap_link_suspected");
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finish());

    let kinds: Vec<String> = answers(&answered)
        .into_iter()
        .map(|(kind, _)| kind)
        .collect();
    assert_eq!(kinds, vec!["added".to_owned(); 11], "{answered}");
    let added: Vec<(Value, Value)> = answered["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|link| (link["id"].clone(), link["url"].clone()))
        .collect();
    // The page's links in order: the four honeypots of every page, the
    // transparent one, the clipped one, the two invisible `nofollow` ones
    // (`display:none`, then off the top), then the three items.
    let mut marked: Vec<(Value, Value)> = suspected
        .iter()
        .map(|entry| (entry["crawl_request_id"].clone(), entry["url"].clone()))
        .collect();
    marked.sort_by_key(|(id, _)| id.to_string());
    let mut expected: Vec<(Value, Value)> = [0, 1, 2, 3, 4, 6]
        .iter()
        .map(|at| added[*at].clone())
        .collect();
    expected.sort_by_key(|(id, _)| id.to_string());
    assert_eq!(marked, expected, "{suspected:?}");
    assert_eq!(honeypots_fetched(&observations), 0);
}

/// A page reached through a link its HTTP-tier page marked as a suspected
/// trap ends `blocked` with `decoy` on the browser tier, which never
/// follows such a link: its document is never fetched, it is attempted
/// once, never retried, rotated or escalated, and none of its links is
/// enqueued; the links a person sees are followed in the browser.
///
/// Covers: story:8-227, block-cause:decoy
#[test]
fn a_page_reached_through_a_trap_ends_decoy_in_the_browser_and_is_never_fetched() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seeds = [json!({ "url": proving_ground.url(HOST, "/"), "label": "home" })];

    let mut answered = Value::Null;
    crawl_declaring(
        &mut engine,
        "trap-links-decoy",
        &seeds,
        &json!({
            "home": { "settings": { "fetch": { "tier": "http" } } },
            "page": { "settings": { "fetch": { "tier": "browser" } } },
        }),
        &settings(
            &proving_ground,
            &json!({ "site_preferences": { "respect_robots_txt": false } }),
        ),
        &mut enqueuing("home", "page", &mut answered),
    );
    let ended = endings(&mut engine, "trap-links-decoy");
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finish());

    let decoy = json!(["blocked", "decoy", 1]);
    let honeypots: Vec<&Value> = ended
        .iter()
        .filter(|(path, _)| path.starts_with("/hp/"))
        .map(|(_, ending)| ending)
        .collect();
    assert_eq!(honeypots, vec![&decoy; 4], "{ended:?}");
    let succeeded = json!(["succeeded", null, 1]);
    let others: Vec<(&str, &Value)> = ended
        .iter()
        .filter(|(path, _)| !path.starts_with("/hp/"))
        .map(|(path, ending)| (path.as_str(), ending))
        .collect();
    assert_eq!(
        others,
        [
            ("/", &succeeded),
            ("/archive/", &succeeded),
            ("/calendar/2026-01", &succeeded),
            ("/categories/1", &succeeded),
            ("/categories/2", &succeeded),
            ("/categories/3", &succeeded),
        ],
        "{answered}"
    );
    assert_eq!(honeypots_fetched(&observations), 0);
}

/// The labyrinth page, marked `noindex` by its `X-Robots-Tag` header and
/// its robots `meta` element, failing its Label's Page check ends
/// `blocked` with `decoy` rather than failing with `page_check_failed`,
/// on the browser tier and under a forced HTTP tier alike, attempted
/// once, no handler given it, so its hidden link to more of the same is
/// never followed.
///
/// Covers: story:8-227, block-cause:decoy, setting:session.page_check
#[test]
fn a_noindex_page_failing_its_page_check_ends_decoy() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let labyrinth = proving_ground.url("access.test", "/corpus/labyrinth");
    let expecting = |tier: &str| expecting("article.item", &json!({ "fetch": { "tier": tier } }));

    let mut handled = 0;
    crawl_declaring(
        &mut engine,
        "labyrinth",
        &[
            json!({ "url": labyrinth, "label": "browser" }),
            json!({ "url": format!("{labyrinth}?copy=1"), "label": "http" }),
        ],
        &json!({ "browser": expecting("browser"), "http": expecting("http") }),
        &settings(&proving_ground, &json!({})),
        &mut |_: &mut Inside<'_>, _: &Value| {
            handled += 1;
            json!({ "result": { "records": [] } })
        },
    );
    let ended = endings(&mut engine, "labyrinth");
    assert!(engine.finish());

    let decoy = json!(["blocked", "decoy", 1]);
    assert_eq!(
        (handled, ended),
        (
            0,
            vec![
                ("/corpus/labyrinth".to_owned(), decoy.clone()),
                ("/corpus/labyrinth?copy=1".to_owned(), decoy),
            ]
        )
    );
}

/// The paths the Proving ground served on [`HOST`], robots.txt aside, each
/// once.
fn fetched_once(observations: &Value) -> Vec<String> {
    let mut fetched: Vec<String> = ReceivedRequests::of(observations)
        .of_the_case()
        .filter(|request| request["host"] == HOST)
        .map(|request| request["target"].as_str().unwrap().to_owned())
        .filter(|target| target != "/robots.txt")
        .collect();
    fetched.sort();
    fetched.dedup();
    fetched
}

/// A page a hidden link reaches before a visible one is also reached
/// through a link a person sees, so it is no decoy: on the cards page,
/// served over HTTP, item 1 and category 1 are linked first through links
/// the markup hides, then visibly, and are fetched in the browser like
/// item 2, whose visible link came first. Item 3, which only its hidden
/// image link reaches, stays marked: it ends `blocked` with `decoy`, never
/// fetched, and only its mark is written in Provenance, the others'
/// cleared in the same call before it committed.
///
/// Covers: story:8-227, story:8-228, block-cause:decoy
#[test]
fn a_page_also_reached_through_a_visible_link_is_fetched_in_the_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seeds = [json!({ "url": proving_ground.url(HOST, "/cards"), "label": "cards" })];

    let mut answered = Value::Null;
    crawl_declaring(
        &mut engine,
        "trap-links-seen",
        &seeds,
        &json!({
            "cards": { "settings": { "fetch": { "tier": "http" } } },
            "item": { "settings": { "fetch": { "tier": "browser" } } },
        }),
        &settings(&proving_ground, &json!({})),
        &mut enqueuing("cards", "item", &mut answered),
    );
    let ended = endings(&mut engine, "trap-links-seen");
    let suspected = provenance(&env, "trap-links-seen", "trap_link_suspected");
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finish());

    let succeeded = json!(["succeeded", null, 1]);
    let marked: Vec<String> = suspected
        .iter()
        .map(|entry| entry["url"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        (ended, fetched_once(&observations), marked),
        (
            vec![
                ("/cards".to_owned(), succeeded.clone()),
                ("/categories/1".to_owned(), succeeded.clone()),
                ("/items/I-0001".to_owned(), succeeded.clone()),
                ("/items/I-0002".to_owned(), succeeded),
                ("/items/I-0003".to_owned(), json!(["blocked", "decoy", 1])),
            ],
            vec![
                "/cards".to_owned(),
                "/categories/1".to_owned(),
                "/items/I-0001".to_owned(),
                "/items/I-0002".to_owned(),
            ],
            vec![proving_ground.url(HOST, "/items/I-0003")],
        ),
        "{answered}"
    );
}

/// A request marked as reached through a suspected trap, while its Label
/// still samples, is a sample like any other: its browser run is weighed,
/// never ended `decoy` on the mark alone, so it counts toward its Label's
/// choice. Item 3, which only a hidden link on the cards page reaches, is
/// the item Label's first sample; three matching samples move the Label to
/// `http`, and every request succeeds.
///
/// Covers: story:8-227, story:5-114
#[test]
fn a_marked_request_in_a_sampling_label_is_weighed_and_never_a_decoy() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seeds = [json!({ "url": proving_ground.url(HOST, "/cards"), "label": "cards" })];

    let mut answered = Value::Null;
    crawl_declaring(
        &mut engine,
        "trap-links-sampled",
        &seeds,
        &json!({
            "cards": { "settings": { "fetch": { "tier": "http" } } },
            "item": {},
        }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut enqueuing("cards", "item", &mut answered),
    );
    let ended = endings(&mut engine, "trap-links-sampled");
    let chosen: Vec<String> = tiers(&env, "trap-links-sampled")
        .iter()
        .filter(|entry| entry[0] == "item")
        .map(|entry| {
            format!(
                "{} {}",
                entry[1].as_str().unwrap(),
                entry[2].as_str().unwrap()
            )
        })
        .collect();
    assert!(engine.finish());

    let succeeded = json!(["succeeded", null, 1]);
    assert_eq!(
        (ended, chosen),
        (
            vec![
                ("/cards".to_owned(), succeeded.clone()),
                ("/categories/1".to_owned(), succeeded.clone()),
                ("/items/I-0001".to_owned(), succeeded.clone()),
                ("/items/I-0002".to_owned(), succeeded.clone()),
                ("/items/I-0003".to_owned(), succeeded),
            ],
            [
                "http sample",
                "browser sample",
                "http sample",
                "browser sample",
                "http sample",
                "browser sample",
                "http compared",
            ]
            .map(str::to_owned)
            .to_vec(),
        ),
        "{answered}"
    );
}

/// A run whose staging is discarded writes no `trap_link_suspected`
/// Provenance entry: the cards page's Label samples, so its HTTP run,
/// which adds the links its markup hides and marks them, is discarded for
/// the browser's, which adds none of them (ADR 0103). The entries are
/// written as an attempt commits, for the marks it committed.
///
/// Covers: story:8-228
#[test]
fn a_discarded_run_writes_no_suspected_trap_link_entry() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seeds = [json!({ "url": proving_ground.url(HOST, "/cards"), "label": "cards" })];

    let mut answered = Value::Null;
    crawl_declaring(
        &mut engine,
        "trap-links-discarded",
        &seeds,
        &json!({
            "cards": { "settings": { "fetch": { "tier": "auto" } } },
            "item": { "settings": { "fetch": { "tier": "http" } } },
        }),
        &settings(&proving_ground, &json!({})),
        &mut enqueuing("cards", "item", &mut answered),
    );
    let suspected = provenance(&env, "trap-links-discarded", "trap_link_suspected");
    let ended = endings(&mut engine, "trap-links-discarded");
    assert!(engine.finish());

    let succeeded = json!(["succeeded", null, 1]);
    assert_eq!(
        (suspected, ended),
        (
            Vec::new(),
            vec![
                ("/cards".to_owned(), succeeded.clone()),
                ("/categories/1".to_owned(), succeeded.clone()),
                ("/items/I-0001".to_owned(), succeeded.clone()),
                ("/items/I-0002".to_owned(), succeeded),
            ],
        ),
        "{answered}"
    );
}
