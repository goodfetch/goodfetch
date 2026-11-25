//! A Page whose reader fell 4,096 events behind is closed (ADR 0146; ADR
//! 0085's note of 2026-11-10): its call in flight and every later call
//! raise `page_events_lost`, its target closes, so no request it had paused
//! stays paused, and the Run trace records the close once. The whole engine
//! runs, on the paused clock, over a scripted browser's in-memory pipe,
//! which floods the page's session as its navigation starts.
#![expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::SystemTime;

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, Network};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_test_support::{OUTCOME_TARGET, capturing, runtime};
use serde_json::{Map, Value, json};
use tokio::sync::watch;

use crate::session_counts::session::{Client, Page, RUN, Wire, answered, dependencies, engine};

/// How many events the browser sends at once on the page's session: far
/// past the 4,096 its reader may hold, so the reader, which takes a share of
/// them each turn while the connection reads many more, falls behind.
const FLOOD: usize = 20_000;

/// Where the program navigates the page.
const URL: &str = "https://a.test/";

/// The request the browser pauses after its flood, whose event the engine
/// never reads.
const PAUSED: &str = "paused-after-the-flood";

/// What the browser saw of the page it flooded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Seen {
    /// Its target, once opened.
    target: Option<String>,
    /// Whether the engine closed that target, which ends the requests it
    /// paused.
    closed: bool,
    /// Whether the engine answered the request paused after the flood.
    answered: bool,
}

/// The flood, then the paused request, on `session`, as one write: the
/// events joined by the framing's NUL, the last one's added on sending.
fn flood(session: &Value) -> String {
    let event = json!({
        "method": "Animation.animationCanceled",
        "sessionId": session,
        "params": { "id": "animation" },
    })
    .to_string();
    let paused = json!({
        "method": "Fetch.requestPaused",
        "sessionId": session,
        "params": {
            "requestId": PAUSED,
            "request": {
                "url": URL,
                "method": "GET",
                "headers": {},
                "initialPriority": "VeryHigh",
                "referrerPolicy": "strict-origin-when-cross-origin",
            },
            "frameId": "page-target",
            "resourceType": "Document",
        },
    })
    .to_string();
    let mut events = vec![event; FLOOD];
    events.push(paused);
    events.join("\0")
}

/// Plays the browser the engine launches: answers each command, but floods
/// the page's session in place of answering its navigation, and tells
/// `seen` what the engine did to the page.
async fn browser(mut launched: ScriptedBrowsers, seen: &watch::Sender<Seen>) {
    let pipe = launched.next().await.expect("the engine launches it");
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    while let Some(text) = wire.next().await {
        let command: Value = serde_json::from_str(&text).expect("a command is JSON");
        let method = command["method"].as_str().expect("it names its method");
        let params = &command["params"];
        let target = seen.borrow().target.clone();
        if method == "Target.closeTarget" && target.as_deref() == params["targetId"].as_str() {
            seen.send_modify(|seen| seen.closed = true);
        }
        if method.starts_with("Fetch.") && params["requestId"] == PAUSED {
            seen.send_modify(|seen| seen.answered = true);
        }
        // A headless browser's identity read navigates a page of its own
        // first, which the browser answers.
        if method == "Page.navigate" && params["url"] == URL {
            if !wire.send(flood(&command["sessionId"])).await {
                return;
            }
            continue;
        }
        let mut answer = match answered(method, params, &mut page) {
            Ok((result, _)) => json!({ "result": result }),
            Err(error) => json!({ "error": error }),
        };
        if method == "Target.createTarget" && params["background"] != json!(true) {
            let opened = answer["result"]["targetId"].as_str().map(str::to_owned);
            seen.send_modify(|seen| seen.target = opened);
        }
        answer["id"] = command["id"].clone();
        if let Some(session) = command.get("sessionId") {
            answer["sessionId"] = session.clone();
        }
        if !wire.send(answer.to_string()).await || method == "Browser.close" {
            return;
        }
    }
}

/// What a flooded page's navigation and a later call on it raised, and the
/// codes the Run trace recorded meanwhile, one line each.
struct Flooded {
    navigation: Value,
    later: Value,
    seen: Seen,
    recorded: Vec<String>,
}

/// Opens a page, starts its navigation, which the browser answers with a
/// flood, then calls on the page once the navigation ended.
fn flooded() -> Flooded {
    let home = tempfile::tempdir().expect("the OS gives a temporary folder");
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (telling, seen) = watch::channel(Seen::default());
    let (subscriber, captured) = capturing(&[(OUTCOME_TARGET, tracing::Level::TRACE)]);
    let (navigation, later) = tracing::subscriber::with_default(subscriber, || {
        runtime().block_on(async {
            let (network, dialer) = transport::in_memory();
            let client = async {
                let mut client = Client::connected(&dialer).await;
                client.start_run(Map::new()).await;
                let session = client
                    .call("run.new_session", &json!({ "run_id": RUN }))
                    .await;
                let opened = client
                    .call(
                        "session.new_page",
                        &json!({ "session_id": session["session_id"] }),
                    )
                    .await;
                let page_id = &opened["page_id"];
                let goto = client
                    .start("page.goto", &json!({ "page_id": page_id, "url": URL }))
                    .await;
                let navigation = client.answer(goto).await;
                let evaluate = client
                    .start(
                        "page.evaluate",
                        &json!({ "page_id": page_id, "function": "() => 1" }),
                    )
                    .await;
                let later = client.answer(evaluate).await;
                client.close().await;
                (navigation["error"].clone(), later["error"].clone())
            };
            let dependencies = Dependencies {
                clock: Clock::paused(SystemTime::UNIX_EPOCH),
                ..dependencies(home.path())
            };
            let (ended, errors, ()) = tokio::join!(
                engine.run_attached(Network::InMemory(network), dependencies),
                client,
                browser(browsers, &telling),
            );
            ended.expect("the engine got its Connection");
            errors
        })
    });
    let written = String::from_utf8(captured.written()).expect("the log is UTF-8");
    let recorded = written
        .lines()
        .filter_map(|line| line.split("error_code=").nth(1))
        .map(|code| code.trim_matches('"').to_owned())
        .collect();
    let seen = seen.borrow().clone();
    Flooded {
        navigation,
        later,
        seen,
        recorded,
    }
}

/// A Page whose reader fell 4,096 events behind is closed: its navigation in
/// flight and its later call raise `page_events_lost`, naming the page, its
/// target closes, which ends the request it paused that the engine never
/// read, and the Run trace records the close once.
/// Covers: error:page_events_lost
#[test]
fn a_page_that_fell_behind_is_closed_with_page_events_lost() {
    let Flooded {
        navigation,
        later,
        seen,
        recorded,
    } = flooded();

    assert_eq!(
        (
            &navigation["data"]["code"],
            &later["data"]["code"],
            &later["data"]["page_id"],
        ),
        (
            &json!("page_events_lost"),
            &json!("page_events_lost"),
            &navigation["data"]["page_id"],
        ),
        "{navigation} {later}"
    );
    assert!(later["data"]["page_id"].is_string(), "{later}");
    assert_eq!(
        (seen.closed, seen.answered),
        (true, false),
        "the page's target closes: {seen:?}"
    );
    assert_eq!(recorded, ["page_events_lost"]);
}
