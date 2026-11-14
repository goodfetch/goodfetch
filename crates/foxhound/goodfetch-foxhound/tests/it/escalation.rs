//! A Challenge a crawl's pool Session meets (spec #600's `escalation`, the
//! escalation cases; ADRs 0020, 0021, 0035, 0093 and 0104), on Foxhound's
//! executable against the Proving ground and Chrome for Testing, the
//! Session ownership case of a crawl's Session pool: a Challenge met on a
//! tier short of the browser moves its request to the browser in the same
//! attempt, and its domain's later requests start there; under a forced
//! `fetch.tier: http` it ends `blocked` with its cause, never moved; a
//! Challenge the program's own code names moves as one the Block
//! classification found does; and a Challenge met on the browser ends
//! `blocked`, since no Challenge ladder runs for a pool Session yet.
//!
//! Under `fetch.http_from: transport`, a request a Session's own transport
//! fetched runs again in another Session, one whose client is its browser
//! (#2416).
//!
//! Each crawl's Label samples one request (`fetch.sample.count: 1`) on a
//! page the tiers serve alike, so its later requests run on `http` under
//! `auto`, not as samples, whose browser run would hide the move.

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings, tiers};
use crate::frontier::crawl_declaring;

/// The Proving ground's seed.
const SEED: u64 = 1130;

/// The host of the `access` family's corpus pages.
const HOST: &str = "access.test";

/// The page the tiers serve alike, which the Label's one sample reads.
const SAMPLED: &str = "/corpus/article?n=1";

fn seeds(proving_ground: &ProvingGround, paths: &[&str]) -> Vec<Value> {
    paths
        .iter()
        .map(|path| json!({ "url": proving_ground.url(HOST, path), "label": "page" }))
        .collect()
}

/// The settings of a crawl on `tier`, its Label sampling one request, with
/// one Session in each pool at a time.
fn crawl_settings(proving_ground: &ProvingGround, tier: &str) -> Value {
    settings(
        proving_ground,
        &json!({
            "fetch": { "tier": tier, "sample": { "count": 1 } },
            "session": { "pool": { "max_size": 1 } },
        }),
    )
}

/// Each handler request as the handler saw it: the path, the pool Session
/// that served it and its tier.
fn seen(handle: &Value) -> (String, Value, Value) {
    let url = handle["crawl_request"]["url"].as_str().unwrap();
    let path = &url[url.find("/corpus").unwrap()..];
    (
        path.to_owned(),
        handle["session"]["id"].clone(),
        handle["tier"].clone(),
    )
}

fn handled(path: &str, session: &str, tier: &str) -> (String, Value, Value) {
    (path.to_owned(), json!(session), json!(tier))
}

/// The kind each request of `run_id` ended with and its Block cause, by
/// its path.
fn ended(engine: &mut Attached, run_id: &str) -> Vec<(String, Value, Value)> {
    let answer = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": run_id }),
        &mut |_| None,
    );
    let mut ended: Vec<(String, Value, Value)> = answer["result"]["outcomes"]
        .as_array()
        .unwrap_or_else(|| panic!("no outcomes: {answer}"))
        .iter()
        .map(|outcome| {
            let url = outcome["request"]["url"].as_str().unwrap();
            (
                url[url.find("/corpus").unwrap()..].to_owned(),
                outcome["kind"].clone(),
                outcome["cause"].clone(),
            )
        })
        .collect();
    ended.sort_by(|left, right| left.0.cmp(&right.0));
    ended
}

/// How many requests for `path` the Proving ground received from the case:
/// one per tier that fetched it.
fn fetched(proving_ground: &ProvingGround, path: &str) -> usize {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.target == path)
        .count()
}

