//! Input with no target (ADR 0085): `page.keyboard`, `page.mouse` and
//! `page.touchscreen` against the in-process engine's scripted browser on
//! the paused clock. Verifies held modifier and button state across calls,
//! stepped moves, delayed presses and clicks, typed ASCII + non-ASCII
//! text, invalid parameter refusals before the browser, and Provenance's
//! `unguarded_input` entries for every operation.
#![expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]

use std::path::Path;
use std::time::SystemTime;

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, Network};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::runtime;
use rstest::rstest;
use serde_json::{Map, Value, json};
use tokio::sync::watch;

use crate::session_counts::session::{Client, Page, RUN, Wire, answered, dependencies, engine};

/// Plays a scripted browser that records every command it hears and answers
/// each one immediately.
async fn browser(mut launched: ScriptedBrowsers, heard: &watch::Sender<Vec<Value>>) {
    let pipe = launched.next().await.expect("the engine launches it");
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    while let Some(text) = wire.next().await {
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
        if !wire.send(answer.to_string()).await || method == "Browser.close" {
            return;
        }
    }
}

/// Runs `calls` against one page of the in-process engine, returning the
/// CDP commands the browser heard, the Provenance lines written for `RUN`,
/// and the `(session_id, page_id)` of the page.
fn drive<F, Fut>(calls: F) -> (Vec<Value>, Vec<Value>, (Value, Value))
where
    F: FnOnce(Client, Value) -> Fut,
    Fut: Future<Output = Client>,
{
    let home = tempfile::tempdir().expect("the OS gives a temporary folder");
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (telling, heard) = watch::channel(Vec::new());
    let ids = runtime().block_on(async {
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
            let session_id = session["session_id"].clone();
            let page_id = opened["page_id"].clone();
            let mut client = calls(client, page_id.clone()).await;
            client.close().await;
            (session_id, page_id)
        };
        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, ids, ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            client,
            browser(browsers, &telling),
        );
        ended.expect("the engine got its Connection");
        ids
    });
    let commands = heard.borrow().clone();
    let provenance = provenance_lines(home.path());
    (commands, provenance, ids)
}

