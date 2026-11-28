//! Borrowing the program's own browser over the SDK's JSON-RPC connection
//! (`run.borrow()`, ADRs 0056, 0078, 0089, 0092 and 0140; ticket #744):
//! - relays CDP frames in both directions over JSON-RPC (`$/borrow_command`
//!   and `$/borrow_frame`) under `$/credit` with no listening port;
//! - maps each borrowed browser context to one Session of the Run (joining
//!   an existing Session when borrowing a second page in the same context);
//! - Collie's calls act on the borrowed page while the client's own browser
//!   commands keep working;
//! - closing the Run never closes the borrowed browser or its pages;
//! - `$/cancel` ends the borrow; closing the page from the crawler's side
//!   makes the Collie Page report `closed`; retiring the browser ends the
//!   borrow;
//! - Firefox and WebKit are refused with `borrow_requires_cdp`;
//! - withholding `$/credit` holds the outgoing relay stream;
//! - Secret placeholders raise `secret_input_refused`, results are marked
//!   `unguarded`, and `session.save_identity` raises `session_not_savable`.
//!
//! Covers: story:5-70, error:borrow_requires_cdp, error:secret_input_refused, error:session_not_savable
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
    reason = "each integration test drives one multi-step JSON-RPC borrow relay scenario end to end"
)]

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{
    self, ConnectionReader, ConnectionWriter, InMemoryDialer, Network,
};
use goodfetch_page::Launcher;
use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::runtime;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Mutex;

use crate::session_counts::session::{Page, RUN, answered, dependencies, engine};

/// Simulated crawler-owned Chromium browser that answers CDP commands over
/// the `run.borrow()` JSON-RPC relay and records every command sent to it.
#[derive(Clone)]
struct FakeCrawlerBrowser {
    state: Arc<Mutex<FakeBrowserState>>,
}

struct FakeBrowserState {
    default_context: String,
    targets: BTreeMap<String, String>,
    attached: BTreeMap<String, String>,
    next_session: u32,
    page: Page,
    seen_methods: Vec<String>,
}

impl FakeCrawlerBrowser {
    fn new(default_context: &str, targets: &[(&str, &str)]) -> Self {
        let mut map = BTreeMap::new();
        for (tid, ctx) in targets {
            map.insert((*tid).to_owned(), (*ctx).to_owned());
        }
        Self {
            state: Arc::new(Mutex::new(FakeBrowserState {
                default_context: default_context.to_owned(),
                targets: map,
                attached: BTreeMap::new(),
                next_session: 1,
                page: Page::blank(),
                seen_methods: Vec::new(),
            })),
        }
    }

    /// Processes one raw CDP JSON command from the engine and returns the raw
    /// CDP JSON response string.
    async fn handle_command(&self, raw: &str) -> String {
        let cmd: Value = serde_json::from_str(raw).expect("CDP command is JSON");
        let method = cmd["method"].as_str().expect("CDP command names method");
        let params = cmd.get("params").cloned().unwrap_or_else(|| json!({}));
        let mut state = self.state.lock().await;
        state.seen_methods.push(method.to_owned());

        let mut answer = match method {
            "Browser.getVersion" => json!({
                "result": {
                    "protocolVersion": "1.3",
                    "product": "Chrome/135.0.7049.0",
                    "revision": "0",
                    "userAgent": "Chrome/135.0.7049.0",
                    "jsVersion": "0",
                }
            }),
            "Target.getBrowserContexts" => {
                let mut others: Vec<String> = state
                    .targets
                    .values()
                    .filter(|c| **c != state.default_context)
                    .cloned()
                    .collect();
                others.sort();
                others.dedup();
                json!({
                    "result": {
                        "browserContextIds": others,
                        "defaultBrowserContextId": state.default_context,
                    }
                })
            }
            "Target.getTargetInfo" => {
                let tid = params
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or("target-a1");
                let ctx = state
                    .targets
                    .get(tid)
                    .cloned()
                    .unwrap_or_else(|| state.default_context.clone());
                json!({
                    "result": {
                        "targetInfo": {
                            "targetId": tid,
                            "type": "page",
                            "title": "Crawler page",
                            "url": "https://example.test/",
                            "attached": true,
                            "canAccessOpener": false,
                            "browserContextId": ctx,
                        }
                    }
                })
            }
            "Target.attachToTarget" => {
                let tid = params
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or("target-a1")
                    .to_owned();
                let sid = format!("cdp-sid-{tid}-{}", state.next_session);
                state.next_session += 1;
                state.attached.insert(tid, sid.clone());
                json!({ "result": { "sessionId": sid } })
            }
            "Target.detachFromTarget" | "Target.closeTarget" => {
                json!({ "result": { "success": true } })
            }
            _ => match answered(method, &params, &mut state.page) {
                Ok((result, _)) => json!({ "result": result }),
                Err(error) => json!({ "error": error }),
            },
        };
        answer["id"] = cmd["id"].clone();
        if let Some(session) = cmd.get("sessionId") {
            answer["sessionId"] = session.clone();
        }
        answer.to_string()
    }

