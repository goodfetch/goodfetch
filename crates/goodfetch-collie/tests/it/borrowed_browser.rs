//! A Borrowed browser under the egress policy over stdio (spec #563's
//! stories 14 and 15; ADRs 0017, 0056 and 0115), against a Chrome for
//! Testing the test starts with a debugging port and the Proving ground,
//! whose request record is the oracle. The test's browser resolves every
//! name to this host's loopback, where the Proving ground and the stand-ins
//! are, while the engine resolves each host itself through the Run's
//! `network.resolve`: a name the engine finds public reaches the Proving
//! ground, and one it finds private, or a private address, is refused at
//! interception before the browser connects. The two resolutions are the
//! rebinding gap Provenance records.
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
    reason = "the tests start their own browser, read its port and the Run trace, and count a stand-in's connections on their own thread, never an async worker"
)]

use std::net::TcpListener;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use goodfetch_engine::pinned_download::chromium_background_traffic_off;
use goodfetch_test_support::{
    Answer, BROWSERS_PATH_VARIABLE, Family, Outcome, Process, ProvingGround, Refusal, SsrfCase,
    SsrfTable, TestEnv, browser_cache, run_cases,
};
use serde_json::{Map, Value, json};

use crate::attached::{Attached, hello};
use crate::background_traffic::{chrome_for_testing, reported_debugging_port};
use crate::page::{BROWSER_DEADLINE, opened};
use crate::run::call;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 633;

/// What the engine resolves every Proving ground host to: a globally
/// reachable address, which the test's browser never connects to, since
/// it resolves every name to loopback itself.
const PUBLIC: &str = "93.184.215.14";

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

    /// Starts the Run `run_id` whose `network.resolve` is `resolve`.
    fn run(&mut self, run_id: &str, resolve: &Map<String, Value>) {
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

/// `network.resolve`, each Proving ground host mapped to [`PUBLIC`] and
/// each of `more` to its answers.
fn resolve(proving_ground: &ProvingGround, more: &[(&str, &[&str])]) -> Map<String, Value> {
    let mut mapped: Map<String, Value> = proving_ground
        .description()
        .resolve
        .keys()
        .map(|host| (host.clone(), json!([PUBLIC])))
        .collect();
    for (host, answers) in more {
        mapped.insert((*host).to_owned(), json!(answers));
    }
    mapped
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

/// The hosts of the requests the Proving ground received, the browser's
/// own for an icon left out.
fn hosts_requested(proving_ground: &ProvingGround) -> Vec<String> {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|request| request["target"] != "/favicon.ico")
        .map(|request| request["host"].as_str().unwrap_or_default().to_owned())
        .collect()
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
/// address, or for a private address, is refused at interception, and the
/// Proving ground never sees the request, which the browser would have
/// sent it; a name the engine finds public reaches it (positive control).
/// The Session's Provenance records that interception alone holds it, and
/// the rebinding gap.
///
/// Covers: story:4-14, error:egress_refused, operation:run.new_session
#[test]
fn a_borrowed_browser_is_refused_a_private_address_at_interception_and_provenance_names_the_gap() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let mut program = Program::start(&env, &cache, "sdk_python");
    program.run(
        "borrowed",
        &resolve(&proving_ground, &[("internal.test", &["10.0.0.1"])]),
    );
    let borrowed = program.borrow("borrowed", &lent.cdp_url);
    let session = borrowed["result"]["session_id"].clone();
    let page = program.new_page(&session);
    let port = proving_ground.address().port();

    let public = program.goto(&page, &proving_ground.url("catalog.test", "/catalog/1"));
    let private = program.goto(&page, &proving_ground.url("internal.test", "/catalog/1"));
    let loopback = program.goto(&page, &format!("http://127.0.0.1:{port}/catalog/1"));
    let requested = hosts_requested(&proving_ground);
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
                    "url": proving_ground.url("internal.test", "/catalog/1"),
                    "rule": "private_address",
                    "address": "10.0.0.1",
                    "setting": "egress.private_ranges",
                })
            ),
            (
                json!("egress_refused"),
                json!({
                    "url": format!("http://127.0.0.1:{port}/catalog/1"),
                    "rule": "private_address",
                    "address": "127.0.0.1",
                    "setting": "egress.private_ranges",
                })
            ),
            vec!["catalog.test".to_owned()],
            vec![json!({
                "action": "interception_only",
                "session_id": session,
                "egress": "interception_only",
                "gap": "dns_rebinding",
            })],
        )
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
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let (port, accepted) = stand_in();
    let stand_in_url = format!("ws://127.0.0.1:{port}/devtools/browser/stand-in");
    let mixed_url = format!("ws://debugger.test:{port}/devtools/browser/stand-in");
    let resolved = resolve(
        &proving_ground,
        &[("debugger.test", &["127.0.0.1", "10.0.0.9"])],
    );

    let mut program = Program::start(&env, &cache, "sdk_typescript");
    program.run("own", &resolved);
    let own = program.borrow("own", &lent.cdp_url);
    let mixed = program.borrow("own", &mixed_url);
    program.finish("own");
    let mut model = Program::start(&env, &cache, "mcp");
    model.run("model", &resolved);
    let refused = model.borrow("model", &stand_in_url);
    model.finish("model");
    lent.stop();

    let fix = "set the address in Operator policy or in user or project configuration, or give a public address";
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
                    "fix": fix,
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
                    "fix": fix,
                })
            ),
            0,
        ),
        "{own}"
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
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    operator_policy(&env, &lent.cdp_url, false);
    let mut model = Program::start(&env, &cache, "mcp");
    model.run("configured", &resolve(&proving_ground, &[]));

    let page = model.call("run.new_page", &json!({ "run_id": "configured" }));
    let went = model.goto(
        &opened(&page).page_id.0,
        &proving_ground.url("catalog.test", "/catalog/1"),
    );
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
    program.run("configured", &Map::new());

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
                    "fix": "set the address in Operator policy or in user or project configuration, or give a public address",
                })
            ),
            0
        )
    );
}