/// The Challenge page `/corpus/interstitial` answers over HTTP moves its
/// request, on `http` under `auto`, to the browser in the same attempt:
/// its handler never sees the Challenge page, the browser meets the same
/// Challenge, where no ladder runs yet, so the request ends `blocked` (two
/// requests reached the page: the HTTP tier's and the browser's), and the
/// domain's next request starts on the browser.
///
/// Covers: story:8-209
#[test]
fn a_challenge_met_over_http_moves_its_request_to_the_browser_and_its_domain_starts_there() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut handled_by = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "moved",
        &seeds(
            &proving_ground,
            &[SAMPLED, "/corpus/interstitial", "/corpus/vendor-script"],
        ),
        &json!({ "page": {} }),
        &crawl_settings(&proving_ground, "auto"),
        &mut |_, handle| {
            handled_by.push(seen(handle));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, "moved");
    assert!(engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            handled_by,
            tiers(&env, "moved"),
            ended,
            fetched(&proving_ground, "/corpus/interstitial"),
        ),
        (
            &json!("finished"),
            vec![
                handled(SAMPLED, "session-1", "http"),
                handled(SAMPLED, "session-1", "browser"),
                handled("/corpus/vendor-script", "session-2", "browser"),
            ],
            vec![
                json!(["page", "browser_fetch", "sample", null, null]),
                json!(["page", "browser", "sample", null, null]),
                json!(["page", "browser_fetch", "compared", null, null]),
                json!(["page", "browser", "compared", null, null]),
            ],
            vec![
                (SAMPLED.to_owned(), json!("succeeded"), Value::Null),
                (
                    "/corpus/interstitial".to_owned(),
                    json!("blocked"),
                    json!("challenge")
                ),
                (
                    "/corpus/vendor-script".to_owned(),
                    json!("succeeded"),
                    Value::Null
                ),
            ],
            2,
        )
    );
}

/// Under `fetch.tier: http` forced, the same Challenge page ends its
/// request `blocked` with its cause, and nothing moves it: the browser
/// never loads it, and the domain's next request stays on `http`.
///
/// Covers: story:8-210
#[test]
fn a_challenge_under_a_forced_http_tier_ends_its_request_blocked_and_never_moves_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut handled_by = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "forced",
        &seeds(
            &proving_ground,
            &["/corpus/interstitial", "/corpus/vendor-script"],
        ),
        &json!({ "page": {} }),
        &crawl_settings(&proving_ground, "http"),
        &mut |_, handle| {
            handled_by.push(seen(handle));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, "forced");
    assert!(engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            handled_by,
            tiers(&env, "forced"),
            ended,
            fetched(&proving_ground, "/corpus/interstitial"),
        ),
        (
            &json!("finished"),
            vec![handled("/corpus/vendor-script", "session-2", "http")],
            vec![
                json!(["page", "http", "forced", "run", null]),
                json!(["page", "http", "forced", "run", null]),
            ],
            vec![
                (
                    "/corpus/interstitial".to_owned(),
                    json!("blocked"),
                    json!("challenge")
                ),
                (
                    "/corpus/vendor-script".to_owned(),
                    json!("succeeded"),
                    Value::Null
                ),
            ],
            1,
        )
    );
}

/// With `fetch.block_classification` off, the same Challenge page under
/// the forced `http` tier is read by no Block classification rule: its
/// handler is given it as the page it is, and its request succeeds, fetched
/// once (ADR 0138).
///
/// Covers: story:8-210, setting:fetch.block_classification
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn with_block_classification_off_a_challenge_page_is_handled_as_it_is() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut handled_by = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "unclassified",
        &seeds(&proving_ground, &["/corpus/interstitial"]),
        &json!({ "page": {} }),
        &settings(
            &proving_ground,
            &json!({
                "fetch": { "tier": "http", "sample": { "count": 1 }, "block_classification": false },
                "session": { "pool": { "max_size": 1 } },
            }),
        ),
        &mut |_, handle| {
            handled_by.push(seen(handle));
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, "unclassified");
    assert!(engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            handled_by,
            ended,
            fetched(&proving_ground, "/corpus/interstitial"),
        ),
        (
            &json!("finished"),
            vec![handled("/corpus/interstitial", "session-1", "http")],
            vec![(
                "/corpus/interstitial".to_owned(),
                json!("succeeded"),
                Value::Null
            )],
            1,
        )
    );
}

