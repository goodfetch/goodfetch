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

use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_test_support::{
    Answer, BROWSERS_PATH_VARIABLE, Family, Outcome, Process, Refusal, SsrfCase, SsrfTable,
    TestEnv, browser_cache, run_cases,
};
use serde_json::{Map, Value, json};

use crate::attached::{Attached, hello};
use crate::background_traffic::{chrome_for_testing, reported_debugging_port};
use crate::page::{BROWSER_DEADLINE, opened};
use crate::run::call;

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

/// What `cdp_url_not_allowed` asks for.
const FIX: &str = "set the address in Operator policy or in user or project configuration, or give a public address";

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
                std::thread::spawn(move || answer(stream, &recorded));
            }
        });
        Self { port, hosts }
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
/// page; a preconnected socket that sends nothing records nothing.
fn answer(mut stream: TcpStream, recorded: &Mutex<Vec<String>>) {
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
struct Program<'env> {
    engine: Attached<'env>,
    next_id: i64,
}

impl<'env> Program<'env> {
    fn start(env: &'env TestEnv, cache: &Path, surface: &str) -> Self {
        let mut engine = Attached::start_waiting(env, BROWSER_DEADLINE, |command| {
            command.env(BROWSERS_PATH_VARIABLE, cache);
        });
        let mut greeting = hello("collie", "0.1");
        greeting["params"]["surface"] = json!(surface);
        engine.send(&greeting);
        let _ = engine.next();
        Self { engine, next_id: 0 }
    }

    fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        call(&mut self.engine, self.next_id, method, params)
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
    let file = env
        .home()
        .join("0.0")
        .join("runs")
        .join(run_id)
        .join("trace")
        .join("provenance.jsonl");
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
    let mut program = Program::start(&env, &cache, "sdk_python");
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

    let mut program = Program::start(&env, &cache, "sdk_typescript");
    program.run("own", resolve);
    let own = program.borrow("own", &lent.cdp_url);
    let mixed = program.borrow("own", &mixed_url);
    program.finish("own");
    let mut model = Program::start(&env, &cache, "mcp");
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
    let mut program = Program::start(&env, &cache, "sdk_typescript");
    program.run("borrowed-resumed", &[]);
    let session = program.borrow("borrowed-resumed", &lent.cdp_url)["result"]["session_id"].clone();
    let _ = program.new_page(&session);
    program.engine.kill();

