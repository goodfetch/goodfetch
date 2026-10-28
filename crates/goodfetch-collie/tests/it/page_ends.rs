//! A page or a browser that ends under a call, on Collie's executable over
//! stdio (spec #594; ADRs 0040, 0113 and 0128; pitfall B24): a renderer
//! killed mid-call raises `page_crashed` and emits `crash`; a browser
//! killed by a signal raises `page_crashed` with how it exited, counts in
//! Usage and leaves its Session `Crashed`, which a close releases without
//! changing; a browser quit raises `call_cancelled` naming the browser; and
//! a Session closed while a call is in flight raises `call_cancelled` naming
//! the program. Each test has its own Goodfetch home, Proving ground and
//! Browser cache (ADRs 0059 and 0094); the processes it kills are the
//! test's engine's own browser's.
#![expect(
    clippy::disallowed_methods,
    reason = "the test lists and signals its own browser's processes on its own thread, never an async worker"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::process::Command;
use std::time::Duration;

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use serde_json::{Map, Value, json};

use crate::page::{opened, started_with};
use crate::page_events::Exchange;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 731;

/// How long a locator action's call has been in flight when the test ends
/// its page or browser: the guard is then waiting for an element that
/// never comes.
const IN_FLIGHT: Duration = Duration::from_millis(1500);

/// A selector no Proving ground page matches.
const NOWHERE: &str = "#no-such-element";