    async fn session_for_target(&self, target_id: &str) -> Option<String> {
        self.state.lock().await.attached.get(target_id).cloned()
    }

    async fn seen_methods(&self) -> Vec<String> {
        self.state.lock().await.seen_methods.clone()
    }
}

/// Multiplexing JSON-RPC client for testing `run.borrow()` alongside ordinary
/// Collie RPC calls on the same connection.
struct BorrowRpcClient {
    reader: BufReader<ConnectionReader>,
    writer: ConnectionWriter,
    next_id: u64,
    relays: BTreeMap<u64, FakeCrawlerBrowser>,
    buffered_notifications: VecDeque<Value>,
    buffered_replies: BTreeMap<u64, Value>,
    auto_credit: bool,
}

impl BorrowRpcClient {
    async fn connect(dialer: &InMemoryDialer) -> Self {
        let (read, write) = dialer
            .connect()
            .await
            .expect("invariant: the engine listens")
            .split();
        let mut client = Self {
            reader: BufReader::new(read),
            writer: write,
            next_id: 1,
            relays: BTreeMap::new(),
            buffered_notifications: VecDeque::new(),
            buffered_replies: BTreeMap::new(),
            auto_credit: true,
        };
        let hello = client
            .call(
                "hello",
                json!({
                    "protocol_version": "0.1",
                    "surface": "rust",
                    "needs": "collie",
                    "product_version": "0.0.0",
                }),
            )
            .await;
        assert!(hello.get("result").is_some(), "hello failed: {hello}");
        client
    }

    async fn send_json(&mut self, msg: &Value) {
        let line = serde_json::to_string(msg).unwrap();
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
    }

    async fn send_request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await;
        id
    }

    async fn grant_credits(&mut self, id: u64, credits: u32) {
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "method": "$/credit",
            "params": { "id": id, "credits": credits },
        }))
        .await;
    }

    async fn send_borrow_frame(&mut self, id: u64, frame: &str) {
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "method": "$/borrow_frame",
            "params": { "id": id, "frame": frame },
        }))
        .await;
    }

    async fn send_borrow_end(&mut self, id: u64) {
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "method": "$/borrow_frame",
            "params": { "id": id, "end": true },
        }))
        .await;
    }

    async fn cancel(&mut self, id: u64) {
        self.send_json(&json!({
            "jsonrpc": "2.0",
            "method": "$/cancel",
            "params": { "id": id },
        }))
        .await;
    }

    /// Reads one message from the wire, servicing any active `$/borrow_command`
    /// relay frames transparently when `auto_credit` is enabled.
    async fn pump_one(&mut self) -> Value {
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.reader.read_line(&mut buf).await.unwrap();
            assert!(n > 0, "connection closed unexpectedly");
            let msg: Value = serde_json::from_str(&buf).unwrap();
            if msg.get("method").and_then(Value::as_str) == Some("$/borrow_command") {
                let id = msg["params"]["id"].as_u64().unwrap();
                let frame = msg["params"]["frame"].as_str().unwrap().to_owned();
                if let Some(browser) = self.relays.get(&id).cloned() {
                    let reply = browser.handle_command(&frame).await;
                    self.send_borrow_frame(id, &reply).await;
                    if self.auto_credit {
                        self.grant_credits(id, 1).await;
                    }
                    continue;
                }
            }
            return msg;
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.send_request(method, params).await;
        self.wait_reply(id).await
    }

    async fn wait_reply(&mut self, id: u64) -> Value {
        if let Some(reply) = self.buffered_replies.remove(&id) {
            return reply;
        }
        loop {
            let msg = self.pump_one().await;
            if let Some(reply_id) = msg.get("id").and_then(Value::as_u64) {
                if reply_id == id {
                    return msg;
                }
                self.buffered_replies.insert(reply_id, msg);
            } else {
                self.buffered_notifications.push_back(msg);
            }
        }
    }

    /// Starts `run.borrow`, grants initial credits, relays CDP frames until
    /// `$/borrow_ready` arrives, and returns `(call_id, ready_params)`.
    async fn start_borrow(&mut self, params: Value, browser: FakeCrawlerBrowser) -> (u64, Value) {
        let id = self.send_request("run.borrow", params).await;
        self.relays.insert(id, browser);
        self.grant_credits(id, 64).await;
        loop {
            if let Some(pos) = self.buffered_notifications.iter().position(|n| {
                n.get("method").and_then(Value::as_str) == Some("$/borrow_ready")
                    && n["params"]["id"].as_u64() == Some(id)
            }) {
                let msg = self.buffered_notifications.remove(pos).unwrap();
                return (id, msg["params"].clone());
            }
            let msg = self.pump_one().await;
            if msg.get("method").and_then(Value::as_str) == Some("$/borrow_ready")
                && msg["params"]["id"].as_u64() == Some(id)
            {
                return (id, msg["params"].clone());
            }
            if let Some(reply_id) = msg.get("id").and_then(Value::as_u64) {
                assert!(
                    reply_id != id,
                    "run.borrow returned before $/borrow_ready: {msg}"
                );
                self.buffered_replies.insert(reply_id, msg);
            } else {
                self.buffered_notifications.push_back(msg);
            }
        }
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
    }
}