/// A Challenge the program's own code names on a page served over HTTP,
/// as an after-response Hook's `block(cause)` will (ADR 0104), takes the
/// same way as one the rules found: the same attempt runs again in the
/// browser, in the same Session, where its page passes, and the domain's
/// next request starts on the browser.
///
/// Covers: story:8-216, story:8-209
#[test]
fn a_challenge_the_programs_code_names_moves_its_request_to_the_browser_as_a_detected_one_does() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut handled_by = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "named",
        &seeds(
            &proving_ground,
            &[SAMPLED, "/corpus/article?n=2", "/corpus/article?n=3"],
        ),
        &json!({ "page": {} }),
        &crawl_settings(&proving_ground, "auto"),
        &mut |_, handle| {
            let (path, session, tier) = seen(handle);
            handled_by.push((path.clone(), session, tier.clone()));
            if path == "/corpus/article?n=2" && tier == "http" {
                json!({ "error": { "code": -32003, "message": "a soft block", "data": {
                    "code": "blocked", "retryable": false, "cause": "challenge",
                } } })
            } else {
                json!({ "result": { "records": [] } })
            }
        },
    );
    let ended = ended(&mut engine, "named");
    assert!(engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            handled_by,
            tiers(&env, "named"),
            ended
                .iter()
                .map(|(_, kind, _)| kind.clone())
                .collect::<Vec<_>>(),
        ),
        (
            &json!("finished"),
            vec![
                handled(SAMPLED, "session-1", "http"),
                handled(SAMPLED, "session-1", "browser"),
                handled("/corpus/article?n=2", "session-1", "http"),
                handled("/corpus/article?n=2", "session-1", "browser"),
                handled("/corpus/article?n=3", "session-1", "browser"),
            ],
            vec![
                json!(["page", "browser_fetch", "sample", null, null]),
                json!(["page", "browser", "sample", null, null]),
                json!(["page", "browser_fetch", "compared", null, null]),
                json!(["page", "browser", "escalation", null, "blocked"]),
                json!(["page", "browser", "compared", null, null]),
            ],
            vec![json!("succeeded"); 3],
        )
    );
}

/// A Challenge a handler's own `goto` meets on its page served over HTTP,
/// under `auto`, takes the same way as one its request's own document
/// met: the attempt runs again in the browser, in the same Session, its
/// HTTP run discarded, and the domain's next request starts on the browser
/// (spec #600; #1130).
///
/// Covers: story:8-209, operation:page.goto
#[test]
fn a_challenge_a_handlers_goto_meets_over_http_moves_its_request_to_the_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let challenge = proving_ground.url(HOST, "/corpus/interstitial");

    let (mut handled_by, mut went) = (Vec::new(), Value::Null);
    let finished = crawl_declaring(
        &mut engine,
        "navigated",
        &seeds(
            &proving_ground,
            &[SAMPLED, "/corpus/article?n=2", "/corpus/article?n=3"],
        ),
        &json!({ "page": {} }),
        &crawl_settings(&proving_ground, "auto"),
        &mut |inside, handle| {
            let (path, session, tier) = seen(handle);
            handled_by.push((path.clone(), session, tier.clone()));
            if path == "/corpus/article?n=2" && tier == "http" {
                went = inside.call(
                    "page.goto",
                    &json!({ "page_id": handle["page_id"], "url": challenge }),
                );
            }
            json!({ "result": { "records": [] } })
        },
    );
    let ended = ended(&mut engine, "navigated");
    assert!(engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            &went["error"]["data"]["code"],
            handled_by,
            tiers(&env, "navigated"),
            ended
                .iter()
                .map(|(_, kind, _)| kind.clone())
                .collect::<Vec<_>>(),
        ),
        (
            &json!("finished"),
            &json!("attempt_ended"),
            vec![
                handled(SAMPLED, "session-1", "http"),
                handled(SAMPLED, "session-1", "browser"),
                handled("/corpus/article?n=2", "session-1", "http"),
                handled("/corpus/article?n=2", "session-1", "browser"),
                handled("/corpus/article?n=3", "session-1", "browser"),
            ],
            vec![
                json!(["page", "browser_fetch", "sample", null, null]),
                json!(["page", "browser", "sample", null, null]),
                json!(["page", "browser_fetch", "compared", null, null]),
                json!(["page", "browser", "escalation", null, "blocked"]),
                json!(["page", "browser", "compared", null, null]),
            ],
            vec![json!("succeeded"); 3],
        ),
        "{went}"
    );
}

