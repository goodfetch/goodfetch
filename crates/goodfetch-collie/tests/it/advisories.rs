//! Regression tests for disclosed security bypasses in comparable projects,
//! each naming its advisory and running at the real layer it hit (ADRs 0017
//! and 0067; ticket 650).
//!
//! ## Contributing: adding a new bypass regression test and fuzz seed
//!
//! When a new bypass or decoder crash is disclosed (in Goodfetch or a
//! comparable project):
//!
//! 1. **Regression test:** add one named test at the layer the bypass hit,
//!    citing its advisory in the function name and doc comment, declaring
//!    `Covers: story:4-92` beside any story/error IDs, and pairing each
//!    refusal with a positive control:
//!    - Browser egress (redirects, rebinding, encoded addresses) and page
//!      secret masking: add to this file
//!      (`crates/goodfetch-collie/tests/it/advisories.rs`) or, for SSRF table
//!      families, `test-support/fixtures/ssrf/table.toml` and
//!      `crates/goodfetch-collie/tests/it/navigation_egress.rs`.
//!    - Local endpoint authentication, `Host` and `Origin` checks: add the row
//!      with `advisories = ["CVE-..."]` to
//!      `test-support/fixtures/endpoint-table.toml` and call
//!      `EndpointRunner::run_advisory` in
//!      `crates/goodfetch-collie/tests/it/endpoint.rs`.
//!    - Webhook / notification egress under `egress.no_private_endpoints`: add
//!      to `crates/goodfetch-notification/tests/it/webhook.rs`.
//!    - Cross-OS in-process egress relay (DNS rebinding, request-head or
//!      ClientHello handling): add to
//!      `crates/goodfetch-page/src/egress_relay/tests.rs`.
//! 2. **Fuzz seed or crasher:** commit the seed under
//!    `crates/<crate>/fuzz/<target>/seeds/` (or a minimised crasher under
//!    `crates/<crate>/fuzz/<target>/crashers/`), replayed on Linux, macOS and
//!    Windows by `goodfetch_test_support::fuzz_replay!` in the crate's unit
//!    tests. Area 4's targets are:
//!    - `crates/goodfetch-engine/fuzz/address_classification/` (`egress.rs`)
//!    - `crates/goodfetch-engine/fuzz/secret_reference/` (`secrets.rs`)
//!    - `crates/goodfetch-engine/fuzz/session_bundle/` (`session_bundle/tests.rs`)
//!    - `crates/goodfetch-engine/fuzz/audit_jsonl/` (`audit.rs`)
//!    - `crates/goodfetch-engine/fuzz/certificate_file/` (`tls.rs`)
//!    - `crates/goodfetch-page/fuzz/relay_request_head/` (`egress_relay.rs`)
//!    - `crates/goodfetch-page/fuzz/relay_client_hello/` (`egress_relay.rs`)
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests run a local HTTP stand-in and watched target on a thread of their own"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the tests run a local HTTP stand-in and watched target outside the engine (ADR 0060)"
)]

use std::io::{BufRead as _, BufReader, ErrorKind, Write as _};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::Surface;
use goodfetch_test_client::{Attached, Hello};
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, Canaries, SecretKind, TestEnv, browser_cache,
};
use serde_json::{Value, json};

use crate::COLLIE;
use crate::page::{BROWSER_DEADLINE, opened};

/// A minimal local HTTP server standing in for a public origin on `127.0.0.1`.
struct LocalHttp {
    port: u16,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl LocalHttp {
    fn start(respond: impl Fn(&str) -> String + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let respond = Arc::new(respond);
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => serve_one(stream, &respond),
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            port,
            stop,
            thread: Some(thread),
        }
    }

    fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for LocalHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve_one(mut stream: TcpStream, respond: &Arc<impl Fn(&str) -> String + Send + Sync>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let mut header = String::new();
    while reader.read_line(&mut header).is_ok() {
        if header == "\r\n" || header == "\n" || header.is_empty() {
            break;
        }
        header.clear();
    }
    let response = respond(path);
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn http_redirect(status: &str, location: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    )
}

fn http_ok(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn started_collie(
    env: &TestEnv,
    run_id: &str,
    settings: &Value,
    variables: &[(&str, &str)],
) -> (Attached, String) {
    let cache = browser_cache(env).unwrap();
    let mut engine = Attached::start_with(env, COLLIE, BROWSER_DEADLINE, |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
        for (name, value) in variables {
            command.env(name, value);
        }
    });
    engine.send(
        &Hello::needing(Product::Collie)
            .surface(Surface::SdkPython)
            .message(),
    );
    let _ = engine.next();
    let started = engine.ask(
        1,
        "run.start",
        &json!({ "run_id": run_id, "settings": settings }),
    );
    assert!(started.get("result").is_some(), "{started}");
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": run_id })));
    (engine, page.page_id.0)
}

fn watched_ipv6_loopback() -> Option<(TcpListener, u16)> {
    let listener = TcpListener::bind("[::1]:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    listener.set_nonblocking(true).ok()?;
    Some((listener, port))
}

fn accepted_any(listener: &TcpListener) -> bool {
    listener.accept().is_ok()
}

/// Firecrawl CVE-2024-56800 (SSRF through an HTTP redirect to cloud metadata
/// or an intranet address): a page navigation to an allowed origin that
/// redirects with HTTP 302 to cloud metadata (`169.254.169.254`) or a watched
/// private listener (`[::1]`, outside the stand-in's `127.0.0.1/32` range) is
/// refused at the browser egress relay with `egress_refused`
/// (`private_address`) and zero connections reach the watched listener, while
/// a redirect to the allowed origin succeeds (positive control; ADR 0017,
/// ADR 0067; ticket 650).
///
/// Covers: story:4-92, story:4-5, error:egress_refused, operation:page.goto
#[test]
fn cve_2024_56800_a_page_redirect_to_metadata_or_an_intranet_is_refused_on_chromium() {
    let env = TestEnv::new();
    let watched = watched_ipv6_loopback();
    let watched_port = watched.as_ref().map(|(_, port)| *port);
    let server = LocalHttp::start(move |path| match path {
        "/to-metadata" => http_redirect("302 Found", "http://169.254.169.254/latest/meta-data/"),
        "/to-intranet" => {
            let target = watched_port.map_or_else(
                || "http://10.0.0.1/admin".to_owned(),
                |port| format!("http://[::1]:{port}/admin"),
            );
            http_redirect("302 Found", &target)
        }
        "/to-public" => http_redirect("302 Found", "/ok"),
        _ => http_ok("<html><body><p>allowed-public</p></body></html>"),
    });
    let base = format!("http://public.advisory.test:{}", server.port());
    let settings = json!({
        "network": { "resolve": { "public.advisory.test": ["127.0.0.1"] } },
        "egress": { "private_ranges": ["127.0.0.1/32"] },
    });
    let (mut engine, page) = started_collie(&env, "cve-2024-56800", &settings, &[]);

    let metadata = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page, "url": format!("{base}/to-metadata") }),
    );
    let intranet = engine.ask(
        4,
        "page.goto",
        &json!({ "page_id": page, "url": format!("{base}/to-intranet") }),
    );
    let control = engine.ask(
        5,
        "page.goto",
        &json!({ "page_id": page, "url": format!("{base}/to-public") }),
    );
    let _ = engine.ask(6, "run.finish", &json!({ "run_id": "cve-2024-56800" }));
    let _ = engine.finish();

    let watched_reached = watched.as_ref().is_some_and(|(l, _)| accepted_any(l));
    assert_eq!(
        (
            &metadata["error"]["data"]["code"],
            &metadata["error"]["data"]["rule"],
            &intranet["error"]["data"]["code"],
            &intranet["error"]["data"]["rule"],
            watched_reached,
            &control["result"]["response"]["status"],
        ),
        (
            &json!("egress_refused"),
            &json!("private_address"),
            &json!("egress_refused"),
            &json!("private_address"),
            false,
            &json!(200),
        ),
        "metadata={metadata} intranet={intranet} control={control}"
    );
}

