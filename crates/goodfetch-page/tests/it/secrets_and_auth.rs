//! Secrets typed into fields, Secret placeholders in route overrides, and
//! domain-bound `session.http_credentials` (ADR 0092; ticket #733) against the
//! in-process engine's scripted browser:
//! - `locator.fill` with `secret` types on its bound domain, refuses with
//!   `secret_domain_not_allowed` on another frame's origin without typing, and
//!   keeps the password canary out of responses;
//! - `route.continue` with `secret_headers` resolves on a bound domain and
//!   drops the header on a redirect hop to another domain;
//! - `session.http_credentials` answers Basic and Digest 401 challenges on a
//!   listed domain and answers `Default` (no credentials) on an unlisted one;
//! - a Client acting for a model setting `session.http_credentials` receives
//!   `setting_locked`.
#![expect(
    clippy::disallowed_types,
    reason = "integration test observer records CDP commands across tasks, never production lock order (ADR 0060)"
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
    reason = "each integration test drives one multi-step JSON-RPC scenario end to end"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, runtime};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::session_counts::session::{Page, RUN, Wire, answered, dependencies, engine};

const TOKEN: &str = "b2-launch-token";

/// Recorded CDP commands the scripted browser observed.
#[derive(Default, Clone)]
struct Observed {
    inserted_text: Arc<Mutex<Vec<String>>>,
    continued_requests: Arc<Mutex<Vec<Value>>>,
    auth_answers: Arc<Mutex<Vec<Value>>>,
}

/// Commands the test can inject into the scripted browser loop.
enum BrowserAction {
    /// Set the `frame_url` reported by `guard/check.js` `Focused` and ack.
    SetFrameUrl(String, oneshot::Sender<()>),
    /// Emit `Fetch.requestPaused` for a request (with `networkId`).
    PauseRequest {
        request_id: String,
        network_id: String,
        url: String,
        headers: Value,
    },
    /// Emit `Fetch.authRequired` for an origin 401 challenge.
    AuthRequired {
        request_id: String,
        url: String,
        scheme: String,
    },
}

