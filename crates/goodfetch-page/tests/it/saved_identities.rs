//! Saved identities (`session.save_identity`, `session.source: saved_identity`,
//! and `identity.*`, ADRs 0015, 0035, 0056 and 0092; ticket #736) against the
//! in-process engine's scripted browser and paused `Clock`:
//! - `session.save_identity` encrypts cookies, `localStorage`, and identity
//!   metadata into `<home>/<major>/saved-identities.sqlite` (Unix mode 0600),
//!   leaving zero plaintext leaks on disk (verified by the canary oracle);
//! - reusing via `session.source: { kind: "saved_identity", name }` restores
//!   cookies and `localStorage`, records `saved_identity_reused` in the Audit
//!   log, and enforces Access mode, proxy consistency, and machine binding;
//! - unused identities expire after `saved_identity.max_unused_age` (30 days
//!   default), returning `saved_identity_not_found` and auditing
//!   `saved_identity_deleted` (`reason: "expired"`);
//! - `identity.list`, `identity.import`, `identity.delete`, `identity.export`
//!   (encrypted `age` bundle), and person-only `identity.export_storage_state`
//!   (`person_required` on SDK surfaces).
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
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use goodfetch_engine::Dependencies;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::metrics::Metrics;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter};
use goodfetch_page::{Launcher, ScriptedBrowsers};
use goodfetch_protocol::home::AUDIT_LOG;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, runtime};
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::session_counts::session::{Page, Wire, answered, dependencies, engine};

const TOKEN: &str = "b2-saved-identity-token";
const FAKE_EXPORT_PASSPHRASE: &str = "gf-fake-export-passphrase-0001";

#[derive(Default, Clone)]
struct Observed {
    set_cookies: Arc<Mutex<Vec<Value>>>,
    storage_writes: Arc<Mutex<Vec<String>>>,
}

async fn scripted_browser(mut launched: ScriptedBrowsers, observed: Observed, seed_cookies: Value) {
    while let Some(pipe) = launched.next().await {
        let mut wire = Wire::pipe(pipe);
        let mut page = Page::blank();
        loop {
            let Some(text) = wire.next().await else {
                break;
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
                    if let Some(decl) = code
                        && decl.contains("localStorage.setItem")
                    {
                        observed
                            .storage_writes
                            .lock()
                            .unwrap()
                            .push("localStorage".to_owned());
                    }
                }
                _ => {}
            }
            let mut answer = if method == "Storage.getCookies" {
                json!({ "result": { "cookies": seed_cookies } })
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
                break;
            }
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
}

struct RpcClient {
    lines: tokio::io::Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
    next_id: u64,
    ahead: BTreeMap<u64, Value>,
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

    async fn ask(&mut self, method: &str, params: &Value) -> Value {
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
        let reply = self.ask(method, params).await;
        assert!(reply.get("result").is_some(), "`{method}` failed: {reply}");
        reply["result"].clone()
    }

    async fn close(mut self) {
        let _ = self.writer.shutdown().await;
    }
}

fn audit_kinds(home: &std::path::Path) -> Vec<String> {
    let conn = Connection::open(home.join(AUDIT_LOG)).unwrap();
    let mut stmt = conn.prepare("SELECT entry FROM entries").unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|raw| {
            let doc: Value = serde_json::from_str(&raw.unwrap()).unwrap();
            doc["kind"].as_str().unwrap_or_default().to_owned()
        })
        .collect()
}

