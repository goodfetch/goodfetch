//! Two-factor `code_sources` integration tests (ADRs 0006, 0059, 0060, 0076
//! and 0092; ticket #734):
//! - all 6 kinds (`imap`, `gmail`, `microsoft_graph`, `webhook`, `endpoint`,
//!   `twilio`) tested against local stand-in servers of their real shape;
//! - matching sender, pattern and time window returns a code used once, and a
//!   second login with the same message finds no code;
//! - messages outside `window` or from another sender are ignored, raising a
//!   Human pause when `session.recovery.human_pause` is `true` (default) and
//!   `secret_unavailable` (`no_code`) when `false`;
//! - short durations go through settings on the live clock (never a test-only
//!   clock, ADR 0059);
//! - a Client acting for a model setting `code_sources` gets `setting_locked`.
#![expect(
    clippy::disallowed_types,
    reason = "integration test stand-in servers bind loopback sockets and record state outside production lock order (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "integration test stand-in servers open loopback TCP listeners and read wall-clock timestamps (ADR 0060)"
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
    reason = "each integration test drives one multi-step JSON-RPC scenario against a stand-in server"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use data_encoding::{BASE64, BASE64URL_NOPAD};
use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_test_support::{TestEnv, runtime};
use jiff::fmt::rfc2822;
use jiff::{SignedDuration, Timestamp};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};

use crate::session_counts::session::{Page, RUN, Wire, answered, dependencies, engine};

const TOKEN: &str = "code-source-test-token";

/// Recorded `Input.insertText` values typed into the scripted browser.
#[derive(Default, Clone)]
struct Observed {
    inserted_text: Arc<Mutex<Vec<String>>>,
}

