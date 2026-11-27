//! The Human pause (`page.pause` and `page.pause_answer`, ADRs 0092, 0102
//! and 0113; ticket #732) against the in-process engine's scripted browser on
//! the paused clock:
//! - ends on a person's Done / Hand back / Decline (`human_pause_declined` as
//!   a `RefusedError` with `person` and `note`), on `until` (URL matcher), on
//!   short `timeout.human_pause`, and on the program's `$/cancel`;
//! - `timeout.human_pause` of `0` is refused with `setting_invalid`;
//! - `page.pause_answer` from a non-person surface is refused with
//!   `person_required`;
//! - `progress().pauses` lists the pause with its fields and removes it when
//!   it ends; `trace/pauses.jsonl` records how each pause ended; Audit log
//!   records person answers and browser closes, never condition/timeout/cancel;
//! - enclosing clocks (`timeout.run`, held Gesture's `timeout.action`) stop
//!   during a pause while reads run and a second pause waits for the first;
//! - closing the paused page's target or quitting the browser ends the pause
//!   at once with `call_cancelled` (`by: "browser"`, `closed: "page"` or
//!   `"browser"`), never `human_pause_declined`.
#![expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]
#![expect(
    clippy::too_many_lines,
    reason = "each integration test drives one multi-step JSON-RPC scenario end to end"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime};

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_protocol::home::{AUDIT_LOG, RUN_TRACE_PAUSES};
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::runtime;
use rusqlite::Connection;
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::mpsc;

use crate::session_counts::session::{Page, RUN, Wire, answered, dependencies, engine};

const TOKEN: &str = "b2-launch-token";

/// Commands the test can inject into the scripted browser loop.
enum BrowserAction {
    /// Emit `Target.targetDestroyed` for the opened page target (simulates
    /// closing the page's window outside every Client).
    DestroyPage,
    /// Emit `Page.navigatedWithinDocument` to `url` on the main frame
    /// (simulates a person navigating / completing login in the headful window).
    NavigateTo(String),
}

/// Scripted browser that answers CDP commands and reacts to injected
/// [`BrowserAction`] events.
async fn scripted_browser(
    mut launched: ScriptedBrowsers,
    mut actions: mpsc::Receiver<BrowserAction>,
) {
    let Some(pipe) = launched.next().await else {
        return;
    };
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    let mut current_url = "https://example.test/login".to_owned();
    let mut target_id = String::new();
    let mut session_id = String::new();
    let mut frame_id = String::new();
    loop {
        tokio::select! {
            incoming = wire.next() => {
                let Some(text) = incoming else {
                    return;
                };
                let command: Value = serde_json::from_str(&text).expect("a command is JSON");
                let method = command["method"].as_str().expect("it names its method");
                let expr = command["params"]["expression"].as_str().unwrap_or("");
                let mut answer = if method == "Runtime.evaluate"
                    && (expr.contains("window.location.href") || expr.contains("() => 1"))
                {
                    let value = if expr.contains("window.location.href") {
                        json!(current_url)
                    } else {
                        json!(1)
                    };
                    json!({
                        "result": {
                            "result": {
                                "type": "object",
                                "value": { "ok": value },
                            }
                        }
                    })
                } else {
                    match answered(method, &command["params"], &mut page) {
                        Ok((result, _)) => {
                            if method == "Target.createTarget"
                                && let Some(id) = result["targetId"].as_str()
                            {
                                id.clone_into(&mut target_id);
                            }
                            if method == "Target.attachToTarget"
                                && let Some(id) = result["sessionId"].as_str()
                            {
                                id.clone_into(&mut session_id);
                            }
                            if method == "Page.getFrameTree"
                                && let Some(id) = result["frameTree"]["frame"]["id"].as_str()
                            {
                                id.clone_into(&mut frame_id);
                            }
                            json!({ "result": result })
                        }
                        Err(error) => json!({ "error": error }),
                    }
                };
                answer["id"] = command["id"].clone();
                if let Some(session) = command.get("sessionId") {
                    answer["sessionId"] = session.clone();
                }
                if !wire.send(answer.to_string()).await || method == "Browser.close" {
                    return;
                }
                if method == "Target.closeTarget"
                    && let Some(closed) = command["params"]["targetId"].as_str()
                {
                    let detached = json!({
                        "method": "Target.detachedFromTarget",
                        "params": { "sessionId": session_id, "targetId": closed },
                    });
                    let _ = wire.send(detached.to_string()).await;
                    let event = json!({
                        "method": "Target.targetDestroyed",
                        "params": { "targetId": closed },
                    });
                    let _ = wire.send(event.to_string()).await;
                }
            }
            Some(action) = actions.recv() => {
                match action {
                    BrowserAction::DestroyPage => {
                        let detached = json!({
                            "method": "Target.detachedFromTarget",
                            "params": { "sessionId": session_id, "targetId": target_id },
                        });
                        let _ = wire.send(detached.to_string()).await;
                        let event = json!({
                            "method": "Target.targetDestroyed",
                            "params": { "targetId": target_id },
                        });
                        let _ = wire.send(event.to_string()).await;
                    }
                    BrowserAction::NavigateTo(url) => {
                        current_url.clone_from(&url);
                        let fid = if frame_id.is_empty() {
                            target_id.clone()
                        } else {
                            frame_id.clone()
                        };
                        let nav = json!({
                            "method": "Page.navigatedWithinDocument",
                            "sessionId": session_id,
                            "params": {
                                "frameId": fid,
                                "url": url,
                                "navigationType": "fragment",
                            }
                        });
                        let _ = wire.send(nav.to_string()).await;
                    }
                }
            }
        }
    }
}

