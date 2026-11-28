//! Borrowing the person's running Chrome or Edge through Chrome's consented
//! remote debugging (`session.source = { kind: "running_browser" }`, ADR 0056;
//! SA-08; ticket #742) against Collie's executable with prepared Profile
//! directories and a real/scripted Chrome.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "integration tests prepare Profile directories, start a local browser and TCP proxy on their own threads, and read the Run trace"
)]
#![expect(
    clippy::too_many_lines,
    reason = "end-to-end running_browser scenarios verify file watching, single connection per Run, shared-browser protections, and drop handling together"
)]

use std::io::{Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_client::{Attached, Hello, HomeLayout};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, Process, TestEnv, browser_cache, chrome_headless,
    reported_debugging_port,
};
use serde_json::{Value, json};

use crate::COLLIE;
use crate::page::{BROWSER_DEADLINE, opened};
use crate::page_ends::call_in_flight;
use crate::page_events::Exchange;

const PUBLIC: &str = "93.184.215.14";
const SITE: &str = "site.test";

/// A Chrome for Testing instance acting as the person's running browser.
struct RunningChrome {
    process: Process,
    port: u16,
    ws_path: String,
    cdp_url: String,
}

impl RunningChrome {
    fn start(env: &TestEnv, cache: &Path) -> Self {
        let profile = env.folder().join("running-chrome-real-profile");
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

/// A TCP proxy in front of `RunningChrome` that counts connections and allows
/// dropping active connections on demand without closing Chrome.
struct CountingProxy {
    port: u16,
    connections: Arc<AtomicUsize>,
    active: Arc<Mutex<Vec<TcpStream>>>,
    stop: Arc<AtomicBool>,
}

impl CountingProxy {
    fn start(upstream_port: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let connections = Arc::new(AtomicUsize::new(0));
        let active: Arc<Mutex<Vec<TcpStream>>> = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));

        let c_count = Arc::clone(&connections);
        let c_active = Arc::clone(&active);
        let c_stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !c_stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((client, _)) => {
                        let _ = client.set_nonblocking(false);
                        c_count.fetch_add(1, Ordering::SeqCst);
                        let Ok(upstream) = TcpStream::connect(("127.0.0.1", upstream_port)) else {
                            continue;
                        };
                        if let Ok(c_dup) = client.try_clone() {
                            c_active.lock().unwrap().push(c_dup);
                        }
                        if let Ok(u_dup) = upstream.try_clone() {
                            c_active.lock().unwrap().push(u_dup);
                        }
                        let mut c_read = client.try_clone().unwrap();
                        let mut u_write = upstream.try_clone().unwrap();
                        std::thread::spawn(move || {
                            let _ = std::io::copy(&mut c_read, &mut u_write);
                            let _ = u_write.shutdown(Shutdown::Write);
                        });
                        let mut u_read = upstream;
                        let mut c_write = client;
                        std::thread::spawn(move || {
                            let _ = std::io::copy(&mut u_read, &mut c_write);
                            let _ = c_write.shutdown(Shutdown::Write);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            connections,
            active,
            stop,
        }
    }

    fn connection_count(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    fn drop_connections(&self) {
        let streams = std::mem::take(&mut *self.active.lock().unwrap());
        for stream in streams {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

impl Drop for CountingProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.drop_connections();
    }
}

/// A scripted WebSocket server that completes the CDP WebSocket upgrade and
/// immediately drops the connection as soon as the engine sends its first
/// opening command (`Browser.getVersion`).
fn start_drop_on_open_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept()
            && let Ok(mut ws) = tungstenite::accept(stream)
        {
            let _ = ws.read();
            drop(ws);
        }
    });
    port
}

/// Stand-in HTTP site recording every request `Host` header.
struct Site {
    port: u16,
    hosts: Arc<Mutex<Vec<String>>>,
}

impl Site {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hosts = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&hosts);
        std::thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let recorded = Arc::clone(&recorded);
                std::thread::spawn(move || serve_http(stream, &recorded));
            }
        });
        Self { port, hosts }
    }

    fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }

    fn hosts(&self) -> Vec<String> {
        self.hosts.lock().unwrap().clone()
    }
}

