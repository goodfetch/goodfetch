//! Browser-fetch (spec #579; ADR 0020; pitfall H20), on Foxhound's
//! executable against the Proving ground and Chrome for Testing: a Session
//! that may move to the browser, any Session under `fetch.tier: auto`,
//! sends its HTTP tier's requests from inside its pool Session's browser
//! context, so the cookies a page set in the browser are sent by the HTTP
//! tier's next request in the same Session and by no other Session (the
//! tier parity of cookies, Foxhound's half), and the site sees one TLS
//! fingerprint across both tiers. A body cut once the browser read it fails
//! each attempt on the browser-fetch tier and the browser tier (#1732).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_protocol::home::RUN_TRACE_CDP_COMMANDS;
use goodfetch_test_client::{Attached, HomeLayout};
use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, ProvingGround, TestEnv, browser_cache};
use rstest::rstest;
use serde_json::{Value, json};

use crate::attached_with;
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 769;

/// The host of the `access` family's corpus pages.
const HOST: &str = "access.test";

/// An engine whose pages launch Chrome for Testing from the test's cache.
fn engine(env: &TestEnv) -> Attached {
    engine_silent_for(env, Duration::from_secs(60))
}

/// [`engine`], whose messages may be `silent` apart.
fn engine_silent_for(env: &TestEnv, silent: Duration) -> Attached {
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    attached_with(env, Some(silent), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    })
}

/// The settings of a crawl that reaches every host of `proving_ground` on
/// `tier`, one Session in each pool at a time.
fn settings(proving_ground: &ProvingGround, tier: &str) -> Value {
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    json!({
        "network": { "resolve": resolve },
        // The egress relay judges a mapping's answers as any other.
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
        "fetch": { "tier": tier },
        "session": { "pool": { "max_size": 1 } },
    })
}

/// A seed of `label` at `path` of the corpus.
fn seed(proving_ground: &ProvingGround, path: &str, label: &str) -> Value {
    json!({ "url": proving_ground.url(HOST, path), "label": label })
}

/// Each request the site received for a corpus article, in order: its
/// target and the cookies it carried.
fn articles(proving_ground: &ProvingGround) -> Vec<(String, Value)> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.target.starts_with("/corpus/article"))
        .map(|request| (request.target.clone(), json!(request.cookies)))
        .collect()
}

/// The handler: on a browser page of the Label `a` it sets the cookie
/// `who=a` once; it answers no Record.
fn handler(set: &mut bool) -> impl FnMut(&mut Inside<'_>, &Value) -> Value + '_ {
    move |inside, handle| {
        if handle["tier"] == "browser" && handle["crawl_request"]["label"] == "a" && !*set {
            *set = true;
            let _ = inside.call(
                "page.evaluate",
                &json!({
                    "page_id": handle["page_id"],
                    "function": "() => { document.cookie = 'who=a; path=/'; }",
                }),
            );
        }
        json!({ "result": { "records": [] } })
    }
}

/// Under `auto`, each of a Label's first requests is a sample: its run on
/// `http`, then on `browser`. The cookie the first sample's browser page
/// set is sent by the `http` run of the Label's next request, in the same
/// pool Session, and never by a request of the Label `b`, whose pool is
/// its own.
///
/// Covers: story:5-123, story:5-121
#[test]
fn a_cookie_the_browser_sample_set_is_sent_by_the_http_tiers_next_request_in_its_session_only() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut set = false;
    let finished = crawl_declaring(
        &mut engine,
        "cookie-parity",
        &[
            seed(&proving_ground, "/corpus/article?n=1", "a"),
            seed(&proving_ground, "/corpus/article?n=2", "a"),
            seed(&proving_ground, "/corpus/article?n=3", "b"),
        ],
        &json!({
            "a": {},
            "b": { "settings": { "session": { "source": null } } },
        }),
        &settings(&proving_ground, "auto"),
        &mut handler(&mut set),
    );
    assert!(engine.finished());

    let who = json!([["who", "a"]]);
    let none = json!([]);
    assert_eq!(
        (articles(&proving_ground), &finished["result"]["lifecycle"]),
        (
            vec![
                // The first sample: on `http`, then on `browser`.
                ("/corpus/article?n=1".to_owned(), none.clone()),
                ("/corpus/article?n=1".to_owned(), none.clone()),
                // The next, after the browser page set its cookie.
                ("/corpus/article?n=2".to_owned(), who.clone()),
                ("/corpus/article?n=2".to_owned(), who),
                // Another Session's.
                ("/corpus/article?n=3".to_owned(), none.clone()),
                ("/corpus/article?n=3".to_owned(), none),
            ],
            &json!("finished")
        )
    );
}

