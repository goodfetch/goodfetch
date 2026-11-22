//! A page or a browser that ends under a call, on Collie's executable over
//! stdio (spec #594; ADRs 0040, 0113 and 0128; pitfall B24): a renderer
//! killed mid-call raises `page_crashed` and emits `crash`; a browser
//! killed by a signal raises `page_crashed` with how it exited, counts once
//! in Usage however many pages it held, leaves its Session `Crashed`, which
//! a close releases without changing, and leaves its Run's pool, so the
//! next Session opens in a new browser; a browser quit raises
//! `call_cancelled` naming the browser; and a Session closed, a page closed
//! running its `beforeunload` handlers, or a Run stopped while a call is in
//! flight raises `call_cancelled` naming the program. Each test has its own
//! Goodfetch home, Proving ground and Browser cache (ADRs 0059 and 0094);
//! the processes it kills are the test's engine's own browser's.
//!
//! A call is in flight once the page function it runs wrote to its page's
//! console, which the page's `console` event shows: the function never
//! settles, so the call waits until its page or browser ends under it. A
//! body read is in flight once the engine's `$/log` at `debug` says it
//! waits for its response to end.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test lists and signals its own browser's processes on its own thread, never an async worker"
)]

use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_protocol::home::USAGE_LEDGER;
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache, skipped_without_gui_session};
use rusqlite::Connection;
use serde_json::{Map, Value, json};

use crate::page::{opened, started_with};
use crate::page_events::Exchange;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 731;

/// What the call in flight writes to its page's console once it runs.
const IN_FLIGHT: &str = "in flight";

/// The engine's `tracing` target of a body read that waits for its
/// response to end, which a Connection's `$/log` at `debug` shows: no
/// console line can say a `response.body` is in flight, as [`IN_FLIGHT`]
/// says for a page function.
const BODY_READ: &str = "goodfetch::page::body_read";

/// The page function of the call in flight: it writes [`IN_FLIGHT`], then
/// never settles.
const NEVER_SETTLES: &str = "() => { console.log('in flight'); return new Promise(() => {}); }";

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
        .filter(|line| line.contains(&marker) && of_a_browser(line) && holds(line))
        .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
        .collect()
}

/// Sends `signal` to each of `processes` that is still there: a spare
/// renderer the browser ends on its own between the listing and the
/// signal is no failure, so long as one process took it.
fn signal(signal: &str, processes: &[String]) {
    let signalled = processes
        .iter()
        .filter(|process| {
            Command::new("kill")
                .arg(format!("-{signal}"))
                .arg(process)
                .status()
                .unwrap()
                .success()
        })
        .count();
    assert!(signalled > 0, "kill -{signal} {processes:?}");
}

