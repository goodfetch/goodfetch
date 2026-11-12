//! A Borrowed browser under the egress policy over stdio (spec #563's
//! stories 14 and 15; ADRs 0017, 0056 and 0115), against a Chrome for
//! Testing the test starts with a debugging port. The test's browser
//! resolves every name to this host's loopback, where a stand-in site
//! records the head of every request it receives, whatever its host, while
//! the engine resolves each host itself through the Run's
//! `network.resolve`: a name the engine finds public reaches the site, and
//! one it finds private, or a private address, is refused at interception
//! before the browser sends it. The two resolutions are the rebinding gap
//! Provenance records.
//!
//! The site's record of request heads is the oracle rather than its
//! accepted connections: the browser preconnects to a page's host as its
//! navigation starts, before the request reaches the interception, and a
//! Borrowed browser's preferences are never changed (ADR 0056), so a
//! refused navigation may still open a connection that carries nothing.
//!
//! A `cdp_url`'s own address is judged before any connection, which a
//! stand-in listener's count of accepted connections shows.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the tests start their own browser, read its port and the Run trace, and run their stand-ins on their own threads, never an async worker"
)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::home::RUN_TRACE_PROVENANCE;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{
    Answer, BROWSERS_PATH_VARIABLE, Canaries, Canary, Outcome, Output, Process, Redirect, Refusal,
    Runner, SecretKind, SsrfCase, SsrfTable, TestEnv, browser_cache, chrome_for_testing,
    reported_debugging_port,
};
use serde_json::{Map, Value, json};

use crate::COLLIE;
use crate::page::{BROWSER_DEADLINE, opened};
use crate::page_ends::{call_in_flight, crashes_counted};
use crate::page_events::Exchange;
use goodfetch_test_client::{Attached, Hello};

/// What the engine resolves the site's public name to: a globally
/// reachable address, which the test's browser never connects to, since
/// it resolves every name to loopback itself.
const PUBLIC: &str = "93.184.215.14";

/// The site's public name.
const SITE: &str = "site.test";

/// The site's page whose button opens a popup of `/popup`.
const OPENER: &str = "/opener";

/// The site's popup page, whose button opens a popup on `leak.test` and
/// closes the page itself, so the new popup's opener is gone at once.
const POPUP: &str = "/popup";

/// The site's large page: a plain-text body of [`LARGE_BYTES`].
const LARGE: &str = "/large";

/// The person's own tab, which they open themselves.
const MINE: &str = "/mine";

/// How long the site stays silent, once the popup that opens the leaking
/// one has closed, for the test to say nothing leaked: the request would
/// leave at once, so a short window the test sets is enough.
const LEAK_WINDOW: Duration = Duration::from_secs(2);

/// How long the site waits before it answers [`MINE`]: the tab's
/// navigation is still on its way when the person first reaches the tab,
/// so a script they send at once would run in the blank document the
/// navigation then replaces.
const MINE_DELAY: Duration = Duration::from_millis(500);

/// The large page's bytes: 20 MiB, whose Base64 in one CDP message passes
/// tungstenite's default frame cap of 16 MiB.
const LARGE_BYTES: usize = 20 * 1024 * 1024;

/// How long a redirect case waits for its hop once its page loaded: a
/// refresh or a script hops once the page is in.
const HOP_DEADLINE: Duration = Duration::from_secs(10);

/// What `cdp_url_not_allowed` asks for.
const FIX: &str =
    "set the address in Operator policy or in user configuration, or give a public address";

/// A Chrome for Testing the test started with a debugging port, which a
/// Session borrows through its `cdp_url`.
struct Lent {
    process: Process,
    /// Its browser's own WebSocket address.
    cdp_url: String,
}

impl Lent {
    /// Starts the browser on a profile of its own in the test's folder,
    /// every name resolved to loopback.
    fn start(env: &TestEnv, cache: &Path) -> Self {
        let profile = env.folder().join("lent-browser-profile");
        let mut command = env.process_command(chrome_for_testing(cache));
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
        let path = file.lines().nth(1).unwrap().to_owned();
        Self {
            process,
            cdp_url: format!("ws://127.0.0.1:{port}{path}"),
        }
    }

    fn stop(self) {
        self.process.interrupt().unwrap();
        let _ = self.process.child().wait();
    }
}

/// A site on loopback that answers every request with a page and records
/// the `Host` of each request head it received.
struct Site {
    port: u16,
    hosts: Arc<Mutex<Vec<String>>>,
    /// The whole response each path given a redirect answers instead.
    redirects: Arc<Mutex<BTreeMap<String, String>>>,
}

impl Site {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hosts = Arc::new(Mutex::new(Vec::new()));
        let redirects = Arc::new(Mutex::new(BTreeMap::new()));
        let (recorded, answered) = (Arc::clone(&hosts), Arc::clone(&redirects));
        std::thread::spawn(move || {
            for stream in listener.incoming().map_while(Result::ok) {
                let (recorded, answered) = (Arc::clone(&recorded), Arc::clone(&answered));
                std::thread::spawn(move || answer(stream, &recorded, &answered));
            }
        });
        Self {
            port,
            hosts,
            redirects,
        }
    }

    /// Answers `path` with `redirect` from now on, its location as given.
    fn redirect(&self, path: &str, redirect: &Redirect) {
        let page = |location: &str, element: &str| {
            let body = element.replace("LOCATION", location);
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
        };
        let response = match redirect {
            Redirect::Status { status, location } => format!(
                "HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ),
            Redirect::RefreshHeader { location } => format!(
                "HTTP/1.1 200 OK\r\nRefresh: 0; url={location}\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ),
            Redirect::MetaRefresh { location } => page(
                location,
                "<html><head><meta http-equiv=\"refresh\" content=\"0; url=LOCATION\"></head><body>Refresh</body></html>",
            ),
            Redirect::Script { location } => page(
                location,
                "<html><body><script>location.href = \"LOCATION\";</script></body></html>",
            ),
        };
        self.redirects
            .lock()
            .unwrap()
            .insert(path.to_owned(), response);
    }

    /// The hosts of the requests it received, the browser's own for an
    /// icon included.
    fn hosts(&self) -> Vec<String> {
        self.hosts.lock().unwrap().clone()
    }

    /// The URL of `path` on the site's port at `host`.
    fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }
}