/// Scripted browser that records `Input.insertText`, `Fetch.continueRequest`,
/// and `Fetch.continueWithAuth` while reacting to [`BrowserAction`]s.
async fn scripted_browser(
    mut launched: ScriptedBrowsers,
    mut actions: mpsc::Receiver<BrowserAction>,
    observed: Observed,
) {
    let Some(pipe) = launched.next().await else {
        return;
    };
    let mut wire = Wire::pipe(pipe);
    let mut page = Page::blank();
    let mut frame_url = "https://login.test/form".to_owned();
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
                match method {
                    "Input.insertText" => {
                        if let Some(t) = command["params"]["text"].as_str() {
                            observed.inserted_text.lock().unwrap().push(t.to_owned());
                        }
                    }
                    "Fetch.continueRequest" => {
                        observed
                            .continued_requests
                            .lock()
                            .unwrap()
                            .push(command["params"].clone());
                    }
                    "Fetch.continueWithAuth" => {
                        observed
                            .auth_answers
                            .lock()
                            .unwrap()
                            .push(command["params"].clone());
                    }
                    _ => {}
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
                                        "password": true,
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
            }
            Some(action) = actions.recv() => {
                let fid = if frame_id.is_empty() {
                    target_id.clone()
                } else {
                    frame_id.clone()
                };
                match action {
                    BrowserAction::SetFrameUrl(url, ack) => {
                        frame_url = url;
                        let _ = ack.send(());
                    }
                    BrowserAction::PauseRequest {
                        request_id,
                        network_id,
                        url,
                        headers,
                    } => {
                        let event = json!({
                            "method": "Fetch.requestPaused",
                            "sessionId": session_id,
                            "params": {
                                "requestId": request_id,
                                "networkId": network_id,
                                "request": {
                                    "url": url,
                                    "method": "GET",
                                    "headers": headers,
                                    "initialPriority": "VeryHigh",
                                    "referrerPolicy": "strict-origin-when-cross-origin",
                                },
                                "frameId": fid,
                                "resourceType": "Fetch",
                            }
                        });
                        let _ = wire.send(event.to_string()).await;
                    }
                    BrowserAction::AuthRequired {
                        request_id,
                        url,
                        scheme,
                    } => {
                        let event = json!({
                            "method": "Fetch.authRequired",
                            "sessionId": session_id,
                            "params": {
                                "requestId": request_id,
                                "request": {
                                    "url": url,
                                    "method": "GET",
                                    "headers": {},
                                    "initialPriority": "VeryHigh",
                                    "referrerPolicy": "strict-origin-when-cross-origin",
                                },
                                "frameId": fid,
                                "resourceType": "Document",
                                "authChallenge": {
                                    "source": "Server",
                                    "origin": url,
                                    "scheme": scheme,
                                    "realm": "Restricted",
                                }
                            }
                        });
                        let _ = wire.send(event.to_string()).await;
                    }
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
    requests: Vec<Value>,
}

impl RpcClient {
    async fn connect(dialer: &transport::InMemoryDialer, surface: &str) -> Self {
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
            requests: Vec::new(),
        };
        client
            .call(
                "hello",
                &json!({
                    "protocol_version": "0.1",
                    "surface": surface,
                    "needs": "collie",
                    "product_version": "0.0.0",
                    "token": TOKEN,
                }),
            )
            .await;
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
        self.writer.flush().await.unwrap();
        id
    }

    async fn reply(&mut self, id: &Value, result: Value) {
        let line = json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        self.writer.flush().await.unwrap();
    }

    async fn answer(&mut self, id: u64) -> Value {
        if let Some(reply) = self.ahead.remove(&id) {
            return reply;
        }
        loop {
            let line = self.lines.next_line().await.unwrap().expect("engine open");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message.get("method").is_some() {
                self.requests.push(message);
            } else if let Some(got) = message.get("id").and_then(Value::as_u64) {
                if got == id {
                    return message;
                }
                self.ahead.insert(got, message);
            }
        }
    }

    async fn next_request(&mut self, method: &str) -> Value {
        if let Some(pos) = self
            .requests
            .iter()
            .position(|msg| msg.get("method").and_then(Value::as_str) == Some(method))
        {
            return self.requests.remove(pos);
        }
        loop {
            let line = self.lines.next_line().await.unwrap().expect("engine open");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message.get("method").and_then(Value::as_str) == Some(method) {
                return message;
            } else if message.get("method").is_some() {
                self.requests.push(message);
            } else if let Some(got) = message.get("id").and_then(Value::as_u64) {
                self.ahead.insert(got, message);
            }
        }
    }

    async fn call(&mut self, method: &str, params: &Value) -> Value {
        let id = self.start(method, params).await;
        let reply = self.answer(id).await;
        assert!(reply.get("result").is_some(), "`{method}` failed: {reply}");
        reply["result"].clone()
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
    }
}

/// Covers: story:5-84, operation:locator.fill, error:secret_domain_not_allowed
#[test]
fn secret_fill_types_on_bound_domain_and_refuses_other_origin_without_leaking_canary() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let password = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);
    let observed = Observed::default();

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "rust").await;
            client
                .call(
                    "run.start",
                    &json!({
                        "run_id": RUN,
                        "settings": {
                            "secrets.placeholders": {
                                "site_pw": {
                                    "reference": "env:LOGIN_PW",
                                    "domains": ["login.test"]
                                }
                            }
                        }
                    }),
                )
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let page = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            let page_id = page["page_id"].as_str().unwrap().to_owned();

            // 1. Bound domain: types the resolved secret via `%name%` syntax.
            let filled = client
                .call(
                    "locator.fill",
                    &json!({
                        "locator": { "page_id": page_id, "selector": "#password" },
                        "value": "%site_pw%",
                    }),
                )
                .await;
            assert_eq!(
                observed.inserted_text.lock().unwrap().as_slice(),
                std::slice::from_ref(&password)
            );

            // 2. Different frame origin: refused with secret_domain_not_allowed, nothing typed.
            let (ack_tx, ack_rx) = oneshot::channel();
            action_tx
                .send(BrowserAction::SetFrameUrl(
                    "https://evil.test/phish".to_owned(),
                    ack_tx,
                ))
                .await
                .unwrap();
            ack_rx.await.unwrap();
            let id = client
                .start(
                    "locator.fill",
                    &json!({
                        "locator": { "page_id": page_id, "selector": "#password" },
                        "value": "",
                        "secret": "site_pw",
                    }),
                )
                .await;
            let refused = client.answer(id).await;
            assert_eq!(
                refused["error"]["data"]["code"],
                json!("secret_domain_not_allowed"),
                "{refused}"
            );
            assert_eq!(observed.inserted_text.lock().unwrap().len(), 1);

            // 3. Passing both `value` and `secret` is rejected with invalid_params.
            let id = client
                .start(
                    "locator.fill",
                    &json!({
                        "locator": { "page_id": page_id, "selector": "#password" },
                        "value": "plain",
                        "secret": "site_pw",
                    }),
                )
                .await;
            let both = client.answer(id).await;
            assert_eq!(both["error"]["code"], json!(-32602), "{both}");

            // 4. Canary oracle: no password canary in responses or Run files.
            let filled_bytes = filled.to_string().into_bytes();
            let refused_bytes = refused.to_string().into_bytes();
            let leaks = canaries
                .leaks(&[
                    Output::Read {
                        name: "fill_reply",
                        bytes: &filled_bytes,
                    },
                    Output::Read {
                        name: "refused_reply",
                        bytes: &refused_bytes,
                    },
                    Output::Path(home.path()),
                ])
                .unwrap();
            assert!(leaks.is_empty(), "password canary leaked: {leaks:?}");

            client.call("run.finish", &json!({ "run_id": RUN })).await;
            drop(action_tx);
            client.close().await;
        };
        let mut deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        deps.environment
            .insert("LOGIN_PW".into(), password.clone().into());
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(transport::Network::InMemory(network), deps),
            scripted_browser(browsers, action_rx, observed.clone()),
            driver,
        );
        ended.unwrap();
    });
}