/// The process IDs of the test's engine's browser processes whose command
/// line `holds` (ADR 0062: its profile is in the test's folder).
fn browser_processes(env: &TestEnv, holds: impl Fn(&str) -> bool) -> Vec<String> {
    let name = env.folder().file_name().unwrap().to_string_lossy();
    let marker = name.rsplit(' ').next().unwrap().to_owned();
    let listed = Command::new("ps")
        .args(["-axww", "-o", "pid=,args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listed.stdout)
        .lines()
        .filter(|line| line.contains(&marker) && line.contains("--user-data-dir=") && holds(line))
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect()
}

/// Sends `signal` to each of `processes`.
fn signal(signal: &str, processes: &[String]) {
    assert!(!processes.is_empty(), "no browser process to signal");
    let status = Command::new("kill")
        .arg(format!("-{signal}"))
        .args(processes)
        .status()
        .unwrap();
    assert!(status.success(), "kill -{signal} {processes:?}");
}

/// The browser process itself: its helpers each name the `--type` they are.
fn browser(line: &str) -> bool {
    !line.contains("--type=")
}

/// A renderer process of the browser.
fn renderer(line: &str) -> bool {
    line.contains("--type=renderer")
}

/// An engine with the Run `run_id` and one page on the catalog, listening
/// to the page's `crash` and `close`: the exchange, the page and its
/// Session.
fn on_a_page<'env>(
    env: &'env TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
) -> (Exchange<'env>, String, String) {
    let cache = browser_cache(env).unwrap();
    let engine = started_with(env, &cache, proving_ground, run_id, &Map::new(), |_| {});
    let mut exchange = Exchange::new(engine);
    let page = opened(&exchange.call("run.new_page", &json!({ "run_id": run_id })));
    let url = proving_ground.url("catalog.test", "/catalog/1");
    let went = exchange.call("page.goto", &json!({ "page_id": page.page_id, "url": url }));
    assert_eq!(went["result"]["response"]["status"], json!(200), "{went}");
    exchange.subscribe();
    exchange.listen(&page.page_id.0, "crash");
    exchange.listen(&page.page_id.0, "close");
    (exchange, page.page_id.0, page.session_id.0)
}

/// Starts a click that waits for an element no page has, then lets it get
/// in flight: answers its request's ID.
fn click_in_flight(exchange: &mut Exchange<'_>, page: &str) -> i64 {
    let id = exchange.request(
        "locator.click",
        &json!({ "locator": { "page_id": page, "selector": NOWHERE } }),
    );
    std::thread::sleep(IN_FLIGHT);
    id
}

/// The answers to the requests `ids`, in that order, whatever order they
/// came in.
fn answers(exchange: &mut Exchange<'_>, ids: &[i64]) -> Vec<Value> {
    ids.iter().map(|id| exchange.answer(*id)).collect()
}

/// An answer's Error code and details, which its `data` carries beside
/// them.
fn failure(answer: &Value) -> (Value, Value) {
    let mut details = answer["error"]["data"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let code = details.remove("code").unwrap_or_default();
    details.remove("retryable");
    (code, Value::Object(details))
}

/// What a later call on `page` raises.
fn later_on_page(exchange: &mut Exchange<'_>, page: &str) -> Value {
    failure(&exchange.call(
        "page.formats",
        &json!({ "page_id": page, "formats": { "markdown": true } }),
    ))
    .0
}

/// Covers: story:5-27, event:page.crash
#[test]
fn a_renderer_killed_mid_call_raises_page_crashed_emits_crash_and_refuses_later_calls() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, _) = on_a_page(&env, &proving_ground, "renderer");
    let clicking = click_in_flight(&mut exchange, &page);

    signal("KILL", &browser_processes(&env, renderer));
    let clicked = exchange.answer(clicking);
    exchange.until("page.crash", |data| data["page_id"] == json!(page));
    let later = later_on_page(&mut exchange, &page);
    let _ = exchange.call("run.finish", &json!({ "run_id": "renderer" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (failure(&clicked), later),
        (
            (json!("page_crashed"), json!({ "browser_exit": null })),
            json!("page_crashed")
        ),
        "{clicked}"
    );
}

/// A body read is a call on its page (B24): a renderer killed while
/// `response.body` waits for a response that never ends, the live board's
/// event stream, raises `page_crashed` for it, and so does a later read of
/// a body that finished before the crash, rather than answering only once
/// the Run ends; a body `page.wait_for_request_finished` kept still reads
/// from the Run's folder (ADR 0085).
/// Covers: story:5-27, operation:response.body, operation:page.wait_for_request_finished, error:page_crashed
#[test]
fn a_renderer_killed_under_a_body_read_raises_page_crashed_for_it_and_for_later_reads() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "body-crash",
        &Map::new(),
        |_| {},
    );
    let mut exchange = Exchange::new(engine);
    let page = opened(&exchange.call("run.new_page", &json!({ "run_id": "body-crash" }))).page_id;
    let board = proving_ground.start_url("live/1").unwrap();
    let streaming = exchange.request(
        "page.wait_for_response",
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/events" } }),
    );
    let keeping = exchange.request(
        "page.wait_for_request_finished",
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/updates/1" } }),
    );
    let went = exchange.call("page.goto", &json!({ "page_id": page, "url": board }));
    let stream = exchange.answer(streaming);
    let update = exchange.answer(keeping);
    exchange.subscribe();
    exchange.listen(&page.0, "crash");
    let reading = exchange.request(
        "response.body",
        &json!({ "page_id": page, "request_id": stream["result"]["request"]["request_id"], "timeout": null }),
    );
    std::thread::sleep(IN_FLIGHT);

    signal("KILL", &browser_processes(&env, renderer));
    let read = exchange.answer(reading);
    exchange.until("page.crash", |data| data["page_id"] == json!(page));
    let finished_before = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": went["result"]["response"]["request_id"] }),
    );
    let kept = exchange.call(
        "response.body",
        &json!({ "page_id": page, "request_id": update["result"]["request"]["request_id"] }),
    );
    let _ = exchange.call("run.finish", &json!({ "run_id": "body-crash" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (
            went["result"]["response"]["status"].clone(),
            failure(&read).0,
            failure(&finished_before).0,
            kept["result"]["path"].is_string(),
        ),
        (
            json!(200),
            json!("page_crashed"),
            json!("page_crashed"),
            true
        ),
        "{went} {stream} {read} {finished_before} {kept}"
    );
}

/// A browser the OS kills, as its OOM killer does with `SIGKILL`, counts
/// as a page crash, in Usage too (ADR 0040's shedding ladder).
///
/// Covers: story:5-28, story:5-27, story:3-59, event:page.crash
#[test]
fn a_browser_killed_by_a_signal_raises_page_crashed_with_its_exit_and_its_session_stays_crashed() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, session) = on_a_page(&env, &proving_ground, "killed");
    let clicking = click_in_flight(&mut exchange, &page);

    signal("KILL", &browser_processes(&env, browser));
    let clicked = exchange.answer(clicking);
    exchange.until("page.crash", |data| data["page_id"] == json!(page));
    let closed = exchange.call("session.close", &json!({ "session_id": session }));
    let after_close = exchange.call("session.new_page", &json!({ "session_id": session }));
    let usage = exchange.call("run.usage", &json!({ "run_id": "killed" }));
    let _ = exchange.call("run.finish", &json!({ "run_id": "killed" }));
    let _ = exchange.engine.finish();

    let killed = json!({ "browser_exit": { "code": null, "signal": 9 } });
    let crashes: Vec<Value> = usage["result"]["other"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|kind| kind["usage_kind"] == json!("page_crashes"))
        .map(|kind| kind["amount"].clone())
        .collect();
    assert_eq!(
        (
            failure(&clicked),
            closed.get("result").is_some(),
            failure(&after_close),
            crashes
        ),
        (
            (json!("page_crashed"), killed.clone()),
            true,
            (json!("page_crashed"), killed),
            vec![json!(1)]
        ),
        "{clicked} {closed} {after_close} {usage}"
    );
}