/// Reads one request head from `stream`, records its `Host`, and answers a
/// page, or the redirect its path was given; a preconnected socket that
/// sends nothing records nothing.
fn answer(
    mut stream: TcpStream,
    recorded: &Mutex<Vec<String>>,
    redirects: &Mutex<BTreeMap<String, String>>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut head = Vec::new();
    let mut buffer = [0; 4096];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => head.extend_from_slice(&buffer[..read]),
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
    let host = host.rsplit_once(':').map_or(host, |(name, _)| name);
    recorded.lock().unwrap().push(host.to_owned());
    let path = text.split(' ').nth(1).unwrap_or("/");
    if let Some(redirect) = redirects.lock().unwrap().get(path) {
        let _ = stream.write_all(redirect.as_bytes());
        return;
    }
    if path == MINE {
        std::thread::sleep(MINE_DELAY);
    }
    if path == LARGE {
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {LARGE_BYTES}\r\nConnection: close\r\n\r\n"
        );
        let chunk = vec![b'a'; 1024 * 1024];
        for _ in 0..LARGE_BYTES / chunk.len() {
            if stream.write_all(&chunk).is_err() {
                return;
            }
        }
        return;
    }
    let body = match path {
        OPENER => {
            "<html><body><button id=open onclick=\"window.open('/popup')\">Open</button></body></html>"
        }
        POPUP => {
            "<html><body><button id=leak onclick=\"window.open('http://leak.test:' + location.port + '/leak'); window.close()\">Leak</button></body></html>"
        }
        _ => "<html><body><h1>Borrowed</h1></body></html>",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// A listener on loopback that counts the connections it accepts and
/// closes each: where a refused `cdp_url` would have connected.
fn stand_in() -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&accepted);
    std::thread::spawn(move || {
        for stream in listener.incoming().map_while(Result::ok) {
            counted.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    (port, accepted)
}

/// A Connection to an Attached engine whose `hello` names `surface`.
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

    /// Starts the Run `run_id` whose `network.resolve` maps each of
    /// `resolve`'s names to its answers.
    fn run(&mut self, run_id: &str, resolve: &[(&str, &[&str])]) {
        let resolve: Map<String, Value> = resolve
            .iter()
            .map(|(host, answers)| ((*host).to_owned(), json!(answers)))
            .collect();
        let started = self.call(
            "run.start",
            &json!({ "run_id": run_id, "settings": { "network": { "resolve": resolve } } }),
        );
        assert!(started.get("result").is_some(), "{started}");
    }

    /// Opens a Session of `run_id` on the browser at `cdp_url`.
    fn borrow(&mut self, run_id: &str, cdp_url: &str) -> Value {
        self.call(
            "run.new_session",
            &json!({ "run_id": run_id, "source": { "kind": "cdp_url", "url": cdp_url } }),
        )
    }

    fn new_page(&mut self, session: &Value) -> String {
        opened(&self.call("session.new_page", &json!({ "session_id": session })))
            .page_id
            .0
    }

    fn goto(&mut self, page: &str, url: &str) -> Value {
        self.call("page.goto", &json!({ "page_id": page, "url": url }))
    }

    fn finish(mut self, run_id: &str) {
        let _ = self.call("run.finish", &json!({ "run_id": run_id }));
        let _ = self.engine.finish();
    }
}

/// The Error code of a call's error and its details.
fn error(answer: &Value) -> (Value, Value) {
    let mut details = answer["error"]["data"].clone();
    let code = details["code"].take();
    if let Some(details) = details.as_object_mut() {
        details.remove("code");
        details.remove("retryable");
    }
    (code, details)
}

/// The Run trace's Provenance entries of the Run `run_id`.
fn provenance(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let file = HomeLayout::of(env.home()).run_trace(run_id, RUN_TRACE_PROVENANCE);
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// A Borrowed browser asked for a name the engine resolves to a private
/// address, or for a private address, is refused at interception, and no
/// request reaches the site, where the browser would have sent it; a name
/// the engine finds public reaches it (positive control). The Session's
/// Provenance records that interception alone holds it, and the rebinding
/// gap.
///
/// Covers: story:4-14, error:egress_refused, operation:run.new_session
#[test]
fn a_borrowed_browser_is_refused_a_private_address_at_interception_and_provenance_names_the_gap() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    program.run(
        "borrowed",
        &[(SITE, &[PUBLIC]), ("internal.test", &["10.0.0.1"])],
    );
    let borrowed = program.borrow("borrowed", &lent.cdp_url);
    let session = borrowed["result"]["session_id"].clone();
    let page = program.new_page(&session);

    let public = program.goto(&page, &site.url(SITE, "/"));
    let private = program.goto(&page, &site.url("internal.test", "/"));
    let loopback = program.goto(&page, &site.url("127.0.0.1", "/"));
    let requested: Vec<String> = site
        .hosts()
        .into_iter()
        .filter(|host| host != SITE)
        .collect();
    program.finish("borrowed");
    lent.stop();

    let marked: Vec<Value> = provenance(&env, "borrowed")
        .into_iter()
        .filter(|entry| entry["action"] == "interception_only")
        .collect();
    assert_eq!(
        (
            &public["result"]["response"]["status"],
            error(&private),
            error(&loopback),
            requested,
            marked,
        ),
        (
            &json!(200),
            (
                json!("egress_refused"),
                json!({
                    "url": site.url("internal.test", "/"),
                    "rule": "private_address",
                    "address": "10.0.0.1",
                    "setting": "egress.private_ranges",
                })
            ),
            (
                json!("egress_refused"),
                json!({
                    "url": site.url("127.0.0.1", "/"),
                    "rule": "private_address",
                    "address": "127.0.0.1",
                    "setting": "egress.private_ranges",
                })
            ),
            Vec::<String>::new(),
            vec![json!({
                "action": "interception_only",
                "session_id": session,
                "egress": "interception_only",
                "gap": "dns_rebinding",
            })],
        ),
        "{borrowed}"
    );
}

/// A body whose Base64 passes 16 MiB reads with `page.request` on a
/// Borrowed browser, as on a launched one, and the Session goes on:
/// Chromium sends each CDP message as one WebSocket frame, and the
/// connection takes one as long as the debugging pipe's (finding 563-3).
///
/// Covers: operation:page.request
#[test]
fn a_body_past_sixteen_mebibytes_reads_on_a_borrowed_browser_and_the_session_goes_on() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    program.run("large", &[(SITE, &[PUBLIC])]);
    let borrowed = program.borrow("large", &lent.cdp_url);
    let page = program.new_page(&borrowed["result"]["session_id"]);

    let read = program.call(
        "page.request",
        &json!({ "page_id": page, "url": site.url(SITE, LARGE) }),
    );
    let after = program.goto(&page, &site.url(SITE, "/"));
    program.finish("large");
    lent.stop();

    assert_eq!(
        (
            &read["result"]["status"],
            &after["result"]["response"]["status"]
        ),
        (&json!(200), &json!(200)),
        "{read}"
    );
}

