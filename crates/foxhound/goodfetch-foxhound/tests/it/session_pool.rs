//! A crawl's Session pool on Foxhound's executable over stdio (spec #581's
//! seam: the protocol against the Proving ground; ADRs 0006, 0021 and
//! 0088): each `crawl_request.handle` names the pool's Session that serves
//! it; the pool's settings retire a Session by its uses, its age and its
//! error score; a block on the `access` family's corpus pages retires its
//! Session, the replacement opening at the next proxy tier when the block
//! escalates (the Session ownership and proxy limit conformance cases, the
//! crawl pool's part); and a crawl whose engine is killed keeps its
//! Sessions when it resumes, each revalidated on its first page. Until the
//! engine detects blocks itself (area 5), the test's handler names a page's
//! Block cause from its status and headers, as the block-detection rules
//! will.
#![expect(
    clippy::disallowed_types,
    reason = "the test's handler fetches the Proving ground's pages from outside the engine (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test's handler takes its time on its own thread, as a slow handler does (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{Read as _, Write as _};
use std::time::Duration;

use goodfetch_test_client::Attached;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};

/// The Proving ground's seed.
const SEED: u64 = 920;

/// The Block cause the test's handler reads in a response's head, `None`
/// when it is no block: a Challenge's `cf-mitigated` header, a 429 and a
/// 403.
fn block_cause(head: &str) -> Option<&'static str> {
    let status: u16 = head.split(' ').nth(1).unwrap().parse().unwrap();
    if head
        .to_ascii_lowercase()
        .contains("cf-mitigated: challenge")
    {
        Some("challenge")
    } else if status == 429 {
        Some("rate_limit")
    } else if status == 403 {
        Some("ip_block")
    } else {
        None
    }
}

/// Fetches the crawl request's URL, a page of the Proving ground, and
/// answers as a handler does: its block with the Error code `blocked`, or
/// a Record naming the page.
fn handle(proving_ground: &ProvingGround, params: &Value) -> Value {
    let url = params["crawl_request"]["url"].as_str().unwrap();
    let authority_and_path = url.strip_prefix("http://").unwrap();
    let (authority, path) = authority_and_path.split_at(authority_and_path.find('/').unwrap());
    let mut stream = std::net::TcpStream::connect(proving_ground.address()).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, _) = response.split_once("\r\n\r\n").unwrap();
    match block_cause(head) {
        Some(cause) => failure("blocked", &json!({ "cause": cause })),
        None => {
            json!({ "result": { "records": [{ "dataset": "pages", "data": { "url": url } }] } })
        }
    }
}

fn failure(code: &str, details: &Value) -> Value {
    let mut data = json!({ "code": code, "retryable": false });
    data.as_object_mut()
        .unwrap()
        .extend(details.as_object().unwrap().clone());
    json!({ "error": { "code": -32003, "message": "the page failed", "data": data } })
}

/// The settings of a crawl that hands its handler one request at a time,
/// with `pool` as its `session.pool` settings.
fn settings(pool: &Value) -> Value {
    json!({ "crawl": { "handler": { "concurrency": 1 } }, "session": { "pool": pool } })
}

fn seeds(proving_ground: &ProvingGround, paths: &[&str]) -> Vec<Value> {
    paths
        .iter()
        .map(|path| json!({ "url": proving_ground.url("access.test", path), "label": "page" }))
        .collect()
}

fn session(id: &str, proxy_tier: &str, revalidate: bool) -> Value {
    json!({ "id": id, "proxy_tier": proxy_tier, "revalidate": revalidate })
}

/// Starts the crawl `run_id`, answers each handler request with
/// `handler`, which sees how many came before it, until `run.finish`
/// answers, and answers each request's Session in order and the Run's
/// lifecycle.
fn crawl(
    engine: &mut Attached,
    run_id: &str,
    seeds: &[Value],
    settings: &Value,
    handler: &mut impl FnMut(usize, &Value) -> Value,
) -> (Vec<Value>, Value) {
    let mut sessions = Vec::new();
    let mut answer = |params: &Value| {
        sessions.push(params["session"].clone());
        Some(handler(sessions.len() - 1, params))
    };
    let started = engine.call(
        "start",
        "crawl.start",
        &json!({ "run_id": run_id, "seeds": seeds, "settings": settings, "labels": { "page": {} } }),
        &mut answer,
    );
    assert_eq!(started["result"]["run_id"], run_id, "{started}");
    let finished = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": run_id }),
        &mut answer,
    );
    (sessions, finished["result"]["lifecycle"].clone())
}