fn provenance_entries(home: &Path, run_id: &str) -> Vec<Value> {
    let path = HomeLayout::of(home).run_trace(run_id, RUN_TRACE_PROVENANCE);
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Covers: story:5-70, error:secret_input_refused, error:session_not_savable
#[test]
fn run_borrow_maps_contexts_to_sessions_acts_on_page_unguarded_and_refuses_secrets_and_save() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, _browsers) = Launcher::scripted(None);
    let eng = engine(Metrics::none(), launcher);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = BorrowRpcClient::connect(&dialer).await;

            let started = client
                .call(
                    "run.start",
                    json!({
                        "run_id": RUN,
                        "settings": {
                            "secrets.placeholders": {
                                "account_password": {
                                    "reference": "env:BORROW_TEST_SECRET",
                                    "domains": ["*"],
                                }
                            }
                        }
                    }),
                )
                .await;
            assert!(
                started.get("result").is_some(),
                "run.start failed: {started}"
            );

            let browser = FakeCrawlerBrowser::new(
                "ctx-default",
                &[
                    ("target-a1", "ctx-default"),
                    ("target-a2", "ctx-default"),
                    ("target-b1", "ctx-incognito"),
                ],
            );

            // Borrow page in ctx-default.
            let (borrow_a1, ready_a1) = client
                .start_borrow(
                    json!({
                        "run_id": RUN,
                        "browser": "chromium",
                        "target_id": "target-a1",
                    }),
                    browser.clone(),
                )
                .await;
            let session_a = ready_a1["session_id"].as_str().unwrap().to_owned();
            let page_a1 = ready_a1["page_id"].as_str().unwrap().to_owned();
            assert_eq!(ready_a1["browser_context_id"], json!("ctx-default"));

            // Borrowing a second page in the same context joins the same Session.
            let (borrow_a2, ready_a2) = client
                .start_borrow(
                    json!({
                        "run_id": RUN,
                        "target_id": "target-a2",
                    }),
                    browser.clone(),
                )
                .await;
            assert_eq!(ready_a2["session_id"], json!(session_a));
            assert_ne!(ready_a2["page_id"], json!(page_a1));

            // Borrowing a page in a different context becomes a distinct Session of the Run.
            let (incognito_borrow, incognito_ready) = client
                .start_borrow(
                    json!({
                        "run_id": RUN,
                        "target_id": "target-b1",
                    }),
                    browser.clone(),
                )
                .await;
            let session_b = incognito_ready["session_id"].as_str().unwrap().to_owned();
            assert_ne!(session_b, session_a);
            assert_eq!(
                incognito_ready["browser_context_id"],
                json!("ctx-incognito")
            );

            // Collie's calls act on the borrowed page and results are marked unguarded.
            let formats = client
                .call(
                    "page.formats",
                    json!({
                        "page_id": page_a1,
                        "formats": { "html": true },
                    }),
                )
                .await;
            assert!(
                formats.get("result").is_some(),
                "page.formats on borrowed page failed: {formats}"
            );

            // The client's own CDP commands on the same browser keep working.
            let client_own = browser
                .handle_command(r#"{"id":999,"method":"Browser.getVersion","params":{}}"#)
                .await;
            assert!(client_own.contains("Chrome/135"));

            // Secret placeholder on a borrowed page raises secret_input_refused.
            let secret_fill = client
                .call(
                    "locator.fill",
                    json!({
                        "locator": { "page_id": page_a1, "selector": "#password" },
                        "value": "",
                        "secret": "account_password",
                    }),
                )
                .await;
            assert_eq!(
                secret_fill["error"]["data"]["code"],
                json!("secret_input_refused"),
                "expected secret_input_refused on borrowed page: {secret_fill}"
            );

            // session.save_identity on a borrowed Session raises session_not_savable.
            let save = client
                .call(
                    "session.save_identity",
                    json!({
                        "session_id": session_a,
                        "name": "borrowed-login",
                    }),
                )
                .await;
            assert_eq!(
                save["error"]["data"]["code"],
                json!("session_not_savable"),
                "expected session_not_savable on borrowed session: {save}"
            );

            // Cancel borrows cleanly; closing the Run never sends Browser.close.
            client.cancel(borrow_a1).await;
            client.cancel(borrow_a2).await;
            client.cancel(incognito_borrow).await;
            let ended_a1 = client.wait_reply(borrow_a1).await;
            assert!(
                ended_a1["error"]["data"]["code"] == json!("call_cancelled")
                    || ended_a1["result"]["reason"] == json!("cancelled"),
                "expected $/cancel to end run.borrow cleanly: {ended_a1}"
            );

            let finished = client.call("run.finish", json!({ "run_id": RUN })).await;
            assert!(
                finished.get("result").is_some(),
                "run.finish failed: {finished}"
            );

            // Provenance marks borrowed results as unguarded.
            let prov = provenance_entries(home.path(), RUN);
            assert!(
                prov.iter()
                    .any(|entry| entry.get("unguarded") == Some(&json!(true))),
                "expected unguarded provenance entry, got: {prov:?}"
            );

            let seen = browser.seen_methods().await;
            assert!(
                !seen.iter().any(|m| m == "Browser.close"),
                "Collie must never close a borrowed browser: {seen:?}"
            );
            assert!(
                !seen.iter().any(|m| m == "Target.closeTarget"),
                "Collie must never close a borrowed page target: {seen:?}"
            );

            client.close().await;
        };
        let deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, ()) = tokio::join!(eng.run_attached(Network::InMemory(network), deps), driver,);
        assert!(ended.is_ok(), "engine ended with error: {ended:?}");
    });
}