fn serve_http(mut stream: TcpStream, recorded: &Mutex<Vec<String>>) {
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
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    recorded.lock().unwrap().push(host.to_owned());

    let path = text.split(' ').nth(1).unwrap_or("/");
    let body = match path {
        "/person" => "<!doctype html><html><body><h1>Person Tab</h1></body></html>",
        "/secret" => {
            "<!doctype html><html><body><form><input id=\"token\" type=\"text\"></form></body></html>"
        }
        _ => "<!doctype html><html><body><h1>Running Browser Site</h1></body></html>",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// Direct CDP client representing the person's own tabs in their browser.
struct Person {
    socket: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
}

impl Person {
    fn connect(cdp_url: &str) -> Self {
        let authority = cdp_url
            .trim_start_matches("ws://")
            .split('/')
            .next()
            .unwrap()
            .to_owned();
        let stream = TcpStream::connect(&authority).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let socket = tungstenite::client(cdp_url, stream).unwrap().0;
        Self { socket, next_id: 0 }
    }

    fn send(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        let cmd = json!({ "id": self.next_id, "method": method, "params": params });
        self.socket
            .send(tungstenite::Message::text(cmd.to_string()))
            .unwrap();
        loop {
            let msg = self.socket.read().unwrap();
            if let tungstenite::Message::Text(text) = msg {
                let parsed: Value = serde_json::from_str(&text).unwrap();
                if parsed["id"] == json!(self.next_id) {
                    assert!(parsed.get("error").is_none(), "{method}: {parsed}");
                    return parsed["result"].clone();
                }
            }
        }
    }

    fn pages(&mut self) -> Vec<Value> {
        self.send("Target.getTargets", &json!({}))["targetInfos"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t["type"] == "page")
            .collect()
    }
}

struct Program {
    engine: Attached,
    next_id: i64,
}

impl Program {
    fn start(env: &TestEnv, cache: &Path, extra_env: &[(&str, &str)]) -> Self {
        let mut engine = Attached::start_with(env, COLLIE, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
            for (k, v) in extra_env {
                command.env(k, v);
            }
        });
        engine.send(
            &Hello::needing(Product::Collie)
                .surface(Surface::SdkTypescript)
                .message(),
        );
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

fn error_code(answer: &Value) -> (&str, &Value) {
    (
        answer["error"]["data"]["code"].as_str().unwrap_or(""),
        &answer["error"]["data"],
    )
}

fn provenance(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let file = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_PROVENANCE);
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Consented `running_browser` borrows a running Chrome once per Run via the
/// OS file watcher on `DevToolsActivePort`, opens its own tabs without
/// touching the person's tab, enforces shared-browser protections (no secrets,
/// unguarded results, private-range egress block + rebinding gap provenance,
/// `session_not_savable` naming `installed_browser`, `setting_not_applicable`
/// for `browser.viewport`), and closes its Session without closing the browser.
///
/// Covers: story:5-69, error:session_not_savable, error:secret_input_refused, error:setting_not_applicable, error:egress_refused, operation:run.new_session, operation:session.save_identity, operation:session.close
#[test]
fn running_browser_borrows_once_via_file_watcher_protects_persons_tab_and_enforces_shared_rules() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let chrome = RunningChrome::start(&env, &cache);
    let proxy = CountingProxy::start(chrome.port);
    let site = Site::start();

    // Open the person's own tab before Collie attaches.
    let mut person = Person::connect(&chrome.cdp_url);
    let person_url = site.url(SITE, "/person");
    let person_target =
        person.send("Target.createTarget", &json!({ "url": person_url }))["targetId"]
            .as_str()
            .unwrap()
            .to_owned();

    // Prepared Profile directory with NO DevToolsActivePort yet: a background
    // thread writes it after `run.new_session` starts so the OS file watcher
    // (not polling) must wake up on the write.
    let watched_profile = env.folder().join("watched-running-chrome");
    std::fs::create_dir_all(&watched_profile).unwrap();

    let mut program = Program::start(&env, &cache, &[("TEST_BORROW_SECRET", "s3cr3t-value")]);
    let started = program.call(
        "run.start",
        &json!({
            "run_id": "running-borrow",
            "settings": {
                "network": {
                    "resolve": {
                        SITE: [PUBLIC],
                        "internal.test": ["10.0.0.42"],
                    }
                },
                "secrets": {
                    "placeholders": {
                        "pw": {
                            "reference": "env:TEST_BORROW_SECRET",
                            "domains": [SITE],
                        }
                    }
                }
            }
        }),
    );
    assert!(started.get("result").is_some(), "{started}");

    let source = json!({
        "kind": "running_browser",
        "browser": "chrome",
        "domains": [SITE],
        "path": watched_profile.display().to_string(),
    });

    // `browser.viewport` where the Borrowed Session opens raises `setting_not_applicable`.
    let viewport_refused = program.call(
        "run.new_session",
        &json!({
            "run_id": "running-borrow",
            "source": source,
            "settings": {
                "browser": {
                    "viewport": { "width": 800, "height": 600 }
                }
            }
        }),
    );
    assert_eq!(
        error_code(&viewport_refused).0,
        "setting_not_applicable",
        "{viewport_refused}"
    );
    assert_eq!(
        viewport_refused["error"]["data"]["key"],
        json!("browser.viewport"),
        "{viewport_refused}"
    );

    // Delayed write of DevToolsActivePort while `run.new_session` waits on the
    // OS file watcher.
    let port_file = watched_profile.join("DevToolsActivePort");
    let proxy_port = proxy.port;
    let ws_path = chrome.ws_path.clone();
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        std::fs::write(&port_file, format!("{proxy_port}\n{ws_path}\n")).unwrap();
    });

    let s1 = program.call(
        "run.new_session",
        &json!({ "run_id": "running-borrow", "source": source }),
    );
    writer.join().unwrap();
    assert!(s1.get("result").is_some(), "{s1}");
    let session1 = s1["result"]["session_id"].clone();

    // A second Session in the same Run reuses the single consented connection.
    let s2 = program.call(
        "run.new_session",
        &json!({ "run_id": "running-borrow", "source": source }),
    );
    assert!(s2.get("result").is_some(), "{s2}");
    let session2 = s2["result"]["session_id"].clone();
    assert_eq!(
        proxy.connection_count(),
        1,
        "running_browser must open one consented connection per Run"
    );

    // Engine opens its own tab, navigates, reads formats (marked unguarded),
    // refuses Secret typing, blocks private ranges, and refuses save_identity.
    let page1 = opened(&program.call("session.new_page", &json!({ "session_id": session1 })))
        .page_id
        .0;
    let went_secret = program.call(
        "page.goto",
        &json!({ "page_id": page1, "url": site.url(SITE, "/secret") }),
    );
    assert_eq!(
        went_secret["result"]["response"]["status"],
        json!(200),
        "{went_secret}"
    );

    let formats = program.call(
        "page.formats",
        &json!({ "page_id": page1, "formats": { "text": true } }),
    );
    assert!(formats.get("result").is_some(), "{formats}");

    let secret_refused = program.call(
        "locator.fill",
        &json!({
            "locator": { "page_id": page1, "selector": "#token" },
            "value": "%pw%",
        }),
    );
    assert_eq!(
        error_code(&secret_refused).0,
        "secret_input_refused",
        "{secret_refused}"
    );
    assert_eq!(
        secret_refused["error"]["data"]["reason"],
        json!("other_code_may_read"),
        "{secret_refused}"
    );

    let private_refused = program.call(
        "page.goto",
        &json!({ "page_id": page1, "url": site.url("internal.test", "/") }),
    );
    assert_eq!(
        error_code(&private_refused).0,
        "egress_refused",
        "{private_refused}"
    );
    assert!(
        !site.hosts().iter().any(|h| h == "internal.test"),
        "private-range host must not receive a request"
    );

    let save_refused = program.call(
        "session.save_identity",
        &json!({ "session_id": session1, "name": "corp" }),
    );
    assert_eq!(
        error_code(&save_refused).0,
        "session_not_savable",
        "{save_refused}"
    );
    assert_eq!(
        save_refused["error"]["data"]["source"],
        json!("running_browser"),
        "{save_refused}"
    );
    assert_eq!(
        save_refused["error"]["data"]["alternative"],
        json!("installed_browser"),
        "{save_refused}"
    );

    // Person's own tab is untouched (still open and not attached by Collie).
    let person_tab_before = person
        .pages()
        .into_iter()
        .find(|t| t["targetId"] == json!(person_target))
        .expect("person's tab must still exist");
    assert_eq!(person_tab_before["attached"], json!(false));

    // Closing Session 1 does not close the browser or the person's tab, and
    // Session 2 keeps working over the same single Run connection.
    let closed1 = program.call("session.close", &json!({ "session_id": session1 }));
    assert!(closed1.get("result").is_some(), "{closed1}");

    let page2 = opened(&program.call("session.new_page", &json!({ "session_id": session2 })))
        .page_id
        .0;
    let went2 = program.call(
        "page.goto",
        &json!({ "page_id": page2, "url": site.url(SITE, "/") }),
    );
    assert_eq!(went2["result"]["response"]["status"], json!(200), "{went2}");
    assert_eq!(proxy.connection_count(), 1);

    program.finish("running-borrow");

    // Browser and person's tab remain alive after the Run finishes.
    let person_tab_after = person
        .pages()
        .into_iter()
        .find(|t| t["targetId"] == json!(person_target))
        .expect("Collie must never close the person's running browser or tab");
    assert_eq!(person_tab_after["attached"], json!(false));

    // Provenance records unguarded results and the interception-only DNS rebinding gap.
    let entries = provenance(&env, "running-borrow");
    assert!(
        entries
            .iter()
            .any(|e| e["action"] == "interception_only" && e["gap"] == "dns_rebinding"),
        "expected dns_rebinding provenance entry: {entries:#?}"
    );
    assert!(
        entries.iter().any(|e| e["unguarded"] == json!(true)),
        "expected unguarded provenance entry for Borrowed browser results: {entries:#?}"
    );

    drop(person);
    drop(proxy);
    chrome.stop();
}