/// A process of a browser of the test: the full build's each name its
/// profile, while the headless shell's helpers name their `--type` alone,
/// and the test's marker in the path of the shell's build (#3224).
fn of_a_browser(line: &str) -> bool {
    line.contains("--user-data-dir=") || line.contains("--type=")
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
/// to the page's `crash`, `close` and `console`: the exchange, the page and
/// its Session.
fn on_a_page(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
) -> (Exchange, String, String) {
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
    exchange.listen(&page.page_id.0, "console");
    (exchange, page.page_id.0, page.session_id.0)
}

/// Starts a page function on `page` that never settles, and waits until it
/// runs: answers its request's ID.
pub(crate) fn call_in_flight(exchange: &mut Exchange, page: &str) -> i64 {
    let id = exchange.request(
        "page.evaluate",
        &json!({ "page_id": page, "function": NEVER_SETTLES }),
    );
    exchange.until("page.console", |data| {
        data["page_id"] == json!(page) && data["text"] == json!(IN_FLIGHT)
    });
    id
}

/// Reads until the engine's `$/log` says that a body read of the request
/// `request_id` waits for its response to end, so the page's end now ends
/// the read in flight.
fn body_read_waits(exchange: &mut Exchange, request_id: &Value) {
    loop {
        let Some(message) = exchange.read() else {
            continue;
        };
        if message["method"] == json!("$/log")
            && message["params"]["target"] == json!(BODY_READ)
            && message["params"]["fields"]["request_id"] == *request_id
        {
            return;
        }
    }
}

/// The answers to the requests `ids`, in that order, whatever order they
/// came in.
fn answers(exchange: &mut Exchange, ids: &[i64]) -> Vec<Value> {
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
fn later_on_page(exchange: &mut Exchange, page: &str) -> Value {
    failure(&exchange.call(
        "page.formats",
        &json!({ "page_id": page, "formats": { "markdown": true } }),
    ))
    .0
}

/// The amounts of `page_crashes` the Run `run_id` counted in Usage.
pub(crate) fn crashes_counted(exchange: &mut Exchange, run_id: &str) -> Vec<Value> {
    let usage = exchange.call("run.usage", &json!({ "run_id": run_id }));
    usage["result"]["other"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|kind| kind["usage_kind"] == json!("page_crashes"))
        .map(|kind| kind["amount"].clone())
        .collect()
}

/// Covers: story:5-27, event:page.crash
#[test]
fn a_renderer_killed_mid_call_raises_page_crashed_emits_crash_and_refuses_later_calls() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, _) = on_a_page(&env, &proving_ground, "renderer");
    let calling = call_in_flight(&mut exchange, &page);

    signal("KILL", &browser_processes(&env, renderer));
    let called = exchange.answer(calling);
    exchange.until("page.crash", |data| data["page_id"] == json!(page));
    let later = later_on_page(&mut exchange, &page);
    let _ = exchange.call("run.finish", &json!({ "run_id": "renderer" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (failure(&called), later),
        (
            (json!("page_crashed"), json!({ "browser_exit": null })),
            json!("page_crashed")
        ),
        "{called}"
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
    let streamed = stream["result"]["request"]["request_id"].clone();
    let logging = exchange.call("connection.set_log_level", &json!({ "log_level": "debug" }));
    assert!(logging.get("result").is_some(), "{logging}");
    let reading = exchange.request(
        "response.body",
        &json!({ "page_id": page, "request_id": streamed, "timeout": null }),
    );
    body_read_waits(&mut exchange, &streamed);

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
/// as one page crash in Usage however many pages it held (ADR 0040's
/// shedding ladder and its note on #731): its Session stays `Crashed`, its
/// pages and its new pages refused with how the browser exited.
///
/// Covers: story:5-28, story:5-27, story:3-59, event:page.crash, operation:session.pages
#[test]
fn a_browser_killed_by_a_signal_raises_page_crashed_with_its_exit_and_its_session_stays_crashed() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, session) = on_a_page(&env, &proving_ground, "killed");
    let second = exchange.call("session.new_page", &json!({ "session_id": session }));
    assert!(second.get("result").is_some(), "{second}");
    let calling = call_in_flight(&mut exchange, &page);

    signal("KILL", &browser_processes(&env, browser));
    let called = exchange.answer(calling);
    exchange.until("page.crash", |data| data["page_id"] == json!(page));
    let pages = exchange.call("session.pages", &json!({ "session_id": session }));
    let closed = exchange.call("session.close", &json!({ "session_id": session }));
    let after_close = exchange.call("session.new_page", &json!({ "session_id": session }));
    let crashes = crashes_counted(&mut exchange, "killed");
    let _ = exchange.call("run.finish", &json!({ "run_id": "killed" }));
    let _ = exchange.engine.finish();

    let killed = json!({ "browser_exit": { "code": null, "signal": 9 } });
    assert_eq!(
        (
            failure(&called),
            failure(&pages),
            closed.get("result").is_some(),
            failure(&after_close),
            crashes
        ),
        (
            (json!("page_crashed"), killed.clone()),
            (json!("page_crashed"), killed.clone()),
            true,
            (json!("page_crashed"), killed),
            vec![json!(1)]
        ),
        "{called} {pages} {closed} {after_close}"
    );
}

/// How long [`a_browsers_crash_counts_in_usage_before_any_of_its_pages_crash_goes_out`]
/// holds the Usage ledger after it killed the browser: long past how soon a
/// page's `crash` follows its browser's exit when nothing holds it back, and
/// well within the 10 s the ledger's connections wait for a write. A page
/// whose `crash` went out first shows only if it did so within this; a right
/// order passes however long it is.
const LEDGER_HELD: Duration = Duration::from_secs(2);

/// A program that read a page's `crash` reads the crash in its Run's Usage,
/// whichever of the browser's Sessions and pages took the crash first: with
/// the first charge held up, by a Limit on the crashes whose Slice the ledger
/// cannot give yet, no page's `crash` goes out, so a call on the page's
/// `crash` never finds the count still under way (ADR 0040).
///
/// Covers: story:5-28, event:page.crash, operation:run.usage
#[test]
fn a_browsers_crash_counts_in_usage_before_any_of_its_pages_crash_goes_out() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, _, session) = on_a_page(&env, &proving_ground, "counted");
    let second = opened(&exchange.call("session.new_page", &json!({ "session_id": session })));
    exchange.listen(&second.page_id.0, "crash");
    let limit = exchange.call(
        "usage.set",
        &json!({ "scope": "install", "usage_kind": "page_crashes", "amount": 10 }),
    );
    assert!(limit.get("result").is_some(), "{limit}");
    let ledger = Connection::open(env.home().join(USAGE_LEDGER)).unwrap();
    ledger.execute_batch("BEGIN IMMEDIATE").unwrap();
    let (releasing, released) = mpsc::channel();

    signal("KILL", &browser_processes(&env, browser));
    let (crash_went_out_while_held, crashes) = std::thread::scope(|scope| {
        scope.spawn(move || {
            std::thread::sleep(LEDGER_HELD);
            let _ = releasing.send(());
            drop(ledger);
        });
        exchange.until("page.crash", |_| true);
        let held = released.try_recv().is_err();
        (held, crashes_counted(&mut exchange, "counted"))
    });
    let _ = exchange.call("run.finish", &json!({ "run_id": "counted" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (crash_went_out_while_held, crashes),
        (false, vec![json!(1)])
    );
}

/// A browser killed by a signal leaves its Run's pool at once: a Session
/// seated there that held no page crashes with it, and the Run's next
/// Session opens in a new browser, its page loading (S9, ADR 0040).
///
/// Covers: story:5-28, operation:run.new_session, operation:session.new_page
#[test]
fn a_browser_killed_by_a_signal_leaves_its_runs_pool_and_the_next_session_opens_in_a_new_one() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, _) = on_a_page(&env, &proving_ground, "replaced");
    let pageless = exchange.call("run.new_session", &json!({ "run_id": "replaced" }));
    let pageless = pageless["result"]["session_id"].clone();
    assert!(pageless.is_string(), "{pageless}");

    signal("KILL", &browser_processes(&env, browser));
    exchange.until("page.crash", |data| data["page_id"] == json!(page));
    let on_the_dead = exchange.call("session.new_page", &json!({ "session_id": pageless }));
    let next = exchange.call("run.new_session", &json!({ "run_id": "replaced" }));
    let opened_page = exchange.call(
        "session.new_page",
        &json!({ "session_id": next["result"]["session_id"] }),
    );
    let went = exchange.call(
        "page.goto",
        &json!({
            "page_id": opened_page["result"]["page_id"],
            "url": proving_ground.url("catalog.test", "/catalog/1"),
        }),
    );
    let _ = exchange.call("run.finish", &json!({ "run_id": "replaced" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (
            failure(&on_the_dead).0,
            next["result"]["session_id"].is_string(),
            &went["result"]["response"]["status"],
        ),
        (json!("page_crashed"), true, &json!(200)),
        "{on_the_dead} {next} {opened_page} {went}"
    );
}

/// A browser a person quits exits with status 0, as on `SIGTERM`: calls in
/// flight raise `call_cancelled` naming the browser, and the Session ends.
///
/// Covers: story:5-28
#[test]
fn a_browser_quit_raises_call_cancelled_naming_the_browser_and_ends_its_session() {
    if skipped_without_gui_session(
        "the headless shell, which no person quits, exits with status 143 on `SIGTERM`, where the full build exits with 0",
    ) {
        return;
    }
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, session) = on_a_page(&env, &proving_ground, "quit");
    let calling = call_in_flight(&mut exchange, &page);

    signal("TERM", &browser_processes(&env, browser));
    let called = exchange.answer(calling);
    let later = later_on_page(&mut exchange, &page);
    let new_page = exchange.call("session.new_page", &json!({ "session_id": session }));
    let _ = exchange.call("run.finish", &json!({ "run_id": "quit" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (failure(&called), later, failure(&new_page).0),
        (
            (
                json!("call_cancelled"),
                json!({ "by": "browser", "closed": "browser" })
            ),
            json!("closed"),
            json!("closed")
        ),
        "{called} {new_page}"
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
    let calling = call_in_flight(&mut exchange, &page);

    let closing = exchange.request("session.close", &json!({ "session_id": session }));
    let mut answers = answers(&mut exchange, &[closing, calling]).into_iter();
    let (closed, called) = (answers.next().unwrap(), answers.next().unwrap());
    exchange.until("page.close", |data| data["page_id"] == json!(page));
    let later = later_on_page(&mut exchange, &page);
    let _ = exchange.call("run.finish", &json!({ "run_id": "closing" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (closed.get("result").is_some(), failure(&called), later),
        (
            true,
            (
                json!("call_cancelled"),
                json!({ "by": "program", "closed": "session" })
            ),
            json!("closed")
        ),
        "{closed} {called}"
    );
}

/// `page.close({ runBeforeUnload: true })` is the program's close: the
/// call in flight raises `call_cancelled` naming the program and the page,
/// once the page let itself close (spec #594's P8 and P29).
///
/// Covers: story:5-27, operation:page.close, event:page.close
#[test]
fn a_page_closed_running_its_before_unload_handlers_cancels_its_call_naming_the_program() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, _) = on_a_page(&env, &proving_ground, "unloaded");
    let calling = call_in_flight(&mut exchange, &page);

    let closed = exchange.call(
        "page.close",
        &json!({ "page_id": page, "run_before_unload": true }),
    );
    let called = exchange.answer(calling);
    exchange.until("page.close", |data| data["page_id"] == json!(page));
    let _ = exchange.call("run.finish", &json!({ "run_id": "unloaded" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (closed.get("result").is_some(), failure(&called)),
        (
            true,
            (
                json!("call_cancelled"),
                json!({ "by": "program", "closed": "page" })
            )
        ),
        "{closed} {called}"
    );
}

/// `run.stop` while a call is in flight is the program's close of the Run:
/// the call raises `call_cancelled` naming the program and the Run (ADR
/// 0113), never a retryable `page_crashed`, and the stop answers.
///
/// Covers: story:5-27, story:11-48, operation:run.stop
#[test]
fn stopping_a_run_under_a_call_raises_call_cancelled_naming_the_program_and_the_run() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, _) = on_a_page(&env, &proving_ground, "stopping");
    let calling = call_in_flight(&mut exchange, &page);

    let stopping = exchange.request("run.stop", &json!({ "run_id": "stopping" }));
    let mut answers = answers(&mut exchange, &[stopping, calling]).into_iter();
    let (stopped, called) = (answers.next().unwrap(), answers.next().unwrap());
    let _ = exchange.engine.finish();

    assert_eq!(
        (stopped.get("result").is_some(), failure(&called)),
        (
            true,
            (
                json!("call_cancelled"),
                json!({ "by": "program", "closed": "run" })
            )
        ),
        "{stopped} {called}"
    );
}

/// `run.abort` while a call is in flight stops the Run at once for a
/// Client: the call raises `call_cancelled` naming the Client and the Run
/// (ADR 0113), and the abort answers.
///
/// Covers: story:11-48, operation:run.abort
#[test]
fn aborting_a_run_under_a_call_raises_call_cancelled_naming_the_client_and_the_run() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (mut exchange, page, _) = on_a_page(&env, &proving_ground, "aborting");
    let calling = call_in_flight(&mut exchange, &page);

    let aborting = exchange.request("run.abort", &json!({ "run_id": "aborting" }));
    let mut answers = answers(&mut exchange, &[aborting, calling]).into_iter();
    let (aborted, called) = (answers.next().unwrap(), answers.next().unwrap());
    let _ = exchange.engine.finish();

    assert_eq!(
        (aborted.get("result").is_some(), failure(&called)),
        (
            true,
            (
                json!("call_cancelled"),
                json!({ "by": "client", "closed": "run" })
            )
        ),
        "{aborted} {called}"
    );
}
