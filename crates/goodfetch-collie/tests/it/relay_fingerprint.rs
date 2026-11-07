//! The egress relay's terminating mode shows the site the browser's own
//! fingerprint (spec #564, ADR 0042's conformance comparison): on Chromium's
//! pinned build, the JA4 and the HTTP/2 settings the Proving ground observes
//! through the relay equal those of the same build connecting directly, and
//! the relay resumes the upstream TLS session of a returning connection
//! within a Session alone. The Proving ground's `fetch-edge.test` hosts are
//! terminated by trusting its authority for them (`tls.trusted_cas`); its
//! observations are the oracle.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the direct case launches the pinned build itself and waits for it on the test's own thread"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use serde_json::{Value, json};

use crate::egress_relay::{Program, started};

/// The Proving ground's own seed for these tests.
const SEED: u64 = 639;

/// The signature algorithms Chrome for Testing 156 offers that the relay's
/// pinned BoringSSL cannot send (ML-DSA, `0x0904`–`0x0906`): #1624 drops
/// them from this comparison once it can.
const SIGNATURES_NOT_SENDABLE: [u64; 3] = [0x0904, 0x0905, 0x0906];

/// The extensions left out of the comparison: those a resumed handshake
/// adds (`pre_shared_key`, `early_data`), and one Chrome for Testing 156
/// sends that the relay's pinned BoringSSL does not know (`0x12e0`), which
/// #1624 drops from here once it can.
const EXTENSIONS_LEFT_OUT: [u64; 3] = [41, 42, 0x12e0];

/// How long the directly connecting build may take to load its page.
const DIRECT_DEADLINE: Duration = Duration::from_secs(60);

/// The user configuration trusting the Proving ground's authority for the
/// `fetch-edge.test` hosts, so the relay terminates them.
fn trusting(env: &TestEnv, proving_ground: &ProvingGround) {
    let ca = proving_ground.description().ca.display().to_string();
    env.user_configuration(&format!(
        "tls.trusted_cas = {{ \"fetch-edge.test\" = [{}] }}\n",
        json!(ca)
    ));
}

/// The `https://` URL of `path` on `host` at the Proving ground.
fn secure(proving_ground: &ProvingGround, host: &str, path: &str) -> String {
    format!("https://{host}:{}{path}", proving_ground.address().port())
}

/// The pinned build's executable in the test's Browser cache.
fn pinned_executable(cache: &Path) -> PathBuf {
    let names = ["chrome", "chrome.exe", "Google Chrome for Testing"];
    let mut folders = vec![cache.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else if names
                .iter()
                .any(|name| path.file_name().is_some_and(|file| file == *name))
            {
                return path;
            }
        }
    }
    panic!("no pinned build in {}", cache.display());
}

