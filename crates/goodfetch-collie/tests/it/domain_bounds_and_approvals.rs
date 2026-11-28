//! Source domain bounds on top-level navigations and new tabs, registrable
//! domain validation, Human pause approvals for model and Recipe login actions
//! (ignoring `session.recovery.human_pause: false`), and Audit log coverage for
//! import, borrow, save, reuse, and deletion (ADRs 0056, 0092 and 0117;
//! ticket #745).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "integration tests prepare Profile directories, start a local Chrome and HTTP server, and run concurrent Daemon clients"
)]
#![expect(
    clippy::too_many_lines,
    reason = "end-to-end ADR 0056 tests verify bound sources, model/Recipe approvals, decline/approve paths, and Audit log records together"
)]

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::{Attached, DaemonClient, Hello};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, DaemonEndpoint, Process, RuntimeFile, TestEnv, browser_cache,
    chrome_headless, cli, reported_debugging_port,
};
use serde_json::{Value, json};

use crate::COLLIE;
use crate::daemon::{StopsTheDaemon, runtime_file, short_idle};
use crate::page::{BROWSER_DEADLINE, opened};

const PUBLIC: &str = "93.184.215.14";
const SITE: &str = "site.test";
const OTHER: &str = "other.test";
const THIRD: &str = "third.test";

/// Stand-in HTTP site recording every `(host, path)` requested.
struct Site {
    port: u16,
    hits: Arc<Mutex<Vec<(String, String)>>>,
}

impl Site {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&hits);
        std::thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let recorded = Arc::clone(&recorded);
                std::thread::spawn(move || serve_http(stream, port, &recorded));
            }
        });
        Self { port, hits }
    }

    fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }

    fn hosts(&self) -> Vec<String> {
        self.hits
            .lock()
            .unwrap()
            .iter()
            .map(|(h, _)| h.clone())
            .collect()
    }
}

fn serve_http(mut stream: TcpStream, port: u16, recorded: &Mutex<Vec<(String, String)>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut head = Vec::new();
    let mut buf = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&head);
    let host = text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("host").then(|| value.trim())
        })
        .unwrap_or_default();
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h).to_owned();
    let path = text.split(' ').nth(1).unwrap_or("/").to_owned();
    recorded.lock().unwrap().push((host.clone(), path.clone()));

    let body = if path == "/with-frame" {
        format!(
            "<!doctype html><html><body><h1>Allowed Page</h1><iframe src=\"http://{OTHER}:{port}/subframe\"></iframe></body></html>"
        )
    } else if path == "/subframe" {
        "<!doctype html><html><body><p>Subframe loaded</p></body></html>".to_owned()
    } else {
        format!("<!doctype html><html><body><h1>{host}{path}</h1></body></html>")
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// A real Chrome for Testing instance standing in as a running / `cdp_url` browser.
struct RunningChrome {
    process: Process,
    port: u16,
    ws_path: String,
    cdp_url: String,
}

impl RunningChrome {
    fn start(env: &TestEnv, cache: &Path, name: &str) -> Self {
        let profile = env.folder().join(name);
        let mut command = env.process_command(chrome_headless(cache));
        command
            .args(chromium_background_traffic_off())
            .args([
                "--headless",
                "--no-first-run",
                "--use-mock-keychain",
                "--password-store=basic",
                "--remote-debugging-port=0",
                "--host-resolver-rules=MAP * 127.0.0.1",
            ])
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("about:blank")
            .stderr(Stdio::null());
        let process = env.spawn(&mut command).unwrap();
        let port = reported_debugging_port(&profile);
        let file = std::fs::read_to_string(profile.join("DevToolsActivePort")).unwrap();
        let ws_path = file.lines().nth(1).unwrap().trim().to_owned();
        let cdp_url = format!("ws://127.0.0.1:{port}{ws_path}");
        Self {
            process,
            port,
            ws_path,
            cdp_url,
        }
    }

    fn stop(self) {
        self.process.interrupt().unwrap();
        let _ = self.process.child().wait();
    }
}

struct Program {
    engine: Attached,
    next_id: i64,
}

impl Program {
    fn start(env: &TestEnv, cache: &Path, surface: Surface) -> Self {
        let mut engine = Attached::start_with(env, COLLIE, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
        });
        engine.send(&Hello::needing(Product::Collie).surface(surface).message());
        let _ = engine.next();
        Self { engine, next_id: 0 }
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        self.engine.ask(self.next_id, method, params)
    }

    fn finish(mut self, run_id: &str) {
        let _ = self.call("run.finish", &json!({ "run_id": run_id }));
        let _ = self.engine.finish();
    }
}