/// The project configuration `run.start` sends whose `session.source` is
/// the `cdp_url` `url`.
fn project_source(url: &str) -> Value {
    json!({
        "path": "goodfetch.toml",
        "settings": { "session": { "source": { "kind": "cdp_url", "url": url } } },
    })
}

/// O8 of the deep review (accepted by the owner on 2026-10-28): a Client
/// acting for a model sends the project configuration itself, so a
/// loopback `cdp_url` there is the model's and the Run's default Session
/// is refused with `cdp_url_not_allowed` naming the project layer, before
/// any connection: the stand-in at that address accepts none. The same
/// project configuration from the program's own code opens its default
/// Session on the browser there (positive control).
///
/// Covers: story:4-15, error:cdp_url_not_allowed, operation:run.new_page
#[test]
fn a_loopback_cdp_url_in_a_models_project_configuration_is_refused() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let (port, accepted) = stand_in();
    let stand_in_url = format!("ws://127.0.0.1:{port}/devtools/browser/stand-in");

    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    let started = program.call(
        "run.start",
        &json!({ "run_id": "own", "project": project_source(&lent.cdp_url) }),
    );
    let own = program.call("run.new_page", &json!({ "run_id": "own" }));
    program.finish("own");
    let mut model = Program::start(&env, &cache, Surface::Mcp);
    let model_started = model.call(
        "run.start",
        &json!({ "run_id": "model", "project": project_source(&stand_in_url) }),
    );
    let refused = model.call("run.new_page", &json!({ "run_id": "model" }));
    model.finish("model");
    lent.stop();

    assert_eq!(
        (
            started.get("result").is_some(),
            own["result"]["page_id"].is_string(),
            model_started.get("result").is_some(),
            error(&refused),
            accepted.load(Ordering::SeqCst),
        ),
        (
            true,
            true,
            true,
            (
                json!("cdp_url_not_allowed"),
                json!({
                    "url": stand_in_url,
                    "layer": "project_configuration",
                    "rule": "loopback",
                    "address": "127.0.0.1",
                    "fix": FIX,
                })
            ),
            0,
        ),
        "{started} {own} {model_started}"
    );
}

/// A `cdp_url` on loopback from the program's own code opens its Session
/// on the browser there (positive control); the same address from a
/// Client acting for a model, or one whose host also resolves to a private
/// network, is refused with `cdp_url_not_allowed` before any connection:
/// the stand-in at that address accepts none.
///
/// Covers: story:4-15, error:cdp_url_not_allowed, operation:run.new_session
#[test]
fn a_loopback_cdp_url_opens_for_the_programs_own_code_alone() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let (port, accepted) = stand_in();
    let stand_in_url = format!("ws://127.0.0.1:{port}/devtools/browser/stand-in");
    let mixed_url = format!("ws://debugger.test:{port}/devtools/browser/stand-in");
    let resolve: &[(&str, &[&str])] = &[("debugger.test", &["127.0.0.1", "10.0.0.9"])];

    let mut program = Program::start(&env, &cache, Surface::SdkTypescript);
    program.run("own", resolve);
    let own = program.borrow("own", &lent.cdp_url);
    let mixed = program.borrow("own", &mixed_url);
    program.finish("own");
    let mut model = Program::start(&env, &cache, Surface::Mcp);
    model.run("model", resolve);
    let refused = model.borrow("model", &stand_in_url);
    model.finish("model");
    lent.stop();

    assert_eq!(
        (
            own["result"]["session_id"].is_string(),
            error(&refused),
            error(&mixed),
            accepted.load(Ordering::SeqCst),
        ),
        (
            true,
            (
                json!("cdp_url_not_allowed"),
                json!({
                    "url": stand_in_url,
                    "layer": "run",
                    "rule": "loopback",
                    "address": "127.0.0.1",
                    "fix": FIX,
                })
            ),
            (
                json!("cdp_url_not_allowed"),
                json!({
                    "url": mixed_url,
                    "layer": "run",
                    "rule": "private_address",
                    "address": "10.0.0.9",
                    "setting": "egress.private_ranges",
                    "fix": FIX,
                })
            ),
            0,
        ),
        "{own}"
    );
}

/// The browsers the test's engine launched itself, beside the lent one.
#[cfg(unix)]
fn launched_by_the_engine(env: &TestEnv) -> usize {
    crate::page::browser_command_lines(env)
        .iter()
        .filter(|line| !line.contains("--type=") && !line.contains("lent-browser-profile"))
        .count()
}

/// A Run resumed after its engine was killed reopens a Session it opened on
/// a Borrowed browser on that browser again: its next page opens there,
/// and the engine launches no browser of its own in its place.
///
/// Covers: story:5-60, operation:run.resume, operation:session.new_page
#[cfg(unix)]
#[test]
fn a_resumed_run_reopens_a_borrowed_session_on_its_browser() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let mut program = Program::start(&env, &cache, Surface::SdkTypescript);
    program.run("borrowed-resumed", &[]);
    let session = program.borrow("borrowed-resumed", &lent.cdp_url)["result"]["session_id"].clone();
    let _ = program.new_page(&session);
    program.engine.kill();

    let mut resumed = Program::start(&env, &cache, Surface::SdkTypescript);
    let resume = resumed.call(
        "run.resume",
        &json!({ "run_id": "borrowed-resumed", "settings": { "network": { "resolve": {} } } }),
    );
    let page = resumed.call("session.new_page", &json!({ "session_id": session }));
    let launched = launched_by_the_engine(&env);
    resumed.finish("borrowed-resumed");
    lent.stop();

    assert_eq!(
        (
            &resume["result"]["started_as"],
            page["result"]["page_id"].is_string(),
            launched,
        ),
        (&json!("resumed"), true, 0),
        "{page}"
    );
}

/// How many data keys the run file of `run_id` holds, wrapped.
fn data_keys(env: &TestEnv, run_id: &str) -> u32 {
    rusqlite::Connection::open(HomeLayout::of(env.home()).run_file(run_id))
        .unwrap()
        .query_row("SELECT count(*) FROM run_key", [], |row| row.get(0))
        .unwrap()
}