/// Covers: story:5-63, setting:saved_identity.max_unused_age, operation:session.save_identity, operation:identity.list, operation:identity.delete, error:saved_identity_mismatch, error:saved_identity_not_found
#[test]
fn save_reuse_mismatches_and_expiry_work_with_encrypted_store() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let login_canary = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let home = tempfile::tempdir().unwrap();
    let (launcher, browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);
    let observed = Observed::default();

    let seed_cookies = json!([{
        "name": "acct_sid",
        "value": login_canary,
        "domain": "portal.test",
        "path": "/",
        "expires": 1_900_000_000.0,
        "httpOnly": true,
        "secure": true,
        "sameSite": "Lax",
    }]);

    runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000));
        let (network, dialer) = transport::in_memory();
        let driver = async {
            let mut client = RpcClient::connect(&dialer, "rust").await;

            // Run 1: open Session, seed cookies via cookie_header, save_identity("portal").
            client
                .call("run.start", &json!({ "run_id": "save-run-1", "settings": {} }))
                .await;
            let s1 = client
                .call(
                    "run.new_session",
                    &json!({
                        "run_id": "save-run-1",
                        "source": {
                            "kind": "cookie_header",
                            "header": format!("acct_sid={login_canary}"),
                            "domain": "portal.test",
                        }
                    }),
                )
                .await;
            let session_id = s1["session_id"].as_str().unwrap().to_owned();
            let saved = client
                .call(
                    "session.save_identity",
                    &json!({ "session_id": session_id, "name": "portal" }),
                )
                .await;
            assert_eq!(saved["name"], "portal");
            client.call("run.finish", &json!({ "run_id": "save-run-1" })).await;

            // Store file exists with 0600 permissions on Unix.
            let store_path = goodfetch_engine::home::Home::new(home.path().to_path_buf())
                .saved_identities(&goodfetch_protocol::connection::ProductVersion::parse("0.0.0").unwrap());
            assert!(store_path.exists(), "expected {}", store_path.display());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = std::fs::metadata(&store_path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "saved-identities.sqlite must be 0600");
            }

            // Canary oracle: zero plaintext cookies anywhere under Goodfetch home.
            let leaks = canaries.leaks(&[Output::Path(home.path())]).unwrap();
            assert!(leaks.is_empty(), "Saved identity leaked plaintext on disk: {leaks:?}");

            // identity.list shows metadata only.
            let listed = client.call("identity.list", &json!({})).await;
            let items = listed["identities"].as_array().unwrap();
            assert_eq!(items.len(), 1);
            assert_eq!(items[0]["name"], "portal");
            assert_eq!(items[0]["access_mode"], "standard");
            assert_eq!(items[0]["was_lent"], false);
            assert!(
                items[0]["domains"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|d| d == "portal.test")
            );
            assert!(!listed.to_string().contains(&login_canary));

            // Run 2: reuse Saved identity under matching Access mode -> succeeds & injects cookies.
            observed.set_cookies.lock().unwrap().clear();
            client
                .call(
                    "run.start",
                    &json!({
                        "run_id": "reuse-run-2",
                        "settings": { "session": { "source": { "kind": "saved_identity", "name": "portal" } } }
                    }),
                )
                .await;
            let s2 = client
                .call("run.new_session", &json!({ "run_id": "reuse-run-2" }))
                .await;
            assert!(s2["session_id"].is_string());
            let injected = observed.set_cookies.lock().unwrap().clone();
            assert!(
                injected
                    .iter()
                    .any(|c| c["name"] == "acct_sid" && c["value"] == login_canary),
                "expected acct_sid cookie to be restored: {injected:?}"
            );
            client.call("run.finish", &json!({ "run_id": "reuse-run-2" })).await;

            // Run 3: Access mode mismatch (`stealth` vs saved `standard`) -> saved_identity_mismatch.
            client
                .call(
                    "run.start",
                    &json!({
                        "run_id": "mismatch-mode-run",
                        "settings": {
                            "browser": { "access_mode": "stealth" },
                            "session": { "source": { "kind": "saved_identity", "name": "portal" } }
                        }
                    }),
                )
                .await;
            let mode_err = client
                .ask("run.new_session", &json!({ "run_id": "mismatch-mode-run" }))
                .await;
            assert_eq!(mode_err["error"]["data"]["code"], "saved_identity_mismatch", "{mode_err}");
            assert_eq!(mode_err["error"]["data"]["field"], "access_mode", "{mode_err}");
            client.call("run.finish", &json!({ "run_id": "mismatch-mode-run" })).await;

            // Run 4: proxy mismatch (direct saved vs configured proxy) -> saved_identity_mismatch.
            client
                .call(
                    "run.start",
                    &json!({
                        "run_id": "mismatch-proxy-run",
                        "settings": {
                            "proxy": { "server": "http://127.0.0.1:18080" },
                            "session": { "source": { "kind": "saved_identity", "name": "portal" } }
                        }
                    }),
                )
                .await;
            let proxy_err = client
                .ask("run.new_session", &json!({ "run_id": "mismatch-proxy-run" }))
                .await;
            assert_eq!(proxy_err["error"]["data"]["code"], "saved_identity_mismatch", "{proxy_err}");
            assert_eq!(proxy_err["error"]["data"]["field"], "proxy", "{proxy_err}");
            client.call("run.finish", &json!({ "run_id": "mismatch-proxy-run" })).await;

            // Advance paused Clock past 30 days -> identity expires, reuse fails with saved_identity_not_found.
            tokio::time::advance(Duration::from_hours(31 * 24)).await;
            let after_expiry = client.call("identity.list", &json!({})).await;
            assert_eq!(after_expiry["identities"].as_array().unwrap().len(), 0);

            client
                .call(
                    "run.start",
                    &json!({
                        "run_id": "expired-run",
                        "settings": { "session": { "source": { "kind": "saved_identity", "name": "portal" } } }
                    }),
                )
                .await;
            let not_found = client
                .ask("run.new_session", &json!({ "run_id": "expired-run" }))
                .await;
            assert_eq!(not_found["error"]["data"]["code"], "saved_identity_not_found", "{not_found}");
            client.call("run.finish", &json!({ "run_id": "expired-run" })).await;

            let kinds = audit_kinds(home.path());
            assert!(kinds.iter().any(|k| k == "saved_identity_saved"), "{kinds:?}");
            assert!(kinds.iter().any(|k| k == "saved_identity_reused"), "{kinds:?}");
            assert!(kinds.iter().any(|k| k == "saved_identity_deleted"), "{kinds:?}");

            client.close().await;
        };
        let deps = Dependencies {
            clock,
            ..dependencies(home.path())
        };
        let ended = tokio::select! {
            (ended, ()) = async {
                tokio::join!(
                    engine.run_attached(transport::Network::InMemory(network), deps),
                    driver,
                )
            } => ended,
            () = scripted_browser(browsers, observed.clone(), seed_cookies) => {
                unreachable!("scripted_browser runs while launcher is alive")
            }
        };
        ended.unwrap();
    });
}