fn error_code(answer: &Value) -> &str {
    answer["error"]["data"]["code"].as_str().unwrap_or("")
}

/// Starts a real Collie Daemon in `env` with `cache` and returns its endpoint
/// and bearer token.
fn start_daemon<'a>(
    env: &'a TestEnv,
    cache: &Path,
) -> (StopsTheDaemon<'a>, DaemonEndpoint, String) {
    short_idle(env, 60_000);
    let stops = StopsTheDaemon::of(env, COLLIE);
    let output = cli(env, COLLIE)
        .env(BROWSERS_PATH_VARIABLE, cache)
        .env("TEST_EXPORT_PASS", "approval-export-passphrase")
        .args(["daemon", "start", "--json"])
        .run()
        .unwrap();
    assert_eq!(output.exit, Some(0), "{}", output.stderr);
    let found = RuntimeFile::read(&runtime_file(env)).unwrap();
    (stops, found.endpoint, found.token)
}

/// Polls `needs_you.list` on a person's `DaemonClient` until an
/// `approval_requested` item matching `predicate` appears.
fn wait_for_approval_item(
    person: &mut DaemonClient,
    next_id: &mut i64,
    label: &str,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    let start = Instant::now();
    let mut last = Value::Null;
    while start.elapsed() < Duration::from_secs(30) {
        *next_id += 1;
        let listed = person.call(
            *next_id,
            "needs_you.list",
            &json!({ "kinds": ["approval_requested"] }),
        );
        last = listed.clone();
        if let Some(items) = listed["result"]["items"].as_array()
            && let Some(found) = items.iter().find(|item| predicate(item))
        {
            return found.clone();
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("timed out waiting for approval_requested item ({label}); last list: {last}");
}

/// For the person's own code:
/// - `www.example.com` in `session.source.domains` (and `identity.import`) is
///   refused with `setting_invalid` naming the registrable domain `example.com`.
/// - Each bound source (`running_browser`, `cdp_url` naming `domains`, and
///   `profile_directory` used in place) allows top-level navigation to its
///   registrable domain (and subframes from other domains follow their page),
///   while top-level navigations and new tabs to an unlisted domain fail at
///   step 3 with `domain_not_allowed` without sending any request there.
/// - Every import, borrow, save, reuse, and deletion appears in the Audit log
///   with its caller actor.
///
/// Covers: story:5-72, story:5-73, error:domain_not_allowed, error:setting_invalid, operation:run.new_session, operation:page.goto, operation:identity.import, operation:identity.delete, operation:session.save_identity, operation:audit.export
#[test]
fn bound_sources_refuse_unlisted_domains_validate_registrable_domains_and_audit_login_lifecycle() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let chrome = RunningChrome::start(&env, &cache, "bound-running-chrome");
    let site = Site::start();

    // Prepared Profile directory for `running_browser` (`DevToolsActivePort`).
    let watched_profile = env.folder().join("running-browser-dir");
    std::fs::create_dir_all(&watched_profile).unwrap();
    std::fs::write(
        watched_profile.join("DevToolsActivePort"),
        format!("{}\n{}\n", chrome.port, chrome.ws_path),
    )
    .unwrap();

    // Prepared closed Profile directory for `profile_directory` used in place.
    let in_place_profile = env.folder().join("in-place-profile");
    std::fs::create_dir_all(in_place_profile.join("Default")).unwrap();
    std::fs::write(in_place_profile.join("Local State"), "{}").unwrap();
    std::fs::write(
        in_place_profile.join("Default").join("Preferences"),
        json!({ "profile": { "name": "Test" } }).to_string(),
    )
    .unwrap();

    let mut program = Program::start(&env, &cache, Surface::SdkTypescript);

    // 1. `www.example.com` in `domains` is `setting_invalid` naming `example.com`
    //    both on `run.new_session` and on `identity.import`.
    let started = program.call(
        "run.start",
        &json!({
            "run_id": "bounds-own",
            "settings": {
                "egress": {
                    "private_ranges": ["127.0.0.0/8"],
                },
                "network": {
                    "resolve": {
                        SITE: ["127.0.0.1"],
                        format!("app.{SITE}"): ["127.0.0.1"],
                        OTHER: ["127.0.0.1"],
                        THIRD: ["127.0.0.1"],
                    }
                }
            }
        }),
    );
    assert!(started.get("result").is_some(), "{started}");

    let bad_session_domain = program.call(
        "run.new_session",
        &json!({
            "run_id": "bounds-own",
            "source": {
                "kind": "running_browser",
                "browser": "chrome",
                "domains": ["www.example.com"],
                "path": watched_profile.display().to_string(),
            }
        }),
    );
    assert_eq!(
        error_code(&bad_session_domain),
        "setting_invalid",
        "{bad_session_domain}"
    );
    assert!(
        bad_session_domain["error"]["data"]["fix"]
            .as_str()
            .unwrap_or("")
            .contains("example.com"),
        "{bad_session_domain}"
    );

    let bad_import_domain = program.call(
        "identity.import",
        &json!({
            "name": "bad-domain-id",
            "source": {
                "kind": "state_file",
                "path": env.folder().join("missing.json").display().to_string(),
                "domains": ["www.example.com"],
            }
        }),
    );
    assert_eq!(
        error_code(&bad_import_domain),
        "setting_invalid",
        "{bad_import_domain}"
    );
    assert!(
        bad_import_domain["error"]["data"]["fix"]
            .as_str()
            .unwrap_or("")
            .contains("example.com"),
        "{bad_import_domain}"
    );

    // 2. Check all three bound sources for the person's own code:
    //    `running_browser`, `cdp_url` naming `domains`, and `profile_directory`.
    let sources: [(&str, Value); 3] = [
        (
            "running_browser",
            json!({
                "kind": "running_browser",
                "browser": "chrome",
                "domains": [SITE],
                "path": watched_profile.display().to_string(),
            }),
        ),
        (
            "cdp_url",
            json!({
                "kind": "cdp_url",
                "url": chrome.cdp_url,
                "domains": [SITE],
            }),
        ),
        (
            "profile_directory",
            json!({
                "kind": "profile_directory",
                "path": in_place_profile.display().to_string(),
                "browser": "chrome",
                "domains": [SITE],
            }),
        ),
    ];

    for (kind, source) in sources {
        let sess = program.call(
            "run.new_session",
            &json!({ "run_id": "bounds-own", "source": source }),
        );
        assert!(sess.get("result").is_some(), "{kind}: {sess}");
        let session_id = sess["result"]["session_id"].clone();

        // Allowed domain + subframe to another domain succeeds.
        let p1 = opened(&program.call("session.new_page", &json!({ "session_id": session_id })))
            .page_id
            .0;
        let ok_nav = program.call(
            "page.goto",
            &json!({ "page_id": p1, "url": site.url(SITE, "/with-frame") }),
        );
        assert_eq!(
            ok_nav["result"]["response"]["status"],
            json!(200),
            "{kind}: {ok_nav}"
        );

        // Top-level navigation to unlisted domain fails with `domain_not_allowed`.
        let refused_nav = program.call(
            "page.goto",
            &json!({ "page_id": p1, "url": site.url(THIRD, "/") }),
        );
        assert_eq!(
            error_code(&refused_nav),
            "domain_not_allowed",
            "{kind}: {refused_nav}"
        );
        assert_eq!(
            refused_nav["error"]["data"]["domain"],
            json!(THIRD),
            "{kind}: {refused_nav}"
        );
        assert_eq!(
            refused_nav["error"]["data"]["source"],
            json!(kind),
            "{kind}: {refused_nav}"
        );

        // New tab navigating to unlisted domain also fails with `domain_not_allowed`.
        let p2 = opened(&program.call("session.new_page", &json!({ "session_id": session_id })))
            .page_id
            .0;
        let refused_tab = program.call(
            "page.goto",
            &json!({ "page_id": p2, "url": site.url(THIRD, "/tab") }),
        );
        assert_eq!(
            error_code(&refused_tab),
            "domain_not_allowed",
            "{kind} new tab: {refused_tab}"
        );

        let _ = program.call("session.close", &json!({ "session_id": session_id }));
    }

    assert!(
        !site.hosts().iter().any(|h| h == THIRD),
        "unlisted domain `{THIRD}` must never receive a top-level request: {:?}",
        site.hosts()
    );

    // 3. Exercise import (`cookie_header`), save, reuse (`saved_identity`), and
    //    delete, then verify every login event appears in the Audit log with its actor.
    let imported_sess = program.call(
        "run.new_session",
        &json!({
            "run_id": "bounds-own",
            "source": {
                "kind": "cookie_header",
                "header": "sid=abc123",
                "domain": SITE,
            }
        }),
    );
    assert!(imported_sess.get("result").is_some(), "{imported_sess}");
    let imported_sid = imported_sess["result"]["session_id"].clone();

    let saved = program.call(
        "session.save_identity",
        &json!({ "session_id": imported_sid, "name": "audit-login" }),
    );
    assert!(saved.get("result").is_some(), "{saved}");

    let reused_sess = program.call(
        "run.new_session",
        &json!({
            "run_id": "bounds-own",
            "source": { "kind": "saved_identity", "name": "audit-login" }
        }),
    );
    assert!(reused_sess.get("result").is_some(), "{reused_sess}");

    let deleted = program.call("identity.delete", &json!({ "name": "audit-login" }));
    assert_eq!(deleted["result"]["deleted"], json!(true), "{deleted}");

    let exported_audit = program.call("audit.export", &json!({}));
    let entries = exported_audit["result"]["entries"].as_array().unwrap();
    let has_kind_with_actor = |kind: &str| {
        entries.iter().any(|e| {
            e["kind"] == json!(kind)
                && e["actor"]["kind"] == json!("client_surface")
                && e["actor"]["surface"] == json!("sdk_typescript")
        })
    };
    assert!(
        has_kind_with_actor("login_borrowed"),
        "expected login_borrowed in Audit log: {entries:#?}"
    );
    assert!(
        has_kind_with_actor("login_imported"),
        "expected login_imported in Audit log: {entries:#?}"
    );
    assert!(
        has_kind_with_actor("saved_identity_saved"),
        "expected saved_identity_saved in Audit log: {entries:#?}"
    );
    assert!(
        has_kind_with_actor("saved_identity_reused"),
        "expected saved_identity_reused in Audit log: {entries:#?}"
    );
    assert!(
        has_kind_with_actor("saved_identity_deleted"),
        "expected saved_identity_deleted in Audit log: {entries:#?}"
    );

    program.finish("bounds-own");
    chrome.stop();
}

/// For a Client acting for a model (`Surface::Mcp`) and for a Recipe run
/// (`goodfetch.recipe:<name>`), even with `session.recovery.human_pause: false`:
/// - Login import, borrow, save, deletion, and export wait in a Human pause
///   listing what would happen until a person approves or declines.
/// - Navigating a bound source to an unlisted domain raises a Human pause
///   (never `domain_not_allowed` immediately); approving widens the Session's
///   domains and completes the navigation, while declining fails with
///   `human_pause_declined` without touching the site.
/// - A model trying to call `needs_you.answer_approval` is refused with
///   `person_required`.
///
/// Covers: story:5-72, story:5-73, operation:needs_you.answer_approval, operation:needs_you.list, error:human_pause_declined, error:person_required, setting:session.recovery.human_pause
#[test]
fn model_and_recipe_login_actions_and_unlisted_navigations_wait_for_person_approval() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let chrome = RunningChrome::start(&env, &cache, "approval-running-chrome");
    let site = Site::start();

    let watched_profile = env.folder().join("approval-running-dir");
    std::fs::create_dir_all(&watched_profile).unwrap();
    std::fs::write(
        watched_profile.join("DevToolsActivePort"),
        format!("{}\n{}\n", chrome.port, chrome.ws_path),
    )
    .unwrap();

    let (_stops, endpoint, token) = start_daemon(&env, &cache);
    let mut person = DaemonClient::opened(
        &endpoint,
        &Hello::needing(Product::Collie)
            .token(&token)
            .surface(Surface::Cli),
    );
    let mut person_id: i64 = 100;

    // --- Part A: Client acting for a model (`Surface::Mcp`) ---
    let ep_model = endpoint.clone();
    let token_model = token.clone();
    let profile_path = watched_profile.display().to_string();
    let site_url_allowed = site.url(SITE, "/");
    let site_url_other = site.url(OTHER, "/approved");
    let site_url_third = site.url(THIRD, "/declined");

    let model_thread = std::thread::spawn(move || {
        let mut model = DaemonClient::opened(
            &ep_model,
            &Hello::needing(Product::Collie)
                .token(&token_model)
                .surface(Surface::Mcp),
        );
        let mut id: i64 = 1;
        let mut ask = |method: &str, params: Value| -> Value {
            id += 1;
            model.call(id, method, &params)
        };

        // A model cannot answer its own approvals (`person_required`).
        let self_approve = ask(
            "needs_you.answer_approval",
            json!({ "id": "approval-999", "approve": true }),
        );
        assert_eq!(
            error_code(&self_approve),
            "person_required",
            "{self_approve}"
        );

        // Start a Run with `session.recovery.human_pause: false`: approvals MUST still wait!
        let started = ask(
            "run.start",
            json!({
                "run_id": "model-approvals",
                "settings": {
                    "session": { "recovery": { "human_pause": false } },
                    "network": {
                        "resolve": {
                            SITE: [PUBLIC],
                            OTHER: [PUBLIC],
                            THIRD: [PUBLIC],
                        }
                    }
                }
            }),
        );
        assert!(started.get("result").is_some(), "{started}");

        // 1. Model borrows `running_browser` -> waits for person approval -> approved.
        let borrowed = ask(
            "run.new_session",
            json!({
                "run_id": "model-approvals",
                "source": {
                    "kind": "running_browser",
                    "browser": "chrome",
                    "domains": [SITE],
                    "path": profile_path,
                }
            }),
        );
        assert!(borrowed.get("result").is_some(), "{borrowed}");
        let borrowed_sid = borrowed["result"]["session_id"].clone();
        let p_borrow = opened(&ask(
            "session.new_page",
            json!({ "session_id": borrowed_sid }),
        ))
        .page_id
        .0;
        let ok = ask(
            "page.goto",
            json!({ "page_id": p_borrow, "url": site_url_allowed }),
        );
        assert_eq!(ok["result"]["response"]["status"], json!(200), "{ok}");

        // 2. Unlisted domain navigation on model's bound source -> Human pause -> approved.
        let nav_approved = ask(
            "page.goto",
            json!({ "page_id": p_borrow, "url": site_url_other }),
        );
        assert_eq!(
            nav_approved["result"]["response"]["status"],
            json!(200),
            "{nav_approved}"
        );

        // 3. Another unlisted domain -> Human pause -> declined.
        let nav_declined = ask(
            "page.goto",
            json!({ "page_id": p_borrow, "url": site_url_third }),
        );
        assert_eq!(
            error_code(&nav_declined),
            "human_pause_declined",
            "{nav_declined}"
        );

        // 4. Model imports via `identity.import` -> waits -> approved.
        let imported = ask(
            "identity.import",
            json!({
                "name": "model-id",
                "source": {
                    "kind": "cookie_header",
                    "header": "tok=secret1",
                    "domain": SITE,
                }
            }),
        );
        assert!(imported.get("result").is_some(), "{imported}");

        // 5. Model exports via `identity.export` -> waits -> approved.
        let exported = ask(
            "identity.export",
            json!({
                "name": "model-id",
                "public_key": "age1ql3z7hjy54pw3hyww5ayyfg7zqgvc7w3j2elw8zmrj2kg5sfn9aqmcac8p",
            }),
        );
        assert!(exported.get("result").is_some(), "{exported}");

        // 6. Model deletes via `identity.delete` -> waits -> declined first, then approved.
        let del_declined = ask("identity.delete", json!({ "name": "model-id" }));
        assert_eq!(
            error_code(&del_declined),
            "human_pause_declined",
            "{del_declined}"
        );
        let del_approved = ask("identity.delete", json!({ "name": "model-id" }));
        assert_eq!(
            del_approved["result"]["deleted"],
            json!(true),
            "{del_approved}"
        );

        // 7. Model saves a fresh Session -> waits -> approved.
        let fresh_sess = ask("run.new_session", json!({ "run_id": "model-approvals" }));
        let fresh_sid = fresh_sess["result"]["session_id"].clone();
        let saved = ask(
            "session.save_identity",
            json!({ "session_id": fresh_sid, "name": "model-saved" }),
        );
        assert!(saved.get("result").is_some(), "{saved}");

        let _ = ask("run.finish", json!({ "run_id": "model-approvals" }));
    });

    // Person answers each model approval in sequence, verifying each lists what would happen.
    let mut seen_ids = std::collections::BTreeSet::<String>::new();
    let answer = |person: &mut DaemonClient,
                  pid: &mut i64,
                  seen: &mut std::collections::BTreeSet<String>,
                  action: &str,
                  approve: bool,
                  note: Option<&str>| {
        let item = wait_for_approval_item(person, pid, action, |it| {
            it["details"]["action"] == json!(action)
                && it["id"].as_str().is_some_and(|id| !seen.contains(id))
        });
        assert!(
            !item["summary"].as_str().unwrap_or("").is_empty(),
            "approval item must describe what would happen: {item}"
        );
        let item_id = item["id"].as_str().unwrap().to_owned();
        seen.insert(item_id.clone());
        *pid += 1;
        let mut params = json!({
            "id": item_id,
            "approve": approve,
        });
        if let Some(n) = note {
            params["note"] = json!(n);
        }
        let ans = person.call(*pid, "needs_you.answer_approval", &params);
        assert_eq!(ans["result"]["accepted"], json!(true), "{ans}");
        item
    };

    // 1. Borrow running_browser -> approve
    let borrow_item = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "borrow_login",
        true,
        None,
    );
    assert!(
        borrow_item["summary"]
            .as_str()
            .unwrap()
            .contains("running_browser")
    );
    // 2. Unlisted domain (OTHER) -> approve
    let nav1 = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "navigate_unlisted_domain",
        true,
        None,
    );
    assert!(nav1["summary"].as_str().unwrap().contains(OTHER));
    // 3. Unlisted domain (THIRD) -> decline
    let nav2 = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "navigate_unlisted_domain",
        false,
        Some("not allowed"),
    );
    assert!(nav2["summary"].as_str().unwrap().contains(THIRD));
    // 4. identity.import -> approve
    let imp = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "import_login",
        true,
        None,
    );
    assert!(imp["summary"].as_str().unwrap().contains("model-id"));
    // 5. identity.export -> approve
    let exp = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "export_identity",
        true,
        None,
    );
    assert!(exp["summary"].as_str().unwrap().contains("model-id"));
    // 6a. identity.delete -> decline
    let _ = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "delete_identity",
        false,
        Some("keep it"),
    );
    // 6b. identity.delete -> approve
    let _ = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "delete_identity",
        true,
        None,
    );
    // 7. session.save_identity -> approve
    let sav = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "save_identity",
        true,
        None,
    );
    assert!(sav["summary"].as_str().unwrap().contains("model-saved"));

    model_thread.join().unwrap();
    assert!(
        !site.hosts().iter().any(|h| h == THIRD),
        "declined unlisted domain `{THIRD}` must not receive a request"
    );

    // --- Part B: Recipe run (`tags: ["goodfetch.recipe:nightly"]`) ---
    let ep_recipe = endpoint.clone();
    let token_recipe = token.clone();
    let recipe_profile = watched_profile.display().to_string();
    let recipe_other_url = site.url(OTHER, "/recipe-unlisted");
    let recipe_thread = std::thread::spawn(move || {
        let mut client = DaemonClient::opened(
            &ep_recipe,
            &Hello::needing(Product::Collie)
                .token(&token_recipe)
                .surface(Surface::SdkTypescript),
        );
        let mut id: i64 = 1;
        let mut ask = |method: &str, params: Value| -> Value {
            id += 1;
            client.call(id, method, &params)
        };

        let started = ask(
            "run.start",
            json!({
                "run_id": "recipe-approvals",
                "tags": ["goodfetch.recipe:nightly"],
                "settings": {
                    "session": { "recovery": { "human_pause": false } },
                    "network": {
                        "resolve": {
                            SITE: [PUBLIC],
                            OTHER: [PUBLIC],
                        }
                    }
                }
            }),
        );
        assert!(started.get("result").is_some(), "{started}");

        // Recipe borrows running_browser -> waits for person even with human_pause: false!
        let sess = ask(
            "run.new_session",
            json!({
                "run_id": "recipe-approvals",
                "source": {
                    "kind": "running_browser",
                    "browser": "chrome",
                    "domains": [SITE],
                    "path": recipe_profile,
                }
            }),
        );
        assert!(sess.get("result").is_some(), "{sess}");
        let sid = sess["result"]["session_id"].clone();
        let pid = opened(&ask("session.new_page", json!({ "session_id": sid })))
            .page_id
            .0;

        // Recipe navigates to unlisted domain -> waits in Human pause -> approved via page.pause_answer!
        let nav = ask(
            "page.goto",
            json!({ "page_id": pid, "url": recipe_other_url }),
        );
        assert_eq!(nav["result"]["response"]["status"], json!(200), "{nav}");

        // Recipe saves a fresh Session -> waits -> approved.
        let fresh = ask("run.new_session", json!({ "run_id": "recipe-approvals" }));
        let fresh_sid = fresh["result"]["session_id"].clone();
        let saved = ask(
            "session.save_identity",
            json!({ "session_id": fresh_sid, "name": "recipe-saved" }),
        );
        assert!(saved.get("result").is_some(), "{saved}");

        let _ = ask("run.finish", json!({ "run_id": "recipe-approvals" }));
    });

    // 1. Recipe borrow -> approve
    let _ = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "borrow_login",
        true,
        None,
    );
    // 2. Recipe unlisted domain -> verify visible in `needs_you.list` AND answer via `page.pause_answer`
    let recipe_nav = wait_for_approval_item(
        &mut person,
        &mut person_id,
        "recipe navigate_unlisted_domain",
        |it| {
            it["details"]["action"] == json!("navigate_unlisted_domain")
                && it["id"].as_str().is_some_and(|id| !seen_ids.contains(id))
        },
    );
    seen_ids.insert(recipe_nav["id"].as_str().unwrap().to_owned());
    let paused_page_id = recipe_nav["details"]["page_id"]
        .as_str()
        .unwrap()
        .to_owned();
    person_id += 1;
    let answered = person.call(
        person_id,
        "page.pause_answer",
        &json!({ "page_id": paused_page_id, "answer": "done" }),
    );
    assert_eq!(answered["result"]["accepted"], json!(true), "{answered}");
    // 3. Recipe save_identity -> approve
    let _ = answer(
        &mut person,
        &mut person_id,
        &mut seen_ids,
        "save_identity",
        true,
        None,
    );

    recipe_thread.join().unwrap();

    // Verify Audit log recorded human_pause_ended with person actor too.
    person_id += 1;
    let audit = person.call(person_id, "audit.export", &json!({}));
    let entries = audit["result"]["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == json!("human_pause_ended")
                && e["actor"]["kind"] == json!("person")),
        "expected person-approved human_pause_ended in Audit log: {entries:#?}"
    );

    chrome.stop();
}