/// Covers: story:5-84, operation:route.continue
#[test]
fn route_override_secret_headers_reach_bound_domain_and_drop_on_redirect() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let token = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);
    let observed = Observed::default();

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "rust").await;
            client
                .call(
                    "run.start",
                    &json!({
                        "run_id": RUN,
                        "settings": {
                            "egress.allowed_domains": ["login.test", "other.test"],
                            "secrets.placeholders": {
                                "api_key": {
                                    "reference": "env:API_TOKEN",
                                    "domains": ["login.test"]
                                }
                            }
                        }
                    }),
                )
                .await;
            let session = client
                .call("run.new_session", &json!({ "run_id": RUN }))
                .await;
            let page = client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;
            let page_id = page["page_id"].as_str().unwrap().to_owned();

            // Arm a route matching https://api.login.test/*
            let _route_id = client
                .start(
                    "page.route",
                    &json!({
                        "page_id": page_id,
                        "url": { "kind": "glob", "pattern": "https://api.login.test/*" }
                    }),
                )
                .await;
            // Give the engine a turn to arm the route.
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }

            // 1. First hop on bound domain: route.continue adds secret_headers.
            action_tx
                .send(BrowserAction::PauseRequest {
                    request_id: "req-1".to_owned(),
                    network_id: "net-1".to_owned(),
                    url: "https://api.login.test/start".to_owned(),
                    headers: json!({ "Accept": "application/json" }),
                })
                .await
                .unwrap();
            let event = client.next_request("route.handle").await;
            let route_id = event["params"]["route_id"].as_str().unwrap().to_owned();
            client
                .call(
                    "route.continue",
                    &json!({
                        "route_id": route_id,
                        "secret_headers": {
                            "X-Api-Key": { "form": "value", "placeholder": "api_key", "prefix": "Bearer " }
                        }
                    }),
                )
                .await;
            client.reply(&event["id"], json!({})).await;

            tokio::time::timeout(Duration::from_secs(2), async {
                while observed.continued_requests.lock().unwrap().is_empty() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let first_headers = observed.continued_requests.lock().unwrap()[0]["headers"]
                .as_array()
                .unwrap()
                .clone();
            assert!(
                first_headers.iter().any(|h| {
                    h["name"].as_str().unwrap().eq_ignore_ascii_case("x-api-key")
                        && h["value"].as_str().unwrap() == format!("Bearer {token}")
                }),
                "expected X-Api-Key on bound domain, got {first_headers:?}"
            );

            // 2. Redirect hop (same networkId) to another domain carrying the header:
            //    step 7 strips X-Api-Key before continuing!
            action_tx
                .send(BrowserAction::PauseRequest {
                    request_id: "req-2".to_owned(),
                    network_id: "net-1".to_owned(),
                    url: "https://other.test/landed".to_owned(),
                    headers: json!({
                        "Accept": "application/json",
                        "X-Api-Key": format!("Bearer {token}"),
                    }),
                })
                .await
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                while observed.continued_requests.lock().unwrap().len() < 2 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            let second_headers = observed.continued_requests.lock().unwrap()[1]["headers"]
                .as_array()
                .unwrap()
                .clone();
            assert!(
                !second_headers
                    .iter()
                    .any(|h| h["name"].as_str().unwrap().eq_ignore_ascii_case("x-api-key")),
                "expected X-Api-Key dropped on redirect to other.test, got {second_headers:?}"
            );

            client.call("run.finish", &json!({ "run_id": RUN })).await;
            drop(action_tx);
            client.close().await;
        };
        let mut deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        deps.environment
            .insert("API_TOKEN".into(), token.clone().into());
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(transport::Network::InMemory(network), deps),
            scripted_browser(browsers, action_rx, observed.clone()),
            driver,
        );
        ended.unwrap();
    });
}