/// How many browser contexts the Run `run_id` created, by its CDP trace: its
/// one Session's one in each browser it was seated in.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's CDP trace from outside the engine, on no async worker"
)]
fn contexts_created(env: &TestEnv, run_id: &str) -> usize {
    let file = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_CDP_COMMANDS);
    std::fs::read_to_string(file)
        .unwrap()
        .lines()
        .filter(|line| {
            serde_json::from_str::<Value>(line).unwrap()["method"] == "Target.createBrowserContext"
        })
        .count()
}

/// A Session on a forced `browser_fetch` tier opens no page but the engine's
/// own, and is held to its browser's limits all the same: past them, each
/// next request goes out of a replacement browser.
///
/// Covers: story:3-62, setting:governor.browser.recycle_pages, setting:governor.browser.memory_cap
#[rstest]
#[case::after_its_pages(json!({ "recycle_pages": 1 }))]
#[case::past_its_memory_cap(json!({ "memory_cap": 1 }))]
fn a_session_on_the_browser_fetch_tier_moves_off_its_recycled_browser(#[case] limit: Value) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let mut settings = settings(&proving_ground, "browser_fetch");
    settings["governor"] = json!({ "browser": limit });
    settings["browser"]["cdp_command_log"] = json!(true);

    let finished = crawl_declaring(
        &mut engine,
        "recycled",
        &[
            seed(&proving_ground, "/corpus/article?n=1", "a"),
            seed(&proving_ground, "/corpus/article?n=2", "a"),
            seed(&proving_ground, "/corpus/article?n=3", "a"),
        ],
        &json!({ "a": {} }),
        &settings,
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            articles(&proving_ground).len(),
            contexts_created(&env, "recycled") >= 3,
        ),
        (&json!("finished"), 3, true),
        "{finished}"
    );
}

/// Each request of the Run `run_id` that ended unsuccessful, by URL: its
/// Error code, its network error and the timeout it names, if any.
fn failures(engine: &mut Attached, run_id: &str) -> BTreeMap<String, (Value, Value, Value)> {
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    answer["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {answer}"))
        .iter()
        .filter(|outcome| !outcome["error"].is_null())
        .map(|outcome| {
            let error = &outcome["error"];
            (
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                (
                    error["code"].clone(),
                    error["details"]["error"].clone(),
                    error["details"]["limit"]["key"].clone(),
                ),
            )
        })
        .collect()
}

/// The `Accept-Encoding` of each request `host` received, in order.
fn accept_encodings(proving_ground: &ProvingGround, host: &str) -> Vec<Value> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.host.starts_with(host))
        .map(|request| {
            request
                .header("accept-encoding")
                .map_or(Value::Null, |value| json!(value))
        })
        .collect()
}

/// How many requests the site received for `target` on `host` so far.
fn requests_for(proving_ground: &ProvingGround, host: &str, target: &str) -> usize {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host.starts_with(host) && request.target == target)
        .count()
}