/// Single-connection JSON-RPC client supporting concurrent in-flight calls
/// and `$/cancel` on any surface.
struct RpcClient {
    lines: tokio::io::Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
    next_id: u64,
    ahead: BTreeMap<u64, Value>,
}

impl RpcClient {
    async fn connect(
        dialer: &transport::InMemoryDialer,
        surface: &str,
        conn_settings: Value,
    ) -> Self {
        let (reader, writer) = dialer
            .connect()
            .await
            .expect("invariant: the engine listens")
            .split();
        let mut client = Self {
            lines: BufReader::new(reader).lines(),
            writer,
            next_id: 0,
            ahead: BTreeMap::new(),
        };
        let mut hello = json!({
            "protocol_version": "0.1",
            "surface": surface,
            "needs": "collie",
            "product_version": "0.0.0",
            "token": TOKEN,
        });
        if !conn_settings.is_null() {
            hello["settings"] = conn_settings;
        }
        client.call("hello", &hello).await;
        client
    }

    async fn start(&mut self, method: &str, params: &Value) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        let line = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        id
    }

    async fn cancel(&mut self, id: u64) {
        let line = json!({
            "jsonrpc": "2.0",
            "method": "$/cancel",
            "params": { "id": id },
        });
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn answer(&mut self, id: u64) -> Value {
        if let Some(msg) = self.ahead.remove(&id) {
            return msg;
        }
        loop {
            let line = self.lines.next_line().await.unwrap().unwrap();
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg.get("method").is_some() {
                continue;
            }
            match msg["id"].as_u64() {
                Some(got) if got == id => return msg,
                Some(got) => {
                    self.ahead.insert(got, msg);
                }
                None => {}
            }
        }
    }

    async fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.start(method, params).await;
        let msg = self.answer(id).await;
        msg.get("result")
            .cloned()
            .unwrap_or_else(|| panic!("`{method}` failed: {msg}"))
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
    }
}

fn pause_trace_lines(home: &Path) -> Vec<Value> {
    let path = HomeLayout::of(home).run_trace(RUN, RUN_TRACE_PAUSES);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each pause trace line is JSON"))
        .collect()
}

fn audit_pause_entries(home: &Path) -> Vec<Value> {
    let path = home.join(AUDIT_LOG);
    if !path.exists() {
        return Vec::new();
    }
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(|entry| entry["kind"] == "human_pause_ended")
        .collect()
}

/// Non-person surface calling `page.pause_answer` is refused with `person_required`.
/// Covers: story:5-82, operation:page.pause_answer, error:person_required
#[test]
fn non_person_surface_cannot_answer_human_pause() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "rust", Value::Null).await;
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": {} }))
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opened = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            let id = client
                .start(
                    "page.pause_answer",
                    &json!({ "page_id": opened["page_id"], "answer": "done" }),
                )
                .await;
            let reply = client.answer(id).await;
            assert_eq!(reply["error"]["data"]["code"], json!("person_required"));
            client.call("run.finish", &json!({ "run_id": RUN })).await;
            drop(action_tx);
            client.close().await;
        };
        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            driver,
            scripted_browser(browsers, action_rx),
        );
        ended.expect("the engine got its Connection");
    });
}

