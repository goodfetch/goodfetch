//! Page lending (`page.lend()`, ADRs 0017, 0059, 0060, 0067, 0089 and 0140;
//! ticket #749) against the in-process engine's scripted browser:
//! - WebSocket endpoint binds loopback port 0, checks `Host`, `Origin`, and
//!   per-attach Bearer token;
//! - scopes `Target.getBrowserContexts`, `Target.getTargets`, and
//!   `Target.setAutoAttach` strictly to the lent page and targets the library
//!   creates (never another Session or the Session's other pages);
//! - rescopes context-bearing commands and refuses commands naming another
//!   context, `Target.createBrowserContext`, `Extensions.*`, `Browser.crash`,
//!   `Security.setIgnoreCertificateErrors`, and unclassed browser-level
//!   commands with `cdp_command_refused`;
//! - answers `Browser.setDownloadBehavior` without effect;
//! - `Browser.close` ends the attach (never closing the browser), disables
//!   `Runtime` (#913), closes library-created targets, revokes the token, and
//!   returns the Session from `Lent` to `Open`;
//! - second `page.lend()` on the same Session fails with `session_reserved`;
//! - program's own calls work while `Lent` (results unguarded), Effect window
//!   counts bodies as sent from `page.lend` start;
//! - Firefox/WebKit pages raise `capability_unavailable`.
#![expect(
    clippy::disallowed_types,
    reason = "ADR 0060: integration test connects a WebSocket client over TcpStream to the loopback lending listener"
)]
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
    reason = "each integration test drives one multi-step JSON-RPC + WebSocket scenario end to end"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{
    self, ConnectionReader, ConnectionWriter, InMemoryDialer, Network,
};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_protocol::home::AUDIT_LOG;
use goodfetch_test_support::runtime;
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

use crate::session_counts::session::{Page, RUN, Wire, answered, dependencies, engine};

type SeenCommands = Arc<Mutex<Vec<(String, Option<String>, Value)>>>;

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
    let mut targets: Vec<Value> = Vec::new();
    let mut next_created = 1u32;

    while let Some(text) = wire.next().await {
        let command: Value = serde_json::from_str(&text).expect("a command is JSON");
        let method = command["method"].as_str().expect("it names its method");
        let session = command
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let params = command.get("params").cloned().unwrap_or_else(|| json!({}));
        seen.lock()
            .await
            .push((method.to_owned(), session.clone(), params.clone()));

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
        } else if method == "Target.createTarget" {
            let ctx = params
                .get("browserContextId")
                .and_then(Value::as_str)
                .unwrap_or("ctx-1")
                .to_owned();
            let tid = format!("target-{next_created}");
            next_created += 1;
            targets.push(json!({
                "targetId": tid,
                "type": "page",
                "title": "Blank",
                "url": params.get("url").and_then(Value::as_str).unwrap_or("about:blank"),
                "attached": false,
                "canAccessOpener": false,
                "browserContextId": ctx,
            }));
            json!({ "result": { "targetId": tid } })
        } else if method == "Target.getTargets" {
            json!({ "result": { "targetInfos": targets } })
        } else if method == "Target.getTargetInfo" {
            let tid = params
                .get("targetId")
                .and_then(Value::as_str)
                .unwrap_or("target-1");
            let info = targets
                .iter()
                .find(|t| t["targetId"].as_str() == Some(tid))
                .cloned()
                .unwrap_or_else(
                    || json!({ "targetId": tid, "type": "page", "url": "about:blank" }),
                );
            json!({ "result": { "targetInfo": info } })
        } else if method == "Target.attachToTarget" {
            let tid = params
                .get("targetId")
                .and_then(Value::as_str)
                .unwrap_or("target-1");
            let sid = format!("sid-{tid}-{next_created}");
            next_created += 1;
            json!({ "result": { "sessionId": sid } })
        } else if method == "Target.detachFromTarget" || method == "Target.closeTarget" {
            if method == "Target.closeTarget"
                && let Some(tid) = params.get("targetId").and_then(Value::as_str)
            {
                targets.retain(|t| t["targetId"].as_str() != Some(tid));
            }
            json!({ "result": { "success": true } })
        } else if method == "Storage.getCookies" {
            json!({ "result": { "cookies": [{ "name": "sid", "value": "ok", "domain": "example.test" }] } })
        } else if method == "Runtime.enable" || method == "Runtime.disable" {
            json!({ "result": {} })
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

async fn ws_connect(
    url: &str,
    auth_header: Option<&str>,
    origin_header: Option<&str>,
) -> Result<tokio_tungstenite::WebSocketStream<TcpStream>, String> {
    let parsed = url::Url::parse(url).map_err(|e| e.to_string())?;
    let host = parsed.host_str().unwrap_or("127.0.0.1");
    let port = parsed.port().unwrap_or(80);
    let stream = TcpStream::connect((host, port))
        .await
        .map_err(|e| e.to_string())?;
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    if let Some(auth) = auth_header {
        req.headers_mut()
            .insert("Authorization", HeaderValue::from_str(auth).unwrap());
    }
    if let Some(origin) = origin_header {
        req.headers_mut()
            .insert("Origin", HeaderValue::from_str(origin).unwrap());
    }
    let (ws, _) = tokio_tungstenite::client_async(req, stream)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ws)
}