/// The `cdp_url` a Session borrows its browser through, which names the
/// one browser and may carry its provider's access token, is sealed in run
/// state under the Run's data key: its browser's own name is in no byte of
/// the run file, its WAL or its Checkpoint, plain or encoded, and the Run
/// resumed after its engine was killed still opens it, since its Session's
/// next page opens on that browser. The Run made one data key for it.
///
/// Covers: story:4-64, story:5-60, operation:run.resume
#[cfg(unix)]
#[test]
fn a_borrowed_sessions_cdp_url_is_sealed_in_run_state_and_opens_after_a_resume() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let mut canaries = Canaries::new(env.seed());
    let browser = lent.cdp_url.rsplit('/').next().unwrap();
    canaries.add(Canary::known("cdp-url", SecretKind::EndpointToken, browser));
    let mut program = Program::start(&env, &cache, Surface::SdkTypescript);
    program.run("sealed-source", &[]);
    let session = program.borrow("sealed-source", &lent.cdp_url)["result"]["session_id"].clone();
    let _ = program.new_page(&session);
    let checkpointed = program.call("run.checkpoint", &json!({ "run_id": "sealed-source" }));
    program.engine.kill();

    let folder = HomeLayout::of(env.home()).run_folder("sealed-source");
    let files = ["run.sqlite", "run.sqlite-wal", "checkpoint.sqlite"].map(|file| folder.join(file));
    let held: Vec<Output<'_>> = files
        .iter()
        .filter(|file| file.exists())
        .map(|file| Output::Path(file))
        .collect();
    let leaks: Vec<String> = canaries
        .search(&held)
        .unwrap()
        .into_iter()
        .map(|hit| format!("{} in {}", hit.canary, hit.output))
        .collect();
    let keys = data_keys(&env, "sealed-source");
    let mut resumed = Program::start(&env, &cache, Surface::SdkTypescript);
    let resume = resumed.call(
        "run.resume",
        &json!({ "run_id": "sealed-source", "settings": { "network": { "resolve": {} } } }),
    );
    let page = resumed.call("session.new_page", &json!({ "session_id": session }));
    let launched = launched_by_the_engine(&env);
    resumed.finish("sealed-source");
    lent.stop();

    assert_eq!(
        (
            leaks,
            checkpointed.get("result").is_some() && files[2].exists(),
            keys,
            &resume["result"]["started_as"],
            page["result"]["page_id"].is_string(),
            launched,
        ),
        (Vec::new(), true, 1, &json!("resumed"), true, 0),
        "{page}"
    );
}

/// A Run whose Sessions borrow no browser writes no secret column, so it
/// makes no data key and never reaches the key-encryption key.
///
/// Covers: story:4-64
#[test]
fn a_run_with_no_borrowed_session_makes_no_data_key() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let mut program = Program::start(&env, &cache, Surface::SdkTypescript);
    program.run("no-secret", &[]);
    let session = program.call("run.new_session", &json!({ "run_id": "no-secret" }));
    program.finish("no-secret");

    assert_eq!(
        (
            session["result"]["session_id"].is_string(),
            data_keys(&env, "no-secret")
        ),
        (true, 0),
        "{session}"
    );
}

/// A call on a Borrowed browser whose connection ended answers at once,
/// however long the Run's `timeout.close`: the engine sees no exit of a
/// browser it did not launch, so it waits for none to name the loss (fix
/// queue 104: a Run finished while its engine page opened waited
/// `timeout.close` for it, twice, past the client's deadline). The loss
/// reads as a quit (spec #594's decision 11), so the Session, which held
/// no page, ends, and its new page raises `closed`.
///
/// Covers: error:closed, operation:session.new_page
#[test]
fn a_call_on_a_borrowed_browser_that_went_answers_without_waiting_for_its_exit() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let mut program = Program::start(&env, &cache, Surface::SdkTypescript);
    // Past the client's deadline for each message (`BROWSER_DEADLINE`), so
    // a call that waited for an exit would never answer in time.
    let started = program.call(
        "run.start",
        &json!({ "run_id": "went", "settings": { "timeout": { "close": 600_000 } } }),
    );
    assert!(started.get("result").is_some(), "{started}");
    let session = program.borrow("went", &lent.cdp_url)["result"]["session_id"].clone();
    lent.stop();

    let page = program.call("session.new_page", &json!({ "session_id": session }));
    program.finish("went");

    assert_eq!(error(&page).0, json!("closed"), "{page}");
}

/// A Borrowed browser that went while a call was in flight on its page
/// quit, since no exit of a browser the engine did not launch tells a quit
/// from a crash (spec #594's decision 11): the call raises
/// `call_cancelled` naming the browser, the Session ends, so its new page
/// raises `closed`, and Usage counts no page crash.
///
/// Covers: story:5-28, error:call_cancelled, operation:session.new_page
#[test]
fn a_borrowed_browser_that_went_under_a_call_cancels_it_naming_the_browser() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let mut program = Program::start(&env, &cache, Surface::SdkTypescript);
    program.run("lent-quit", &[]);
    let session = program.borrow("lent-quit", &lent.cdp_url)["result"]["session_id"].clone();
    let page = program.new_page(&session);
    let mut exchange = Exchange::new(program.engine);
    exchange.subscribe();
    exchange.listen(&page, "console");
    let calling = call_in_flight(&mut exchange, &page);

    lent.stop();
    let called = exchange.answer(calling);
    let new_page = exchange.call("session.new_page", &json!({ "session_id": session }));
    let crashes = crashes_counted(&mut exchange, "lent-quit");
    let _ = exchange.call("run.finish", &json!({ "run_id": "lent-quit" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (error(&called), error(&new_page).0, crashes),
        (
            (
                json!("call_cancelled"),
                json!({ "by": "browser", "closed": "browser" })
            ),
            json!("closed"),
            Vec::<Value>::new()
        ),
        "{called} {new_page}"
    );
}

/// Writes an Operator policy whose `session.source` is the `cdp_url`
/// `url`, and that sets `egress.no_private_endpoints` when `no_private`.
fn operator_policy(env: &TestEnv, url: &str, no_private: bool) {
    let layout = HomeLayout::of(env.home());
    std::fs::create_dir_all(layout.per_major()).unwrap();
    let mut policy = format!(
        "version = \"0.1\"\n\n[settings.session.source]\nkind = \"cdp_url\"\nurl = \"{url}\"\n"
    );
    if no_private {
        policy.push_str("\n[settings.egress]\nno_private_endpoints = true\n");
    }
    std::fs::write(layout.operator_policy(), policy).unwrap();
}

/// A `cdp_url` set in Operator policy is an Operator-configured endpoint:
/// on loopback it opens the Run's default Session even for a Client acting
/// for a model, whose page reaches a public name.
///
/// Covers: story:4-15, story:4-16, operation:run.new_page
#[test]
fn a_cdp_url_in_operator_policy_opens_on_a_private_address() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    operator_policy(&env, &lent.cdp_url, false);
    let mut model = Program::start(&env, &cache, Surface::Mcp);
    model.run("configured", &[(SITE, &[PUBLIC])]);

    let page = model.call("run.new_page", &json!({ "run_id": "configured" }));
    let went = model.goto(&opened(&page).page_id.0, &site.url(SITE, "/"));
    model.finish("configured");
    lent.stop();

    assert_eq!(&went["result"]["response"]["status"], &json!(200), "{went}");
}

