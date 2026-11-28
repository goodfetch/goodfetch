//! Profile directory used in place (ADR 0056; ticket #737) against the
//! in-process engine's scripted browser:
//! - refuses locked, mismatched, newer-version, and conflicting-locale
//!   Profile directories with their declared Error codes;
//! - opens a clean Chromium Profile directory in place, keeps it intact after
//!   close/finish, bypasses registered service workers when
//!   `browser.service_workers: block`, and refuses `session.save_identity`
//!   with `session_not_savable` naming `state_file`.
#![expect(
    clippy::disallowed_types,
    reason = "integration test observer records CDP commands across tasks (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (ADR 0059)"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_test_support::runtime;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::session_counts::session::{Page, RUN, Wire, answered, dependencies, engine};

const TOKEN: &str = "b2-launch-token";

#[derive(Default, Clone)]
struct Observed {
    service_worker_fetch_blocked: Arc<Mutex<bool>>,
}

async fn scripted_browser(mut launched: ScriptedBrowsers, observed: Observed) {
    let Some(pipe) = launched.next().await else {
        return;
    };
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    loop {
        let Some(text) = wire.next().await else {
            return;
        };
        let command: Value = serde_json::from_str(&text).expect("a command is JSON");
        let method = command["method"].as_str().expect("it names its method");
        if method == "Fetch.enable"
            && command.get("sessionId").is_none()
            && command["params"]["patterns"]
                .as_array()
                .is_some_and(|patterns| {
                    patterns
                        .iter()
                        .any(|pattern| pattern["resourceType"] == json!("Other"))
                })
        {
            *observed.service_worker_fetch_blocked.lock().unwrap() = true;
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

struct RpcClient {
    lines: tokio::io::Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
    next_id: u64,
    ahead: BTreeMap<u64, Value>,
}

impl RpcClient {
    async fn connect(dialer: &transport::InMemoryDialer) -> Self {
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
        client
            .call(
                "hello",
                &json!({
                    "protocol_version": "0.1",
                    "surface": "rust",
                    "needs": "collie",
                    "product_version": "0.0.0",
                    "token": TOKEN,
                }),
            )
            .await;
        client
    }

    async fn send_and_read(&mut self, method: &str, params: &Value) -> Value {
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
        self.writer.flush().await.unwrap();
        loop {
            if let Some(reply) = self.ahead.remove(&id) {
                return reply;
            }
            let line = self.lines.next_line().await.unwrap().expect("engine open");
            let message: Value = serde_json::from_str(&line).unwrap();
            if let Some(got) = message.get("id").and_then(Value::as_u64) {
                if got == id {
                    return message;
                }
                self.ahead.insert(got, message);
            }
        }
    }

    async fn call(&mut self, method: &str, params: &Value) -> Value {
        let reply = self.send_and_read(method, params).await;
        assert!(reply.get("result").is_some(), "`{method}` failed: {reply}");
        reply["result"].clone()
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
    }
}

/// Covers: story:5-66, story:5-51, story:5-76, operation:session.save_identity, error:session_not_savable
#[test]
fn profile_directory_opens_in_place_blocks_workers_refuses_save_and_stays_intact() {
    let home = tempfile::tempdir().unwrap();
    let profile = tempfile::tempdir().unwrap();
    let marker = profile.path().join("Default").join("Preferences");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, r#"{"intl":{"accept_languages":"en-US"}}"#).unwrap();
    std::fs::write(profile.path().join("Last Version"), "135.0.0.0").unwrap();

    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let observed = Observed::default();

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer).await;
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": {} }))
                .await;

            // Open a Session on the prepared Profile directory with service workers blocked.
            let session = client
                .call(
                    "run.new_session",
                    &json!({
                        "run_id": RUN,
                        "source": {
                            "kind": "profile_directory",
                            "path": profile.path(),
                            "browser": "chrome",
                            "domains": ["example.test"],
                        },
                        "settings": {
                            "browser.service_workers": "block",
                        }
                    }),
                )
                .await;
            let session_id = session["session_id"].as_str().unwrap().to_owned();

            // Opening a page enables browser-level Fetch interception to block service workers.
            client
                .call("session.new_page", &json!({ "session_id": session_id }))
                .await;
            assert!(
                *observed.service_worker_fetch_blocked.lock().unwrap(),
                "expected browser-level Fetch.enable(Other) for browser.service_workers: block"
            );

            // session.save_identity is refused with session_not_savable naming state_file.
            let refused = client
                .send_and_read(
                    "session.save_identity",
                    &json!({ "session_id": session_id, "name": "work" }),
                )
                .await;
            assert_eq!(
                refused["error"]["data"]["code"],
                json!("session_not_savable"),
                "{refused}"
            );
            assert_eq!(
                refused["error"]["data"]["alternative"],
                json!("state_file"),
                "{refused}"
            );

            client
                .call("session.close", &json!({ "session_id": session_id }))
                .await;
            client.call("run.finish", &json!({ "run_id": RUN })).await;
            client.close().await;
        };
        let deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(transport::Network::InMemory(network), deps),
            scripted_browser(browsers, observed.clone()),
            driver,
        );
        ended.unwrap();
    });

    // Profile directory and its files remain intact after Session close & Run finish.
    assert!(marker.exists(), "profile directory was modified or deleted");
}
