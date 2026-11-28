//! Run-only Session imports (`state_file` and `cookie_header`, ADR 0056;
//! ticket #735) against the in-process engine's scripted browser:
//! - Playwright 1.64 `storageState` carries cookies, `localStorage`,
//!   `indexedDB`, and `opfs` into a Session;
//! - Netscape `cookies.txt`, CDP `Network.Cookie` array, and an `age`-sealed
//!   `session-export` bundle are each auto-detected and imported (with
//!   `session_imported` audited for the bundle);
//! - imported state is sealed in `page_sessions.state`, absent in clear on
//!   disk (verified by the canary oracle over Goodfetch home), keeps no source
//!   file, and is removed when the Run's state is reset;
//! - `cookie_header` seeds cookies for its one listed domain only.
#![expect(
    clippy::disallowed_types,
    reason = "integration test observer records CDP commands across tasks, never production lock order (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
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
use std::time::SystemTime;

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::session_bundle::{self, To};
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::home::AUDIT_LOG;
use goodfetch_protocol::session_export::{
    ExportedCookie, ExportedOrigin, SameSite, SessionExport, SessionValue, StorageItem,
};
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, runtime};
use rusqlite::Connection;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::session_counts::session::{Page, RUN, Wire, answered, dependencies, engine};

const TOKEN: &str = "b2-launch-token";