/// Loads `url` in the pinned build launched by the test, connecting to the
/// Proving ground directly, with no relay and no engine, and told to take
/// its certificate.
fn load_directly(env: &TestEnv, proving_ground: &ProvingGround, url: &str) {
    let executable = pinned_executable(&browser_cache(env).unwrap());
    let profile = env.home().join("direct-profile");
    let mut browser = Command::new(executable)
        .args([
            "--headless=new",
            "--ignore-certificate-errors",
            "--use-mock-keychain",
            "--password-store=basic",
            "--no-first-run",
            "--no-default-browser-check",
            "--dump-dom",
        ])
        .arg(format!(
            "--host-resolver-rules=MAP *.test {}",
            proving_ground.address().ip()
        ))
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(url)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while browser.try_wait().unwrap().is_none() {
        if started.elapsed() > DIRECT_DEADLINE {
            let _ = browser.kill();
            let _ = browser.wait();
            panic!("the directly connecting build did not load {url}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// What a request shows the site of its client, compared between the
/// relay and the browser connecting directly: each value JA4 is computed
/// from (the TLS version, whether a server name was sent, the ALPN list,
/// the cipher suites and extensions as JA4 sorts them, and the signature
/// algorithms in order), but those [`EXTENSIONS_LEFT_OUT`] and
/// [`SIGNATURES_NOT_SENDABLE`] name, and its protocol, HTTP/2 settings,
/// first window update and pseudo-header order.
fn shown(request: &Value) -> Value {
    let numbers = |values: &Value, left_out: &[u64]| -> Vec<u64> {
        values
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .filter(|value| !left_out.contains(value))
            .collect()
    };
    let sorted = |mut values: Vec<u64>| {
        values.sort_unstable();
        values
    };
    let tls = &request["tls"];
    json!({
        "protocol": request["protocol"],
        "version": tls["version"],
        "sni": tls["sni"].is_string(),
        "alpn": tls["alpn"],
        "ciphers": sorted(numbers(&tls["ciphers"], &[])),
        "extensions": sorted(numbers(&tls["extensions"], &EXTENSIONS_LEFT_OUT)),
        "signature_algorithms": numbers(&tls["signature_algorithms"], &SIGNATURES_NOT_SENDABLE),
        "http2_settings": request["http2"]["settings"],
        "window_update": request["http2"]["window_update"],
        "pseudo_header_order": request["pseudo_header_order"],
    })
}

/// The requests the Proving ground's `host` received for `target`.
fn requests_for(proving_ground: &ProvingGround, host: &str, target: &str) -> Vec<Value> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.host.split(':').next() == Some(host) && request.target == target)
        .map(|request| serde_json::to_value(request).unwrap())
        .collect()
}

/// ADR 0042's conformance comparison on Chromium's pinned build: the
/// fingerprint a terminated host sees through the relay (its ClientHello's
/// values and its HTTP/2 settings, which the relay forwards unchanged)
/// equals the one the same build shows connecting directly. The relay's
/// ALPS payload is the build's captured one, compared in process
/// (`terminate_tests`), since the Proving ground accepts no ALPS.
///
/// Covers: story:4-39, setting:tls.trusted_cas, operation:page.goto
#[test]
fn a_terminated_host_sees_the_fingerprint_the_build_shows_connecting_directly() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    load_directly(
        &env,
        &proving_ground,
        &secure(&proving_ground, "fetch-edge.test", "/robots.txt"),
    );
    // Taken before the Run starts: once its document loads, the Run reads
    // the origin's robots.txt itself (spec #580), through the relay, and
    // that read may arrive before the Run finishes.
    let direct = requests_for(&proving_ground, "fetch-edge.test", "/robots.txt");
    trusting(&env, &proving_ground);
    let mut program = started(&env, &proving_ground, "relayed", &json!({}), &[]);
    let page = program.new_page("relayed");
    let answer = program.goto(
        &page,
        &secure(&proving_ground, "fetch-edge.test", "/target"),
    );
    program.finish("relayed");

    let relayed = requests_for(&proving_ground, "fetch-edge.test", "/target");
    assert_eq!(
        answer["result"]["response"]["status"],
        json!(200),
        "{answer}"
    );
    assert_eq!(
        (direct.len(), relayed.len(), &direct[0]["protocol"]),
        (1, 1, &json!("h2"))
    );
    assert_eq!(shown(&relayed[0]), shown(&direct[0]));
}

/// The relay keeps a Session's upstream TLS sessions, as a returning
/// browser resumes, and drops them with the Session: once a Session's
/// first connection to a host is done, each later one resumes its session
/// upstream (a returning navigation needs a new connection, since the
/// site's `GOAWAY` ends the first), while a new Session's first connection
/// to the same host, the one its navigation takes, resumes nothing. That
/// connection is found by the navigation it carried, not by when its
/// handshake arrived: one the first Session had in flight as it closed, a
/// favicon's retry say, may reach the site after `session.close` answered.
///
/// Covers: story:4-39, operation:run.new_session, operation:session.close
#[test]
fn a_returning_navigation_resumes_the_upstream_tls_session_and_a_new_session_does_not() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    trusting(&env, &proving_ground);
    let mut program = started(&env, &proving_ground, "resuming", &json!({}), &[]);
    let url = secure(&proving_ground, "goaway.fetch-edge.test", "/target");

    let first = session(&mut program, "resuming");
    let page = page_in(&mut program, &first);
    let statuses: Vec<Value> = (0..2)
        .map(|_| program.goto(&page, &url)["result"]["response"]["status"].clone())
        .collect();
    let closed = program.call("session.close", &json!({ "session_id": first }));
    let in_first = resumed(&proving_ground);
    let second = session(&mut program, "resuming");
    let page = page_in(&mut program, &second);
    let next = program.goto(&page, &url)["result"]["response"]["status"].clone();
    program.finish("resuming");

    assert_eq!(
        (statuses, closed.get("error"), next),
        (vec![json!(200), json!(200)], None, json!(200))
    );
    assert!(in_first.len() >= 2, "a returning connection: {in_first:?}");
    assert_eq!(
        (in_first[0], in_first[1..].iter().all(|resumed| *resumed)),
        (false, true),
        "the first Session's handshakes: {in_first:?}"
    );
    assert_eq!(
        navigation_resumed(&proving_ground),
        Some(false),
        "the second Session's navigation"
    );
}

/// Whether each TLS handshake `goaway.fetch-edge.test` received so far
/// resumed a session, in order.
fn resumed(proving_ground: &ProvingGround) -> Vec<bool> {
    proving_ground.observations().unwrap()["tls_handshakes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|handshake| handshake["sni"] == json!("goaway.fetch-edge.test"))
        .map(|handshake| handshake["resumed"] == json!(true))
        .collect()
}

/// Whether the connection that carried the latest navigation
/// `goaway.fetch-edge.test` answered resumed a session upstream.
fn navigation_resumed(proving_ground: &ProvingGround) -> Option<bool> {
    let navigation = requests_for(proving_ground, "goaway.fetch-edge.test", "/target")
        .into_iter()
        .rfind(|request| request["status"] == json!(200))?;
    proving_ground.observations().unwrap()["tls_handshakes"]
        .as_array()?
        .iter()
        .find(|handshake| {
            handshake["sni"] == json!("goaway.fetch-edge.test")
                && handshake["connection"] == navigation["connection"]
        })
        .map(|handshake| handshake["resumed"] == json!(true))
}

/// A new Session of the Run `run_id`.
fn session(program: &mut Program, run_id: &str) -> Value {
    let answer = program.call("run.new_session", &json!({ "run_id": run_id }));
    answer["result"]["session_id"].clone()
}

/// A new page in the Session `session`.
fn page_in(program: &mut Program, session: &Value) -> String {
    let answer = program.call("session.new_page", &json!({ "session_id": session }));
    answer["result"]["page_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{answer}"))
        .to_owned()
}