/// Administrator policies (`RemoteDebuggingAllowed: false`) raise
/// `remote_debugging_disabled_by_policy`, and browsers without consented
/// remote debugging (`firefox`, `safari`, `brave`) raise
/// `running_browser_unsupported`.
///
/// Covers: story:5-69, error:remote_debugging_disabled_by_policy, error:running_browser_unsupported, operation:run.new_session
#[test]
fn running_browser_refuses_policy_disabled_and_unsupported_browsers() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();

    let policy_profile = env.folder().join("policy-disabled-profile");
    let managed_dir = policy_profile.join("policies").join("managed");
    std::fs::create_dir_all(&managed_dir).unwrap();
    std::fs::write(
        managed_dir.join("remote_debugging.json"),
        json!({ "RemoteDebuggingAllowed": false }).to_string(),
    )
    .unwrap();

    let mut program = Program::start(&env, &cache, &[]);
    let started = program.call("run.start", &json!({ "run_id": "running-refusals" }));
    assert!(started.get("result").is_some(), "{started}");

    let policy_refused = program.call(
        "run.new_session",
        &json!({
            "run_id": "running-refusals",
            "source": {
                "kind": "running_browser",
                "browser": "chrome",
                "domains": [SITE],
                "path": policy_profile.display().to_string(),
            }
        }),
    );
    assert_eq!(
        error_code(&policy_refused).0,
        "remote_debugging_disabled_by_policy",
        "{policy_refused}"
    );
    assert_eq!(
        policy_refused["error"]["data"]["policy_key"],
        json!("RemoteDebuggingAllowed"),
        "{policy_refused}"
    );

    for browser in ["firefox", "safari", "brave"] {
        let unsupported = program.call(
            "run.new_session",
            &json!({
                "run_id": "running-refusals",
                "source": {
                    "kind": "running_browser",
                    "browser": browser,
                    "domains": [SITE],
                }
            }),
        );
        assert_eq!(
            error_code(&unsupported).0,
            "running_browser_unsupported",
            "browser={browser}: {unsupported}"
        );
        assert_eq!(
            unsupported["error"]["data"]["browser"],
            json!(browser),
            "browser={browser}: {unsupported}"
        );
    }

    program.finish("running-refusals");
}