/// Covers: story:5-70
#[test]
fn run_borrow_ends_on_crawler_page_close_and_browser_retire_and_never_closes_on_run_end() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, _browsers) = Launcher::scripted(None);
    let eng = engine(Metrics::none(), launcher);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = BorrowRpcClient::connect(&dialer).await;

            let started = client
                .call(
                    "run.start",
                    json!({
                        "run_id": RUN,
                        "settings": {
                            "secrets.placeholders": {
                                "account_password": {
                                    "reference": "env:BORROW_TEST_SECRET",
                                    "domains": ["*"],
                                }
                            }
                        }
                    }),
                )
                .await;
            assert!(started.get("result").is_some());

            let browser = FakeCrawlerBrowser::new(
                "ctx-default",
                &[("target-1", "ctx-default"), ("target-2", "ctx-default")],
            );
            let (borrow_1, ready_1) = client
                .start_borrow(
                    json!({
                        "run_id": RUN,
                        "target_id": "target-1",
                    }),
                    browser.clone(),
                )
                .await;
            let page_1 = ready_1["page_id"].as_str().unwrap().to_owned();

            // Crawler closes target-1 -> CDP emits Target.detachedFromTarget -> Collie Page reports closed.
            let sid_1 = browser.session_for_target("target-1").await.unwrap();
            let detach_event = json!({
                "method": "Target.detachedFromTarget",
                "params": { "sessionId": sid_1, "targetId": "target-1" },
            })
            .to_string();
            client.send_borrow_frame(borrow_1, &detach_event).await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            let after_page_close = client
                .call(
                    "page.formats",
                    json!({ "page_id": page_1, "formats": { "html": true } }),
                )
                .await;
            assert_eq!(
                after_page_close["error"]["data"]["code"],
                json!("closed"),
                "expected closed after crawler closed page: {after_page_close}"
            );

            // Crawler retires its browser (end: true) -> borrow ends cleanly.
            client.send_borrow_end(borrow_1).await;
            let ended = client.wait_reply(borrow_1).await;
            assert_eq!(ended["result"]["reason"], json!("browser_closed"));

            // Second borrow active when Run closes -> Run end stops borrow without Browser.close.
            let (borrow_2, _ready_2) = client
                .start_borrow(
                    json!({
                        "run_id": RUN,
                        "target_id": "target-2",
                    }),
                    browser.clone(),
                )
                .await;
            let finished = client.call("run.finish", json!({ "run_id": RUN })).await;
            assert!(finished.get("result").is_some());
            let ended_2 = client.wait_reply(borrow_2).await;
            assert_eq!(ended_2["result"]["reason"], json!("run_ended"));

            let seen = browser.seen_methods().await;
            assert!(
                !seen.iter().any(|m| m == "Browser.close"),
                "Run end must never close the borrowed browser: {seen:?}"
            );

            client.close().await;
        };
        let deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, ()) = tokio::join!(eng.run_attached(Network::InMemory(network), deps), driver,);
        assert!(ended.is_ok(), "engine ended with error: {ended:?}");
    });
}