async fn scripted_browser(mut launched: ScriptedBrowsers, observed: Observed) {
    let Some(pipe) = launched.next().await else {
        return;
    };
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    let frame_url = "https://login.test/2fa";
    while let Some(text) = wire.next().await {
        let command: Value = serde_json::from_str(&text).expect("a command is JSON");
        let method = command["method"].as_str().expect("it names its method");
        if method == "Input.insertText"
            && let Some(t) = command["params"]["text"].as_str()
        {
            observed.inserted_text.lock().unwrap().push(t.to_owned());
        }
        let mut answer = if method == "Runtime.callFunctionOn" {
            let decl = command["params"]["functionDeclaration"]
                .as_str()
                .unwrap_or("");
            if decl.contains("async function check") {
                json!({
                    "result": {
                        "result": {
                            "type": "object",
                            "value": {
                                "state": "focused",
                                "password": false,
                                "frame_url": frame_url,
                            }
                        }
                    }
                })
            } else {
                match answered(method, &command["params"], &mut page) {
                    Ok((result, _)) => json!({ "result": result }),
                    Err(error) => json!({ "error": error }),
                }
            }
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

struct Harness {
    writer: ConnectionWriter,
    lines: tokio::io::Lines<BufReader<ConnectionReader>>,
    observed: Observed,
    engine_task: tokio::task::JoinHandle<()>,
    browser_task: tokio::task::JoinHandle<()>,
    next_id: u64,
    ahead: BTreeMap<u64, Value>,
}

impl Harness {
    async fn start(env: &TestEnv, env_vars: BTreeMap<String, String>) -> Self {
        Self::start_with_surface(env, env_vars, "rust").await
    }

    async fn start_with_surface(
        env: &TestEnv,
        env_vars: BTreeMap<String, String>,
        surface: &str,
    ) -> Self {
        let mut deps = Dependencies {
            clock: Clock::system(),
            ..dependencies(env.folder())
        };
        for (k, v) in env_vars {
            deps.environment.insert(k.into(), v.into());
        }
        let (launcher, browsers) = Launcher::scripted(None);
        let (network, dialer) = transport::in_memory();
        let observed = Observed::default();
        let browser_task = tokio::spawn(scripted_browser(browsers, observed.clone()));
        let eng = engine(Metrics::none(), launcher);
        let engine_task = tokio::spawn(async move {
            let _ = eng
                .run_attached(transport::Network::InMemory(network), deps)
                .await;
        });
        let (reader, writer) = dialer
            .connect()
            .await
            .expect("invariant: the engine listens")
            .split();
        let mut harness = Self {
            writer,
            lines: BufReader::new(reader).lines(),
            observed,
            engine_task,
            browser_task,
            next_id: 1,
            ahead: BTreeMap::new(),
        };
        let hello = harness
            .call(
                "hello",
                json!({
                    "protocol_version": "0.1",
                    "surface": surface,
                    "needs": "collie",
                    "product_version": "0.0.0",
                    "token": TOKEN,
                }),
            )
            .await;
        assert!(hello.get("result").is_some(), "hello failed: {hello}");
        harness
    }

    async fn send(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .expect("write JSON-RPC request");
        self.writer.flush().await.expect("flush JSON-RPC request");
        id
    }

    async fn read_reply(&mut self, expected_id: u64) -> Value {
        if let Some(msg) = self.ahead.remove(&expected_id) {
            return msg;
        }
        loop {
            let line = self
                .lines
                .next_line()
                .await
                .expect("read line")
                .expect("connection closed waiting for reply");
            let msg: Value = serde_json::from_str(&line).expect("JSON-RPC frame");
            if let Some(got) = msg.get("id").and_then(Value::as_u64) {
                if got == expected_id {
                    return msg;
                }
                self.ahead.insert(got, msg);
            }
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params).await;
        self.read_reply(id).await
    }

    async fn finish(mut self) {
        let _ = self.call("run.finish", json!({ "run_id": RUN })).await;
        self.close().await;
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
        drop(self.writer);
        drop(self.lines);
        let _ = self.engine_task.await;
        self.browser_task.abort();
    }
}

fn now_ts() -> Timestamp {
    Timestamp::try_from(SystemTime::now()).unwrap()
}

/// Spawns a local HTTP stand-in server whose handler receives `(method, path,
/// headers_lower, body)` and returns `(status, json_body)`.
async fn spawn_http_server<F>(handler: F) -> (String, tokio::task::JoinHandle<()>)
where
    F: Fn(&str, &str, &[(String, String)], &str) -> (u16, String) + Send + Sync + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let base = format!("http://{addr}");
    let handler = Arc::new(handler);
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 16_384];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                let raw = String::from_utf8_lossy(&buf[..n]);
                let mut parts = raw.splitn(2, "\r\n\r\n");
                let head = parts.next().unwrap_or("");
                let body = parts.next().unwrap_or("");
                let mut lines = head.lines();
                let req_line = lines.next().unwrap_or("");
                let mut req_parts = req_line.split_whitespace();
                let method = req_parts.next().unwrap_or("GET");
                let path = req_parts.next().unwrap_or("/");
                let headers: Vec<(String, String)> = lines
                    .filter_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        Some((k.trim().to_ascii_lowercase(), v.trim().to_owned()))
                    })
                    .collect();
                let (status, resp_body) = handler(method, path, &headers, body);
                let resp = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{resp_body}",
                    resp_body.len()
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });
    (base, task)
}