/// Under `fetch.tier: http` forced, the same `goto` raises `blocked` with
/// the Challenge's cause, and a handler that lets it escape ends its
/// request `blocked`, never moved: the browser never loads the page.
///
/// Covers: story:8-210, error:blocked, operation:page.goto
#[test]
fn a_challenge_a_handlers_goto_meets_under_a_forced_http_tier_raises_blocked() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let challenge = proving_ground.url(HOST, "/corpus/interstitial");

    let (mut handled_by, mut went) = (Vec::new(), Value::Null);
    let finished = crawl_declaring(
        &mut engine,
        "navigated-forced",
        &seeds(&proving_ground, &["/corpus/article?n=2"]),
        &json!({ "page": {} }),
        &crawl_settings(&proving_ground, "http"),
        &mut |inside, handle| {
            handled_by.push(seen(handle));
            went = inside.call(
                "page.goto",
                &json!({ "page_id": handle["page_id"], "url": challenge }),
            );
            json!({ "error": went["error"] })
        },
    );
    let ended = ended(&mut engine, "navigated-forced");
    assert!(engine.finished());

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            (
                &went["error"]["data"]["code"],
                &went["error"]["data"]["cause"]
            ),
            handled_by,
            ended,
        ),
        (
            &json!("finished"),
            (&json!("blocked"), &json!("challenge")),
            vec![handled("/corpus/article?n=2", "session-1", "http")],
            vec![(
                "/corpus/article?n=2".to_owned(),
                json!("blocked"),
                json!("challenge")
            )],
        ),
        "{went}"
    );
}

/// The host whose storage page answers a `mark` cookie and whose quiet
/// page answers none.
const MARKS_HOST: &str = "scope.test";

/// The settings of a crawl under `auto` whose Label sends its HTTP from
/// the transport (`fetch.http_from: transport`) and samples one request,
/// one request at a time, its Frontier learning nothing that would skip a
/// page reading alike.
fn from_the_transport(proving_ground: &ProvingGround) -> Value {
    settings(
        proving_ground,
        &json!({
            "fetch": { "tier": "auto", "http_from": "transport", "sample": { "count": 1 } },
            "politeness": { "domain": { "concurrency": 1 } },
            "frontier": { "learning": false },
        }),
    )
}

/// Each handler request as the handler saw it, by the target its URL asks
/// [`MARKS_HOST`] for: the target, the pool Session that served it and its
/// tier.
fn seen_marked(handle: &Value) -> (String, Value, Value) {
    let url = handle["crawl_request"]["url"].as_str().unwrap();
    let path = url.split_once(MARKS_HOST).unwrap().1;
    let path = &path[path.find('/').unwrap()..];
    (
        path.to_owned(),
        handle["session"]["id"].clone(),
        handle["tier"].clone(),
    )
}