/// Covers: story:5-70, error:borrow_requires_cdp
#[test]
fn run_borrow_refuses_firefox_and_webkit_and_holds_stream_without_credits() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, _browsers) = Launcher::scripted(None);
    let eng = engine(Metrics::none(), launcher);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = BorrowRpcClient::connect(&dialer).await;

            let started = client
                .call(
                    "run.start",
                    json!({
                        "run_id": RUN,
                        "settings": {
                            "secrets.placeholders": {
                                "account_password": {
                                    "reference": "env:BORROW_TEST_SECRET",
                                    "domains": ["*"],
                                }
                            }
                        }
                    }),
                )
                .await;
            assert!(started.get("result").is_some());

            for non_cdp in ["firefox", "webkit"] {
                let err = client
                    .call(
                        "run.borrow",
                        json!({
                            "run_id": RUN,
                            "browser": non_cdp,
                        }),
                    )
                    .await;
                assert_eq!(
                    err["error"]["data"]["code"],
                    json!("borrow_requires_cdp"),
                    "expected borrow_requires_cdp for {non_cdp}: {err}"
                );
                assert_eq!(err["error"]["data"]["browser"], json!(non_cdp));
            }

            // Credits backpressure: with 0 credits granted by client, engine holds
            // outgoing $/borrow_command frames until a credit arrives.
            client.auto_credit = false;
            let borrow_id = client
                .send_request(
                    "run.borrow",
                    json!({
                        "run_id": RUN,
                        "browser": "chromium",
                    }),
                )
                .await;

            // The engine sends its initial $/credit grant to the client, but MUST
            // NOT send $/borrow_command before the client grants at least 1 credit.
            let mut saw_command_before_credit = false;
            let mut buf = String::new();
            while let Ok(Ok(n)) =
                tokio::time::timeout(Duration::from_millis(80), client.reader.read_line(&mut buf))
                    .await
            {
                assert!(n > 0);
                let msg: Value = serde_json::from_str(&buf).unwrap();
                buf.clear();
                if msg.get("method").and_then(Value::as_str) == Some("$/borrow_command") {
                    saw_command_before_credit = true;
                }
            }
            assert!(
                !saw_command_before_credit,
                "outgoing $/borrow_command must wait for client $/credit"
            );

            // Grant 1 credit -> exactly 1 $/borrow_command (Browser.getVersion) arrives.
            client.grant_credits(borrow_id, 1).await;
            let mut got_command = false;
            while let Ok(Ok(n)) = tokio::time::timeout(
                Duration::from_millis(200),
                client.reader.read_line(&mut buf),
            )
            .await
            {
                assert!(n > 0);
                let msg: Value = serde_json::from_str(&buf).unwrap();
                buf.clear();
                if msg.get("method").and_then(Value::as_str) == Some("$/borrow_command") {
                    let frame: Value =
                        serde_json::from_str(msg["params"]["frame"].as_str().unwrap()).unwrap();
                    assert_eq!(frame["method"], json!("Browser.getVersion"));
                    got_command = true;
                    break;
                }
            }
            assert!(
                got_command,
                "granting 1 credit must release the held $/borrow_command"
            );

            client.cancel(borrow_id).await;
            let _ = client.wait_reply(borrow_id).await;
            client.close().await;
        };
        let deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, ()) = tokio::join!(eng.run_attached(Network::InMemory(network), deps), driver,);
        assert!(ended.is_ok(), "engine ended with error: {ended:?}");
    });
}
