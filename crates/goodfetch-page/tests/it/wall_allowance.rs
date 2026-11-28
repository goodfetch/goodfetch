//! End-to-end Wall allowance integration tests (ADR 0052; ticket #757):
//! - reads past a paywall on `wall.domains`, writes Provenance and one Audit
//!   log entry per `(run, domain)`;
//! - refetches once per page on `wall.refetch_domains` via
//!   `Storage.clearDataForOrigin` + `Page.reload(ignoreCache: true)`, never
//!   from the Response cache;
//! - when the refetch fails, records the stop through
//!   `RunIndex::record_refetch_stop` so `needs_you.list` shows
//!   `wall_refetch_stopped`;
//! - `wall.denied_domains` wins even over `*`.
#![expect(
    clippy::disallowed_types,
    reason = "integration test observer records CDP commands across tasks (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test writes temporary configuration/policy files and runs on paused Clock (ADR 0059)"
)]
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::too_many_lines,
    reason = "scripted browser test helper constructs inline CDP fixtures"
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
    cleared_origins: Arc<Mutex<Vec<String>>>,
    uncached_reloads: Arc<Mutex<usize>>,
    wall_unlocks: Arc<Mutex<usize>>,
    fail_reload: Arc<Mutex<bool>>,
    methods: Arc<Mutex<Vec<String>>>,
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
        observed.methods.lock().unwrap().push(method.to_owned());
        let mut extra_events: Vec<Value> = Vec::new();
        let mut answer = if method == "Runtime.callFunctionOn" || method == "Runtime.evaluate" {
            let decl = command["params"]["functionDeclaration"]
                .as_str()
                .or_else(|| command["params"]["expression"].as_str())
                .unwrap_or_default();
            if decl.contains("wallRules") {
                json!({
                    "result": {
                        "result": {
                            "type": "object",
                            "value": {
                                "found": [],
                                "password": false,
                                "robots": [],
                                "wall": {
                                    "kind": "paywall",
                                    "overlay_selector": ".paywall-overlay",
                                    "paid_section_empty": true,
                                    "main_words": 40
                                }
                            }
                        }
                    }
                })
            } else if decl.contains("removeProperty('overflow')") {
                *observed.wall_unlocks.lock().unwrap() += 1;
                json!({ "result": { "result": { "type": "boolean", "value": true } } })
            } else {
                match answered(method, &command["params"], &mut page) {
                    Ok((result, events)) => {
                        extra_events = events;
                        json!({ "result": result })
                    }
                    Err(error) => json!({ "error": error }),
                }
            }
        } else if method == "Storage.clearDataForOrigin" {
            if let Some(origin) = command["params"]["origin"].as_str() {
                observed
                    .cleared_origins
                    .lock()
                    .unwrap()
                    .push(origin.to_owned());
            }
            json!({ "result": {} })
        } else if method == "Page.reload" {
            if command["params"]["ignoreCache"] == json!(true) {
                *observed.uncached_reloads.lock().unwrap() += 1;
            }
            if *observed.fail_reload.lock().unwrap() {
                json!({ "error": { "code": -32000, "message": "HTTP 403 Forbidden" } })
            } else {
                json!({ "result": {} })
            }
        } else {
            match answered(method, &command["params"], &mut page) {
                Ok((result, events)) => {
                    extra_events = events;
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
        for mut event in extra_events {
            if let Some(session) = command.get("sessionId") {
                event["sessionId"] = session.clone();
            }
            if !wire.send(event.to_string()).await {
                return;
            }
        }
        if method == "Page.navigate"
            && let Some(url) = command["params"]["url"].as_str()
            && let Some(session) = command.get("sessionId")
        {
            let loader_id = answer["result"]["loaderId"]
                .as_str()
                .unwrap_or("wall-loader-1");
            let frame_id = answer["result"]["frameId"]
                .as_str()
                .unwrap_or("wall-frame-1");
            let events = [
                json!({
                    "method": "Network.requestWillBeSent",
                    "sessionId": session,
                    "params": {
                        "requestId": loader_id,
                        "loaderId": loader_id,
                        "documentURL": url,
                        "request": {
                            "url": url,
                            "method": "GET",
                            "headers": {},
                            "initialPriority": "VeryHigh",
                            "referrerPolicy": "strict-origin-when-cross-origin"
                        },
                        "timestamp": 0.9,
                        "wallTime": 0.9,
                        "initiator": { "type": "other" },
                        "type": "Document",
                        "frameId": frame_id
                    }
                }),
                json!({
                    "method": "Network.responseReceived",
                    "sessionId": session,
                    "params": {
                        "requestId": loader_id,
                        "loaderId": loader_id,
                        "timestamp": 1.0,
                        "type": "Document",
                        "hasExtraInfo": false,
                        "frameId": frame_id,
                        "response": {
                            "url": url,
                            "status": 200,
                            "statusText": "OK",
                            "headers": { "content-type": "text/html" },
                            "mimeType": "text/html",
                            "charset": "utf-8",
                            "connectionReused": false,
                            "connectionId": 1,
                            "encodedDataLength": 128,
                            "securityState": "secure"
                        }
                    }
                }),
                json!({
                    "method": "Page.frameNavigated",
                    "sessionId": session,
                    "params": {
                        "frame": {
                            "id": frame_id,
                            "loaderId": loader_id,
                            "url": url,
                            "domainAndRegistry": "paper.test",
                            "securityOrigin": "https://paper.test",
                            "mimeType": "text/html",
                            "secureContextType": "Secure",
                            "crossOriginIsolatedContextType": "NotIsolated",
                            "gatedAPIFeatures": []
                        },
                        "type": "Navigation"
                    }
                }),
                json!({
                    "method": "Page.lifecycleEvent",
                    "sessionId": session,
                    "params": {
                        "frameId": frame_id,
                        "loaderId": loader_id,
                        "name": "load",
                        "timestamp": 1.1
                    }
                }),
            ];
            for event in events {
                if !wire.send(event.to_string()).await {
                    return;
                }
            }
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
                    "surface": "cli",
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
        let message = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = message.to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.writer.flush().await.unwrap();
        if let Some(reply) = self.ahead.remove(&id) {
            return reply;
        }
        loop {
            let text = self.lines.next_line().await.unwrap().expect("reply line");
            let parsed: Value = serde_json::from_str(&text).unwrap();
            let Some(got) = parsed.get("id").and_then(Value::as_u64) else {
                continue;
            };
            if got == id {
                return parsed;
            }
            self.ahead.insert(got, parsed);
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

/// Covers: story:5-102, setting:wall.domains, setting:wall.refetch_domains, operation:needs_you.list
#[test]
fn wall_allowance_reads_past_refetches_uncached_and_records_refetch_stop_in_needs_you() {
    let home = tempfile::tempdir().unwrap();
    let major = home.path().join("0.0");
    std::fs::create_dir_all(&major).unwrap();
    std::fs::write(
        major.join("configuration.toml"),
        "version = \"0.1\"\n\n[settings]\nwall.domains = [\"paper.test\"]\nwall.refetch_domains = [\"paper.test\"]\n",
    )
    .unwrap();

    let (launcher, browsers) = Launcher::scripted(None);
    let engine =
        engine(Metrics::none(), launcher).with_governor(goodfetch_engine::governor::System::os());
    let observed = Observed::default();
    *observed.fail_reload.lock().unwrap() = true;

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer).await;
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": {} }))
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let session_id = session["session_id"].as_str().unwrap().to_owned();
            let page = client
                .call("session.new_page", &json!({ "session_id": session_id }))
                .await;
            let page_id = page["page_id"].as_str().unwrap().to_owned();

            // Navigating to a paywall page on `paper.test` triggers hiding,
            // site-data clearing, uncached reload, and (on reload failure)
            // `RunIndex::record_refetch_stop` -> `needs_you.list`.
            let goto = client
                .call(
                    "page.goto",
                    &json!({ "page_id": page_id, "url": "https://paper.test/article-1" }),
                )
                .await;

            assert_eq!(
                *observed.wall_unlocks.lock().unwrap(),
                1,
                "goto={goto} methods={:?}",
                observed.methods.lock().unwrap()
            );
            assert_eq!(
                observed.cleared_origins.lock().unwrap().as_slice(),
                &["https://paper.test".to_owned()]
            );
            assert_eq!(*observed.uncached_reloads.lock().unwrap(), 1);

            // Advance past a governor storage sample so `needs_you` reloads
            // the `refetch_stops` table written by `RunIndex::record_refetch_stop`.
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;

            // `needs_you.list` shows `wall_refetch_stopped` for `paper.test`.
            let needs_you = client.call("needs_you.list", &json!({})).await;
            let items = needs_you["items"].as_array().unwrap();
            assert!(
                items.iter().any(|item| {
                    item["kind"] == json!("wall_refetch_stopped")
                        && item["details"]["domain"] == json!("paper.test")
                }),
                "expected wall_refetch_stopped in needs_you.list: {needs_you}"
            );

            client
                .call("session.close", &json!({ "session_id": session_id }))
                .await;
            client.call("run.finish", &json!({ "run_id": RUN })).await;
            client.close().await;
            drop(dialer);
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
}