/// The transport conformance gate (pitfall H13's acceptance test) on
/// browser-fetch, the `http` tier of a Session that may move: each content
/// coding page decodes as the browser decodes it, the text served to the
/// handler, or fails with `navigation_failed` naming
/// `net::ERR_CONTENT_DECODING_FAILED`, a body compressed twice and labelled
/// once included; the browser asks for its own codings, as its own
/// navigation does; a `page.request` asking for no redirect gets the 301
/// as it came, nothing sent to its `Location`; and a page answering after
/// 8 s ends at a 1 s `timeout.first_byte` with `deadline_exceeded` naming
/// it.
///
/// **Over plain HTTP:** the browser cannot trust the Proving ground's
/// authority, so the pages are fetched over `http://`, where Chrome asks
/// for and decodes only `gzip` and `deflate`: the `br` and `zstd` pages
/// fail there as Chrome's own navigation fails them, which the browser
/// tier would too.
///
/// Covers: story:5-121, story:5-128
#[test]
fn browser_fetch_passes_the_transport_conformance_gate() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let mut settings = settings(&proving_ground, "auto");
    settings["site_preferences"] = json!({ "respect_robots_txt": false });
    settings["crawl"] = json!({ "retry": { "max": 0 } });
    let redirect = proving_ground.url("fetch-edge.test", "/redirect/301");

    let mut texts = BTreeMap::new();
    let mut redirected = None;
    let seeds: Vec<Value> = (1..=11)
        .map(|ordinal| {
            json!({
                "url": proving_ground.url(&format!("{ordinal}.content-coding.test"), "/page"),
                "label": "page",
            })
        })
        .collect();
    let finished = crawl_declaring(
        &mut engine,
        "codings",
        &seeds,
        &json!({ "page": {} }),
        &settings,
        &mut |inside, handle| {
            if handle["tier"] != "http" {
                return json!({ "result": { "records": [] } });
            }
            let url = handle["crawl_request"]["url"].as_str().unwrap().to_owned();
            let read = inside.call(
                "page.formats",
                &json!({ "page_id": handle["page_id"], "formats": { "markdown": true } }),
            );
            texts.insert(url, read["result"]["markdown"].clone());
            if redirected.is_none() {
                let answer = inside.call(
                    "page.request",
                    &json!({ "page_id": handle["page_id"], "url": redirect, "max_redirects": 0 }),
                );
                redirected = Some((
                    answer["result"]["status"].clone(),
                    answer["result"]["headers"]["location"].is_string(),
                    requests_for(&proving_ground, "fetch-edge.test", "/target"),
                ));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let failed = failures(&mut engine, "codings");

    let mut slow = settings.clone();
    slow["timeout"] = json!({ "first_byte": 1_000 });
    let slow_url = proving_ground.url("6.incidents.test", "/catalog/2");
    let finished = crawl_declaring(
        &mut engine,
        "slow",
        &[json!({ "url": slow_url, "label": "page" })],
        &json!({ "page": {} }),
        &slow,
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    let timed_out = failures(&mut engine, "slow");
    assert!(engine.finished());

    let decoding_failed = (
        json!("navigation_failed"),
        json!("net::ERR_CONTENT_DECODING_FAILED"),
        Value::Null,
    );
    for ordinal in 1..=11 {
        let url = proving_ground.url(&format!("{ordinal}.content-coding.test"), "/page");
        // 3, 10 and 11 hold `br` or `zstd`, which Chrome decodes over
        // HTTPS only.
        if [1, 2, 3, 6, 7, 10, 11].contains(&ordinal) {
            assert_eq!(failed.get(&url), Some(&decoding_failed), "{url}");
        } else {
            let text = texts
                .get(&url)
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("{url} not served: {failed:?}"));
            assert!(
                text.contains(&format!(
                    "Content coding page {ordinal}, decoded once and as labelled."
                )),
                "{url}: {text}"
            );
        }
    }
    assert_eq!(failed.len(), 7, "{failed:?}");
    let asked = accept_encodings(&proving_ground, "8.content-coding.test");
    assert!(
        asked.len() >= 2 && asked.iter().all(|coding| coding == &asked[0]),
        "on `http` as on `browser`: {asked:?}"
    );
    assert_eq!(redirected, Some((json!(301), true, 0)));
    assert_eq!(
        timed_out.get(&slow_url),
        Some(&(
            json!("deadline_exceeded"),
            Value::Null,
            json!("timeout.first_byte")
        ))
    );
}

/// The JA4 of each TLS handshake the site's `host` received from the
/// observation `from` on, in order, without the extensions a resumed
/// handshake adds.
fn handshakes(proving_ground: &ProvingGround, host: &str, from: usize) -> Vec<String> {
    proving_ground.observations().unwrap()["tls_handshakes"]
        .as_array()
        .unwrap()
        .iter()
        .skip(from)
        .filter(|handshake| handshake["sni"] == host)
        .map(|handshake| handshake["ja4_base"].as_str().unwrap().to_owned())
        .collect()
}

/// How many TLS handshakes the site recorded so far.
fn handshakes_so_far(proving_ground: &ProvingGround) -> usize {
    proving_ground.observations().unwrap()["tls_handshakes"]
        .as_array()
        .unwrap()
        .len()
}

/// A crawl of one corpus article on `tier`, robots.txt not read, whose
/// handler reaches `https_url` from its page: through `page.request` on
/// the `http` tier, through `page.goto` on `browser`. Neither trusts the
/// Proving ground's certificate, so each request ends at its handshake,
/// whose `ClientHello` the site still records. Answers the tier of each
/// run of the handler.
fn reach_over_tls(
    engine: &mut Attached,
    proving_ground: &ProvingGround,
    run_id: &str,
    tier: &str,
    https_url: &str,
) -> Vec<Value> {
    let mut settings = settings(proving_ground, tier);
    settings["site_preferences"] = json!({ "respect_robots_txt": false });
    let mut tiers = Vec::new();
    let finished = crawl_declaring(
        engine,
        run_id,
        &[seed(proving_ground, "/corpus/article?n=1", "a")],
        &json!({ "a": {} }),
        &settings,
        &mut |inside, handle| {
            tiers.push(handle["tier"].clone());
            let method = if handle["tier"] == "browser" {
                "page.goto"
            } else {
                "page.request"
            };
            let reached = inside.call(
                method,
                &json!({ "page_id": handle["page_id"], "url": https_url }),
            );
            assert!(reached.get("error").is_some(), "{reached}");
            json!({ "result": { "records": [] } })
        },
    );
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    tiers
}

/// `fetch.tier: browser_fetch` forces the HTTP tier sent from inside the
/// Session's browser: the handler is given the `browser_fetch` tier,
/// Provenance records it as forced by the Run, and its `page.request` shows the site the browser's TLS `ClientHello`, the
/// one a `browser` run's navigation shows.
///
/// Covers: story:5-121, setting:fetch.tier, operation:page.request
#[test]
fn a_forced_browser_fetch_tier_sends_from_inside_the_sessions_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let https_url = proving_ground
        .url(HOST, "/corpus/article?n=2")
        .replacen("http://", "https://", 1);

    let forced = reach_over_tls(
        &mut engine,
        &proving_ground,
        "forced",
        "browser_fetch",
        &https_url,
    );
    let seen = handshakes_so_far(&proving_ground);
    let browser = reach_over_tls(
        &mut engine,
        &proving_ground,
        "browser",
        "browser",
        &https_url,
    );
    assert!(engine.finished());

    let browsers = handshakes(&proving_ground, HOST, seen);
    let all = handshakes(&proving_ground, HOST, 0);
    let forced_handshakes = &all[..all.len() - browsers.len()];
    assert_eq!(
        (forced, browser),
        (vec![json!("browser_fetch")], vec![json!("browser")])
    );
    // Provenance records the tier that sent each document, forced by the
    // Run (ADR 0023).
    assert_eq!(
        (
            crate::fetch_tiers::tiers(&env, "forced"),
            crate::fetch_tiers::tiers(&env, "browser"),
        ),
        (
            vec![json!(["a", "browser_fetch", "forced", "run", null])],
            vec![json!(["a", "browser", "forced", "run", null])],
        )
    );
    assert!(
        !forced_handshakes.is_empty()
            && !browsers.is_empty()
            && forced_handshakes
                .iter()
                .chain(&browsers)
                .all(|ja4| ja4 == &browsers[0]),
        "browser-fetch {forced_handshakes:?}, the browser {browsers:?}"
    );
}

/// Under `auto`, the Session's `http` run and its `browser` run show the
/// site one TLS `ClientHello`, the browser's, since its HTTP goes through
/// browser-fetch; a Session forced onto `http`, which never moves, shows
/// reqwest's own, another.
///
/// Covers: story:5-121
#[test]
fn a_session_that_may_move_shows_one_tls_fingerprint_on_the_http_and_browser_tiers() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let https_url = proving_ground
        .url(HOST, "/corpus/article?n=2")
        .replacen("http://", "https://", 1);

    let moving = reach_over_tls(&mut engine, &proving_ground, "moving", "auto", &https_url);
    let seen = handshakes_so_far(&proving_ground);
    let staying = reach_over_tls(&mut engine, &proving_ground, "staying", "http", &https_url);
    assert!(engine.finished());

    let reqwest = handshakes(&proving_ground, HOST, seen);
    let all = handshakes(&proving_ground, HOST, 0);
    let moving_handshakes = &all[..all.len() - reqwest.len()];
    assert_eq!(
        (moving, staying),
        (vec![json!("http"), json!("browser")], vec![json!("http")])
    );
    assert!(moving_handshakes.len() >= 2, "{moving_handshakes:?}");
    assert!(
        moving_handshakes
            .iter()
            .all(|ja4| ja4 == &moving_handshakes[0]),
        "one fingerprint across the tiers: {moving_handshakes:?}"
    );
    assert!(
        !reqwest.is_empty() && reqwest.iter().all(|ja4| ja4 != &moving_handshakes[0]),
        "reqwest {reqwest:?}, the browser {moving_handshakes:?}"
    );
}