/// Under `fetch.http_from: transport`, a call only the browser serves,
/// `page.evaluate`, made on a page a Session's own transport fetched for a
/// Label learned onto `http`, runs the request again in the same attempt
/// in another Session, one whose client is its browser: nothing of the
/// first Session is carried, so the browser's request carries none of the
/// cookie the first Session's transport sent, and the attempt commits once
/// (#2416). Two Sessions at a time make the transport's second request
/// reuse the Session its first one's answer set the cookie in.
///
/// Covers: story:8-209, setting:fetch.http_from, operation:page.evaluate
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn under_transport_a_call_only_the_browser_serves_runs_again_in_a_browser_session() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let origin = proving_ground.url(MARKS_HOST, "");
    let escalated = "/storage?mark=e-3";
    let mut settings = from_the_transport(&proving_ground);
    settings["session"] = json!({ "pool": { "max_size": 2 } });

    let (mut handled_by, mut evaluated) = (Vec::new(), Vec::new());
    let finished = crawl_declaring(
        &mut engine,
        "escalated-across-clients",
        &["/quiet?mark=e-1", "/storage?mark=e-2", escalated]
            .map(|path| json!({ "url": format!("{origin}{path}"), "label": "page" })),
        &json!({ "page": {} }),
        &settings,
        &mut |inside, handle| {
            let (path, session, tier) = seen_marked(handle);
            handled_by.push((path.clone(), session, tier.clone()));
            if path == escalated {
                let reply = inside.call(
                    "page.evaluate",
                    &json!({ "page_id": handle["page_id"], "function": "() => document.title" }),
                );
                evaluated.push((tier, reply.get("error").is_some()));
                if reply.get("error").is_some() {
                    return json!({ "error": reply["error"] });
                }
            }
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "path": path } }] } })
        },
    );
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "escalated-across-clients" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let cookies: Vec<Vec<(String, String)>> = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == MARKS_HOST && request.target == escalated)
        .map(|request| request.cookies.clone())
        .collect();
    let session = |path: &str, tier: &str| -> Vec<Value> {
        handled_by
            .iter()
            .filter(|(seen, _, seen_tier)| seen == path && *seen_tier == json!(tier))
            .map(|(_, session, _)| session.clone())
            .collect()
    };
    let sent = session(escalated, "http");
    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            evaluated,
            sent == session("/storage?mark=e-2", "http"),
            sent.len(),
            sent == session(escalated, "browser"),
            cookies,
            outcomes["result"]["outcomes"]
                .as_array()
                .map(|outcomes| outcomes
                    .iter()
                    .map(|outcome| (outcome["kind"].clone(), outcome["attempts"].clone()))
                    .collect::<Vec<_>>()),
        ),
        (
            &json!("finished"),
            vec![(json!("http"), true), (json!("browser"), false)],
            true,
            1,
            false,
            vec![vec![("mark".to_owned(), "e-2".to_owned())], Vec::new()],
            Some(vec![(json!("succeeded"), json!(1)); 3]),
        ),
        "{handled_by:?}\n{outcomes}"
    );
}

/// Under `fetch.http_from: transport`, a Challenge met on a page a
/// Session's own transport fetched moves its domain to the browser as
/// under `browser`, and the request runs again in the same attempt in
/// another Session, one whose client is its browser; the domain's next
/// request starts on the browser (#2416).
///
/// Covers: story:8-216, story:8-209, setting:fetch.http_from
#[test]
#[expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
fn under_transport_a_challenge_moves_its_request_to_a_browser_session() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);

    let mut handled_by = Vec::new();
    let finished = crawl_declaring(
        &mut engine,
        "moved-across-clients",
        &seeds(
            &proving_ground,
            &[SAMPLED, "/corpus/article?n=2", "/corpus/article?n=3"],
        ),
        &json!({ "page": {} }),
        &from_the_transport(&proving_ground),
        &mut |_, handle| {
            let (path, session, tier) = seen(handle);
            handled_by.push((path.clone(), session, tier.clone()));
            if path == "/corpus/article?n=2" && tier == "http" {
                json!({ "error": { "code": -32003, "message": "a soft block", "data": {
                    "code": "blocked", "retryable": false, "cause": "challenge",
                } } })
            } else {
                json!({ "result": { "records": [] } })
            }
        },
    );
    let ended = ended(&mut engine, "moved-across-clients");
    assert!(engine.finished());

    let of = |path: &str| -> Vec<(Value, Value)> {
        handled_by
            .iter()
            .filter(|(seen, _, _)| seen == path)
            .map(|(_, session, tier)| (session.clone(), tier.clone()))
            .collect()
    };
    let moved = of("/corpus/article?n=2");
    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            moved
                .iter()
                .map(|(_, tier)| tier.clone())
                .collect::<Vec<_>>(),
            moved.first().map(|(session, _)| session) == moved.get(1).map(|(session, _)| session),
            of("/corpus/article?n=3")
                .into_iter()
                .map(|(_, tier)| tier)
                .collect::<Vec<_>>(),
            tiers(&env, "moved-across-clients")[3].clone(),
            ended
                .iter()
                .map(|(_, kind, _)| kind.clone())
                .collect::<Vec<_>>(),
        ),
        (
            &json!("finished"),
            vec![json!("http"), json!("browser")],
            false,
            vec![json!("browser")],
            json!(["page", "browser", "escalation", null, "blocked"]),
            vec![json!("succeeded"); 3],
        ),
        "{handled_by:?}"
    );
}