/// Dropping the running browser's connection while `Opening` raises
/// `cdp_url_unreachable`, and dropping it while `Open` cancels in-flight calls
/// with `closed: "browser"`.
///
/// Covers: story:5-69, error:cdp_url_unreachable, error:call_cancelled, operation:run.new_session
#[test]
fn running_browser_drop_while_opening_is_cdp_url_unreachable_and_while_open_is_closed_browser() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();

    // Case 1: drop during Opening -> cdp_url_unreachable.
    let drop_port = start_drop_on_open_server();
    let opening_profile = env.folder().join("drop-during-open-profile");
    std::fs::create_dir_all(&opening_profile).unwrap();
    std::fs::write(
        opening_profile.join("DevToolsActivePort"),
        format!("{drop_port}\n/devtools/browser/drop-on-open\n"),
    )
    .unwrap();

    let mut program = Program::start(&env, &cache, &[]);
    let started = program.call("run.start", &json!({ "run_id": "running-drops" }));
    assert!(started.get("result").is_some(), "{started}");

    let open_failed = program.call(
        "run.new_session",
        &json!({
            "run_id": "running-drops",
            "source": {
                "kind": "running_browser",
                "browser": "edge",
                "domains": [SITE],
                "path": opening_profile.display().to_string(),
            }
        }),
    );
    assert_eq!(
        error_code(&open_failed).0,
        "cdp_url_unreachable",
        "{open_failed}"
    );

    // Case 2: drop while Open -> call_cancelled { by: "browser", closed: "browser" }.
    let chrome = RunningChrome::start(&env, &cache);
    let proxy = CountingProxy::start(chrome.port);
    let open_profile = env.folder().join("drop-while-open-profile");
    std::fs::create_dir_all(&open_profile).unwrap();
    std::fs::write(
        open_profile.join("DevToolsActivePort"),
        format!("{}\n{}\n", proxy.port, chrome.ws_path),
    )
    .unwrap();

    let opened_session = program.call(
        "run.new_session",
        &json!({
            "run_id": "running-drops",
            "source": {
                "kind": "running_browser",
                "browser": "chrome",
                "domains": [SITE],
                "path": open_profile.display().to_string(),
            }
        }),
    );
    assert!(opened_session.get("result").is_some(), "{opened_session}");
    let session_id = opened_session["result"]["session_id"].clone();
    let page_id = opened(&program.call("session.new_page", &json!({ "session_id": session_id })))
        .page_id
        .0;

    let mut exchange = Exchange::new(program.engine);
    exchange.subscribe();
    exchange.listen(&page_id, "console");
    let in_flight = call_in_flight(&mut exchange, &page_id);

    // Drop the proxied connection without closing Chrome itself.
    proxy.drop_connections();
    let dropped = exchange.answer(in_flight);
    let after = exchange.call("session.new_page", &json!({ "session_id": session_id }));
    let _ = exchange.call("run.finish", &json!({ "run_id": "running-drops" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        dropped["error"]["data"]["code"],
        json!("call_cancelled"),
        "{dropped}"
    );
    assert_eq!(
        dropped["error"]["data"]["by"],
        json!("browser"),
        "{dropped}"
    );
    assert_eq!(
        dropped["error"]["data"]["closed"],
        json!("browser"),
        "{dropped}"
    );
    assert_eq!(after["error"]["data"]["code"], json!("closed"), "{after}");

    drop(proxy);
    chrome.stop();
}

/// Live check (ADR 0064): verifies desktop Chrome stable's consented remote
/// debugging and `DevToolsActivePort` on the OS default user data directory.
/// Never gates a merge; run manually on a desktop with
/// `chrome://inspect/#remote-debugging` enabled.
///
/// Covers: story:5-69
#[test]
#[ignore = "live check (ADR 0064): requires desktop Chrome stable with chrome://inspect/#remote-debugging enabled on the default data directory"]
fn live_check_chrome_stable_consented_remote_debugging_on_default_data_dir() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, &[]);
    let started = program.call("run.start", &json!({ "run_id": "live-running-chrome" }));
    assert!(started.get("result").is_some(), "{started}");
    let session = program.call(
        "run.new_session",
        &json!({
            "run_id": "live-running-chrome",
            "source": {
                "kind": "running_browser",
                "browser": "chrome",
                "domains": ["example.com"],
            }
        }),
    );
    assert!(session.get("result").is_some(), "{session}");
    program.finish("live-running-chrome");
}