/// How the handler answers a crawl's first request.
#[derive(Clone, Copy)]
enum First {
    /// As it answers every other.
    Succeeds,
    /// With this Error code.
    Fails(&'static str),
    /// As every other, after this long.
    Lasts(Duration),
}

/// Covers: story:6-250, setting:session.pool.max_size, setting:session.pool.max_uses, setting:session.pool.max_age, setting:session.pool.max_error_score
#[rstest]
#[case::by_its_uses(
    json!({ "max_size": 1, "max_uses": 2 }),
    &["/corpus", "/corpus/article", "/corpus/vendor-script"],
    First::Succeeds,
    vec!["session-1", "session-1", "session-2"],
)]
#[case::by_its_age(
    json!({ "max_size": 1, "max_age": 500 }),
    &["/corpus", "/corpus/article"],
    First::Lasts(Duration::from_millis(600)),
    vec!["session-1", "session-2"],
)]
#[case::by_its_error_score(
    json!({ "max_size": 1, "max_error_score": 1 }),
    &["/corpus", "/corpus/article"],
    First::Fails("page_check_failed"),
    vec!["session-1", "session-2"],
)]
#[case::serving_while_it_may(
    json!({ "max_size": 1 }),
    &["/corpus", "/corpus/article", "/corpus/vendor-script"],
    First::Succeeds,
    vec!["session-1", "session-1", "session-1"],
)]
fn the_pools_settings_retire_a_session_by_its_uses_its_age_and_its_error_score(
    #[case] pool: Value,
    #[case] paths: &[&str],
    #[case] first: First,
    #[case] served_by: Vec<&str>,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let (sessions, lifecycle) = crawl(
        &mut engine,
        "pooled",
        &seeds(&proving_ground, paths),
        &settings(&pool),
        &mut |served, params| match first {
            First::Fails(code) if served == 0 => failure(code, &json!({})),
            First::Lasts(pause) if served == 0 => {
                std::thread::sleep(pause);
                handle(&proving_ground, params)
            }
            First::Succeeds | First::Fails(_) | First::Lasts(_) => handle(&proving_ground, params),
        },
    );
    assert!(engine.finished());

    let ids: Vec<&str> = sessions
        .iter()
        .map(|session| session["id"].as_str().unwrap())
        .collect();
    assert_eq!((ids, lifecycle), (served_by, json!("finished")));
}

/// Covers: story:6-251, story:5-138, setting:session.pool.max_size
#[rstest]
#[case::a_403("/corpus/ip-block", true, session("session-2", "datacenter", false))]
#[case::a_429("/corpus/rate-limit", true, session("session-2", "datacenter", false))]
// A challenge page is read by the engine on the document it fetched: the
// Session is retired before any handler is given the request.
#[case::a_challenge("/corpus/interstitial", false, session("session-2", "none", false))]
fn a_block_retires_its_session_and_one_that_escalates_opens_the_replacement_at_the_next_proxy_tier(
    #[case] blocked: &str,
    #[case] handed_over: bool,
    #[case] replacement: Value,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let (sessions, lifecycle) = crawl(
        &mut engine,
        "escalated",
        &seeds(&proving_ground, &[blocked, "/corpus/article"]),
        &settings(&json!({ "max_size": 1 })),
        &mut |_, params| handle(&proving_ground, params),
    );
    assert!(engine.finished());

    let first = handed_over.then(|| session("session-1", "none", false));
    assert_eq!(
        (sessions, lifecycle),
        (
            first.into_iter().chain([replacement]).collect::<Vec<_>>(),
            json!("finished")
        )
    );
}

