//! The egress relay on Chromium over stdio (spec #564; ADRs 0017, 0042
//! and 0044): every browser Session the engine launches reaches the network
//! through a loopback relay of its own, which resolves each name itself and
//! judges the address it connects to, so a page reaches no private address
//! whatever the browser's own resolution said. The Proving ground's
//! connection record is the oracle: a refused connection opens none to its
//! target. The relay takes only its Session's credentials, chains a
//! Session's upstream proxy, and leaves the browser's TLS handshake as the
//! browser wrote it; with `security.raw_cdp_egress_proxy` off the Session
//! connects directly and Provenance names the rebinding gap.
//!
//! The Proving ground serves on loopback through the Run's
//! `network.resolve` mapping, whose answers the relay judges as any other:
//! each Run lists the mapped hosts in `egress.private_ranges`, so
//! `localhost` and literal addresses, resolved and judged as any name is,
//! stay refused.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the Run trace and run the ClientHello stand-in on their own threads, never an async worker"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the tests connect to the relay as another local process and listen for the browser's ClientHello on their own threads (ADR 0060)"
)]

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::home::{RUN_TRACE_PROVENANCE, RUN_TRACE_REQUESTS};
use goodfetch_test_client::{HomeLayout, robots_txt_read};
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache, cli};
use rstest::rstest;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

use crate::page::{opened, started_with};
use goodfetch_test_client::Attached;

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The Proving ground's own seed for these tests.
const SEED: u64 = 634;

/// Longer than Chromium's first reload of a page left on a network error
/// page, one second after the error page commits.
pub(crate) const PAST_AUTO_RELOAD: Duration = Duration::from_secs(2);

/// A Connection's calls, each with the next ID.
pub(crate) struct Program {
    engine: Attached,
    next_id: i64,
}

impl Program {
    pub(crate) fn call(&mut self, method: &str, params: &Value) -> Value {
        self.next_id += 1;
        self.engine.ask(self.next_id, method, params)
    }

    pub(crate) fn new_page(&mut self, run_id: &str) -> String {
        opened(&self.call("run.new_page", &json!({ "run_id": run_id })))
            .page_id
            .0
    }

    pub(crate) fn goto(&mut self, page: &str, url: &str) -> Value {
        self.call("page.goto", &json!({ "page_id": page, "url": url }))
    }

    pub(crate) fn finish(mut self, run_id: &str) {
        let _ = self.call("run.finish", &json!({ "run_id": run_id }));
        let _ = self.engine.finish();
    }
}

/// An Attached engine and its Run `run_id`, with `settings` beside the
/// Proving ground's `network.resolve` mapping, and `variables` in the
/// engine's environment.
pub(crate) fn started(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    settings: &Value,
    variables: &[(&str, &str)],
) -> Program {
    let cache = browser_cache(env).unwrap();
    let settings: Map<String, Value> = settings.as_object().cloned().unwrap_or_default();
    let engine = started_with(env, &cache, proving_ground, run_id, &settings, |command| {
        for (name, value) in variables {
            command.env(name, value);
        }
    });
    Program {
        engine,
        next_id: 10,
    }
}

/// The Error code of a call's error and its details.
pub(crate) fn error(answer: &Value) -> (Value, Value) {
    let mut details = answer["error"]["data"].clone();
    let code = details["code"].take();
    (code, details)
}

/// The Proving ground's watched target, which answers any plain request
/// and whose every connection the record lists.
fn target(proving_ground: &ProvingGround) -> SocketAddr {
    proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap()
}

/// How many connections reached the watched target.
fn target_connections(proving_ground: &ProvingGround) -> usize {
    proving_ground.observations().unwrap()["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|connection| connection["listener"] == json!("target"))
        .count()
}

/// The requests the Proving ground's sites received for a page, not the
/// browser's own for an icon, as `(host, target)`.
fn requests(proving_ground: &ProvingGround) -> Vec<(String, String)> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .map(|request| {
            (
                request.host.as_str().to_owned(),
                request.target.as_str().to_owned(),
            )
        })
        .filter(|(_, target)| target != "/favicon.ico")
        .collect()
}