/// The fetch edge pages a late cut breaks off: half the body, then the
/// connection closed 75 ms later, once the browser reads the body as it
/// arrives (#1732). A chunked body and a gzip one, which no length tells
/// cut: a plain body short of its `Content-Length` is the unit tests'.
const LATE_CUTS: [&str; 2] = ["/cut/late/chunked", "/cut/late/gzip-content-length"];

/// How many times each late cut is crawled: the pinned Chromium lost the
/// failure of 4 of 30 such browser-tier crawls before the fix.
const COPIES: usize = 8;

/// The longest the crawl of the late cuts keeps the engine silent: under
/// `fetch.tier: browser_fetch` no handler is asked, so its one message is
/// its end, after each of its 32 loads (about 2 s each with its page).
const LATE_CUTS_CRAWLED: Duration = Duration::from_secs(240);

/// What each request of the Run `run_id` ended as, by URL: its kind, its
/// Error code and its attempts.
fn ended(engine: &mut Attached, run_id: &str) -> BTreeMap<String, Value> {
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    answer["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {answer}"))
        .iter()
        .map(|outcome| {
            (
                outcome["request"]["url"].as_str().unwrap().to_owned(),
                json!([
                    outcome["kind"],
                    outcome["error"]["code"],
                    outcome["attempts"]
                ]),
            )
        })
        .collect()
}