/// Firecrawl CVE-2026-32857 (SSRF through a redirect bypass to an alternate
/// address encoding): an HTTP 307 redirect to IPv4-mapped IPv6 cloud metadata
/// (`[::ffff:169.254.169.254]`) is refused at the browser egress relay with
/// `egress_refused` (`private_address`), paired with an allowed HTTP 307
/// redirect (positive control). The changed-DNS-answer half runs cross-OS in
/// `goodfetch-page`'s
/// `egress_relay::tests::cve_2026_32857_a_changed_dns_answer_to_metadata_or_intranet_is_refused_at_the_egress_relay`
/// (ADR 0017, ADR 0067; ticket 650).
///
/// Covers: story:4-92, story:4-5, error:egress_refused, operation:page.goto
#[test]
fn cve_2026_32857_a_redirect_bypass_to_encoded_metadata_is_refused_on_chromium() {
    let env = TestEnv::new();
    let server = LocalHttp::start(|path| match path {
        "/to-mapped-metadata" => http_redirect(
            "307 Temporary Redirect",
            "http://[::ffff:169.254.169.254]/latest/meta-data/",
        ),
        "/to-public" => http_redirect("307 Temporary Redirect", "/ok"),
        _ => http_ok("<html><body><p>allowed-307</p></body></html>"),
    });
    let base = format!("http://public.advisory.test:{}", server.port());
    let settings = json!({
        "network": { "resolve": { "public.advisory.test": ["127.0.0.1"] } },
        "egress": { "private_ranges": ["127.0.0.1/32"] },
    });
    let (mut engine, page) = started_collie(&env, "cve-2026-32857", &settings, &[]);

    let bypassed = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page, "url": format!("{base}/to-mapped-metadata") }),
    );
    let control = engine.ask(
        4,
        "page.goto",
        &json!({ "page_id": page, "url": format!("{base}/to-public") }),
    );
    let _ = engine.ask(5, "run.finish", &json!({ "run_id": "cve-2026-32857" }));
    let _ = engine.finish();

    assert_eq!(
        (
            &bypassed["error"]["data"]["code"],
            &bypassed["error"]["data"]["rule"],
            &control["result"]["response"]["status"],
        ),
        (
            &json!("egress_refused"),
            &json!("private_address"),
            &json!(200),
        ),
        "bypassed={bypassed} control={control}"
    );
}

/// Playwright MCP #1566 (a password exposed in a page snapshot): a Secret
/// resolved for the Run that appears in a page's `<input type="password">` and
/// DOM is masked as `%run_state.key_reference%` in `page.formats` (HTML and
/// Markdown snapshots) and `page.evaluate`, never returned raw, while ordinary
/// text on the same page is returned intact (positive control; ADR 0014,
/// ADR 0067; ticket 650).
///
/// Covers: story:4-92, story:4-75, operation:page.formats, operation:page.evaluate
#[test]
fn playwright_mcp_1566_a_password_secret_is_masked_in_page_reads_and_never_returned_raw() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let password = canaries.plant(SecretKind::RunDataKey).value().to_owned();
    env.user_configuration("run_state.key_reference = \"env:GF_TEST_SNAPSHOT_SECRET\"\n");

    let page_secret = password.clone();
    let server = LocalHttp::start(move |_| {
        http_ok(&format!(
            "<html><body><main><p>visible-control</p><p>echo:{page_secret}</p><form><input id=\"pw\" type=\"password\" value=\"{page_secret}\"></form></main></body></html>"
        ))
    });
    let url = format!("http://public.advisory.test:{}/login", server.port());
    let settings = json!({
        "network": { "resolve": { "public.advisory.test": ["127.0.0.1"] } },
        "egress": { "private_ranges": ["127.0.0.1/32"] },
    });
    let (mut engine, page) = started_collie(
        &env,
        "playwright-1566",
        &settings,
        &[("GF_TEST_SNAPSHOT_SECRET", &password)],
    );

    let goto = engine.ask(3, "page.goto", &json!({ "page_id": page, "url": url }));
    assert_eq!(goto["result"]["response"]["status"], json!(200), "{goto}");
    let snapshot = engine.ask(
        4,
        "page.formats",
        &json!({
            "page_id": page,
            "formats": { "html": { "kind": "raw" }, "markdown": true },
        }),
    );
    let evaluated = engine.ask(
        5,
        "page.evaluate",
        &json!({
            "page_id": page,
            "function": "() => document.querySelector('#pw').value",
        }),
    );
    let _ = engine.ask(6, "run.finish", &json!({ "run_id": "playwright-1566" }));
    let _ = engine.finish();

    let html = snapshot["result"]["html"].as_str().unwrap_or_default();
    let markdown = snapshot["result"]["markdown"].as_str().unwrap_or_default();
    let evaluated_value = evaluated["result"]["value"].as_str().unwrap_or_default();
    assert!(
        !html.contains(&password)
            && !markdown.contains(&password)
            && !evaluated_value.contains(&password),
        "raw password leaked: html={html} markdown={markdown} eval={evaluated}"
    );
    assert!(
        html.contains("type=\"password\"")
            && html.contains("%run_state.key_reference%")
            && html.contains("visible-control")
            && markdown.contains("%run_state.key_reference%")
            && markdown.contains("visible-control"),
        "expected masked placeholder and visible control in snapshot: html={html} markdown={markdown}"
    );
    assert_eq!(
        evaluated_value, "%run_state.key_reference%",
        "expected masked input value: {evaluated}"
    );
}