/// A file of the Run `run_id`'s trace, one JSON value a line.
fn trace(env: &TestEnv, run_id: &str, file: &str) -> Vec<Value> {
    let file = HomeLayout::of(env.home()).run_trace(run_id, file);
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The Run `run_id`'s Provenance entries of `action`.
pub(crate) fn provenance(env: &TestEnv, run_id: &str, action: &str) -> Vec<Value> {
    trace(env, run_id, RUN_TRACE_PROVENANCE)
        .into_iter()
        .filter(|entry| entry["action"] == json!(action))
        .collect()
}

/// The failures the Run trace recorded for requests to `host` no call
/// waited on, each as its method and Error code.
fn failed_requests(env: &TestEnv, run_id: &str, host: &str) -> Vec<(Value, Value)> {
    trace(env, run_id, RUN_TRACE_REQUESTS)
        .into_iter()
        .filter(|line| line["event"] == json!("page.request_failed"))
        .filter(|line| {
            line["data"]["url"]
                .as_str()
                .and_then(|url| url.split_once("://"))
                .is_some_and(|(_, rest)| rest.split(['/', ':', '?']).next() == Some(host))
        })
        .map(|line| {
            (
                line["data"]["method"].clone(),
                line["data"]["failure"]["code"].clone(),
            )
        })
        .collect()
}

/// A page that names `localhost`, which the system resolver answers with
/// loopback, reaches no private address through the relay: the navigation
/// raises `egress_refused` naming the address, the watched target sees no
/// connection, and the Run trace records the refusal. Positive control: a
/// page the Run's mapping names goes through the same relay to its site.
///
/// Covers: story:4-10, story:4-8, error:egress_refused, operation:page.goto
#[test]
fn on_chromium_a_page_reaches_no_private_address_through_the_relay() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = target(&proving_ground).port();
    let mut program = started(&env, &proving_ground, "relayed", &json!({}), &[]);
    let page = program.new_page("relayed");
    let site = proving_ground.url("catalog.test", "/catalog/1");

    let reached = program.goto(&page, &site);
    let refused = program.goto(&page, &format!("http://localhost:{port}/private"));
    let connections = target_connections(&proving_ground);
    program.finish("relayed");

    assert_eq!(
        &reached["result"]["response"]["url"],
        &json!(site),
        "{reached}"
    );
    let (code, details) = error(&refused);
    assert_eq!(
        (code, &details["rule"], &details["url"]),
        (
            json!("egress_refused"),
            &json!("private_address"),
            &json!(format!("http://localhost:{port}/private"))
        ),
        "{refused}"
    );
    assert!(
        ["127.0.0.1", "::1"].contains(&details["address"].as_str().unwrap_or_default()),
        "{details}"
    );
    assert_eq!(connections, 0);
    // The browser may send a request whose connection closed unanswered
    // once more: each is refused.
    let failed = failed_requests(&env, "relayed", "localhost");
    assert!(
        !failed.is_empty()
            && failed
                .iter()
                .all(|failed| *failed == (json!("GET"), json!("egress_refused"))),
        "{failed:?}"
    );
    assert_eq!(provenance(&env, "relayed", "egress_relay").len(), 1);
}