/// Parses every JSONL entry of `RUN`'s Provenance log under `home`.
fn provenance_lines(home: &Path) -> Vec<Value> {
    let path = HomeLayout::of(home).run_trace(RUN, RUN_TRACE_PROVENANCE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each provenance line is JSON"))
        .collect()
}

/// The CDP `Input.*` commands among `commands`.
fn input_commands(commands: &[Value]) -> Vec<&Value> {
    commands
        .iter()
        .filter(|command| {
            command["method"]
                .as_str()
                .is_some_and(|method| method.starts_with("Input."))
        })
        .collect()
}

/// The `operation` names of Provenance entries that marked an unguarded
/// input on `(session_id, page_id)`.
fn unguarded_operations(provenance: &[Value], session_id: &Value, page_id: &Value) -> Vec<String> {
    provenance
        .iter()
        .filter(|entry| {
            entry["action"] == "unguarded_input"
                && entry["session_id"] == *session_id
                && entry["page_id"] == *page_id
        })
        .map(|entry| {
            entry["operation"]
                .as_str()
                .expect("unguarded_input names its operation")
                .to_owned()
        })
        .collect()
}

/// Keyboard calls carry held modifiers across calls, split ASCII keys and
/// non-ASCII inserts when typing, honour delays, and mark each call
/// unguarded in Provenance.
/// Covers: story:5-9, operation:page.keyboard.down, operation:page.keyboard.up, operation:page.keyboard.press, operation:page.keyboard.type, operation:page.keyboard.insert_text
#[test]
fn keyboard_carries_held_modifiers_types_mixed_text_and_marks_provenance() {
    let (commands, provenance, (session_id, page_id)) = drive(async |mut client, page| {
        for (method, params) in [
            (
                "page.keyboard.down",
                json!({ "page_id": page, "key": "Shift" }),
            ),
            (
                "page.keyboard.press",
                json!({ "page_id": page, "key": "a" }),
            ),
            (
                "page.keyboard.up",
                json!({ "page_id": page, "key": "Shift" }),
            ),
            (
                "page.keyboard.type",
                json!({ "page_id": page, "text": "aé", "delay": 25 }),
            ),
            (
                "page.keyboard.insert_text",
                json!({ "page_id": page, "text": "hi" }),
            ),
            (
                "page.keyboard.press",
                json!({ "page_id": page, "key": "Control+c", "delay": 10 }),
            ),
        ] {
            let answer = client.call(method, &params).await;
            assert_eq!(answer, json!({}), "{method}: {answer}");
        }
        client
    });

    let events: Vec<(String, Option<String>, Option<i64>, Option<String>)> =
        input_commands(&commands)
            .into_iter()
            .map(|command| {
                let method = command["method"].as_str().unwrap_or_default();
                let params = &command["params"];
                if method == "Input.insertText" {
                    (
                        "insertText".to_owned(),
                        None,
                        None,
                        params["text"].as_str().map(str::to_owned),
                    )
                } else {
                    (
                        params["type"].as_str().unwrap_or_default().to_owned(),
                        params["key"].as_str().map(str::to_owned),
                        params["modifiers"].as_i64(),
                        params["text"].as_str().map(str::to_owned),
                    )
                }
            })
            .collect();

    assert_eq!(
        events,
        vec![
            ("rawKeyDown".into(), Some("Shift".into()), Some(8), None),
            (
                "keyDown".into(),
                Some("a".into()),
                Some(8),
                Some("A".into())
            ),
            ("keyUp".into(), Some("a".into()), Some(8), None),
            ("keyUp".into(), Some("Shift".into()), None, None),
            ("keyDown".into(), Some("a".into()), None, Some("a".into())),
            ("keyUp".into(), Some("a".into()), None, None),
            ("insertText".into(), None, None, Some("é".into())),
            ("insertText".into(), None, None, Some("hi".into())),
            ("rawKeyDown".into(), Some("Control".into()), Some(2), None),
            ("rawKeyDown".into(), Some("c".into()), Some(2), None),
            ("keyUp".into(), Some("c".into()), Some(2), None),
            ("keyUp".into(), Some("Control".into()), None, None),
        ]
    );
    assert_eq!(
        unguarded_operations(&provenance, &session_id, &page_id),
        vec![
            "page.keyboard.down",
            "page.keyboard.press",
            "page.keyboard.up",
            "page.keyboard.type",
            "page.keyboard.insert_text",
            "page.keyboard.press",
        ]
    );
}

/// Mouse and touchscreen calls carry held buttons and keyboard modifiers
/// across calls, interpolate stepped moves, and mark every call unguarded
/// in Provenance.
/// Covers: story:5-9, operation:page.mouse.move, operation:page.mouse.down, operation:page.mouse.up, operation:page.mouse.click, operation:page.mouse.dblclick, operation:page.mouse.wheel, operation:page.touchscreen.tap
#[test]
fn mouse_and_touchscreen_carry_state_and_mark_provenance() {
    let (commands, provenance, (session_id, page_id)) = drive(async |mut client, page| {
        for (method, params) in [
            (
                "page.mouse.move",
                json!({ "page_id": page, "x": 10.0, "y": 20.0 }),
            ),
            ("page.mouse.down", json!({ "page_id": page })),
            (
                "page.mouse.move",
                json!({ "page_id": page, "x": 30.0, "y": 40.0, "steps": 2 }),
            ),
            ("page.mouse.up", json!({ "page_id": page })),
            (
                "page.mouse.click",
                json!({ "page_id": page, "x": 5.0, "y": 6.0, "button": "right", "click_count": 2, "delay": 10 }),
            ),
            (
                "page.mouse.dblclick",
                json!({ "page_id": page, "x": 7.0, "y": 8.0 }),
            ),
            (
                "page.mouse.wheel",
                json!({ "page_id": page, "delta_x": 3.0, "delta_y": -12.0 }),
            ),
            (
                "page.keyboard.down",
                json!({ "page_id": page, "key": "Shift" }),
            ),
            (
                "page.touchscreen.tap",
                json!({ "page_id": page, "x": 15.0, "y": 25.0 }),
            ),
        ] {
            let answer = client.call(method, &params).await;
            assert_eq!(answer, json!({}), "{method}: {answer}");
        }
        client
    });

    let inputs = input_commands(&commands);
    // Stepped move while left button is held: midpoint (20, 30) and end (30, 40) carry buttons=1.
    let dragged: Vec<(f64, f64, Option<i64>)> = inputs
        .iter()
        .filter(|command| {
            command["method"] == "Input.dispatchMouseEvent"
                && command["params"]["type"] == "mouseMoved"
                && command["params"]["buttons"] == 1
        })
        .map(|command| {
            (
                command["params"]["x"].as_f64().unwrap_or_default(),
                command["params"]["y"].as_f64().unwrap_or_default(),
                command["params"]["buttons"].as_i64(),
            )
        })
        .collect();
    assert_eq!(dragged, vec![(20.0, 30.0, Some(1)), (30.0, 40.0, Some(1))]);

    // Wheel fires at the last mouse position (7, 8) with the requested deltas.
    let wheel = inputs
        .iter()
        .find(|command| command["params"]["type"] == "mouseWheel")
        .expect("mouseWheel is dispatched");
    assert_eq!(wheel["params"]["x"], json!(7.0));
    assert_eq!(wheel["params"]["y"], json!(8.0));
    assert_eq!(wheel["params"]["deltaX"], json!(3.0));
    assert_eq!(wheel["params"]["deltaY"], json!(-12.0));

    // Touch tap emits touchStart at (15, 25) and touchEnd with no touch points, carrying Shift (8).
    let touches: Vec<&Value> = inputs
        .iter()
        .copied()
        .filter(|command| command["method"] == "Input.dispatchTouchEvent")
        .collect();
    assert_eq!(touches.len(), 2, "{touches:?}");
    assert_eq!(touches[0]["params"]["type"], json!("touchStart"));
    assert_eq!(touches[0]["params"]["modifiers"], json!(8));
    assert_eq!(touches[0]["params"]["touchPoints"][0]["x"], json!(15.0));
    assert_eq!(touches[0]["params"]["touchPoints"][0]["y"], json!(25.0));
    assert_eq!(touches[1]["params"]["type"], json!("touchEnd"));
    assert_eq!(touches[1]["params"]["modifiers"], json!(8));
    assert_eq!(touches[1]["params"]["touchPoints"], json!([]));

    assert_eq!(
        unguarded_operations(&provenance, &session_id, &page_id),
        vec![
            "page.mouse.move",
            "page.mouse.down",
            "page.mouse.move",
            "page.mouse.up",
            "page.mouse.click",
            "page.mouse.dblclick",
            "page.mouse.wheel",
            "page.keyboard.down",
            "page.touchscreen.tap",
        ]
    );
}

/// Out-of-range counts and unknown keys fail with `invalid_params` before
/// sending any input to the browser.
#[rstest]
#[case::unknown_key("page.keyboard.down", json!({ "key": "NotAKey" }))]
#[case::too_many_steps("page.mouse.move", json!({ "x": 1.0, "y": 2.0, "steps": 1001 }))]
#[case::zero_clicks_down("page.mouse.down", json!({ "click_count": 0 }))]
#[case::too_many_clicks("page.mouse.click", json!({ "x": 1.0, "y": 2.0, "click_count": 11 }))]
fn invalid_input_params_are_refused_before_the_browser(
    #[case] method: &str,
    #[case] mut params: Value,
) {
    let mut error = Value::Null;
    let (commands, provenance, (session_id, page_id)) = drive(async |mut client, page| {
        params["page_id"] = page;
        let id = client.start(method, &params).await;
        let answer = client.answer(id).await;
        error = answer["error"].clone();
        client
    });

    assert_eq!(
        error["code"],
        json!(ProtocolFailure::InvalidParams.code()),
        "{error}"
    );
    assert!(input_commands(&commands).is_empty(), "{commands:?}");
    assert!(
        unguarded_operations(&provenance, &session_id, &page_id).is_empty(),
        "{provenance:?}"
    );
}