/// Under an Operator policy that sets `egress.no_private_endpoints`, the
/// same `cdp_url` on loopback is refused with `cdp_url_not_allowed` naming
/// the setting, before any connection.
///
/// Covers: story:4-15, error:cdp_url_not_allowed, setting:egress.no_private_endpoints
#[test]
fn a_cdp_url_in_operator_policy_is_refused_a_private_address_under_no_private_endpoints() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let (port, accepted) = stand_in();
    let url = format!("ws://127.0.0.1:{port}/devtools/browser/stand-in");
    operator_policy(&env, &url, true);
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    program.run("configured", &[]);

    let refused = program.call("run.new_page", &json!({ "run_id": "configured" }));
    program.finish("configured");

    assert_eq!(
        (error(&refused), accepted.load(Ordering::SeqCst)),
        (
            (
                json!("cdp_url_not_allowed"),
                json!({
                    "url": url,
                    "layer": "operator_policy",
                    "rule": "private_address",
                    "address": "127.0.0.1",
                    "setting": "egress.no_private_endpoints",
                    "fix": FIX,
                })
            ),
            0
        )
    );
}

/// A name no resolver answers, which the test's browser still resolves to
/// the site, as a remote browser's network may resolve an intranet name.
const UNRESOLVED: &str = "intranet.invalid";

/// O9 of the deep review (accepted by the owner on 2026-10-28): a name the
/// engine cannot resolve is refused at interception on a Borrowed browser
/// whose `cdp_url` is not loopback, naming `network.resolve`, and no
/// request for it reaches the site, while a name the engine finds public
/// still loads there; on the same browser borrowed at its loopback
/// address, the name is left to the browser, which reaches the site with
/// it (positive control). The browser elsewhere is the test's own, at
/// `0.0.0.0`, which only connects to this machine but is no loopback
/// address, set in the Operator policy, where it is an Operator-configured
/// endpoint.
///
/// Covers: story:4-14, error:egress_refused, setting:network.resolve
#[test]
fn a_name_the_engine_cannot_resolve_is_refused_on_a_borrowed_browser_not_on_loopback() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    operator_policy(&env, &lent.cdp_url.replace("127.0.0.1", "0.0.0.0"), false);
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    program.run("unresolved", &[(SITE, &[PUBLIC])]);

    let elsewhere = opened(&program.call("run.new_page", &json!({ "run_id": "unresolved" })))
        .page_id
        .0;
    let public = program.goto(&elsewhere, &site.url(SITE, "/"));
    let refused = program.goto(&elsewhere, &site.url(UNRESOLVED, "/"));
    let refused_requests = site
        .hosts()
        .iter()
        .filter(|host| *host == UNRESOLVED)
        .count();
    let on_loopback = program.borrow("unresolved", &lent.cdp_url);
    let local = program.new_page(&on_loopback["result"]["session_id"]);
    let passed = program.goto(&local, &site.url(UNRESOLVED, "/"));
    let requests = site
        .hosts()
        .iter()
        .filter(|host| *host == UNRESOLVED)
        .count();
    program.finish("unresolved");
    lent.stop();

    assert_eq!(
        (
            &public["result"]["response"]["status"],
            error(&refused),
            refused_requests,
            &passed["result"]["response"]["status"],
            requests > 0,
        ),
        (
            &json!(200),
            (
                json!("egress_refused"),
                json!({
                    "url": site.url(UNRESOLVED, "/"),
                    "rule": "domain_policy",
                    "setting": "network.resolve",
                })
            ),
            0,
            &json!(200),
            true,
        ),
        "{refused} {on_loopback} {passed}"
    );
}

/// A local address belongs to this machine (ADR 0044): a Run that sets
/// `egress.local_addresses` refuses a Session on a Borrowed browser it lent
/// and one on a third-party CDP browser at a public address with
/// `local_address_unavailable` before connecting to either, while a Session on a browser the
/// engine launches takes the address and reaches its site (positive
/// control).
///
/// Covers: story:4-48, error:local_address_unavailable, setting:egress.local_addresses, operation:run.new_session
#[test]
fn a_local_address_is_refused_on_a_borrowed_and_a_third_party_cdp_browser() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let third_party = "ws://cloud-browser.test:9222/devtools/browser/third-party";
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    let started = program.call(
        "run.start",
        &json!({
            "run_id": "local",
            "settings": {
                "network": { "resolve": {
                    SITE: ["127.0.0.1"],
                    "cloud-browser.test": [PUBLIC],
                } },
                "egress": {
                    "local_addresses": ["127.0.0.1"],
                    // The launched Session's site, on loopback.
                    "private_ranges": [SITE],
                },
            },
        }),
    );

    let borrowed = program.borrow("local", &lent.cdp_url);
    let cloud = program.borrow("local", third_party);
    let launched = opened(&program.call("run.new_page", &json!({ "run_id": "local" })))
        .page_id
        .0;
    let reached = program.goto(&launched, &site.url(SITE, "/"));
    program.finish("local");
    lent.stop();

    let refused = (
        json!("local_address_unavailable"),
        json!({
            "entry": "[\"127.0.0.1\"]",
            "failure": "not_launched_here",
            "setting": "egress.local_addresses",
        }),
    );
    assert!(started.get("result").is_some(), "{started}");
    assert_eq!(
        (
            error(&borrowed),
            error(&cloud),
            &reached["result"]["response"]["status"],
        ),
        (refused.clone(), refused, &json!(200)),
        "{reached}"
    );
}