/// A page whose navigation the relay refused stays on its error page, so
/// the page's next call raises its own refusal, never that earlier one
/// (fix-queue 92, where Chromium's reload of the error page, a second
/// after it, left the link-local refusal for the next call): a navigation
/// to an IPv6 link-local address is refused at the relay, and the page's
/// next navigation, to a URL that does not parse, is refused by the domain
/// policy naming that URL.
///
/// Covers: error:egress_refused, operation:page.goto
#[test]
fn after_a_refused_link_local_navigation_the_pages_next_navigation_raises_its_own_refusal() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = target(&proving_ground).port();
    let mut program = started(&env, &proving_ground, "link-local", &json!({}), &[]);
    let page = program.new_page("link-local");
    let link_local = format!("http://[fe80::1]:{port}/");

    let (first, first_details) = error(&program.goto(&page, &link_local));
    // Nothing to wait on: the page stays as it is, which only time shows.
    std::thread::sleep(PAST_AUTO_RELOAD);
    let (next, next_details) = error(&program.goto(&page, "http://[::1/"));
    program.finish("link-local");

    assert_eq!(
        [
            (first, first_details["url"].clone()),
            (next, next_details["url"].clone())
        ],
        [
            (json!("egress_refused"), json!(link_local)),
            (json!("egress_refused"), json!("http://[::1/")),
        ],
        "{first_details} {next_details}"
    );
    assert_eq!(
        &next_details["rule"],
        &json!("domain_policy"),
        "{next_details}"
    );
}

/// What a client that is not the Session's browser meets at the relay's
/// loopback port: the status of the relay's answer to `request`.
fn answered_status(relay: &str, request: &str) -> String {
    let mut stream = TcpStream::connect(relay).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    String::from_utf8_lossy(&answer)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Another local process that finds the relay's port gets a `407` and no
/// relayed byte, with no credentials or with wrong ones, for a tunnel or a
/// plain request, while the Session itself goes through.
///
/// Covers: story:4-10, operation:page.goto
#[test]
fn another_local_process_without_the_sessions_credentials_is_refused_and_the_session_works() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut program = started(&env, &proving_ground, "relayed", &json!({}), &[]);
    let page = program.new_page("relayed");
    let site = proving_ground.url("catalog.test", "/catalog/1");
    let reached = program.goto(&page, &site);
    let relay = provenance(&env, "relayed", "egress_relay")[0]["address"]
        .as_str()
        .unwrap()
        .to_owned();
    let authority = site
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .unwrap()
        .to_owned();
    // The document's robots.txt read, which its navigation never waits
    // for (ADR 0086), has reached the site: what the site sees from here
    // on came through the relay from the other process.
    robots_txt_read(&env, "relayed");
    let before = requests(&proving_ground).len();

    let answers: Vec<String> = [
        format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n"),
        format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic Z29vZGZldGNoOmd1ZXNz\r\n\r\n"
        ),
        format!("GET {site} HTTP/1.1\r\nHost: {authority}\r\n\r\n"),
    ]
    .iter()
    .map(|request| answered_status(&relay, request))
    .collect();
    let after = requests(&proving_ground);
    program.finish("relayed");

    assert_eq!(
        &reached["result"]["response"]["url"],
        &json!(site),
        "{reached}"
    );
    assert_eq!(
        answers,
        vec!["HTTP/1.1 407 Proxy Authentication Required"; 3]
    );
    assert_eq!(after.get(before..), Some(&[][..]), "{after:?}");
}

/// The variable the engine's environment holds a proxy password in.
const PASSWORD_VARIABLE: &str = "PROVING_GROUND_PROXY_PASSWORD";

