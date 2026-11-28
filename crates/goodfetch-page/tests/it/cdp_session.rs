//! Raw CDP sessions (`page.newCDPSession()`, ADRs 0040, 0085, 0087, 0092 and
//! 0140; ticket #751) against the in-process engine's scripted browser:
//! - raw commands (`Emulation.setCPUThrottlingRate`, `Runtime.evaluate`) reach
//!   the page's target session and return the browser's JSON result;
//! - commands that attach or close targets or close the browser are refused
//!   with `cdp_command_refused` before reaching the browser;
//! - while open, Provenance marks results unguarded, Audit log records
//!   `cdp_session_attached`, and Secret placeholders raise `secret_input_refused`;
//! - Firefox and WebKit raise `capability_unavailable`;
//! - frames are bound to the page's lease check (`attach_ended` when expired).
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

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{
    self, ConnectionReader, ConnectionWriter, InMemoryDialer, Network,
};
use goodfetch_engine::{CallError, Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_page::page_value::{
    DocumentResponse, HandlerPages, Navigation, NavigationGate, Observer, PageValue, Permit,
    Refusal,
};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{CLOSED, ProductVersion};
use goodfetch_protocol::home::AUDIT_LOG;
use goodfetch_protocol::page::SessionId;
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::runtime;
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::{Mutex, watch};

type SeenCommands = Arc<Mutex<Vec<(String, Option<String>)>>>;

use crate::handler_page::NoBlocks;
use crate::session_counts::session::{Page, RUN, Wire, answered, dependencies, engine};

/// Scripted browser that records every CDP command received and answers
/// `Emulation.setCPUThrottlingRate` and `Runtime.evaluate`.
async fn scripted_browser(
    mut browsers: ScriptedBrowsers,
    product: &'static str,
    seen: SeenCommands,
) {
    let Some(pipe) = browsers.next().await else {
        return;
    };
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    while let Some(text) = wire.next().await {
        let command: Value = serde_json::from_str(&text).expect("a command is JSON");
        let method = command["method"].as_str().expect("it names its method");
        let session = command
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        seen.lock().await.push((method.to_owned(), session));
        let mut answer = if method == "Browser.getVersion" {
            json!({
                "result": {
                    "protocolVersion": "1.3",
                    "product": product,
                    "revision": "0",
                    "userAgent": product,
                    "jsVersion": "0",
                }
            })
        } else if method == "Emulation.setCPUThrottlingRate" {
            json!({ "result": {} })
        } else if method == "Runtime.evaluate"
            && command["params"]["expression"].as_str() == Some("1 + 1")
        {
            json!({
                "result": { "result": { "type": "number", "value": 2 } }
            })
        } else {
            match answered(method, &command["params"], &mut page) {
                Ok((result, _)) => json!({ "result": result }),
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
    }
}

struct RpcClient {
    reader: BufReader<ConnectionReader>,
    writer: ConnectionWriter,
    next_id: u64,
}

impl RpcClient {
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

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let line = serde_json::to_string(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .unwrap();
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.reader.read_line(&mut buf).await.unwrap();
            assert!(n > 0, "connection closed waiting for reply to {method}");
            let msg: Value = serde_json::from_str(&buf).unwrap();
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                return msg;
            }
        }
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
    }
}

fn read_audit_kinds(home: &Path) -> Vec<String> {
    let path = home.join(AUDIT_LOG);
    if !path.exists() {
        return Vec::new();
    }
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare("SELECT kind FROM audit_entries ORDER BY seq")
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn read_provenance(home: &Path, run_id: &str) -> Vec<Value> {
    let path = home
        .join("runs")
        .join(run_id)
        .join("trace")
        .join("provenance.jsonl");
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Covers: story:5-106
/// Covers: error:cdp_command_refused
/// Covers: operation:page.new_cdp_session
/// Covers: operation:cdp_session.send
/// Covers: operation:cdp_session.close
/// Covers: operation:cdp_session.on
#[test]
fn cdp_session_relays_raw_commands_refuses_escape_audits_marks_unguarded_and_blocks_secrets() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let eng = engine(Metrics::none(), launcher);
    let seen: SeenCommands = Arc::new(Mutex::new(Vec::new()));
    let browser_seen = Arc::clone(&seen);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut rpc = RpcClient::connect(&dialer).await;
            let started = rpc
                .call(
                    "run.start",
                    json!({
                        "run_id": RUN,
                        "settings": {
                            "secrets": {
                                "placeholders": {
                                    "api_token": {
                                        "secret": "env:API_TOKEN",
                                        "domains": ["example.test"]
                                    }
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

            let opened = rpc.call("run.new_page", json!({ "run_id": RUN })).await;
            let page_id = opened["result"]["page_id"].as_str().unwrap().to_owned();

            // Open raw CDP session.
            let cdp = rpc
                .call("page.new_cdp_session", json!({ "page_id": page_id }))
                .await;
            assert!(cdp.get("result").is_some(), "new_cdp_session failed: {cdp}");
            let cdp_session_id = cdp["result"]["cdp_session_id"].as_str().unwrap().to_owned();

            // Raw commands reach the page target.
            let throttle = rpc
                .call(
                    "cdp_session.send",
                    json!({
                        "cdp_session_id": cdp_session_id,
                        "method": "Emulation.setCPUThrottlingRate",
                        "params": { "rate": 4 }
                    }),
                )
                .await;
            assert!(
                throttle.get("result").is_some(),
                "throttle failed: {throttle}"
            );

            let eval = rpc
                .call(
                    "cdp_session.send",
                    json!({
                        "cdp_session_id": cdp_session_id,
                        "method": "Runtime.evaluate",
                        "params": { "expression": "1 + 1" }
                    }),
                )
                .await;
            assert_eq!(eval["result"]["result"]["result"]["value"], json!(2));

            // Commands that attach or close targets or close the browser are refused.
            for forbidden in [
                "Target.attachToTarget",
                "Target.closeTarget",
                "Page.close",
                "Browser.close",
            ] {
                let refused = rpc
                    .call(
                        "cdp_session.send",
                        json!({
                            "cdp_session_id": cdp_session_id,
                            "method": forbidden
                        }),
                    )
                    .await;
                assert_eq!(
                    refused["error"]["data"]["code"],
                    json!("cdp_command_refused"),
                    "expected cdp_command_refused for {forbidden}, got {refused}"
                );
            }

            // While open, a Secret placeholder raises secret_input_refused.
            let secret_req = rpc
                .call(
                    "page.request",
                    json!({
                        "page_id": page_id,
                        "url": "https://example.test/api",
                        "headers": { "Authorization": "Bearer {{secret:api_token}}" }
                    }),
                )
                .await;
            assert_eq!(
                secret_req["error"]["data"]["code"],
                json!("secret_input_refused"),
                "expected secret_input_refused while CDP session open, got {secret_req}"
            );

            // Verify Audit log recorded cdp_session_attached and Provenance marked unguarded.
            let audit_kinds = read_audit_kinds(home.path());
            assert!(
                audit_kinds.iter().any(|k| k == "cdp_session_attached"),
                "expected cdp_session_attached in audit log: {audit_kinds:?}"
            );
            let prov = read_provenance(home.path(), RUN);
            assert!(
                prov.iter().any(|e| {
                    e.get("action").and_then(Value::as_str) == Some("cdp_session_attached")
                        && e.get("unguarded").and_then(Value::as_bool) == Some(true)
                }),
                "expected unguarded cdp_session_attached in provenance: {prov:?}"
            );

            // Close CDP session (idempotent).
            let closed = rpc
                .call(
                    "cdp_session.close",
                    json!({ "cdp_session_id": cdp_session_id }),
                )
                .await;
            assert!(closed.get("result").is_some(), "close failed: {closed}");
            let closed_again = rpc
                .call(
                    "cdp_session.close",
                    json!({ "cdp_session_id": cdp_session_id }),
                )
                .await;
            assert!(closed_again.get("result").is_some());

            // Sending after close fails with closed.
            let after_close = rpc
                .call(
                    "cdp_session.send",
                    json!({
                        "cdp_session_id": cdp_session_id,
                        "method": "Emulation.setCPUThrottlingRate",
                        "params": { "rate": 1 }
                    }),
                )
                .await;
            assert_eq!(after_close["error"]["data"]["code"], json!("closed"));

            // Verify forbidden commands never reached the browser.
            let sent_methods: Vec<String> =
                seen.lock().await.iter().map(|(m, _)| m.clone()).collect();
            assert!(
                sent_methods
                    .iter()
                    .any(|m| m == "Emulation.setCPUThrottlingRate")
            );
            assert!(!sent_methods.iter().any(|m| m == "Target.closeTarget"));
            assert!(!sent_methods.iter().any(|m| m == "Page.close"));

            let _ = rpc.call("run.finish", json!({ "run_id": RUN })).await;
            rpc.close().await;
        };
        let deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, (), ()) = tokio::join!(
            eng.run_attached(Network::InMemory(network), deps),
            driver,
            scripted_browser(browsers, "HeadlessChrome/131.0.0.0", browser_seen),
        );
        ended.expect("the engine got its Connection");
    });
}

/// Covers: story:5-106
#[test]
fn cdp_session_raises_capability_unavailable_on_firefox_and_webkit() {
    for product in ["Firefox/135.0", "WebKit/619.1"] {
        let home = tempfile::tempdir().unwrap();
        let (launcher, browsers) = Launcher::scripted(None);
        let eng = engine(Metrics::none(), launcher);
        let seen: SeenCommands = Arc::new(Mutex::new(Vec::new()));

        runtime().block_on(async {
            let (network, dialer) = transport::in_memory();
            let driver = async {
                let mut rpc = RpcClient::connect(&dialer).await;
                let _ = rpc
                    .call("run.start", json!({ "run_id": RUN, "settings": {} }))
                    .await;
                let opened = rpc.call("run.new_page", json!({ "run_id": RUN })).await;
                let page_id = opened["result"]["page_id"].as_str().unwrap().to_owned();

                let refused = rpc
                    .call("page.new_cdp_session", json!({ "page_id": page_id }))
                    .await;
                assert_eq!(
                    refused["error"]["data"]["code"],
                    json!("capability_unavailable"),
                    "expected capability_unavailable on {product}, got {refused}"
                );

                let _ = rpc.call("run.finish", json!({ "run_id": RUN })).await;
                rpc.close().await;
            };
            let deps = Dependencies {
                clock: Clock::paused(SystemTime::UNIX_EPOCH),
                ..dependencies(home.path())
            };
            let (ended, (), ()) = tokio::join!(
                eng.run_attached(Network::InMemory(network), deps),
                driver,
                scripted_browser(browsers, product, seen),
            );
            ended.expect("the engine got its Connection");
        });
    }
}

#[derive(Debug)]
struct AlwaysAdmits;

impl NavigationGate for AlwaysAdmits {
    fn admit<'a>(
        &'a self,
        _navigation: &'a Navigation,
    ) -> Pin<Box<dyn Future<Output = Result<Permit, Refusal>> + Send + 'a>> {
        Box::pin(async { Ok(Permit::unlimited()) })
    }

    fn sending<'a>(&'a self, _url: &'a str) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }

    fn judge_document<'a>(
        &'a self,
        _response: &'a DocumentResponse<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<(), Refusal>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn reads_document_body(&self) -> bool {
        false
    }
}

#[derive(Debug)]
struct ToggleLease(Arc<watch::Sender<bool>>);

impl LeaseCheck for ToggleLease {
    fn check(&self) -> Result<(), LeaseEnded> {
        if *self.0.borrow() {
            Ok(())
        } else {
            Err(LeaseEnded::Lost)
        }
    }

    fn withheld(&self, _ended: LeaseEnded, method: &str) -> CallError {
        CallError::new(
            &CLOSED,
            format!("withheld `{method}`"),
            &json!({ "reason": "attach_ended" }),
        )
    }
}

struct LeasedPages(Arc<watch::Sender<bool>>);

impl HandlerPages for LeasedPages {
    fn value(&self, _run: &RunId, _session: &SessionId) -> PageValue {
        PageValue::new(AlwaysAdmits, ToggleLease(Arc::clone(&self.0)))
    }

    fn observer(&self, _run: &RunId, _session: &SessionId) -> Option<Observer> {
        None
    }
}

/// Covers: story:5-106
#[test]
fn cdp_session_frames_are_bound_to_lease_check() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let lease_alive = Arc::new(watch::Sender::new(true));
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_page::contribution_with(
            (Vec::new(), launcher),
            LeasedPages(Arc::clone(&lease_alive)),
            Arc::new(NoBlocks),
        ));
    let eng = Engine::new(identity, &composition)
        .unwrap()
        .with_metrics(Metrics::none());
    let seen: SeenCommands = Arc::new(Mutex::new(Vec::new()));
    let browser_seen = Arc::clone(&seen);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut rpc = RpcClient::connect(&dialer).await;
            let _ = rpc
                .call("run.start", json!({ "run_id": RUN, "settings": {} }))
                .await;
            let opened = rpc.call("run.new_page", json!({ "run_id": RUN })).await;
            let page_id = opened["result"]["page_id"].as_str().unwrap().to_owned();

            let cdp = rpc
                .call("page.new_cdp_session", json!({ "page_id": page_id }))
                .await;
            let cdp_session_id = cdp["result"]["cdp_session_id"].as_str().unwrap().to_owned();

            // While lease is alive, command succeeds.
            let ok = rpc
                .call(
                    "cdp_session.send",
                    json!({
                        "cdp_session_id": cdp_session_id,
                        "method": "Emulation.setCPUThrottlingRate",
                        "params": { "rate": 2 }
                    }),
                )
                .await;
            assert!(ok.get("result").is_some(), "expected ok while leased: {ok}");

            // Expire the lease: next command fails with attach_ended and never reaches the browser.
            let count_before = seen.lock().await.len();
            lease_alive.send_replace(false);
            let ended = rpc
                .call(
                    "cdp_session.send",
                    json!({
                        "cdp_session_id": cdp_session_id,
                        "method": "Emulation.setCPUThrottlingRate",
                        "params": { "rate": 3 }
                    }),
                )
                .await;
            assert_eq!(ended["error"]["data"]["code"], json!("closed"));
            assert_eq!(
                ended["error"]["data"]["details"]["reason"],
                json!("attach_ended")
            );
            assert_eq!(
                seen.lock().await.len(),
                count_before,
                "expired lease must not send command to browser"
            );

            let _ = rpc.call("run.finish", json!({ "run_id": RUN })).await;
            rpc.close().await;
        };
        let deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, (), ()) = tokio::join!(
            eng.run_attached(Network::InMemory(network), deps),
            driver,
            scripted_browser(browsers, "HeadlessChrome/131.0.0.0", browser_seen),
        );
        ended.expect("the engine got its Connection");
    });
}