/// The host and the path of a case's `http:` URL, which names no port.
fn host_and_path(url: &str) -> (&str, &str) {
    let rest = url.strip_prefix("http://").unwrap();
    rest.split_at(rest.find('/').unwrap_or(rest.len()))
}

/// The SSRF table's thin runner on a Borrowed browser (ADR 0067), for its
/// family: each case's URL on the Proving ground's port, its names
/// resolved by the engine as its `lookups` say and by the browser to
/// loopback, so a case let through reaches the Proving ground, which then
/// stands in for its target. Positive control: a public name of the
/// Proving ground goes through.
///
/// Covers: story:4-14, error:egress_refused
#[test]
fn every_borrowed_browser_case_of_the_ssrf_table_is_refused_at_interception() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let lent = Lent::start(&env, &cache);
    let table = SsrfTable::committed().unwrap();
    let cases: Vec<&SsrfCase> = table.cases_of(Family::BorrowedBrowser).collect();
    let mut resolved = resolve(&proving_ground, &[]);
    for case in &cases {
        for lookup in &case.lookups {
            let answers: Vec<String> = lookup
                .answers
                .iter()
                .map(|answer| match answer {
                    Answer::Address(address) => address.to_string(),
                    Answer::Public => PUBLIC.to_owned(),
                })
                .collect();
            resolved.insert(lookup.name.clone(), json!(answers));
        }
    }
    let mut program = Program::start(&env, &cache, "sdk_python");
    program.run("table", &resolved);
    let session = program.borrow("table", &lent.cdp_url)["result"]["session_id"].clone();
    let port = proving_ground.address().port();
    let public = PUBLIC.parse().unwrap();

    let mut run_case = |case: &SsrfCase| {
        let (host, path) = host_and_path(&case.url);
        let url = format!("http://{host}:{port}{path}");
        let before = hosts_requested(&proving_ground).len();
        let page = program.new_page(&session);
        let went = program.goto(&page, &url);
        let reached = hosts_requested(&proving_ground).len() > before;
        let refusal = match error(&went) {
            (code, details) if code == "egress_refused" => {
                serde_json::from_value(details["rule"].clone())
                    .ok()
                    .map(|rule| Refusal::EgressRefused { rule })
            }
            _ => None,
        };
        let target = if host == "catalog.test" {
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
        url: "http://catalog.test/catalog/1".to_owned(),
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