/// The relays the Proving ground's proxies carried out, each as its proxy
/// and target.
fn relays(proving_ground: &ProvingGround) -> Vec<(String, String)> {
    proving_ground.observations().unwrap()["relays"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|relay| relay["outcome"] == json!("relayed"))
        .map(|relay| {
            (
                relay["proxy"].as_str().unwrap_or_default().to_owned(),
                relay["target"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// A Session with an upstream proxy, HTTP or SOCKS5 with credentials, is
/// chained through it: its page reaches the site through the proxy, and a
/// page naming a private address is refused before the proxy sees it.
///
/// Covers: story:4-10, story:4-12, error:egress_refused, setting:proxy.server
#[cfg(unix)]
#[rstest]
#[case::http("http", "open", false)]
/// Covers: setting:proxy.username, setting:proxy.password
#[case::socks5_with_credentials("socks5", "socks5", true)]
fn a_session_with_an_upstream_proxy_is_chained_through_it_and_the_address_check_holds(
    #[case] scheme: &str,
    #[case] name: &str,
    #[case] credentials: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = target(&proving_ground).port();
    let fixtures = &proving_ground.description().fixtures;
    let mut proxy = json!({ "server": format!("{scheme}://{}", fixtures.proxies[name]) });
    if credentials {
        proxy["username"] = json!(fixtures.proxy_credentials.username);
        proxy["password"] = json!(format!("env:{PASSWORD_VARIABLE}"));
    }
    let password = fixtures.proxy_credentials.password.clone();
    let mut program = started(
        &env,
        &proving_ground,
        "chained",
        &json!({ "proxy": proxy }),
        &[(PASSWORD_VARIABLE, &password)],
    );
    let page = program.new_page("chained");
    let site = proving_ground.url("catalog.test", "/catalog/1?mark=chained");

    let reached = program.goto(&page, &site);
    let relayed = relays(&proving_ground);
    let refused = program.goto(&page, &format!("http://localhost:{port}/private"));
    let to_target: Vec<_> = relays(&proving_ground)
        .into_iter()
        .filter(|(_, relayed)| relayed.contains("localhost") || relayed.contains(&port.to_string()))
        .collect();
    let connections = target_connections(&proving_ground);
    program.finish("chained");

    assert_eq!(
        &reached["result"]["response"]["url"],
        &json!(site),
        "{reached}"
    );
    assert!(
        relayed
            .iter()
            .any(|(proxy, target)| proxy == name && target.contains("catalog.test")),
        "{relayed:?}"
    );
    assert_eq!(
        (error(&refused).0, &error(&refused).1["rule"]),
        (json!("egress_refused"), &json!("private_address")),
        "{refused}"
    );
    assert_eq!((to_target, connections), (Vec::new(), 0));
}

/// With `security.raw_cdp_egress_proxy` off the Session connects directly:
/// the same page naming `localhost` reaches the watched target, the switch
/// is recorded in the Audit log (A04-03), and Provenance names the
/// rebinding gap, as on a Borrowed browser.
///
/// Covers: story:4-10, setting:security.raw_cdp_egress_proxy, operation:page.goto
#[test]
fn with_the_switch_off_the_session_connects_directly_and_provenance_names_the_gap() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = target(&proving_ground).port();
    let mut program = started(
        &env,
        &proving_ground,
        "direct",
        &json!({ "security": { "raw_cdp_egress_proxy": false } }),
        &[],
    );
    let page = program.new_page("direct");

    let _ = program.goto(&page, &format!("http://localhost:{port}/direct"));
    let connections = target_connections(&proving_ground);
    program.finish("direct");
    let exported = cli(&env, COLLIE)
        .args(["audit", "export"])
        .run()
        .unwrap()
        .stdout;

    assert!(connections >= 1, "{connections}");
    assert_eq!(
        provenance(&env, "direct", "interception_only")
            .iter()
            .map(|entry| (entry["egress"].clone(), entry["gap"].clone()))
            .collect::<Vec<_>>(),
        vec![(json!("interception_only"), json!("dns_rebinding"))]
    );
    assert_eq!(
        provenance(&env, "direct", "egress_relay"),
        Vec::<Value>::new()
    );
    let switched: Vec<Value> = exported
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|entry| {
            entry["run_id"] == json!("direct")
                && entry["kind"] == json!("security_switch_turned_off")
        })
        .map(|entry| entry["details"]["setting"].clone())
        .collect();
    assert_eq!(switched, vec![json!("security.raw_cdp_egress_proxy")]);
}

/// A loopback listener that keeps the first TLS record of the first
/// connection it accepts, the browser's ClientHello, and closes it.
fn client_hello_stand_in() -> (u16, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sent, received) = mpsc::channel();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        let mut record = Vec::new();
        let mut chunk = [0; 4096];
        while let Ok(read) = stream.read(&mut chunk) {
            if read == 0 {
                break;
            }
            record.extend_from_slice(&chunk[..read]);
            if record.len() >= 5 {
                let length = usize::from(u16::from_be_bytes([record[3], record[4]]));
                if record.len() >= 5 + length {
                    record.truncate(5 + length);
                    break;
                }
            }
        }
        let _ = sent.send(record);
    });
    (port, received)
}