/// Person answers (Done, Hand back, Decline), short `timeout.human_pause`,
/// `timeout: 0` refusal, program cancel, `progress().pauses`, Run trace, and
/// Audit log.
/// Covers: story:5-82, operation:page.pause, operation:page.pause_answer, error:human_pause_declined, setting:timeout.human_pause
#[test]
fn human_pause_answers_decline_timeout_cancel_progress_trace_and_audit() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "cli", Value::Null).await;
            let mut settings = Map::new();
            settings.insert("timeout".to_owned(), json!({ "human_pause": 5_000 }));
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": settings }))
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opened = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            let page_id = opened["page_id"].clone();

            // 1. timeout: 0 is refused with setting_invalid.
            let zero_id = client
                .start("page.pause", &json!({ "page_id": page_id, "timeout": 0 }))
                .await;
            let zero_reply = client.answer(zero_id).await;
            assert_eq!(
                zero_reply["error"]["data"]["code"],
                json!("setting_invalid")
            );

            // 2. Done answer: appears in progress().pauses, then returns HumanPauseResult.
            let before = client.call("run.progress", &json!({ "run_id": RUN })).await;
            let pause_id = client
                .start(
                    "page.pause",
                    &json!({ "page_id": page_id, "reason": "Sign in", "timeout": null }),
                )
                .await;
            let waiting = client
                .call(
                    "run.progress",
                    &json!({ "run_id": RUN, "after": before["position"] }),
                )
                .await;
            let pauses = waiting["pauses"].as_array().unwrap();
            assert_eq!(pauses.len(), 1);
            assert_eq!(pauses[0]["session"], session["session_id"]);
            assert_eq!(pauses[0]["page"], page_id);
            assert_eq!(pauses[0]["reason"], json!("Sign in"));
            assert_eq!(pauses[0]["raised_by"], json!({ "by": "program" }));
            assert!(pauses[0].get("deadline").is_none() || pauses[0]["deadline"].is_null());

            let answered = client
                .call(
                    "page.pause_answer",
                    &json!({ "page_id": page_id, "answer": "done" }),
                )
                .await;
            assert_eq!(answered["accepted"], json!(true));

            let done_reply = client.answer(pause_id).await;
            assert_eq!(done_reply["result"]["resolved_by"], json!("person"));
            assert!(done_reply["result"]["person"].is_string());
            assert!(done_reply["result"]["waited"].is_u64());

            // Leaves progress().pauses.
            let after = client.call("run.progress", &json!({ "run_id": RUN })).await;
            assert_eq!(after["pauses"], json!([]));

            // 3. Hand back ends a program's pause as Done does.
            let hb_id = client
                .start(
                    "page.pause",
                    &json!({ "page_id": page_id, "timeout": null }),
                )
                .await;
            let _ = client
                .call(
                    "run.progress",
                    &json!({ "run_id": RUN, "after": after["position"] }),
                )
                .await;
            let _ = client
                .call(
                    "page.pause_answer",
                    &json!({ "page_id": page_id, "answer": "hand_back" }),
                )
                .await;
            let hb_reply = client.answer(hb_id).await;
            assert_eq!(hb_reply["result"]["resolved_by"], json!("person"));

            // 4. Decline raises human_pause_declined (RefusedError) with note.
            let pos =
                client.call("run.progress", &json!({ "run_id": RUN })).await["position"].clone();
            let decl_id = client
                .start(
                    "page.pause",
                    &json!({ "page_id": page_id, "timeout": null }),
                )
                .await;
            let _ = client
                .call("run.progress", &json!({ "run_id": RUN, "after": pos }))
                .await;
            let _ = client
                .call(
                    "page.pause_answer",
                    &json!({
                        "page_id": page_id,
                        "answer": "decline",
                        "note": "Wrong account",
                    }),
                )
                .await;
            let decl_reply = client.answer(decl_id).await;
            assert_eq!(
                decl_reply["error"]["data"]["code"],
                json!("human_pause_declined")
            );
            assert_eq!(
                goodfetch_protocol::page::pause::HUMAN_PAUSE_DECLINED.group,
                goodfetch_protocol::catalog::ErrorGroup::Refused
            );
            assert_eq!(decl_reply["error"]["data"]["note"], json!("Wrong account"));

            // 5. Short timeout.human_pause expires with deadline_exceeded (and shows deadline in progress).
            let pos =
                client.call("run.progress", &json!({ "run_id": RUN })).await["position"].clone();
            let timeout_id = client
                .start("page.pause", &json!({ "page_id": page_id }))
                .await;
            let timed_waiting = client
                .call("run.progress", &json!({ "run_id": RUN, "after": pos }))
                .await;
            assert!(timed_waiting["pauses"][0]["deadline"].is_string());
            let timeout_reply = client.answer(timeout_id).await;
            assert_eq!(
                timeout_reply["error"]["data"]["code"],
                json!("deadline_exceeded")
            );
            assert_eq!(
                timeout_reply["error"]["data"]["limit"]["key"],
                json!("timeout.human_pause")
            );

            // 6. Program cancel ($/cancel) ends the pause cleanly.
            let pos =
                client.call("run.progress", &json!({ "run_id": RUN })).await["position"].clone();
            let cancel_id = client
                .start(
                    "page.pause",
                    &json!({ "page_id": page_id, "timeout": null }),
                )
                .await;
            let _ = client
                .call("run.progress", &json!({ "run_id": RUN, "after": pos }))
                .await;
            client.cancel(cancel_id).await;
            let _cancel_reply = client.answer(cancel_id).await;

            client.call("run.finish", &json!({ "run_id": RUN })).await;
            drop(action_tx);
            client.close().await;
        };

        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            driver,
            scripted_browser(browsers, action_rx),
        );
        ended.expect("the engine got its Connection");
    });

    let trace = pause_trace_lines(home.path());
    let endings: Vec<&str> = trace.iter().filter_map(|e| e["ended"].as_str()).collect();
    assert_eq!(
        endings,
        vec!["done", "hand_back", "declined", "timeout", "cancelled"]
    );

    // Audit log holds only person answers (done, hand_back, declined) — never timeout or cancel.
    let audit = audit_pause_entries(home.path());
    let audit_endings: Vec<&str> = audit
        .iter()
        .filter_map(|e| e["details"]["ended"].as_str())
        .collect();
    assert_eq!(audit_endings, vec!["done", "hand_back", "declined"]);
}