    let mut resumed = Program::start(&env, &cache, "sdk_typescript");
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

/// Writes an Operator policy whose `session.source` is the `cdp_url`
/// `url`, and that sets `egress.no_private_endpoints` when `no_private`.
fn operator_policy(env: &TestEnv, url: &str, no_private: bool) {
    let major = env.home().join("0.0");
    std::fs::create_dir_all(&major).unwrap();
    let mut policy = format!(
        "version = \"0.1\"\n\n[settings.session.source]\nkind = \"cdp_url\"\nurl = \"{url}\"\n"
    );
    if no_private {
        policy.push_str("\n[settings.egress]\nno_private_endpoints = true\n");
    }
    std::fs::write(major.join("operator-policy.toml"), policy).unwrap();
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
    let mut model = Program::start(&env, &cache, "mcp");
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
    let mut program = Program::start(&env, &cache, "sdk_python");
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
    let mut program = Program::start(&env, &cache, "sdk_python");
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

/// The host and the path of a case's `http:` URL, which names no port.
fn host_and_path(url: &str) -> (&str, &str) {
    let rest = url.strip_prefix("http://").unwrap();
    rest.split_at(rest.find('/').unwrap_or(rest.len()))
}

/// The SSRF table's thin runner on a Borrowed browser (ADR 0067), for its
/// family: each case's URL on the site's port, its names resolved by the
/// engine as its `lookups` say and by the browser to loopback, so a case
/// let through reaches the site, which then stands in for its target.
/// Positive control: the site's public name goes through.
///
/// Covers: story:4-14, error:egress_refused
#[test]
fn every_borrowed_browser_case_of_the_ssrf_table_is_refused_at_interception() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let table = SsrfTable::committed().unwrap();
    let cases: Vec<&SsrfCase> = table.cases_of(Family::BorrowedBrowser).collect();
    let mut names: Vec<(String, Vec<String>)> = vec![(SITE.to_owned(), vec![PUBLIC.to_owned()])];
    for case in &cases {
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
    }
    let answers: Vec<Vec<&str>> = names
        .iter()
        .map(|(_, answers)| answers.iter().map(String::as_str).collect())
        .collect();
    let resolve: Vec<(&str, &[&str])> = names
        .iter()
        .zip(&answers)
        .map(|((name, _), answers)| (name.as_str(), answers.as_slice()))
        .collect();
    let mut program = Program::start(&env, &cache, "sdk_python");
    program.run("table", &resolve);
    let session = program.borrow("table", &lent.cdp_url)["result"]["session_id"].clone();
    let public = PUBLIC.parse().unwrap();

    let mut run_case = |case: &SsrfCase| {
        let (host, path) = host_and_path(&case.url);
        let before = site.hosts().len();
        let page = program.new_page(&session);
        let went = program.goto(&page, &site.url(host, path));
        let reached = site.hosts().len() > before;
        let refusal = match error(&went) {
            (code, details) if code == "egress_refused" => {
                serde_json::from_value(details["rule"].clone())
                    .ok()
                    .map(|rule| Refusal::EgressRefused { rule })
            }
            _ => None,
        };
        let target = if host == SITE {
            Some(public)
        } else {
            case.target_address(public)
        };
        Outcome {
            refusal,
            connections: if reached {
                target.into_iter().collect()
            } else {
                Vec::new()
            },
        }
    };
    let result = run_cases(cases.iter().copied(), public, &mut run_case);
    let control = run_case(&SsrfCase {
        url: format!("http://{SITE}/"),
        ..cases[0].clone()
    });
    program.finish("table");
    lent.stop();

    assert_eq!(
        (
            result.map_err(|let_through| let_through.to_string()),
            control.refusal,
            control.connections
        ),
        (Ok(()), None, vec![public])
    );
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
    let mut program = Program::start(&env, &cache, "sdk_python");
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
        Self { socket, next_id: 0 }
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
            if let tungstenite::Message::Text(text) = self.socket.read().unwrap() {
                let reply: Value = serde_json::from_str(&text).unwrap();
                if reply["id"] == json!(self.next_id) {
                    assert!(reply.get("error").is_none(), "{method}: {reply}");
                    return reply["result"].clone();
                }
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
/// while the Session's own page keeps the engine's (positive control).
///
/// Covers: story:4-14, operation:run.new_session, operation:session.new_page
#[test]
fn a_borrowed_browser_lets_go_of_the_persons_own_tabs() {
    let env = TestEnv::new();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let site = Site::start();
    let mut program = Program::start(&env, &cache, "sdk_python");
    program.run("tabs", &[(SITE, &[PUBLIC])]);
    let session = program.borrow("tabs", &lent.cdp_url)["result"]["session_id"].clone();
    let page = program.new_page(&session);
    let went = program.goto(&page, &site.url(SITE, "/session"));

    let mut person = Person::connect(&lent.cdp_url);
    let mine = person.send(
        "Target.createTarget",
        &json!({ "url": site.url(SITE, "/mine") }),
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
    let opened = person.send(
        "Runtime.evaluate",
        &json!({
            "expression": format!("window.open({:?}) !== null", site.url(SITE, "/opened")),
            "userGesture": true,
        }),
        Some(&attached),
    );
    let pages = person.pages_once(|pages| {
        pages.iter().any(|target| at(target, "/opened"))
            && pages
                .iter()
                .filter(|target| !at(target, "/mine") && !at(target, "/session"))
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
            &json!(true),
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
    let mut program = Program::start(&env, &cache, "sdk_python");
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
    std::thread::sleep(Duration::from_secs(2));
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