/// A GREASE value (RFC 8701), which JA4 leaves out.
fn grease(value: u16) -> bool {
    value & 0x0f0f == 0x0a0a && value >> 8 == value & 0xff
}

/// The JA4 fingerprint (FoxIO's specification) of the ClientHello in
/// `record`, the first TLS record of a connection.
fn ja4(record: &[u8]) -> String {
    let u16_at = |bytes: &[u8], at: usize| u16::from_be_bytes([bytes[at], bytes[at + 1]]);
    let hello = &record[5..];
    assert_eq!(hello[0], 1, "a ClientHello");
    let mut at = 4 + 2 + 32;
    at += 1 + usize::from(hello[at]);
    let cipher_bytes = usize::from(u16_at(hello, at));
    let ciphers: Vec<u16> = (0..cipher_bytes / 2)
        .map(|index| u16_at(hello, at + 2 + 2 * index))
        .filter(|cipher| !grease(*cipher))
        .collect();
    at += 2 + cipher_bytes;
    at += 1 + usize::from(hello[at]);
    let end = at + 2 + usize::from(u16_at(hello, at));
    at += 2;
    let (mut extensions, mut alpn, mut signatures, mut version, mut sni) =
        (Vec::new(), String::new(), Vec::new(), "12".to_owned(), 'i');
    while at + 4 <= end {
        let (kind, length) = (u16_at(hello, at), usize::from(u16_at(hello, at + 2)));
        let body = &hello[at + 4..at + 4 + length];
        at += 4 + length;
        if grease(kind) {
            continue;
        }
        extensions.push(kind);
        match kind {
            0x0000 => sni = 'd',
            0x0010 => {
                let first = &body[3..3 + usize::from(body[2])];
                let first = String::from_utf8_lossy(first);
                alpn = format!(
                    "{}{}",
                    first.chars().next().unwrap(),
                    first.chars().last().unwrap()
                );
            }
            0x000d => {
                signatures = (0..usize::from(u16_at(body, 0)) / 2)
                    .map(|index| u16_at(body, 2 + 2 * index))
                    .filter(|algorithm| !grease(*algorithm))
                    .collect();
            }
            0x002b => {
                let highest = (0..usize::from(body[0]) / 2)
                    .map(|index| u16_at(body, 1 + 2 * index))
                    .filter(|offered| !grease(*offered))
                    .max()
                    .unwrap();
                match highest {
                    0x0304 => "13",
                    0x0303 => "12",
                    _ => "00",
                }
                .clone_into(&mut version);
            }
            _ => {}
        }
    }
    let hex = |values: &[u16]| {
        values
            .iter()
            .map(|value| format!("{value:04x}"))
            .collect::<Vec<_>>()
            .join(",")
    };
    let hash = |text: &str| {
        let digest = Sha256::digest(text.as_bytes());
        let mut hex = String::new();
        for byte in digest {
            std::fmt::Write::write_fmt(&mut hex, format_args!("{byte:02x}"))
                .expect("invariant: a String takes text");
        }
        hex[..12].to_owned()
    };
    let mut sorted_ciphers = ciphers.clone();
    sorted_ciphers.sort_unstable();
    let mut sorted_extensions: Vec<u16> = extensions
        .iter()
        .copied()
        .filter(|kind| *kind != 0x0000 && *kind != 0x0010)
        .collect();
    sorted_extensions.sort_unstable();
    format!(
        "t{version}{sni}{:02}{:02}{alpn}_{}_{}",
        ciphers.len().min(99),
        extensions.len().min(99),
        hash(&hex(&sorted_ciphers)),
        hash(&format!("{}_{}", hex(&sorted_extensions), hex(&signatures))),
    )
}