/// Covers: story:5-86, setting:session.http_credentials
#[test]
fn http_credentials_answers_basic_and_digest_on_listed_domain_only() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let password = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let (action_tx, action_rx) = mpsc::channel(8);
    let observed = Observed::default();

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "rust").await;
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": {} }))
                .await;
            let session = client
                .call(
                    "run.new_session",
                    &json!({
                        "run_id": RUN,
                        "settings": {
                            "session.http_credentials": [{
                                "username": "alice",
                                "password": "env:HTTP_PW",
                                "domains": ["auth.test"],
                            }]
                        }
                    }),
                )
                .await;
            client
                .call(
                    "session.new_page",
                    &json!({ "session_id": session["session_id"] }),
                )
                .await;

            // 1. Basic challenge on listed domain -> ProvideCredentials.
            action_tx
                .send(BrowserAction::AuthRequired {
                    request_id: "auth-1".to_owned(),
                    url: "https://auth.test/basic".to_owned(),
                    scheme: "Basic".to_owned(),
                })
                .await
                .unwrap();
            // 2. Digest challenge on subdomain of listed domain -> ProvideCredentials.
            action_tx
                .send(BrowserAction::AuthRequired {
                    request_id: "auth-2".to_owned(),
                    url: "https://sub.auth.test/digest".to_owned(),
                    scheme: "Digest".to_owned(),
                })
                .await
                .unwrap();
            // 3. Challenge on unlisted domain -> Default (no credentials).
            action_tx
                .send(BrowserAction::AuthRequired {
                    request_id: "auth-3".to_owned(),
                    url: "https://unlisted.test/basic".to_owned(),
                    scheme: "Basic".to_owned(),
                })
                .await
                .unwrap();

            tokio::time::timeout(Duration::from_secs(2), async {
                while observed.auth_answers.lock().unwrap().len() < 3 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();

            let answers = observed.auth_answers.lock().unwrap().clone();
            let by_id = |id: &str| {
                answers
                    .iter()
                    .find(|a| a["requestId"].as_str() == Some(id))
                    .cloned()
                    .unwrap_or_else(|| panic!("missing auth answer for {id}: {answers:?}"))
            };
            let auth_1 = by_id("auth-1");
            assert_eq!(
                auth_1["authChallengeResponse"]["response"],
                "ProvideCredentials"
            );
            assert_eq!(auth_1["authChallengeResponse"]["username"], "alice");
            assert_eq!(auth_1["authChallengeResponse"]["password"], password);

            let auth_2 = by_id("auth-2");
            assert_eq!(
                auth_2["authChallengeResponse"]["response"],
                "ProvideCredentials"
            );
            assert_eq!(auth_2["authChallengeResponse"]["username"], "alice");
            assert_eq!(auth_2["authChallengeResponse"]["password"], password);

            let auth_3 = by_id("auth-3");
            assert_eq!(auth_3["authChallengeResponse"]["response"], "Default");
            assert!(auth_3["authChallengeResponse"].get("password").is_none());

            client.call("run.finish", &json!({ "run_id": RUN })).await;
            drop(action_tx);
            client.close().await;
        };
        let mut deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        deps.environment
            .insert("HTTP_PW".into(), password.clone().into());
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(transport::Network::InMemory(network), deps),
            scripted_browser(browsers, action_rx, observed.clone()),
            driver,
        );
        ended.unwrap();
    });
}

/// Covers: story:5-86, setting:session.http_credentials, error:setting_locked
#[test]
fn model_surface_setting_http_credentials_gets_setting_locked() {
    let home = tempfile::tempdir().unwrap();
    let (launcher, _browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "mcp").await;
            client
                .call("run.start", &json!({ "run_id": RUN, "settings": {} }))
                .await;
            let id = client
                .start(
                    "run.new_session",
                    &json!({
                        "run_id": RUN,
                        "settings": {
                            "session.http_credentials": [{
                                "username": "alice",
                                "password": "env:HTTP_PW",
                                "domains": ["auth.test"],
                            }]
                        }
                    }),
                )
                .await;
            let refused = client.answer(id).await;
            assert_eq!(
                refused["error"]["data"]["code"],
                json!("setting_locked"),
                "expected setting_locked on mcp surface, got {refused}"
            );
            client.call("run.finish", &json!({ "run_id": RUN })).await;
            client.close().await;
        };
        let deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        let (ended, ()) = tokio::join!(
            engine.run_attached(transport::Network::InMemory(network), deps),
            driver
        );
        ended.unwrap();
    });
}
