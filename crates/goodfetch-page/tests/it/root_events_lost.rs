//! A launched browser whose reader of its own events fell 4,096 behind runs
//! on (#3261): no Session is crashed and no crash is charged. What ends
//! instead is the browser's service: it opens no page any more, so the next
//! page the Run opens launches another browser, and it closes once its last
//! page closed. The whole engine runs, on the paused clock, over scripted
//! browsers' in-memory pipes; the first floods its own events when a page's
//! call asks it to.
#![expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]

use std::time::{Duration, SystemTime};

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, Network};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_test_support::runtime;
use serde_json::{Map, Value, json};
use tokio::io::DuplexStream;
use tokio::sync::{Notify, watch};

use crate::session_counts::session::{Client, Page, RUN, Wire, answered, dependencies, engine};

/// How many events the browser sends at once with no session, its own: far
/// past the 4,096 a reader may hold, so the reader of the browser's own
/// events, which takes a share of them each turn while the connection reads
/// many more, falls behind.
const FLOOD: usize = 20_000;

/// The ID of the element whose wait sets the flood off.
const FLOODS: &str = "flood-the-browser";

/// Longer than a lost browser's end takes to be known, so a loss the flood
/// made would have reached the Session.
const SETTLED: Duration = Duration::from_secs(5);

/// What the browsers saw of the engine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Seen {
    /// Whether the first browser flooded its own events.
    flooded: bool,
    /// Whether the engine launched a second browser.
    launched_second: bool,
    /// Whether the engine asked the first browser to close.
    closed_first: bool,
}

/// The flood, as one write: the events joined by the framing's NUL, the
/// last one's added on sending.
fn flood() -> String {
    let event = json!({
        "method": "Animation.animationCanceled",
        "params": { "id": "animation" },
    })
    .to_string();
    vec![event; FLOOD].join("\0")
}

/// Plays a browser on `pipe`: answers each command, and when `floods`, sends
/// the flood before answering the call that sets it off and tells `seen`
/// when the engine asks it to close.
async fn play(pipe: DuplexStream, floods: bool, seen: &watch::Sender<Seen>) {
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    while let Some(text) = wire.next().await {
        let command: Value = serde_json::from_str(&text).expect("a command is JSON");
        let method = command["method"].as_str().expect("it names its method");
        let params = &command["params"];
        if floods && !seen.borrow().flooded && text.contains(FLOODS) {
            seen.send_modify(|seen| seen.flooded = true);
            if !wire.send(flood()).await {
                return;
            }
        }
        if floods && method == "Browser.close" {
            seen.send_modify(|seen| seen.closed_first = true);
        }
        let mut answer = match answered(method, params, &mut page) {
            Ok((result, _)) => json!({ "result": result }),
            Err(error) => json!({ "error": error }),
        };
        answer["id"] = command["id"].clone();
        if let Some(session) = command.get("sessionId") {
            answer["sessionId"] = session.clone();
        }
        if !wire.send(answer.to_string()).await || method == "Browser.close" {
            return;
        }
    }
}

/// Plays the first browser the engine launches, which floods, and the
/// second, if the engine launches one before the program is `done`.
async fn browsers(mut launched: ScriptedBrowsers, seen: &watch::Sender<Seen>, done: &Notify) {
    let first = launched.next().await.expect("the engine launches one");
    let second = async {
        let pipe = tokio::select! {
            pipe = launched.next() => pipe,
            () = done.notified() => None,
        };
        if let Some(pipe) = pipe {
            seen.send_modify(|seen| seen.launched_second = true);
            play(pipe, false, seen).await;
        }
    };
    tokio::join!(play(first, true, seen), second);
}

/// What the program met after the browser flooded its own events.
struct AfterTheFlood {
    /// The answer to a later call on the page open meanwhile.
    later: Value,
    /// The `page_crashes` amounts the Run counted in Usage.
    crashes: Vec<Value>,
    /// The answer to a page another Session opens next.
    next_page: Value,
    /// What the browsers saw once the first page closed.
    seen: Seen,
}

/// Opens a page, has the browser flood its own events, waits for the
/// engine to settle, then calls on the page, reads the Run's Usage, opens a
/// page in another Session and closes the first.
fn after_the_flood() -> AfterTheFlood {
    let home = tempfile::tempdir().expect("the OS gives a temporary folder");
    let (launcher, scripted) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (telling, seen) = watch::channel(Seen::default());
    let done = Notify::new();
    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let program = async {
            let mut client = Client::connected(&dialer).await;
            client.start_run(Map::new()).await;
            let first = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opened = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": first["session_id"] }),
                )
                .await;
            let page_id = &opened["page_id"];
            let waited_for = |selector: &str| {
                json!({
                    "locator": { "page_id": page_id, "selector": selector },
                    "state": "attached",
                })
            };
            client
                .call("locator.wait_for", &waited_for(&format!("#{FLOODS}")))
                .await;
            tokio::time::sleep(SETTLED).await;
            let waiting = client.start("locator.wait_for", &waited_for("body")).await;
            let later = client.answer(waiting).await;
            let usage = client.call("run.usage", &json!({ "run_id": RUN })).await;
            let crashes = usage["other"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|kind| kind["usage_kind"] == json!("page_crashes"))
                .map(|kind| kind["amount"].clone())
                .collect();
            let second = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opening = client
                .start(
                    "session.new_page",
                    &json!({ "session_id": second["session_id"] }),
                )
                .await;
            let next_page = client.answer(opening).await;
            client
                .call("page.close", &json!({ "page_id": page_id }))
                .await;
            tokio::time::sleep(SETTLED).await;
            let seen = *seen.borrow();
            done.notify_one();
            client.close().await;
            AfterTheFlood {
                later,
                crashes,
                next_page,
                seen,
            }
        };
        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, after, ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            program,
            browsers(scripted, &telling, &done),
        );
        ended.expect("the engine got its Connection");
        after
    })
}

/// A launched browser whose reader of its own events fell behind crashes no
/// Session and charges no crash: the page open in it answers later calls.
/// It opens no page any more, so another Session's page launches a second
/// browser, and it closes once its last page closed.
#[test]
fn a_browser_whose_own_events_fell_behind_crashes_no_session_and_is_retired() {
    let AfterTheFlood {
        later,
        crashes,
        next_page,
        seen,
    } = after_the_flood();

    assert!(seen.flooded, "{seen:?}");
    assert!(later.get("result").is_some(), "{later}");
    assert_eq!(crashes, Vec::<Value>::new());
    assert!(next_page["result"]["page_id"].is_string(), "{next_page}");
    assert_eq!(
        (seen.launched_second, seen.closed_first),
        (true, true),
        "{seen:?}"
    );
}
