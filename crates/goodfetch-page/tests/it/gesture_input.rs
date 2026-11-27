//! A Gesture's input goes out as one burst (ADRs 0019 and 0146): no event of
//! it waits for the browser's answer to the one before, and a Gesture cut at
//! its deadline, or cancelled, leaves no key and no button down. Two Gestures
//! sent at once on one page take turns, while reads go out beside them
//! (ADR 0085). The whole engine runs over a scripted browser's in-memory
//! pipe, which withholds its answer to one event, on the paused clock where
//! a Gesture is cut at its deadline.
#![expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]

use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, Network};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_test_support::runtime;
use rstest::rstest;
use serde_json::{Map, Value, json};
use tokio::sync::{oneshot, watch};

use crate::session_counts::session::{Client, Page, RUN, Wire, answered, dependencies, engine};

/// How long the browser may take to hear what the engine sends, on the
/// paused clock: past the action's deadline, so a Gesture that sends the rest
/// of its input only then, or never, is seen to.
const HEARD: Duration = Duration::from_secs(60);

/// The action's window, in milliseconds.
const WINDOW: u64 = 500;

/// What the browser tells it heard when it answers the withheld event.
static RELEASED: LazyLock<Value> = LazyLock::new(|| json!({ "released": true }));

/// The input event `command` sends, by its type and its key, if any, or the
/// text it inserts; none for any other command.
fn input(command: &Value) -> Option<String> {
    let params = &command["params"];
    let kind = || params["type"].as_str();
    match command["method"].as_str()? {
        "Input.dispatchMouseEvent" => kind().map(str::to_owned),
        "Input.dispatchKeyEvent" => Some(format!("{} {}", kind()?, params["key"].as_str()?)),
        "Input.insertText" => Some(format!("insertText {}", params["text"].as_str()?)),
        _ => None,
    }
}

/// The input events among `commands`, in the order the browser read them.
fn inputs(commands: &[Value]) -> Vec<String> {
    commands.iter().filter_map(input).collect()
}

/// Plays the browser the engine launches: tells `heard` each command it
/// reads, and answers each but the input event `withheld`, which it answers
/// once `released` is, if given, telling `heard` [`RELEASED`] then, and
/// never otherwise.
async fn browser(
    mut launched: ScriptedBrowsers,
    withheld: &str,
    heard: &watch::Sender<Vec<Value>>,
    mut released: Option<oneshot::Receiver<()>>,
) {
    let pipe = launched.next().await.expect("the engine launches it");
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    let mut held = None;
    loop {
        let text = tokio::select! {
            text = wire.next() => text,
            release = async { released.as_mut().expect("polled only while given").await },
                if held.is_some() && released.is_some() =>
            {
                released = None;
                heard.send_modify(|commands| commands.push(RELEASED.clone()));
                if release.is_ok() && !wire.send(held.take().expect("checked above")).await {
                    return;
                }
                continue;
            }
        };
        let Some(text) = text else { return };
        let command: Value = serde_json::from_str(&text).expect("a command is JSON");
        let method = command["method"].as_str().expect("it names its method");
        heard.send_modify(|commands| commands.push(command.clone()));
        let mut answer = match answered(method, &command["params"], &mut page) {
            Ok((result, _)) => json!({ "result": result }),
            Err(error) => json!({ "error": error }),
        };
        answer["id"] = command["id"].clone();
        if let Some(session) = command.get("sessionId") {
            answer["sessionId"] = session.clone();
        }
        if input(&command).is_some_and(|event| event == withheld) {
            held = Some(answer.to_string());
            continue;
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
                let sent = heard
                    .wait_for(|commands| inputs(commands).iter().any(|event| event == withheld));
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
            browser(browsers, withheld, &telling, None),
        );
        ended.expect("the engine got its Connection");
        error
    });
    (error, inputs(&heard.borrow()))
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

/// How many reads the program sends while a fill waits on the browser.
const READS: usize = 10;

/// The function `page.read` calls in the page.
const READ_FIELDS: &str = include_str!("../../src/fields/read.js");

/// Two fills sent at once on one page take turns: the second's check and
/// text go out only once the browser answered the first's text. Ten reads of
/// an element's text, sent while the first waits, go out beside it, one
/// command each, and answer.
/// Covers: story:5-7, operation:locator.fill, operation:page.read
#[test]
fn two_fills_at_once_take_turns_while_reads_answer_beside_them() {
    let home = tempfile::tempdir().expect("the OS gives a temporary folder");
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (telling, mut heard) = watch::channel(Vec::new());
    let (release, released) = oneshot::channel();
    let settings = Map::from_iter([("timeout".to_owned(), json!({ "action": 30_000 }))]);
    let withheld = "insertText one";
    let (reads, fills, before) = runtime().block_on(async {
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
            let page = &opened["page_id"];
            let fill = |selector: &str, value: &str| {
                json!({ "locator": { "page_id": page, "selector": selector }, "value": value })
            };
            let first = client.start("locator.fill", &fill("#first", "one")).await;
            let sent = heard
                .wait_for(|commands| inputs(commands).iter().any(|event| event == withheld));
            let sent = tokio::time::timeout(HEARD, sent).await.is_ok();
            assert!(sent, "the first fill's text is sent: {:#?}", heard.borrow());
            let before = heard.borrow().len();
            let second = client.start("locator.fill", &fill("#second", "two")).await;
            let mut started = Vec::new();
            for index in 0..READS {
                let field = json!({ "locator": { "kind": "css", "selector": format!("#item-{index}") } });
                let read = json!({ "page_id": page, "fields": { "text": field } });
                started.push(client.start("page.read", &read).await);
            }
            let mut reads = Vec::new();
            for id in started {
                reads.push(client.answer(id).await);
            }
            release.send(()).expect("the browser waits for it");
            let fills = [client.answer(first).await, client.answer(second).await];
            client.close().await;
            (reads, fills, before)
        };
        let (ended, outcome, ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(home.path())),
            client,
            browser(browsers, withheld, &telling, Some(released)),
        );
        ended.expect("the engine got its Connection");
        outcome
    });
    let heard = heard.borrow().clone();
    let at = heard
        .iter()
        .position(|command| *command == *RELEASED)
        .expect("the browser answered the first fill's text");
    let meanwhile = &heard[before..at];

    let unanswered: Vec<&Value> = reads
        .iter()
        .chain(&fills)
        .filter(|answer| answer.get("result").is_none())
        .collect();
    assert!(unanswered.is_empty(), "{unanswered:?}");
    let (read, worlds): (Vec<&Value>, Vec<&Value>) = meanwhile
        .iter()
        .partition(|command| command["params"]["functionDeclaration"] == READ_FIELDS);
    assert_eq!(read.len(), READS, "{meanwhile:#?}");
    // Each read still makes its world first: Exception X10, until #3237.
    assert!(
        worlds
            .iter()
            .all(|command| command["method"] == "Page.createIsolatedWorld"),
        "{meanwhile:#?}"
    );
    assert_eq!(inputs(&heard), ["insertText one", "insertText two"]);
}