/// Spawns a local IMAP4rev1 stand-in server serving `messages` as `(uid, from,
/// rfc2822_date, body)`.
async fn spawn_imap_server(
    expected_user: &'static str,
    expected_pass: &'static str,
    messages: Vec<(u32, String, String, String)>,
) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let messages = Arc::new(messages);
    let task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let messages = Arc::clone(&messages);
            tokio::spawn(async move {
                let mut reader = BufReader::new(stream);
                let _ = reader
                    .get_mut()
                    .write_all(b"* OK local-imap ready\r\n")
                    .await;
                let mut authed = false;
                loop {
                    let mut line = String::new();
                    let Ok(n) = reader.read_line(&mut line).await else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    let mut tokens = trimmed.splitn(3, ' ');
                    let tag = tokens.next().unwrap_or("A0");
                    let cmd = tokens.next().unwrap_or("").to_ascii_uppercase();
                    let rest = tokens.next().unwrap_or("");
                    match cmd.as_str() {
                        "LOGIN" => {
                            let expected = format!("\"{expected_user}\" \"{expected_pass}\"");
                            if rest == expected {
                                authed = true;
                                let _ = reader
                                    .get_mut()
                                    .write_all(format!("{tag} OK LOGIN completed\r\n").as_bytes())
                                    .await;
                            } else {
                                let _ = reader
                                    .get_mut()
                                    .write_all(format!("{tag} NO bad credentials\r\n").as_bytes())
                                    .await;
                            }
                        }
                        "EXAMINE" if authed => {
                            let _ = reader
                                .get_mut()
                                .write_all(format!("{tag} OK EXAMINE completed\r\n").as_bytes())
                                .await;
                        }
                        "UID" if authed && rest.starts_with("SEARCH") => {
                            let uids: Vec<String> =
                                messages.iter().map(|(u, ..)| u.to_string()).collect();
                            let reply = format!(
                                "* SEARCH {}\r\n{tag} OK UID SEARCH completed\r\n",
                                uids.join(" ")
                            );
                            let _ = reader.get_mut().write_all(reply.as_bytes()).await;
                        }
                        "UID" if authed && rest.starts_with("FETCH") => {
                            let uid: u32 = rest
                                .split_whitespace()
                                .nth(1)
                                .and_then(|s| s.parse().ok())
                                .unwrap_or(0);
                            if let Some((_, from, date, body)) =
                                messages.iter().find(|(u, ..)| *u == uid)
                            {
                                let rfc822 =
                                    format!("From: {from}\r\nDate: {date}\r\n\r\n{body}\r\n");
                                let reply = format!(
                                    "* 1 FETCH (UID {uid} BODY[] {{{}}}\r\n{rfc822})\r\n{tag} OK UID FETCH completed\r\n",
                                    rfc822.len()
                                );
                                let _ = reader.get_mut().write_all(reply.as_bytes()).await;
                            } else {
                                let _ = reader
                                    .get_mut()
                                    .write_all(
                                        format!("{tag} OK UID FETCH completed\r\n").as_bytes(),
                                    )
                                    .await;
                            }
                        }
                        "LOGOUT" => {
                            let _ = reader
                                .get_mut()
                                .write_all(format!("* BYE\r\n{tag} OK LOGOUT\r\n").as_bytes())
                                .await;
                            break;
                        }
                        _ => {
                            let _ = reader
                                .get_mut()
                                .write_all(format!("{tag} BAD unknown\r\n").as_bytes())
                                .await;
                        }
                    }
                }
            });
        }
    });
    (port, task)
}

async fn run_one_time_code_scenario(
    env: &TestEnv,
    env_vars: BTreeMap<String, String>,
    code_source_cfg: Value,
    expected_code: &str,
) {
    let mut h = Harness::start(env, env_vars).await;
    let started = h
        .call(
            "run.start",
            json!({
                "run_id": RUN,
                "settings": {
                    "code_sources": { "inbox": code_source_cfg },
                    "secrets": {
                        "placeholders": {
                            "otp": { "code_source": "inbox", "domains": ["login.test"] }
                        }
                    },
                    "session": {
                        "recovery": { "human_pause": false }
                    }
                }
            }),
        )
        .await;
    assert!(
        started.get("result").is_some(),
        "run.start failed: {started}"
    );

    let opened = h.call("run.new_page", json!({ "run_id": RUN })).await;
    let page_id = opened["result"]["page_id"].as_str().unwrap().to_owned();

    // First login: finds the matching code inside the window and types it.
    let first = h
        .call(
            "locator.fill",
            json!({
                "locator": { "page_id": page_id, "selector": "#otp" },
                "value": "",
                "secret": "otp"
            }),
        )
        .await;
    assert!(first.get("result").is_some(), "first fill failed: {first}");
    assert_eq!(
        h.observed.inserted_text.lock().unwrap().as_slice(),
        &[expected_code.to_owned()]
    );

    // Second login with the same message still on the server: used once, so no
    // code is found and with session.recovery.human_pause: false it ends with
    // secret_unavailable (no_code).
    let second = h
        .call(
            "locator.fill",
            json!({
                "locator": { "page_id": page_id, "selector": "#otp" },
                "value": "",
                "secret": "otp"
            }),
        )
        .await;
    assert_eq!(
        second["error"]["data"]["code"],
        json!("secret_unavailable"),
        "expected secret_unavailable on second use, got: {second}"
    );
    assert_eq!(
        second["error"]["data"]["failure"],
        json!("no_code"),
        "expected failure=no_code, got: {second}"
    );

    h.finish().await;
}