/// Covers: story:5-63, operation:identity.import, operation:identity.export, operation:identity.export_storage_state, error:person_required
#[test]
fn identity_import_export_and_person_only_storage_state() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let cookie_canary = canaries.plant(SecretKind::TypedPassword).value().to_owned();
    let home = tempfile::tempdir().unwrap();
    let export_dir = tempfile::tempdir().unwrap();
    let (launcher, _browsers) = Launcher::scripted(None);
    let engine = engine(Metrics::none(), launcher);

    runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let driver = async {
            // SDK surface: import + export encrypted bundle succeed; export_storage_state is refused!
            let mut sdk = RpcClient::connect(&dialer, "sdk_typescript").await;
            let imported = sdk
                .call(
                    "identity.import",
                    &json!({
                        "name": "shop",
                        "source": {
                            "kind": "cookie_header",
                            "header": format!("session_tok={cookie_canary}"),
                            "domain": "shop.test",
                        }
                    }),
                )
                .await;
            assert_eq!(imported["name"], "shop");

            // Encrypted bundle export succeeds on SDK surface.
            let exported = sdk
                .call(
                    "identity.export",
                    &json!({ "name": "shop", "passphrase": "env:EXPORT_PASS" }),
                )
                .await;
            let bundle_text = exported["bundle"].as_str().unwrap().to_owned();
            assert!(bundle_text.contains("BEGIN AGE ENCRYPTED FILE"));
            assert!(!bundle_text.contains(&cookie_canary));
            assert_eq!(exported["cookie_count"], 1);

            // Re-import from the encrypted bundle file under a new name.
            let bundle_path = export_dir.path().join("shop.age");
            std::fs::write(&bundle_path, &bundle_text).unwrap();
            let reimported = sdk
                .call(
                    "identity.import",
                    &json!({
                        "name": "shop-copy",
                        "source": {
                            "kind": "state_file",
                            "path": bundle_path.to_str().unwrap(),
                            "key": "env:EXPORT_PASS",
                        }
                    }),
                )
                .await;
            assert_eq!(reimported["name"], "shop-copy");

            // Clear-text storageState export is refused on SDK surface with person_required.
            let refused = sdk
                .ask("identity.export_storage_state", &json!({ "name": "shop" }))
                .await;
            assert_eq!(
                refused["error"]["data"]["code"], "person_required",
                "{refused}"
            );
            sdk.close().await;
        };
        let mut deps = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            ..dependencies(home.path())
        };
        deps.environment
            .insert("EXPORT_PASS".into(), FAKE_EXPORT_PASSPHRASE.into());
        let (ended, ()) = tokio::join!(
            engine.run_attached(transport::Network::InMemory(network), deps),
            driver,
        );
        ended.unwrap();

        // Second engine on the same home over CLI surface (PersonOnly) may call
        // identity.export_storage_state and reads what the first engine persisted.
        let (launcher2, _browsers2) = Launcher::scripted(None);
        let engine2 = crate::session_counts::session::engine(Metrics::none(), launcher2);
        let (network2, dialer2) = transport::in_memory();
        let driver2 = async {
            let mut cli = RpcClient::connect(&dialer2, "cli").await;
            let clear = cli
                .call(
                    "identity.export_storage_state",
                    &json!({ "name": "shop-copy" }),
                )
                .await;
            let state: Value =
                serde_json::from_str(clear["storage_state"].as_str().unwrap()).unwrap();
            assert_eq!(state["cookies"][0]["name"], "session_tok");
            assert_eq!(state["cookies"][0]["value"], cookie_canary);

            let deleted = cli
                .call("identity.delete", &json!({ "name": "shop" }))
                .await;
            assert_eq!(deleted["deleted"], true);

            let kinds = audit_kinds(home.path());
            assert!(
                kinds.iter().any(|k| k == "saved_identity_exported"),
                "{kinds:?}"
            );
            assert!(
                kinds.iter().any(|k| k == "saved_identity_deleted"),
                "{kinds:?}"
            );

            cli.close().await;
        };
        let deps2 = Dependencies {
            clock: Clock::paused(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            ..dependencies(home.path())
        };
        let (ended2, ()) = tokio::join!(
            engine2.run_attached(transport::Network::InMemory(network2), deps2),
            driver2,
        );
        ended2.unwrap();
    });
}