/// The names a Run maps and their answers.
type Mapping = Vec<(String, Vec<String>)>;

/// The Run's `network.resolve` mapping for `case`: the site's name, public,
/// and the case's lookups.
fn mapping_of(case: &SsrfCase) -> Mapping {
    let mut names: Mapping = vec![(SITE.to_owned(), vec![PUBLIC.to_owned()])];
    for lookup in &case.lookups {
        let answers = lookup
            .answers
            .iter()
            .map(|answer| match answer {
                Answer::Address(address) => address.to_string(),
                Answer::Public => PUBLIC.to_owned(),
            })
            .collect();
        names.push((lookup.name.clone(), answers));
    }
    names
}

/// The page of the Run whose mapping is `names`, started with a Session on
/// the browser at `cdp_url` the first time.
fn borrowed_page(
    program: &mut Program,
    cdp_url: &str,
    sessions: &mut Vec<(Mapping, String)>,
    names: Mapping,
) -> String {
    if let Some((_, page)) = sessions.iter().find(|(mapped, _)| *mapped == names) {
        return page.clone();
    }
    let run_id = format!("table-{}", sessions.len());
    let answers: Vec<Vec<&str>> = names
        .iter()
        .map(|(_, answers)| answers.iter().map(String::as_str).collect())
        .collect();
    let resolve: Vec<(&str, &[&str])> = names
        .iter()
        .zip(&answers)
        .map(|((name, _), answers)| (name.as_str(), answers.as_slice()))
        .collect();
    program.run(&run_id, &resolve);
    let session = program.borrow(&run_id, cdp_url)["result"]["session_id"].clone();
    let page = program.new_page(&session);
    sessions.push((names, page.clone()));
    page
}

/// The SSRF table's thin runner on a Borrowed browser (ADR 0067), for its
/// own family and the address families: each case's URL on the site's
/// port, as the WHATWG parser writes it (as written when it refuses it),
/// its names resolved by the engine as its `lookups` say and by the browser
/// to loopback, so a case let through reaches the site, which then stands
/// in for its target. One Run, and its Borrowed Session and page, per
/// mapping its cases share. Positive control: the site's public name goes through.
///
/// Covers: story:4-14, story:4-89, error:egress_refused
#[test]
fn every_borrowed_browser_case_of_the_ssrf_table_is_refused_at_interception() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let runner = Runner::committed(
        "borrowed_browser",
        file!(),
        "every_borrowed_browser_case_of_the_ssrf_table_is_refused_at_interception",
    )
    .unwrap();
    let controls = SsrfTable::parse(&format!(
        r#"
        [[cases]]
        name = "the_sites_public_name"
        family = "borrowed_browser"
        url = "http://{SITE}/"
        target = "public"
        refusal = {{ code = "egress_refused", rule = "private_address" }}
        "#
    ))
    .unwrap();
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    let public = PUBLIC.parse().unwrap();
    // One Run per mapping, each with its Session on the lent browser.
    let mut sessions: Vec<(Mapping, String)> = Vec::new();

    let result = runner.run(&controls.cases, public, |case| {
        let page = borrowed_page(&mut program, &lent.cdp_url, &mut sessions, mapping_of(case));
        let url = match url::Url::parse(&case.url) {
            Ok(mut url) => {
                let _ = url.set_port(Some(site.port));
                url.to_string()
            }
            Err(_) => case.url.clone(),
        };
        let before = site.hosts().len();
        let went = program.goto(&page, &url);
        // Only a request for the case's own host counts: the browser's own
        // for an earlier page's icon may arrive meanwhile.
        let own = url::Url::parse(&url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned));
        let reached = site.hosts().iter().skip(before).any(|host| {
            let name = host
                .rsplit_once(':')
                .map_or(host.as_str(), |(name, _)| name);
            own.as_deref() == Some(name)
        });
        let refusal = match error(&went) {
            (code, details) if code == "egress_refused" => {
                serde_json::from_value(details["rule"].clone())
                    .ok()
                    .map(|rule| Refusal::EgressRefused { rule })
            }
            _ => None,
        };
        // The browser resolves every name to the site, which then stands
        // in for the case's target; an address it connects to itself.
        let target = case.target_address(public);
        Outcome {
            refusal,
            connections: if reached {
                target.into_iter().collect()
            } else {
                Vec::new()
            },
        }
    });
    for run in 1..sessions.len() {
        let _ = program.call("run.finish", &json!({ "run_id": format!("table-{run}") }));
    }
    program.finish("table-0");
    lent.stop();

    assert!(
        runner.cases().len() > 70,
        "only {} cases",
        runner.cases().len()
    );
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}

/// On a Borrowed browser, whose launch the engine never chose, the
/// back/forward cache may restore the page `goBack` returns to: the call
/// answers once the page is restored, rather than waiting out
/// `timeout.navigation` for a `load` a restored page never fires
/// (ADR 0121; pitfall B21).
///
/// Covers: story:5-24, operation:page.go_back
#[test]
fn going_back_on_a_borrowed_browser_answers_a_page_its_cache_restored() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    let started = program.call(
        "run.start",
        &json!({
            "run_id": "restored",
            "settings": {
                "network": { "resolve": { SITE: [PUBLIC] } },
                "timeout": { "navigation": 5000 },
            },
        }),
    );
    assert!(started.get("result").is_some(), "{started}");
    let borrowed = program.borrow("restored", &lent.cdp_url);
    let page = program.new_page(&borrowed["result"]["session_id"]);

    program.goto(&page, &site.url(SITE, "/a"));
    program.goto(&page, &site.url(SITE, "/b"));
    let back = program.call("page.go_back", &json!({ "page_id": page }));
    let path = program.call(
        "page.evaluate",
        &json!({ "page_id": page, "function": "() => location.pathname" }),
    );
    program.finish("restored");
    lent.stop();

    assert_eq!(
        (back.get("result").is_some(), &path["result"]["value"]),
        (true, &json!("/a")),
        "{back}"
    );
}