/// Every `code_source` kind (`imap`, `gmail`, `microsoft_graph`, `webhook`,
/// `endpoint`, `twilio`) reads a matching code inside its window from a local
/// stand-in server of its shape and uses each message once.
///
/// Covers: story:5-85, setting:code_sources, setting:session.recovery.human_pause, error:secret_unavailable
#[test]
fn all_six_code_source_kinds_read_once_from_local_stand_ins() {
    runtime().block_on(async {
        let now = now_ts();
        let now_rfc2822 = rfc2822::to_string(&now.to_zoned(jiff::tz::TimeZone::UTC)).unwrap();

        // 1. IMAP
        {
            let env = TestEnv::new();
            let (port, server) = spawn_imap_server(
                "alice@login.test",
                "imap-pass-secret",
                vec![(
                    42,
                    "Security <noreply@login.test>".to_owned(),
                    now_rfc2822.clone(),
                    "Your verification code is 482910.".to_owned(),
                )],
            )
            .await;
            let vars = BTreeMap::from([
                ("IMAP_USER".to_owned(), "alice@login.test".to_owned()),
                ("IMAP_PASS".to_owned(), "imap-pass-secret".to_owned()),
            ]);
            run_one_time_code_scenario(
                &env,
                vars,
                json!({
                    "kind": {
                        "type": "imap",
                        "host": "127.0.0.1",
                        "port": port,
                        "username": "env:IMAP_USER",
                        "password": "env:IMAP_PASS",
                        "mailbox": "INBOX",
                        "tls": false
                    },
                    "from": "noreply@login.test",
                    "pattern": "code is (\\d{6})",
                    "window": 1500
                }),
                "482910",
            )
            .await;
            server.abort();
        }

        // 2. Gmail
        {
            let env = TestEnv::new();
            let body_b64 = BASE64URL_NOPAD.encode(b"Enter OTP 731904 to sign in.");
            let (base, server) = spawn_http_server(move |_method, path, headers, _body| {
                let auth = headers
                    .iter()
                    .find(|(k, _)| k == "authorization")
                    .map(|(_, v)| v.as_str());
                if auth != Some("Bearer gmail-oauth-token") {
                    return (401, json!({"error": "unauthorized"}).to_string());
                }
                if path.starts_with("/gmail/v1/users/me/messages?") {
                    (200, json!({"messages": [{"id": "gm-1"}]}).to_string())
                } else if path.starts_with("/gmail/v1/users/me/messages/gm-1") {
                    (
                        200,
                        json!({
                            "id": "gm-1",
                            "internalDate": now_ts().as_millisecond().to_string(),
                            "snippet": "Sign in",
                            "payload": {
                                "headers": [{"name": "From", "value": "auth@login.test"}],
                                "body": { "data": body_b64 }
                            }
                        })
                        .to_string(),
                    )
                } else {
                    (404, "{}".to_owned())
                }
            })
            .await;
            let vars = BTreeMap::from([(
                "GMAIL_CREDS".to_owned(),
                json!({"access_token": "gmail-oauth-token"}).to_string(),
            )]);
            run_one_time_code_scenario(
                &env,
                vars,
                json!({
                    "kind": {
                        "type": "gmail",
                        "credentials": "env:GMAIL_CREDS",
                        "endpoint": base
                    },
                    "from": "auth@login.test",
                    "pattern": "OTP (\\d{6})",
                    "window": 200
                }),
                "731904",
            )
            .await;
            server.abort();
        }

        // 3. Microsoft Graph
        {
            let env = TestEnv::new();
            let (base, server) = spawn_http_server(move |method, path, headers, body| {
                if method == "POST" && path == "/my-tenant/oauth2/v2.0/token" {
                    if body.contains("client_id=graph-cid")
                        && body.contains("client_secret=graph-csec")
                    {
                        return (200, json!({"access_token": "graph-token"}).to_string());
                    }
                    return (401, "{}".to_owned());
                }
                let auth = headers
                    .iter()
                    .find(|(k, _)| k == "authorization")
                    .map(|(_, v)| v.as_str());
                if auth != Some("Bearer graph-token") {
                    return (401, "{}".to_owned());
                }
                if path.starts_with("/v1.0/users/ops%40login.test/messages") {
                    (
                        200,
                        json!({
                            "value": [{
                                "id": "mg-1",
                                "receivedDateTime": now_ts().to_string(),
                                "from": { "emailAddress": { "address": "mfa@login.test" } },
                                "subject": "Login code",
                                "body": { "content": "One-time passcode: 590214" }
                            }]
                        })
                        .to_string(),
                    )
                } else {
                    (404, "{}".to_owned())
                }
            })
            .await;
            let vars = BTreeMap::from([
                ("GRAPH_CID".to_owned(), "graph-cid".to_owned()),
                ("GRAPH_CSEC".to_owned(), "graph-csec".to_owned()),
            ]);
            run_one_time_code_scenario(
                &env,
                vars,
                json!({
                    "kind": {
                        "type": "microsoft_graph",
                        "tenant": "my-tenant",
                        "client_id": "env:GRAPH_CID",
                        "client_secret": "env:GRAPH_CSEC",
                        "mailbox": "ops@login.test",
                        "endpoint": base,
                        "authority": base
                    },
                    "from": "mfa@login.test",
                    "window": 200
                }),
                "590214",
            )
            .await;
            server.abort();
        }

        // 4. Endpoint
        {
            let env = TestEnv::new();
            let (base, server) = spawn_http_server(move |_method, path, headers, _body| {
                let auth = headers
                    .iter()
                    .find(|(k, _)| k == "authorization")
                    .map(|(_, v)| v.as_str());
                if path == "/latest" && auth == Some("Bearer ep-secret") {
                    (
                        200,
                        json!({
                            "messages": [{
                                "id": "ep-1",
                                "from": "codes@login.test",
                                "body": "Code: 846201",
                                "received_at": now_ts().to_string()
                            }]
                        })
                        .to_string(),
                    )
                } else {
                    (401, "{}".to_owned())
                }
            })
            .await;
            let vars = BTreeMap::from([("EP_AUTH".to_owned(), "Bearer ep-secret".to_owned())]);
            run_one_time_code_scenario(
                &env,
                vars,
                json!({
                    "kind": {
                        "type": "endpoint",
                        "url": format!("{base}/latest"),
                        "headers": { "Authorization": "env:EP_AUTH" }
                    },
                    "from": "codes@login.test",
                    "window": 200
                }),
                "846201",
            )
            .await;
            server.abort();
        }

        // 5. Twilio
        {
            let env = TestEnv::new();
            let expected_basic = format!("Basic {}", BASE64.encode(b"AC12345:twilio-auth-tok"));
            let date_sent =
                rfc2822::to_string(&now_ts().to_zoned(jiff::tz::TimeZone::UTC)).unwrap();
            let (base, server) = spawn_http_server(move |_method, path, headers, _body| {
                let auth = headers
                    .iter()
                    .find(|(k, _)| k == "authorization")
                    .map(|(_, v)| v.as_str());
                if auth != Some(expected_basic.as_str()) {
                    return (401, "{}".to_owned());
                }
                if path.starts_with("/2010-04-01/Accounts/AC12345/Messages.json")
                    && path.contains("To=%2B15550001111")
                {
                    (
                        200,
                        json!({
                            "messages": [{
                                "sid": "SM0001",
                                "from": "+15559990000",
                                "to": "+15550001111",
                                "body": "SMS code 319472",
                                "date_sent": date_sent
                            }]
                        })
                        .to_string(),
                    )
                } else {
                    (404, "{}".to_owned())
                }
            })
            .await;
            let vars = BTreeMap::from([
                ("TWILIO_SID".to_owned(), "AC12345".to_owned()),
                ("TWILIO_TOK".to_owned(), "twilio-auth-tok".to_owned()),
            ]);
            run_one_time_code_scenario(
                &env,
                vars,
                json!({
                    "kind": {
                        "type": "twilio",
                        "account_sid": "env:TWILIO_SID",
                        "auth_token": "env:TWILIO_TOK",
                        "to": "+15550001111",
                        "endpoint": base
                    },
                    "from": "+15559990000",
                    "window": 1500
                }),
                "319472",
            )
            .await;
            server.abort();
        }

        // 6. Webhook (test POSTs directly to the engine's webhook listener)
        {
            let env = TestEnv::new();
            // Reserve a free loopback port, then drop the probe listener.
            let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let webhook_addr = probe.local_addr().unwrap().to_string();
            drop(probe);

            let mut h = Harness::start(&env, BTreeMap::new()).await;
            let started = h
                .call(
                    "run.start",
                    json!({
                        "run_id": RUN,
                        "settings": {
                            "code_sources": {
                                "hook": {
                                    "kind": {
                                        "type": "webhook",
                                        "path": "/codes/2fa",
                                        "listen": webhook_addr
                                    },
                                    "from": "sms@login.test",
                                    "window": 300
                                }
                            },
                            "secrets": {
                                "placeholders": {
                                    "otp": { "code_source": "hook", "domains": ["login.test"] }
                                }
                            },
                            "session": { "recovery": { "human_pause": false } }
                        }
                    }),
                )
                .await;
            assert!(started.get("result").is_some(), "run.start failed: {started}");

            let opened = h.call("run.new_page", json!({ "run_id": RUN })).await;
            let page_id = opened["result"]["page_id"].as_str().unwrap().to_owned();

            // POST a code payload to the webhook listener.
            let payload = json!({
                "id": "wh-1",
                "from": "sms@login.test",
                "body": "Webhook code 654321",
                "received_at": now_ts().to_string()
            })
            .to_string();
            let mut client = TcpStream::connect(&webhook_addr).await.unwrap();
            let req = format!(
                "POST /codes/2fa HTTP/1.1\r\nHost: {webhook_addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            client.write_all(req.as_bytes()).await.unwrap();
            let mut resp = String::new();
            let _ = client.read_to_string(&mut resp).await;
            assert!(resp.starts_with("HTTP/1.1 200"), "webhook POST failed: {resp}");

            let first = h
                .call(
                    "locator.fill",
                    json!({
                        "locator": { "page_id": page_id, "selector": "#otp" },
                        "value": "",
                        "secret": "otp"
                    }),
                )
                .await;
            assert!(first.get("result").is_some(), "webhook fill failed: {first}");
            assert_eq!(
                h.observed.inserted_text.lock().unwrap().as_slice(),
                &["654321".to_owned()]
            );

            // Second login sees the message already consumed -> secret_unavailable.
            let second = h
                .call(
                    "locator.fill",
                    json!({
                        "locator": { "page_id": page_id, "selector": "#otp" },
                        "value": "",
                        "secret": "otp"
                    }),
                )
                .await;
            assert_eq!(second["error"]["data"]["code"], json!("secret_unavailable"));
            assert_eq!(second["error"]["data"]["failure"], json!("no_code"));

            h.finish().await;
        }
    });
}

/// Messages outside `window` or from another sender are ignored: with
/// `session.recovery.human_pause: false` the call ends with
/// `secret_unavailable` (`no_code`), and with `session.recovery.human_pause:
/// true` (default) a Human pause is raised that a person can answer with
/// `page.pause_answer`.
///
/// Covers: story:5-85, setting:code_sources, setting:session.recovery.human_pause, error:secret_unavailable, operation:page.pause_answer
#[test]
fn out_of_window_or_wrong_sender_raises_human_pause_or_secret_unavailable() {
    runtime().block_on(async {
        let stale = now_ts()
            .checked_sub(SignedDuration::from_secs(60))
            .unwrap()
            .to_string();
        let fresh = now_ts().to_string();
        let (base, server) = spawn_http_server(move |_method, _path, _headers, _body| {
            (
                200,
                json!({
                    "messages": [
                        {
                            "id": "stale-1",
                            "from": "noreply@login.test",
                            "body": "Old code 111111",
                            "received_at": stale
                        },
                        {
                            "id": "wrong-sender-1",
                            "from": "attacker@other.test",
                            "body": "Wrong sender code 222222",
                            "received_at": fresh
                        }
                    ]
                })
                .to_string(),
            )
        })
        .await;

        // Case A: session.recovery.human_pause = false -> secret_unavailable (no_code)
        {
            let env = TestEnv::new();
            let mut h = Harness::start(&env, BTreeMap::new()).await;
            let _ = h
                .call(
                    "run.start",
                    json!({
                        "run_id": RUN,
                        "settings": {
                            "code_sources": {
                                "inbox": {
                                    "kind": { "type": "endpoint", "url": format!("{base}/codes") },
                                    "from": "noreply@login.test",
                                    "window": 150
                                }
                            },
                            "secrets": {
                                "placeholders": {
                                    "otp": { "code_source": "inbox", "domains": ["login.test"] }
                                }
                            },
                            "session": { "recovery": { "human_pause": false } }
                        }
                    }),
                )
                .await;
            let opened = h.call("run.new_page", json!({ "run_id": RUN })).await;
            let page_id = opened["result"]["page_id"].as_str().unwrap().to_owned();

            let reply = h
                .call(
                    "locator.fill",
                    json!({
                        "locator": { "page_id": page_id, "selector": "#otp" },
                        "value": "",
                        "secret": "otp"
                    }),
                )
                .await;
            assert_eq!(reply["error"]["data"]["code"], json!("secret_unavailable"));
            assert_eq!(reply["error"]["data"]["failure"], json!("no_code"));
            assert!(h.observed.inserted_text.lock().unwrap().is_empty());
            h.finish().await;
        }

        // Case B: session.recovery.human_pause = true (default) -> raises Human pause
        {
            let env = TestEnv::new();
            let mut h = Harness::start_with_surface(&env, BTreeMap::new(), "cli").await;
            let _ = h
                .call(
                    "run.start",
                    json!({
                        "run_id": RUN,
                        "settings": {
                            "code_sources": {
                                "inbox": {
                                    "kind": { "type": "endpoint", "url": format!("{base}/codes") },
                                    "from": "noreply@login.test",
                                    "window": 150
                                }
                            },
                            "secrets": {
                                "placeholders": {
                                    "otp": { "code_source": "inbox", "domains": ["login.test"] }
                                }
                            },
                            "timeout": { "human_pause": 5000 }
                        }
                    }),
                )
                .await;
            let opened = h.call("run.new_page", json!({ "run_id": RUN })).await;
            let page_id = opened["result"]["page_id"].as_str().unwrap().to_owned();

            let fill_id = h
                .send(
                    "locator.fill",
                    json!({
                        "locator": { "page_id": page_id, "selector": "#otp" },
                        "value": "",
                        "secret": "otp"
                    }),
                )
                .await;

            // Wait until the short 150ms code_source window expires and the Human pause is active.
            let mut answered_pause = false;
            for _ in 0..30 {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                let ans = h
                    .call(
                        "page.pause_answer",
                        json!({
                            "page_id": page_id,
                            "answer": "done"
                        }),
                    )
                    .await;
                if ans.get("result").and_then(|r| r.get("accepted")) == Some(&json!(true)) {
                    answered_pause = true;
                    break;
                }
            }
            assert!(
                answered_pause,
                "expected Human pause to become active after code_source window expired"
            );

            let fill_reply = h.read_reply(fill_id).await;
            assert!(
                fill_reply.get("result").is_some(),
                "expected locator.fill to complete after Human pause answered: {fill_reply}"
            );
            assert!(h.observed.inserted_text.lock().unwrap().is_empty());
            h.finish().await;
        }

        server.abort();
    });
}

/// A Client acting for a model cannot set `code_sources`: receives `setting_locked`.
///
/// Covers: story:5-85, setting:code_sources, error:setting_locked
#[test]
fn model_client_cannot_set_code_sources() {
    runtime().block_on(async {
        let env = TestEnv::new();
        let mut h = Harness::start_with_surface(&env, BTreeMap::new(), "mcp").await;
        let reply = h
            .call(
                "run.start",
                json!({
                    "run_id": RUN,
                    "settings": {
                        "code_sources": {
                            "inbox": {
                                "kind": { "type": "webhook", "path": "/codes" },
                                "window": 100
                            }
                        }
                    }
                }),
            )
            .await;
        assert_eq!(
            reply["error"]["data"]["code"],
            json!("setting_locked"),
            "expected setting_locked for model client setting code_sources: {reply}"
        );
        h.close().await;
    });
}