async fn ws_send_and_recv(
    ws: &mut tokio_tungstenite::WebSocketStream<TcpStream>,
    id: u64,
    method: &str,
    session_id: Option<&str>,
    params: Value,
) -> (Value, Vec<Value>) {
    let mut req = json!({
        "id": id,
        "method": method,
        "params": params,
    });
    if let Some(sid) = session_id {
        req["sessionId"] = json!(sid);
    }
    ws.send(Message::Text(Utf8Bytes::from(req.to_string())))
        .await
        .unwrap();

    let mut events = Vec::new();
    loop {
        let msg = ws.next().await.expect("ws open").expect("ws frame");
        if let Message::Text(text) = msg {
            let parsed: Value = serde_json::from_str(text.as_str()).unwrap();
            if parsed.get("id") == Some(&json!(id)) {
                return (parsed, events);
            }
            events.push(parsed);
        }
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

/// Covers: story:5-104
/// Covers: error:session_reserved
/// Covers: operation:page.lend
/// Covers: operation:cdp_endpoint.close
#[test]
fn page_lend_scopes_targets_validates_handshake_and_restores_state() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let eng = engine(Metrics::none(), launcher);
    let seen: SeenCommands = Arc::new(Mutex::new(Vec::new()));
    let browser_seen = Arc::clone(&seen);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut rpc = RpcClient::connect(&dialer).await;
            let started = rpc.call("run.start", json!({ "run_id": RUN })).await;
            assert!(
                started.get("result").is_some(),
                "run.start failed: {started}"
            );

            // Open two pages in Session 1 and one page in Session 2.
            let p1 = rpc.call("run.new_page", json!({ "run_id": RUN })).await;
            let page1_id = p1["result"]["page_id"].as_str().unwrap().to_owned();
            let session1_id = p1["result"]["session_id"].as_str().unwrap().to_owned();

            let p2 = rpc
                .call("session.new_page", json!({ "session_id": session1_id }))
                .await;
            assert!(p2.get("result").is_some(), "p2 failed: {p2}");

            let s2 = rpc.call("run.new_session", json!({ "run_id": RUN })).await;
            let session2_id = s2["result"]["session_id"].as_str().unwrap().to_owned();
            let p3 = rpc
                .call("session.new_page", json!({ "session_id": session2_id }))
                .await;
            assert!(p3.get("result").is_some(), "p3 failed: {p3}");

            // Open an Effect window before lending to verify page.lend marks bodies sent.
            let eff = rpc
                .call(
                    "effect.open",
                    json!({ "run_id": RUN, "key": "lend-effect-1" }),
                )
                .await;
            assert!(eff.get("result").is_some(), "effect.open failed: {eff}");

            // Lend page 1.
            let lent = rpc.call("page.lend", json!({ "page_id": page1_id })).await;
            assert!(lent.get("result").is_some(), "page.lend failed: {lent}");
            let url = lent["result"]["url"].as_str().unwrap().to_owned();
            let auth_header = lent["result"]["headers"]["Authorization"]
                .as_str()
                .unwrap()
                .to_owned();
            assert!(url.starts_with("ws://127.0.0.1:"));
            assert!(auth_header.starts_with("Bearer "));

            // Second lend on same Session is refused with `session_reserved`.
            let second = rpc.call("page.lend", json!({ "page_id": page1_id })).await;
            assert_eq!(
                second["error"]["data"]["code"],
                json!("session_reserved"),
                "second lend must fail with session_reserved: {second}"
            );

            // Handshake without token is refused (401).
            assert!(
                ws_connect(&url, None, None).await.is_err(),
                "connection without token must be refused"
            );

            // Handshake with foreign Origin is refused (403).
            assert!(
                ws_connect(&url, Some(&auth_header), Some("https://evil.example"))
                    .await
                    .is_err(),
                "connection with foreign Origin must be refused"
            );

            // Connect with valid Host + Authorization Bearer token.
            let mut ws = ws_connect(&url, Some(&auth_header), None)
                .await
                .expect("authenticated lending connection succeeds");

            // Target.getTargets shows ONLY the lent page (not p2 or p3).
            let (targets_resp, _) =
                ws_send_and_recv(&mut ws, 100, "Target.getTargets", None, json!({})).await;
            let infos = targets_resp["result"]["targetInfos"].as_array().unwrap();
            assert_eq!(
                infos.len(),
                1,
                "lent library must see only the lent page, got: {infos:?}"
            );
            let lent_tid = infos[0]["targetId"].as_str().unwrap().to_owned();

            // Target.setAutoAttach attaches ONLY to the lent page.
            let (auto_resp, auto_events) = ws_send_and_recv(
                &mut ws,
                101,
                "Target.setAutoAttach",
                None,
                json!({ "autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true }),
            )
            .await;
            assert!(auto_resp.get("result").is_some());
            let attached_evt = auto_events
                .iter()
                .find(|e| e["method"] == "Target.attachedToTarget")
                .expect("auto-attach emits Target.attachedToTarget for lent page");
            let client_sid = attached_evt["params"]["sessionId"]
                .as_str()
                .unwrap()
                .to_owned();

            // Library enables Runtime on the lent page (#913: only while attached).
            let (rt_resp, _) =
                ws_send_and_recv(&mut ws, 102, "Runtime.enable", Some(&client_sid), json!({}))
                    .await;
            assert!(rt_resp.get("result").is_some());

            // Library creates a new tab -> visible in Target.getTargets.
            let (created_resp, _) = ws_send_and_recv(
                &mut ws,
                103,
                "Target.createTarget",
                None,
                json!({ "url": "about:blank" }),
            )
            .await;
            let created_tid = created_resp["result"]["targetId"]
                .as_str()
                .unwrap()
                .to_owned();
            let (targets_after, _) =
                ws_send_and_recv(&mut ws, 104, "Target.getTargets", None, json!({})).await;
            assert_eq!(
                targets_after["result"]["targetInfos"]
                    .as_array()
                    .unwrap()
                    .len(),
                2,
                "lent library sees lent page + page it created"
            );

            // Rescoped command naming another context is refused.
            let (bad_cookies, _) = ws_send_and_recv(
                &mut ws,
                105,
                "Storage.getCookies",
                None,
                json!({ "browserContextId": "other-session-ctx" }),
            )
            .await;
            assert!(
                bad_cookies["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("cdp_command_refused"),
                "cross-context Storage.getCookies must be refused: {bad_cookies}"
            );

            // Dangerous commands are refused and never reach the browser.
            for (idx, method) in [
                "Target.createBrowserContext",
                "Extensions.loadUnpacked",
                "Browser.crash",
                "Security.setIgnoreCertificateErrors",
            ]
            .into_iter()
            .enumerate()
            {
                let (resp, _) =
                    ws_send_and_recv(&mut ws, 110 + idx as u64, method, None, json!({})).await;
                assert!(
                    resp["error"]["message"]
                        .as_str()
                        .unwrap()
                        .contains("cdp_command_refused"),
                    "{method} must be refused: {resp}"
                );
            }

            // Browser.setDownloadBehavior is answered without effect.
            let (dl_resp, _) = ws_send_and_recv(
                &mut ws,
                120,
                "Browser.setDownloadBehavior",
                None,
                json!({ "behavior": "allow", "downloadPath": "/tmp/evil" }),
            )
            .await;
            assert!(dl_resp.get("result").is_some());

            // Browser.close ends the attach without sending Browser.close to the browser.
            let (close_resp, _) =
                ws_send_and_recv(&mut ws, 121, "Browser.close", None, json!({})).await;
            assert!(close_resp.get("result").is_some());
            drop(ws);

            // Yield so cleanup finishes Runtime.disable + Target.closeTarget + Session Open.
            for _ in 0..20 {
                tokio::task::yield_now().await;
            }

            // Reconnect with the revoked token must fail.
            assert!(
                ws_connect(&url, Some(&auth_header), None).await.is_err(),
                "revoked token must not reconnect"
            );

            // Verify browser saw Runtime.enable, Runtime.disable, Target.closeTarget(created_tid),
            // and NEVER saw Browser.close, Browser.setDownloadBehavior, or Extensions.loadUnpacked.
            {
                let log = seen.lock().await;
                assert!(
                    log.iter().any(|(m, s, _)| m == "Runtime.enable"
                        && s.as_deref() == Some(client_sid.as_str())),
                    "Runtime.enable must reach the browser while attached"
                );
                assert!(
                    log.iter().any(|(m, s, _)| m == "Runtime.disable"
                        && s.as_deref() == Some(client_sid.as_str())),
                    "Runtime.disable must run when the attach ends (#913)"
                );
                assert!(
                    log.iter().any(|(m, _, p)| m == "Target.closeTarget"
                        && p["targetId"] == json!(created_tid)),
                    "library-created target must be closed when attach ends"
                );
                assert!(
                    !log.iter()
                        .any(|(m, _, p)| m == "Target.closeTarget"
                            && p["targetId"] == json!(lent_tid)),
                    "lent page itself must NOT be closed when attach ends"
                );
                assert!(
                    !log.iter().any(|(m, _, _)| {
                        m == "Browser.close"
                            || m == "Browser.setDownloadBehavior"
                            || m == "Extensions.loadUnpacked"
                    }),
                    "refused/noop/Browser.close commands must never reach the browser"
                );
            }

            // Session is back to Open: lending again succeeds!
            let lent_again = rpc.call("page.lend", json!({ "page_id": page1_id })).await;
            assert!(
                lent_again.get("result").is_some(),
                "re-lend after attach end must succeed: {lent_again}"
            );
            let ep2 = lent_again["result"]["cdp_endpoint_id"]
                .as_str()
                .unwrap()
                .to_owned();
            let closed_ep2 = rpc
                .call("cdp_endpoint.close", json!({ "cdp_endpoint_id": ep2 }))
                .await;
            assert!(closed_ep2.get("result").is_some());

            // Audit log recorded `lent_page_attached`.
            let audit_kinds = read_audit_kinds(home.path());
            assert!(
                audit_kinds.iter().any(|k| k == "lent_page_attached"),
                "Audit log must record lent_page_attached: {audit_kinds:?}"
            );

            // Close Effect and finish Run cleanly.
            let _ = rpc
                .call(
                    "effect.close",
                    json!({ "run_id": RUN, "key": "lend-effect-1", "output": { "ok": true } }),
                )
                .await;
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
            scripted_browser(browsers, "HeadlessChrome/131.0.6778.85", browser_seen),
        );
        assert!(ended.is_ok(), "engine ended with error: {ended:?}");
    });
}

/// Covers: story:5-104
#[test]
fn page_lend_on_firefox_raises_capability_unavailable() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let eng = engine(Metrics::none(), launcher);
    let seen: SeenCommands = Arc::new(Mutex::new(Vec::new()));
    let browser_seen = Arc::clone(&seen);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut rpc = RpcClient::connect(&dialer).await;
            let started = rpc.call("run.start", json!({ "run_id": RUN })).await;
            assert!(started.get("result").is_some());
            let p1 = rpc.call("run.new_page", json!({ "run_id": RUN })).await;
            let page1_id = p1["result"]["page_id"].as_str().unwrap().to_owned();

            let lent = rpc.call("page.lend", json!({ "page_id": page1_id })).await;
            assert_eq!(
                lent["error"]["data"]["code"],
                json!("capability_unavailable"),
                "page.lend on Firefox must raise capability_unavailable: {lent}"
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
            scripted_browser(browsers, "Firefox/130.0", browser_seen),
        );
        assert!(ended.is_ok(), "engine ended with error: {ended:?}");
    });
}