/// The person's own CDP connection to the lent browser, as their own tools
/// would hold one.
struct Person {
    socket: tungstenite::WebSocket<TcpStream>,
    next_id: u64,
    /// The events the browser sent while a command waited for its reply,
    /// oldest first, which [`Self::event`] has not taken yet.
    events: Vec<Value>,
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
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let socket = tungstenite::client(cdp_url, stream).unwrap().0;
        Self {
            socket,
            next_id: 0,
            events: Vec::new(),
        }
    }

    /// The result of the CDP command `method` with `params`, on the target
    /// session `session` when one is given.
    fn send(&mut self, method: &str, params: &Value, session: Option<&str>) -> Value {
        self.next_id += 1;
        let mut command = json!({ "id": self.next_id, "method": method, "params": params });
        if let Some(session) = session {
            command["sessionId"] = json!(session);
        }
        self.socket
            .send(tungstenite::Message::text(command.to_string()))
            .unwrap();
        loop {
            let reply = self.message(method);
            if reply["id"] == json!(self.next_id) {
                assert!(reply.get("error").is_none(), "{method}: {reply}");
                return reply["result"].clone();
            }
            if reply.get("method").is_some() {
                self.events.push(reply);
            }
        }
    }

    /// The first event on the target session `session` that `wanted`
    /// holds of, among those already received or the next ones.
    fn event(&mut self, session: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        let of = |event: &Value| event["sessionId"] == json!(session) && wanted(event);
        if let Some(index) = self.events.iter().position(of) {
            return self.events.remove(index);
        }
        loop {
            let event = self.message("an event");
            if of(&event) {
                return event;
            }
            if event.get("method").is_some() {
                self.events.push(event);
            }
        }
    }

    /// The next message from the browser, read for `awaited`, within the
    /// socket's 30 s read timeout.
    fn message(&mut self, awaited: &str) -> Value {
        loop {
            let message = self
                .socket
                .read()
                .unwrap_or_else(|error| panic!("waiting for {awaited}: {error}"));
            if let tungstenite::Message::Text(text) = message {
                return serde_json::from_str(&text).unwrap();
            }
        }
    }

    /// The browser's page targets, as `Target.getTargets` describes them.
    fn pages(&mut self) -> Vec<Value> {
        self.send("Target.getTargets", &json!({}), None)["targetInfos"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|target| target["type"] == "page")
            .collect()
    }

    /// The page targets once `settled` holds of them, or as they are after
    /// 10 s.
    fn pages_once(&mut self, settled: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            let pages = self.pages();
            if settled(&pages) || std::time::Instant::now() > deadline {
                return pages;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

/// Whether the page target `target`'s URL ends with `path`.
fn at(target: &Value, path: &str) -> bool {
    target["url"]
        .as_str()
        .is_some_and(|url| url.ends_with(path))
}

/// The person's own tabs stay theirs while a Session borrows their browser
/// (ADR 0056): the tab already open, one they open and one their own tab
/// opens are each let go of at once, with no engine session left on them,
/// while the Session's own page keeps the engine's (positive control). The
/// person's script runs in their tab's own page, in the context the browser
/// names once the tab's navigation is in.
///
/// Covers: story:4-14, operation:run.new_session, operation:session.new_page
#[test]
fn a_borrowed_browser_lets_go_of_the_persons_own_tabs() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    program.run("tabs", &[(SITE, &[PUBLIC])]);
    let session = program.borrow("tabs", &lent.cdp_url)["result"]["session_id"].clone();
    let page = program.new_page(&session);
    let went = program.goto(&page, &site.url(SITE, "/session"));

    let mut person = Person::connect(&lent.cdp_url);
    let mine = person.send(
        "Target.createTarget",
        &json!({ "url": site.url(SITE, MINE) }),
        None,
    )["targetId"]
        .clone();
    let attached = person.send(
        "Target.attachToTarget",
        &json!({ "targetId": mine, "flatten": true }),
        None,
    )["sessionId"]
        .as_str()
        .unwrap()
        .to_owned();
    // The tab is still on its way to `MINE` (`MINE_DELAY`): its script runs
    // in the context the browser names once that document is in, which no
    // later navigation replaces, never in the blank one the navigation ends.
    person.send("Runtime.enable", &json!({}), Some(&attached));
    let origin = site.url(SITE, "");
    let context = person.event(&attached, |event| {
        let context = &event["params"]["context"];
        event["method"] == "Runtime.executionContextCreated"
            && context["origin"] == json!(origin)
            && context["auxData"]["frameId"] == mine
            && context["auxData"]["isDefault"] == json!(true)
    })["params"]["context"]["id"]
        .clone();
    let opened = person.send(
        "Runtime.evaluate",
        &json!({
            "expression": format!(
                "[location.pathname, window.open({:?}) !== null]",
                site.url(SITE, "/opened")
            ),
            "contextId": context,
            "returnByValue": true,
            "userGesture": true,
        }),
        Some(&attached),
    );
    let pages = person.pages_once(|pages| {
        pages.iter().any(|target| at(target, "/opened"))
            && pages
                .iter()
                .filter(|target| !at(target, MINE) && !at(target, "/session"))
                .all(|target| target["attached"] == false)
    });
    program.finish("tabs");
    lent.stop();

    let attached_of = |path: &str| {
        pages
            .iter()
            .find(|target| at(target, path))
            .map(|target| target["attached"].clone())
    };
    assert_eq!(
        (
            &went["result"]["response"]["status"],
            &opened["result"]["value"],
            attached_of("about:blank"),
            attached_of("/opened"),
            attached_of("/session"),
        ),
        (
            &json!(200),
            &json!([MINE, true]),
            Some(json!(false)),
            Some(json!(false)),
            Some(json!(true)),
        ),
        "{pages:#?}"
    );
}

/// A popup the Session cannot take in never runs outside its interception
/// (ADR 0085; pitfall B15): its popup's popup, opened as that popup closes
/// itself, so its opener is gone at once, is closed before it runs or taken
/// in, and either way its request to a name the engine resolves to a
/// private address never reaches the site, where the browser, which
/// resolves it to loopback, would send it.
///
/// Covers: story:4-14, story:5-55, operation:locator.click, operation:session.pages
#[test]
fn a_popup_whose_opener_went_never_runs_outside_the_interception() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    program.run("popups", &[(SITE, &[PUBLIC]), ("leak.test", &["10.0.0.1"])]);
    let session = program.borrow("popups", &lent.cdp_url)["result"]["session_id"].clone();
    let page = program.new_page(&session);
    let went = program.goto(&page, &site.url(SITE, OPENER));
    let clicked = program.call(
        "locator.click",
        &json!({ "locator": { "page_id": page, "selector": "#open" } }),
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let popup = loop {
        let pages = program.call("session.pages", &json!({ "session_id": session }));
        if let Some(popup) = pages["result"]["pages"]
            .as_array()
            .and_then(|pages| pages.iter().find(|listed| **listed != json!(page)))
        {
            break popup.clone();
        }
        assert!(std::time::Instant::now() < deadline, "no popup: {pages}");
        std::thread::sleep(Duration::from_millis(100));
    };
    let loaded = program.call(
        "page.wait_for_load_state",
        &json!({ "page_id": popup, "state": "load" }),
    );
    let _ = program.call(
        "locator.click",
        &json!({ "locator": { "page_id": popup, "selector": "#leak" } }),
    );
    // The popup opens its own popup, then closes itself in the same
    // handler: once it is gone from the Session, the leak path ran, and
    // only then does the site's silence say anything.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let pages = program.call("session.pages", &json!({ "session_id": session }));
        // An error reply says nothing of the popup: only a list without it
        // says it closed.
        let listed = pages["result"]["pages"]
            .as_array()
            .unwrap_or_else(|| panic!("no list of pages: {pages}"))
            .contains(&popup);
        if !listed {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the popup never closed: {pages}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    std::thread::sleep(LEAK_WINDOW);
    let leaked = site.hosts().iter().any(|host| host == "leak.test");
    program.finish("popups");
    lent.stop();

    assert_eq!(
        (
            &went["result"]["response"]["status"],
            clicked.get("result").is_some(),
            loaded.get("result").is_some(),
            leaked,
        ),
        (&json!(200), true, true, false),
    );
}

/// `redirect` with its location, when it is an `http:` URL, on the site's
/// `port`, so a hop let through reaches the site, which records it.
fn on_port(redirect: &Redirect, port: u16) -> Redirect {
    let moved = |location: &str| match url::Url::parse(location) {
        Ok(mut url) if url.scheme() == "http" => {
            let _ = url.set_port(Some(port));
            url.to_string()
        }
        _ => location.to_owned(),
    };
    match redirect {
        Redirect::Status { status, location } => Redirect::Status {
            status: *status,
            location: moved(location),
        },
        Redirect::RefreshHeader { location } => Redirect::RefreshHeader {
            location: moved(location),
        },
        Redirect::MetaRefresh { location } => Redirect::MetaRefresh {
            location: moved(location),
        },
        Redirect::Script { location } => Redirect::Script {
            location: moved(location),
        },
    }
}

/// The host a redirect's location names, if it names one.
fn hop_host(redirect: &Redirect) -> Option<String> {
    let (Redirect::Status { location, .. }
    | Redirect::RefreshHeader { location }
    | Redirect::MetaRefresh { location }
    | Redirect::Script { location }) = redirect;
    url::Url::parse(location).ok().and_then(|url| {
        url.host_str()
            .map(|host| host.trim_matches(['[', ']']).to_owned())
    })
}

/// The SSRF table's thin runner on a Borrowed browser (ADR 0067), for the
/// redirect family: the stand-in site answers each case's page with its
/// redirect, every status and kind of redirect and every scheme other than
/// HTTP's, and interception judges the hop as the browser follows it, the
/// hop's name resolved by the engine as the case's `lookups` say and by the
/// browser to loopback, so a hop let through reaches the site, which then
/// stands in for its target. A refresh or a script hops once the page is
/// in, so its refusal is the page's next call's. Positive control: a hop to
/// the site's public name goes through.
///
/// Covers: story:4-5, story:4-14, error:egress_refused
#[test]
fn every_redirect_case_of_the_ssrf_table_is_refused_at_its_hop_on_a_borrowed_browser() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let runner = Runner::committed(
        "borrowed_browser",
        file!(),
        "every_redirect_case_of_the_ssrf_table_is_refused_at_its_hop_on_a_borrowed_browser",
    )
    .unwrap();
    let controls = SsrfTable::parse(&format!(
        r#"
        [[cases]]
        name = "status_to_the_sites_public_name"
        family = "redirect"
        url = "http://redirect.ssrf.test/"
        lookups = [{{ name = "redirect.ssrf.test", answers = ["public"] }}]
        redirect = {{ kind = "status", status = 302, location = "http://{SITE}/" }}
        target = "public"
        refusal = {{ code = "egress_refused", rule = "private_address" }}
        "#
    ))
    .unwrap();
    let mut program = Program::start(&env, &cache, Surface::SdkPython);
    let public = PUBLIC.parse().unwrap();
    let mut sessions: Vec<(Mapping, String)> = Vec::new();

    let result = runner.run(&controls.cases, public, |case| {
        let page = borrowed_page(&mut program, &lent.cdp_url, &mut sessions, mapping_of(case));
        let path = format!("/hop/{}", case.name);
        let Some(redirect) = &case.redirect else {
            return Outcome::connected(Vec::new());
        };
        site.redirect(&path, &on_port(redirect, site.port));
        let first = url::Url::parse(&case.url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .unwrap_or_default();
        let hop = hop_host(redirect);
        let before = site.hosts().len();
        let reached = || {
            site.hosts()
                .iter()
                .skip(before)
                .any(|host| hop.as_deref() == Some(host.as_str()) && *host != first)
        };
        let outcome = |refusal, reached: bool| Outcome {
            refusal,
            connections: if reached {
                case.target_address(public).into_iter().collect()
            } else {
                Vec::new()
            },
        };
        let refusal = |answer: &Value| match error(answer) {
            (code, details) if code == "egress_refused" => {
                serde_json::from_value(details["rule"].clone())
                    .ok()
                    .map(|rule| Refusal::EgressRefused { rule })
            }
            _ => None,
        };
        let went = program.goto(&page, &site.url(&first, &path));
        if went.get("error").is_some() {
            return outcome(refusal(&went), reached());
        }
        // A refresh or a script hops once the page is in.
        let until = Instant::now() + HOP_DEADLINE;
        while Instant::now() < until {
            let read = program.call(
                "page.formats",
                &json!({ "page_id": page, "formats": { "markdown": true } }),
            );
            if let Some(refused) = refusal(&read) {
                return outcome(Some(refused), reached());
            }
            if reached() {
                return outcome(None, true);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        outcome(None, reached())
    });
    for run in 1..sessions.len() {
        let _ = program.call("run.finish", &json!({ "run_id": format!("table-{run}") }));
    }
    program.finish("table-0");
    lent.stop();

    assert_eq!(runner.cases().len(), 15);
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}