/// A browser a person quits exits with status 0, as on `SIGTERM`: calls in
/// flight raise `call_cancelled` naming the browser, and the Session ends.
///
/// Covers: story:5-28
#[test]
fn a_browser_quit_raises_call_cancelled_naming_the_browser_and_ends_its_session() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, session) = on_a_page(&env, &proving_ground, "quit");
    let clicking = click_in_flight(&mut exchange, &page);

    signal("TERM", &browser_processes(&env, browser));
    let clicked = exchange.answer(clicking);
    let later = later_on_page(&mut exchange, &page);
    let new_page = exchange.call("session.new_page", &json!({ "session_id": session }));
    let _ = exchange.call("run.finish", &json!({ "run_id": "quit" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (failure(&clicked), later, failure(&new_page).0),
        (
            (
                json!("call_cancelled"),
                json!({ "by": "browser", "closed": "browser" })
            ),
            json!("closed"),
            json!("closed")
        ),
        "{clicked} {new_page}"
    );
}

/// `close()` while a call is in flight ends it with `call_cancelled`
/// naming the program and the Session; the page emits `close`, and a later
/// call raises `closed`.
///
/// Covers: story:5-27, event:page.close
#[test]
fn closing_a_session_under_a_call_raises_call_cancelled_naming_the_program() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, session) = on_a_page(&env, &proving_ground, "closing");
    let clicking = click_in_flight(&mut exchange, &page);

    let closing = exchange.request("session.close", &json!({ "session_id": session }));
    let mut answers = answers(&mut exchange, &[closing, clicking]).into_iter();
    let (closed, clicked) = (answers.next().unwrap(), answers.next().unwrap());
    exchange.until("page.close", |data| data["page_id"] == json!(page));
    let later = later_on_page(&mut exchange, &page);
    let _ = exchange.call("run.finish", &json!({ "run_id": "closing" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (closed.get("result").is_some(), failure(&clicked), later),
        (
            true,
            (
                json!("call_cancelled"),
                json!({ "by": "program", "closed": "session" })
            ),
            json!("closed")
        ),
        "{closed} {clicked}"
    );
}