/// A body broken off once the browser was reading it, chunked or coded,
/// which no length tells cut, fails each attempt with `response_truncated`
/// on the browser-fetch tier and on the browser tier alike, though the pinned Chromium's interception, which
/// reads the body at its response stage, loses the network's failure of a
/// few such loads (#1732): each is retried once under `crawl.retry.max`,
/// then ends `failed`, never handed to the handler. The site processed
/// each attempt, so none is sent again: it sees two requests for each.
///
/// Covers: story:5-126, setting:crawl.retry.max, error:response_truncated
#[rstest::rstest]
#[case::browser_fetch("browser_fetch")]
#[case::browser("browser")]
fn a_body_cut_once_the_browser_read_it_fails_each_attempt_on_the_browser_tiers(#[case] tier: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine_silent_for(&env, LATE_CUTS_CRAWLED);
    let mut settings = settings(&proving_ground, tier);
    settings["site_preferences"] = json!({ "respect_robots_txt": false });
    settings["crawl"] = json!({ "retry": { "max": 1 } });
    settings["politeness"] = json!({ "domain": { "delay": 0 } });
    let targets: Vec<String> = LATE_CUTS
        .iter()
        .flat_map(|path| (1..=COPIES).map(move |copy| format!("{path}?copy={copy}")))
        .collect();
    let seeds: Vec<Value> = targets
        .iter()
        .map(|target| json!({ "url": proving_ground.url("fetch-edge.test", target), "label": "page" }))
        .collect();

    let mut handled = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "late-cuts",
        &seeds,
        &json!({ "page": {} }),
        &settings,
        &mut |_, handle| {
            handled.push(handle["crawl_request"]["url"].clone());
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, "late-cuts");
    assert!(engine.finished());

    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
    assert_eq!(handled, Vec::<Value>::new(), "handed to the handler");
    let otherwise: BTreeMap<&String, &Value> = ended
        .iter()
        .filter(|(_, ended)| **ended != json!(["failed", "response_truncated", 2]))
        .collect();
    assert_eq!(otherwise, BTreeMap::new(), "of {} late cuts", targets.len());
    assert_eq!(ended.len(), targets.len(), "{ended:?}");
    let sent: Vec<(&String, usize)> = targets
        .iter()
        .map(|target| {
            (
                target,
                requests_for(&proving_ground, "fetch-edge.test", target),
            )
        })
        .filter(|(_, sent)| *sent != 2)
        .collect();
    assert_eq!(
        sent,
        Vec::<(&String, usize)>::new(),
        "requests other than two"
    );
}