/// `until` URL matcher: "Done" is rejected until `until` holds, then `until`
/// resolves the pause automatically with `resolved_by: "condition"`.
/// Covers: story:5-82, operation:page.pause, operation:page.pause_answer
#[test]
fn human_pause_until_url_gates_done_and_resolves_on_condition() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "cli", Value::Null).await;
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": {} }))
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opened = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            let page_id = opened["page_id"].clone();

            let before = client.call("run.progress", &json!({ "run_id": RUN })).await;
            let pause_id = client
                .start(
                    "page.pause",
                    &json!({
                        "page_id": page_id,
                        "timeout": null,
                        "until": {
                            "kind": "url",
                            "url": { "kind": "glob", "pattern": "**/account" }
                        }
                    }),
                )
                .await;
            let _ = client
                .call(
                    "run.progress",
                    &json!({ "run_id": RUN, "after": before["position"] }),
                )
                .await;

            // Done before URL matches is rejected.
            let early = client
                .call(
                    "page.pause_answer",
                    &json!({ "page_id": page_id, "answer": "done" }),
                )
                .await;
            assert_eq!(early["accepted"], json!(false));

            // Person completes login in browser -> navigates to /account -> condition resolves pause.
            action_tx
                .send(BrowserAction::NavigateTo(
                    "https://example.test/account".to_owned(),
                ))
                .await
                .unwrap();

            let reply = client.answer(pause_id).await;
            assert_eq!(reply["result"]["resolved_by"], json!("condition"));
            assert!(reply["result"]["person"].is_null());

            client.call("run.finish", &json!({ "run_id": RUN })).await;
            drop(action_tx);
            client.close().await;
        };

        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            driver,
            scripted_browser(browsers, action_rx),
        );
        ended.expect("the engine got its Connection");
    });

    let trace = pause_trace_lines(home.path());
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0]["ended"], json!("condition"));
    assert_eq!(audit_pause_entries(home.path()), Vec::<Value>::new());
}