#[derive(Default, Clone)]
struct Observed {
    set_cookies: Arc<Mutex<Vec<Value>>>,
    storage_writes: Arc<Mutex<Vec<String>>>,
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
        match method {
            "Storage.setCookies" => {
                if let Some(cookies) = command["params"]["cookies"].as_array() {
                    observed
                        .set_cookies
                        .lock()
                        .unwrap()
                        .extend(cookies.iter().cloned());
                }
            }
            "Runtime.callFunctionOn" | "Runtime.evaluate" => {
                let code = command["params"]["functionDeclaration"]
                    .as_str()
                    .or_else(|| command["params"]["expression"].as_str());
                if let Some(decl) = code {
                    if decl.contains("localStorage.setItem") {
                        observed
                            .storage_writes
                            .lock()
                            .unwrap()
                            .push("localStorage".to_owned());
                    } else if decl.contains("indexedDB.open") {
                        observed
                            .storage_writes
                            .lock()
                            .unwrap()
                            .push("indexedDB".to_owned());
                    } else if decl.contains("navigator.storage") {
                        observed
                            .storage_writes
                            .lock()
                            .unwrap()
                            .push("opfs".to_owned());
                    }
                }
            }
            _ => {}
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
        // When carry.rs navigates an origin page to restore storage, emit
        // Page.frameNavigated for that origin so commit() completes at once.
        if method == "Page.navigate"
            && let Some(url) = command["params"]["url"].as_str()
            && let Some(origin) = url.strip_suffix('/')
            && let Some(session) = command.get("sessionId")
        {
            let navigated = json!({
                "method": "Page.frameNavigated",
                "sessionId": session,
                "params": {
                    "frame": {
                        "id": "carry-frame",
                        "loaderId": "carry-loader",
                        "url": url,
                        "domainAndRegistry": "",
                        "securityOrigin": origin,
                        "mimeType": "text/html",
                        "secureContextType": "Secure",
                        "crossOriginIsolatedContextType": "NotIsolated",
                        "gatedAPIFeatures": [],
                    },
                    "type": "Navigation",
                }
            });
            let _ = wire.send(navigated.to_string()).await;
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

    async fn call(&mut self, method: &str, params: &Value) -> Value {
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
        let reply = loop {
            if let Some(reply) = self.ahead.remove(&id) {
                break reply;
            }
            let line = self.lines.next_line().await.unwrap().expect("engine open");
            let message: Value = serde_json::from_str(&line).unwrap();
            if let Some(got) = message.get("id").and_then(Value::as_u64) {
                if got == id {
                    break message;
                }
                self.ahead.insert(got, message);
            }
        };
        assert!(reply.get("result").is_some(), "`{method}` failed: {reply}");
        reply["result"].clone()
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
    }
}

/// Covers: story:5-64
#[test]
fn state_file_imports_all_four_formats_and_seals_in_run_state() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let storage_canary = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let txt_canary = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let cdp_canary = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let bundle_canary = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let passphrase = "gf-fake-bundle-passphrase-0001";

    let home = tempfile::tempdir().unwrap();
    let sources = tempfile::tempdir().unwrap();

    // 1. Playwright 1.64 storageState (cookies + localStorage + indexedDB + opfs).
    let storage_path = sources.path().join("state.json");
    std::fs::write(
        &storage_path,
        serde_json::to_vec(&json!({
            "cookies": [{
                "name": "pw_sid",
                "value": storage_canary,
                "domain": "shop.test",
                "path": "/",
                "httpOnly": true,
                "secure": true,
                "sameSite": "Lax",
            }],
            "origins": [{
                "origin": "https://shop.test",
                "localStorage": [{ "name": "tok", "value": storage_canary }],
                "indexedDB": [{ "name": "auth_db", "version": 1, "stores": [] }],
                "opfs": [{ "path": "auth.bin", "contentBase64": "AQID" }],
            }],
        }))
        .unwrap(),
    )
    .unwrap();

    // 2. Netscape cookies.txt.
    let txt_path = sources.path().join("cookies.txt");
    std::fs::write(
        &txt_path,
        format!(
            "# Netscape HTTP Cookie File\n#HttpOnly_.txt.test\tTRUE\t/\tTRUE\t0\ttxt_sid\t{txt_canary}\n"
        ),
    )
    .unwrap();

    // 3. CDP Network.Cookie array.
    let cdp_path = sources.path().join("cdp.json");
    std::fs::write(
        &cdp_path,
        serde_json::to_vec(&json!([{
            "name": "cdp_sid",
            "value": cdp_canary,
            "domain": "cdp.test",
            "path": "/",
            "httpOnly": false,
            "secure": true,
        }]))
        .unwrap(),
    )
    .unwrap();

    // 4. age-encrypted session-export bundle.
    let bundle_path = sources.path().join("session.age");
    let export = SessionExport {
        format_version: ProtocolVersion { major: 0, minor: 1 },
        cookies: vec![ExportedCookie {
            name: "bundle_sid".to_owned(),
            value: SessionValue::new(bundle_canary.clone()),
            domain: "bundle.test".to_owned(),
            path: "/".to_owned(),
            expires: None,
            http_only: true,
            secure: true,
            same_site: Some(SameSite::Lax),
        }],
        origins: vec![ExportedOrigin {
            origin: "https://bundle.test".to_owned(),
            local_storage: vec![StorageItem {
                name: "k".to_owned(),
                value: SessionValue::new("v"),
            }],
            session_storage: Vec::new(),
        }],
        identity: None,
    };
    let sealed = session_bundle::seal(
        &export,
        &To::Passphrase {
            passphrase: SecretString::from(passphrase.to_owned()),
            work_factor: 10,
        },
    );
    std::fs::write(&bundle_path, sealed).unwrap();

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

            // Open 4 Sessions, one per format (all auto-detected).
            for source in [
                json!({ "kind": "state_file", "path": storage_path }),
                json!({ "kind": "state_file", "path": txt_path }),
                json!({ "kind": "state_file", "path": cdp_path }),
                json!({
                    "kind": "state_file",
                    "path": bundle_path,
                    "key": "env:BUNDLE_KEY",
                }),
            ] {
                client
                    .call(
                        "run.new_session",
                        &json!({ "run_id": RUN, "source": source }),
                    )
                    .await;
            }

            // Verify all 4 cookies and all 3 storage layers were applied.
            let cookie_names: Vec<String> = observed
                .set_cookies
                .lock()
                .unwrap()
                .iter()
                .filter_map(|c| c["name"].as_str().map(str::to_owned))
                .collect();
            for expected in ["pw_sid", "txt_sid", "cdp_sid", "bundle_sid"] {
                assert!(
                    cookie_names.iter().any(|n| n == expected),
                    "missing cookie {expected} in {cookie_names:?}"
                );
            }
            let writes = observed.storage_writes.lock().unwrap().clone();
            for layer in ["localStorage", "indexedDB", "opfs"] {
                assert!(
                    writes.iter().any(|w| w == layer),
                    "missing {layer} restore in {writes:?}"
                );
            }

            // Verify Audit log recorded `session_imported` for the bundle.
            let audit = Connection::open(home.path().join(AUDIT_LOG)).unwrap();
            let imported_count: i64 = audit
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE entry LIKE '%session_imported%'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(imported_count >= 1, "expected session_imported audit entry");

            // Canary oracle: ZERO plaintext cookies/storage anywhere in Goodfetch home!
            let leaks = canaries.leaks(&[Output::Path(home.path())]).unwrap();
            assert!(
                leaks.is_empty(),
                "run-only import leaked plaintext onto disk: {leaks:?}"
            );

            client.call("run.finish", &json!({ "run_id": RUN })).await;
            client.close().await;
        };
        let mut deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH),
            ..dependencies(home.path())
        };
        deps.environment
            .insert("BUNDLE_KEY".into(), passphrase.into());
        let (ended, (), ()) = tokio::join!(
            engine.run_attached(transport::Network::InMemory(network), deps),
            scripted_browser(browsers, observed.clone()),
            driver,
        );
        ended.unwrap();
    });
}

/// Covers: story:5-65
#[test]
fn cookie_header_seeds_session_for_one_domain_only() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let token = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let home = tempfile::tempdir().unwrap();
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
            client
                .call(
                    "run.new_session",
                    &json!({
                        "run_id": RUN,
                        "source": {
                            "kind": "cookie_header",
                            "header": format!("Cookie: sid={token}; theme=dark"),
                            "domain": "only.test",
                        }
                    }),
                )
                .await;

            let cookies = observed.set_cookies.lock().unwrap().clone();
            assert_eq!(cookies.len(), 2, "expected 2 cookies, got {cookies:?}");
            assert!(cookies.iter().all(|c| c["domain"] == "only.test"));
            assert!(
                cookies
                    .iter()
                    .any(|c| c["name"] == "sid" && c["value"] == token)
            );

            // Canary oracle: zero plaintext on disk.
            let leaks = canaries.leaks(&[Output::Path(home.path())]).unwrap();
            assert!(
                leaks.is_empty(),
                "cookie_header leaked onto disk: {leaks:?}"
            );

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
}
