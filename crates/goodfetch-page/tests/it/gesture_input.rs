//! A Gesture's input goes out as one burst (ADRs 0019 and 0146): no event of
//! it waits for the browser's answer to the one before, and a Gesture cut at
//! its deadline, or cancelled, leaves no key and no button down. The whole
//! engine runs, on the paused clock, over a scripted browser's in-memory pipe,
//! which withholds its answer to one event.
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
use rstest::rstest;
use serde_json::{Map, Value, json};
use tokio::sync::watch;

use crate::session_counts::session::{Client, Page, RUN, Wire, answered, dependencies, engine};

/// How long the browser may take to hear what the engine sends, on the
/// paused clock: past the action's deadline, so a Gesture that sends the rest
/// of its input only then, or never, is seen to.
const HEARD: Duration = Duration::from_secs(60);

/// The action's window, in milliseconds.
const WINDOW: u64 = 500;

/// The input event `command` sends, by its type and its key, if any; none
/// for any other command.
fn input(command: &Value) -> Option<String> {
    let params = &command["params"];
    let kind = params["type"].as_str()?;
    match command["method"].as_str()? {
        "Input.dispatchMouseEvent" => Some(kind.to_owned()),
        "Input.dispatchKeyEvent" => Some(format!("{kind} {}", params["key"].as_str()?)),
        _ => None,
    }
}

/// Plays the browser the engine launches: tells `heard` each input event it
/// reads, and answers each command but the event `withheld`.
async fn browser(
    mut launched: ScriptedBrowsers,
    withheld: &str,
    heard: &watch::Sender<Vec<String>>,
) {
    let pipe = launched.next().await.expect("the engine launches it");
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    while let Some(text) = wire.next().await {
        let command: Value = serde_json::from_str(&text).expect("a command is JSON");
        let method = command["method"].as_str().expect("it names its method");
        if let Some(event) = input(&command) {
            let withholds = event == withheld;
            heard.send_modify(|events| events.push(event));
            if withholds {
                continue;
            }
        }
        let mut answer = match answered(method, &command["params"], &mut page) {
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

/// How a Gesture ends before its input is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cut {
    /// Its window passes.
    Deadline,
    /// Its call is cancelled once the browser heard the withheld event.
    Cancelled,
}

/// Calls `operation` with `params` on a page whose browser withholds its
/// answer to the input event `withheld`, until `cut`: the error it raises
/// and the input events the browser heard.
fn cut_gesture(
    cut: Cut,
    operation: &str,
    mut params: Value,
    withheld: &str,
) -> (Value, Vec<String>) {
    let home = tempfile::tempdir().expect("the OS gives a temporary folder");
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (telling, mut heard) = watch::channel(Vec::new());
    let settings = Map::from_iter([("timeout".to_owned(), json!({ "action": WINDOW }))]);
    let error = runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let client = async {
            let mut client = Client::connected(&dialer).await;
            client.start_run(settings).await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opened = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            params["locator"] = json!({ "page_id": opened["page_id"], "selector": "#target" });
            let id = client.start(operation, &params).await;
            if cut == Cut::Cancelled {
                let sent = heard.wait_for(|events| events.iter().any(|event| event == withheld));
                let _ = tokio::time::timeout(HEARD, sent)
                    .await
                    .expect("the withheld event is sent");
                client
                    .write(
                        &json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": id } }),
                    )
                    .await;
            }
            let answer = client.answer(id).await;
            client.close().await;
            answer["error"].clone()
        };
        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, error, ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client,
            browser(browsers, withheld, &telling),
        );
        ended.expect("the engine got its Connection");
        error
    });
    (error, heard.borrow().clone())
}

/// The events of a click.
const CLICK: [&str; 3] = ["mouseMoved", "mousePressed", "mouseReleased"];

/// The events of a press of `Control+a`.
const COMBINATION: [&str; 4] = [
    "rawKeyDown Control",
    "rawKeyDown a",
    "keyUp a",
    "keyUp Control",
];

/// The events of a fill with no text, which deletes the field's.
const DELETION: [&str; 2] = ["rawKeyDown Delete", "keyUp Delete"];

/// A Gesture whose window passes with one of its input events unanswered
/// ends at its deadline, and the browser received every one of them, those
/// that release its keys and buttons among them.
/// Covers: operation:locator.click
#[rstest]
#[case::a_click_at_its_first_event("locator.click", json!({}), "mouseMoved", &CLICK)]
#[case::a_click_at_its_press("locator.click", json!({}), "mousePressed", &CLICK)]
/// Covers: operation:locator.press
#[case::a_combination_at_its_first_key_up(
    "locator.press",
    json!({ "key": "Control+a" }),
    "keyUp a",
    &COMBINATION,
)]
#[case::a_combination_at_its_modifier(
    "locator.press",
    json!({ "key": "Control+a" }),
    "rawKeyDown Control",
    &COMBINATION,
)]
/// Covers: operation:locator.fill
#[case::an_emptied_field_at_its_key_down(
    "locator.fill",
    json!({ "value": "" }),
    "rawKeyDown Delete",
    &DELETION,
)]
fn a_gesture_past_its_deadline_sent_all_its_input(
    #[case] operation: &str,
    #[case] params: Value,
    #[case] withheld: &str,
    #[case] events: &[&str],
) {
    let (error, heard) = cut_gesture(Cut::Deadline, operation, params, withheld);

    assert_eq!(error["data"]["code"], "deadline_exceeded", "{error}");
    assert_eq!(heard, events);
}

/// A Gesture cancelled with one of its input events unanswered leaves no key
/// and no button down either.
/// Covers: operation:locator.click
#[rstest]
#[case::a_click_at_its_press("locator.click", json!({}), "mousePressed", &CLICK)]
/// Covers: operation:locator.press
#[case::a_combination_at_its_key(
    "locator.press",
    json!({ "key": "Control+a" }),
    "rawKeyDown a",
    &COMBINATION,
)]
fn a_cancelled_gesture_sent_all_its_input(
    #[case] operation: &str,
    #[case] params: Value,
    #[case] withheld: &str,
    #[case] events: &[&str],
) {
    let (error, heard) = cut_gesture(Cut::Cancelled, operation, params, withheld);

    assert_eq!(error["data"]["code"], "call_cancelled", "{error}");
    assert_eq!(heard, events);
}