/// Covers: story:6-255, operation:run.resume
#[test]
fn a_crawl_killed_and_resumed_keeps_its_sessions_and_revalidates_each_on_its_first_page() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = seeds(
        &proving_ground,
        &[
            "/corpus",
            "/corpus/article",
            "/corpus/vendor-script",
            "/corpus/not-found",
        ],
    );
    // At `full`, each page's claim, which writes the pool's Sessions, is in
    // the file before its page is served: at `batch` the kill takes the
    // open batch, and with it the Sessions it opened (ADR 0142). Growth is
    // `eager`, so the one request in flight at a time opens a Session of
    // its own while the pool has room and the kill finds two to keep.
    let mut settings = settings(&json!({ "max_size": 2, "growth": "eager" }));
    settings["run_state"] = json!({ "durability": "full" });
    let mut killed = crate::site::engine_on(&env, &proving_ground);
    let mut before = Vec::new();
    killed.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "kept", "seeds": seeds, "settings": settings, "labels": { "page": {} } },
    }));
    while before.len() < 3 {
        let message = killed.next().unwrap();
        if message["method"] == "crawl_request.handle" {
            before.push(message["params"]["session"].clone());
            if before.len() < 3 {
                let mut reply = handle(&proving_ground, &message["params"]);
                reply["jsonrpc"] = json!("2.0");
                reply["id"] = message["id"].clone();
                killed.send(&reply);
            }
        }
    }
    killed.kill();
    let mut resuming = crate::site::engine_on(&env, &proving_ground);

    let mut after: Vec<Value> = Vec::new();
    let mut answer = |params: &Value| {
        after.push(params["session"].clone());
        Some(handle(&proving_ground, params))
    };
    let resumed = resuming.call(
        "resume",
        "run.resume",
        &json!({ "run_id": "kept", "settings": settings }),
        &mut answer,
    );
    let finished = resuming.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "kept" }),
        &mut answer,
    );
    assert!(resuming.finished());

    let first_seen = |n: usize, served: &Value| {
        !after[..n]
            .iter()
            .any(|earlier: &Value| earlier["id"] == served["id"])
    };
    let kept: Vec<Value> = after
        .iter()
        .enumerate()
        .map(|(n, served)| {
            let id = served["id"].as_str().unwrap();
            let id = if ["session-1", "session-2"].contains(&id) {
                id
            } else {
                "another"
            };
            session(id, "none", first_seen(n, served))
        })
        .collect();
    assert_eq!(
        (
            &before[..2],
            ["session-1", "session-2"].contains(&before[2]["id"].as_str().unwrap()),
            after.clone(),
            resumed.get("error").cloned(),
            &finished["result"]["lifecycle"]
        ),
        (
            &[
                session("session-1", "none", false),
                session("session-2", "none", false),
            ][..],
            true,
            kept,
            None,
            &json!("finished"),
        )
    );
}

/// The grilling's case of G-06 (#2178), at two places where it had 32: a
/// browser crawl with three requests in flight and two browser-page places
/// opens two Sessions of the browser client, one per place, and the third
/// request opens none: it waits for one of the two to have room and runs
/// in it. Each Session makes its one browser context. The first draw also
/// opens `session-2`, a Session's own transport that reads the site's
/// files (#2422), so the browser's two are `session-1` and `session-3`.
///
/// Covers: setting:session.pool.growth, setting:governor.browser_page.ceiling
#[test]
fn browser_requests_past_the_places_wait_for_a_session_with_room_and_open_none() {
    let env = TestEnv::new();
    env.user_configuration("[settings.governor.browser_page]\nfloor = 2\nceiling = 2\n");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = crate::fetch_tiers::engine(&env);
    let settings = crate::fetch_tiers::settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "browser" },
            "crawl": { "handler": { "concurrency": 3 } },
            "politeness": { "domain": { "concurrency": 3 } },
            "browser": { "cdp_command_log": true },
        }),
    );
    let seeds = seeds(
        &proving_ground,
        &[
            "/corpus/article?n=1",
            "/corpus/article?n=2",
            "/corpus/article?n=3",
        ],
    );

    // The first requests are held unanswered until two are in flight
    // together, so the third is drawn while both Sessions are busy.
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": {
            "run_id": "placed", "seeds": seeds, "settings": settings, "labels": { "page": {} },
        },
    }));
    let mut held = Vec::new();
    while held.len() < 2 {
        let message = engine.next().unwrap();
        assert!(message.get("error").is_none(), "{message}");
        if message["method"] == "crawl_request.handle" {
            held.push(message);
        }
    }
    let mut sessions: Vec<Value> = held
        .iter()
        .map(|message| message["params"]["session"]["id"].clone())
        .collect();
    for message in held {
        let mut reply = handle(&proving_ground, &message["params"]);
        reply["jsonrpc"] = json!("2.0");
        reply["id"] = message["id"].clone();
        engine.send(&reply);
    }
    let finished = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "placed" }),
        &mut |params: &Value| {
            sessions.push(params["session"]["id"].clone());
            Some(handle(&proving_ground, params))
        },
    );
    assert!(engine.finished());

    let opened: std::collections::BTreeSet<&str> =
        sessions.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        (
            sessions.len(),
            opened,
            crate::browser_fetch::contexts_created(&env, "placed"),
            &finished["result"]["lifecycle"],
        ),
        (3, ["session-1", "session-3"].into(), 2, &json!("finished"),),
        "{sessions:?}"
    );
}