/// The JA4 of the ClientHello the browser of a Run sends to a TLS stand-in
/// its mapping names, through the relay or not, as `settings` say.
fn browsers_ja4(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    run_id: &str,
    settings: &Value,
) -> String {
    let (port, received) = client_hello_stand_in();
    let mut settings = settings.clone();
    settings["network"] = json!({ "resolve": { "hello.test": ["127.0.0.1"] } });
    settings["egress"] = json!({ "private_ranges": ["hello.test"] });
    let cache = browser_cache(env).unwrap();
    let engine = Attached::start_with(env, COLLIE, crate::page::BROWSER_DEADLINE, |command| {
        command.env(goodfetch_test_support::BROWSERS_PATH_VARIABLE, &cache);
    });
    let mut program = Program { engine, next_id: 0 };
    program
        .engine
        .send(&goodfetch_test_client::hello(Product::Collie));
    let _ = program.engine.next();
    let started = program.call(
        "run.start",
        &json!({ "run_id": run_id, "settings": settings }),
    );
    assert!(started.get("result").is_some(), "{started}");
    let page = program.new_page(run_id);
    let _ = program.goto(&page, &format!("https://hello.test:{port}/"));
    let record = received.recv_timeout(Duration::from_secs(30)).unwrap();
    program.finish(run_id);
    let _ = proving_ground;
    ja4(&record)
}

/// Whether the Run `run_id`'s Session went through its egress relay.
fn went_through_the_relay(env: &TestEnv, run_id: &str) -> bool {
    !provenance(env, run_id, "egress_relay").is_empty()
}

/// The relay tunnels the browser's TLS bytes unchanged: the ClientHello's
/// JA4 is the same through the relay as with the Session connecting
/// directly.
///
/// Covers: story:4-10
#[test]
fn the_relay_leaves_the_browsers_client_hello_unchanged() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let relayed = browsers_ja4(&env, &proving_ground, "relayed", &json!({}));
    let direct = browsers_ja4(
        &env,
        &proving_ground,
        "direct",
        &json!({ "security": { "raw_cdp_egress_proxy": false } }),
    );

    assert_eq!(
        (
            went_through_the_relay(&env, "relayed"),
            went_through_the_relay(&env, "direct")
        ),
        (true, false)
    );
    assert!(relayed.starts_with("t13d"), "{relayed}");
    assert_eq!(relayed, direct);
}

/// A site the relay cannot reach itself fails as the browser names that
/// failure when it connects on its own (Playwright's names), never as the
/// relay's answer: a name with no address is `net::ERR_NAME_NOT_RESOLVED`,
/// over a tunnel and over plain HTTP alike.
///
/// Covers: story:4-10, error:navigation_failed, operation:page.goto
#[test]
fn a_site_the_relay_cannot_reach_fails_as_the_browser_names_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut program = started(&env, &proving_ground, "unreached", &json!({}), &[]);
    let page = program.new_page("unreached");

    let failed: Vec<(Value, Value)> = ["http://nowhere.invalid/", "https://nowhere.invalid/"]
        .iter()
        .map(|url| {
            let (code, details) = error(&program.goto(&page, url));
            (code, details["error"].clone())
        })
        .collect();
    program.finish("unreached");

    assert_eq!(
        failed,
        vec![
            (
                json!("navigation_failed"),
                json!("net::ERR_NAME_NOT_RESOLVED")
            );
            2
        ]
    );
}