/// Enclosing clocks (`timeout.run`, held Gesture's `timeout.action`) stop
/// during a pause; reads run while paused; a second pause waits for the first.
/// Covers: story:5-82, operation:page.pause, setting:timeout.action, setting:timeout.run
#[test]
fn human_pause_stops_enclosing_clocks_holds_gestures_allows_reads_and_queues_second_pause() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client =
                RpcClient::connect(&dialer, "cli", json!({ "timeout": { "run": 400 } })).await;
            let mut settings = Map::new();
            settings.insert("timeout".to_owned(), json!({ "action": 200 }));
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": settings }))
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opened = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            let page_id = opened["page_id"].clone();

            let before = client.call("run.progress", &json!({ "run_id": RUN })).await;
            let pause1_id = client
                .start(
                    "page.pause",
                    &json!({ "page_id": page_id, "timeout": null }),
                )
                .await;
            let _ = client
                .call(
                    "run.progress",
                    &json!({ "run_id": RUN, "after": before["position"] }),
                )
                .await;

            // Queue a Gesture (timeout.action = 200ms) and a second pause while paused.
            let gesture_id = client
                .start(
                    "page.keyboard.press",
                    &json!({ "page_id": page_id, "key": "a" }),
                )
                .await;
            let pause2_id = client
                .start(
                    "page.pause",
                    &json!({ "page_id": page_id, "timeout": null }),
                )
                .await;

            // A Read runs immediately while paused.
            let eval_result = client
                .call(
                    "page.evaluate",
                    &json!({ "page_id": page_id, "function": "() => 1" }),
                )
                .await;
            assert!(eval_result.is_object());

            // Advance the paused clock by 1000ms (> timeout.action 200ms AND > timeout.run 400ms).
            tokio::time::sleep(Duration::from_millis(1000)).await;

            // End first pause -> Gesture succeeds and second pause starts.
            let _ = client
                .call(
                    "page.pause_answer",
                    &json!({ "page_id": page_id, "answer": "done" }),
                )
                .await;
            let p1_reply = client.answer(pause1_id).await;
            assert_eq!(p1_reply["result"]["resolved_by"], json!("person"));

            // Wait until second pause appears in progress().pauses, then answer it too.
            let mut progress = client.call("run.progress", &json!({ "run_id": RUN })).await;
            while progress["pauses"].as_array().is_none_or(Vec::is_empty) {
                progress = client
                    .call(
                        "run.progress",
                        &json!({ "run_id": RUN, "after": progress["position"] }),
                    )
                    .await;
            }
            let _ = client
                .call(
                    "page.pause_answer",
                    &json!({ "page_id": page_id, "answer": "done" }),
                )
                .await;
            let p2_reply = client.answer(pause2_id).await;
            assert_eq!(p2_reply["result"]["resolved_by"], json!("person"));

            let gesture_reply = client.answer(gesture_id).await;
            assert!(
                gesture_reply.get("result").is_some(),
                "held gesture must pass after pause: {gesture_reply}"
            );

            // Run is still alive (timeout.run did not fire during the 1000ms pause!).
            let report = client.call("run.finish", &json!({ "run_id": RUN })).await;
            assert_eq!(report["lifecycle"], json!("finished"), "{report}");
            drop(action_tx);
            client.close().await;
        };

        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            driver,
            scripted_browser(browsers, action_rx),
        );
        ended.expect("the engine got its Connection");
    });
}

/// Closing a paused page's window (`Target.targetDestroyed`) or quitting the
/// browser ends the pause at once with `call_cancelled` (`by: "browser"`,
/// `closed: "page"` / `"browser"`), never `human_pause_declined`, and records
/// both Run trace and Audit log.
/// Covers: story:5-83, operation:page.pause
#[test]
fn human_pause_ends_with_call_cancelled_on_page_close_and_browser_quit() {
    // Part A: page window closed outside every Client.
    let home_page = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine_page = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "rust", Value::Null).await;
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": {} }))
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opened = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            let page_id = opened["page_id"].clone();

            let before = client.call("run.progress", &json!({ "run_id": RUN })).await;
            let pause_id = client
                .start(
                    "page.pause",
                    &json!({ "page_id": page_id, "timeout": null }),
                )
                .await;
            let _ = client
                .call(
                    "run.progress",
                    &json!({ "run_id": RUN, "after": before["position"] }),
                )
                .await;
            let gesture_id = client
                .start(
                    "page.keyboard.press",
                    &json!({ "page_id": page_id, "key": "a" }),
                )
                .await;

            action_tx.send(BrowserAction::DestroyPage).await.unwrap();

            let pause_reply = client.answer(pause_id).await;
            assert_eq!(
                pause_reply["error"]["data"]["code"],
                json!("call_cancelled")
            );
            assert_eq!(pause_reply["error"]["data"]["by"], json!("browser"));
            assert_eq!(pause_reply["error"]["data"]["closed"], json!("page"));

            let gesture_reply = client.answer(gesture_id).await;
            assert_eq!(
                gesture_reply["error"]["data"]["code"],
                json!("call_cancelled")
            );
            assert_eq!(gesture_reply["error"]["data"]["by"], json!("browser"));
            assert_eq!(gesture_reply["error"]["data"]["closed"], json!("page"));

            // Later call on the closed page raises `closed`.
            let after_id = client
                .start(
                    "page.evaluate",
                    &json!({ "page_id": page_id, "function": "() => 1" }),
                )
                .await;
            let after_reply = client.answer(after_id).await;
            assert_eq!(after_reply["error"]["data"]["code"], json!("closed"));

            // Session stays open and opens a new page.
            let second_page = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            assert!(second_page["page_id"].is_string());

            client.call("run.finish", &json!({ "run_id": RUN })).await;
            drop(action_tx);
            client.close().await;
        };

        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home_page.path())
        };
        let (ended, (), ()) = tokio::join!(
            engine_page.run_attached(Network::InMemory(network), dependencies),
            driver,
            scripted_browser(browsers, action_rx),
        );
        ended.expect("the engine got its Connection");
    });

    let trace = pause_trace_lines(home_page.path());
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0]["ended"], json!("stopped"));
    assert_eq!(trace[0]["by"], json!("browser"));
    assert_eq!(trace[0]["closed"], json!("page"));
    let audit = audit_pause_entries(home_page.path());
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0]["details"]["by"], json!("browser"));
    assert_eq!(audit[0]["details"]["closed"], json!("page"));

    // Part B: program closes the paused page from another call -> call_cancelled (by: program, closed: page).
    let home_prog = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine_prog = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "rust", Value::Null).await;
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": {} }))
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let opened = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            let page_id = opened["page_id"].clone();

            let before = client.call("run.progress", &json!({ "run_id": RUN })).await;
            let pause_id = client
                .start(
                    "page.pause",
                    &json!({ "page_id": page_id, "timeout": null }),
                )
                .await;
            let _ = client
                .call(
                    "run.progress",
                    &json!({ "run_id": RUN, "after": before["position"] }),
                )
                .await;

            client
                .call("page.close", &json!({ "page_id": page_id }))
                .await;

            let pause_reply = client.answer(pause_id).await;
            assert_eq!(
                pause_reply["error"]["data"]["code"],
                json!("call_cancelled")
            );
            assert_eq!(pause_reply["error"]["data"]["by"], json!("program"));
            assert_eq!(pause_reply["error"]["data"]["closed"], json!("page"));

            client.call("run.finish", &json!({ "run_id": RUN })).await;
            drop(action_tx);
            client.close().await;
        };

        let dependencies = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home_prog.path())
        };
        let (ended, (), ()) = tokio::join!(
            engine_prog.run_attached(Network::InMemory(network), dependencies),
            driver,
            scripted_browser(browsers, action_rx),
        );
        ended.expect("the engine got its Connection");
    });

    let trace = pause_trace_lines(home_prog.path());
    assert_eq!(trace.len(), 1);
    assert_eq!(trace[0]["ended"], json!("stopped"));
    assert_eq!(trace[0]["by"], json!("program"));
    assert_eq!(trace[0]["closed"], json!("page"));
}
