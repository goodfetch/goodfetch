//! Tests that need the extra loopback addresses `127.0.0.2` to `127.0.0.9`
//! (TESTING.md, Storage, network and OS services): a host without them skips
//! this module.
//!
//! A browser Session with a local address on Chromium over stdio (ADR 0044,
//! spec #564): its egress relay binds the Session's address, so the Proving
//! ground's connection record shows every connection to its site, over
//! HTTP/1.1, HTTP/2 and a WebSocket, coming from that address, with
//! `security.raw_cdp_egress_proxy` on or off; and WebRTC, held to
//! `disable_non_proxied_udp`, sends no UDP, which the Proving ground's UDP
//! fixture's record of datagrams shows.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    clippy::disallowed_types,
    reason = "the tests send the UDP record's control datagram from their own thread, never an async worker (ADR 0060)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::net::{SocketAddr, UdpSocket};
use std::process::Command;
use std::time::{Duration, Instant};

use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv, browser_cache};
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::egress_relay::{Program, provenance, started};
use crate::page::{opened, resumed_with, started_with};
use crate::proxy::{PASSWORD_VARIABLE, adapter_configuration, sticky_relays, visit_marked};
use goodfetch_test_client::Attached;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 638;

/// The extra loopback address each Session binds.
const BOUND: &str = "127.0.0.3";

/// How long a page has to settle.
const SETTLE: Duration = Duration::from_secs(20);

/// The Run settings binding `BOUND`, beside `more`.
fn bound(more: &Value) -> Value {
    let mut settings = json!({
        "egress": { "local_addresses": [BOUND] },
        "timeout": { "wait": 15_000 },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    settings
}

/// The source addresses of the connections that carried a request to one
/// of the pages' hosts, never the test's own to the control host.
fn site_sources(observations: &Value) -> BTreeSet<String> {
    let carried: BTreeSet<u64> = ReceivedRequests::of(observations)
        .every()
        .filter(|request| PAGE_HOSTS.contains(&request.host.as_str()))
        .map(|request| request.connection)
        .collect();
    observations["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|connection| {
            connection["id"]
                .as_u64()
                .is_some_and(|id| carried.contains(&id))
        })
        .filter_map(|connection| connection["source"].as_str())
        .filter_map(|source| source.parse::<SocketAddr>().ok())
        .map(|source| source.ip().to_string())
        .collect()
}

/// The hosts the tests' pages are on.
const PAGE_HOSTS: [&str; 2] = ["catalog.test", "access.test"];

/// The protocol and kind of each request the site received, such as
/// `("h2", "request")` or `("http/1.1", "websocket")`.
fn request_kinds(observations: &Value) -> BTreeSet<(String, String)> {
    ReceivedRequests::of(observations)
        .every()
        .filter(|request| PAGE_HOSTS.contains(&request.host.as_str()))
        .map(|request| {
            (
                request.protocol.as_str().to_owned(),
                request.kind.as_str().to_owned(),
            )
        })
        .collect()
}

/// Waits until the page `page`'s title is `done`.
fn settle(program: &mut Program, page: &str) {
    let _ = program.call(
        "page.wait_for_function",
        &json!({
            "page_id": page,
            "function": "() => document.title === 'done'",
            "polling_interval": 50,
        }),
    );
}

/// A browser Session with a local address reaches its site from that
/// address over HTTP/1.1 and over a WebSocket, and, with the site's HTTPS
/// under a certificate exception, through the relay's terminating mode,
/// over HTTP/1.1 alone so the relay reads each request head and no Session
/// cookie reaches the host (ADR 0042), and Provenance records the address
/// its relay binds.
/// Switching `security.raw_cdp_egress_proxy` off, with no exception that
/// would keep a relay anyway, keeps the binding (A04-23): a Session with a
/// local address always goes through its relay.
///
/// Covers: story:4-45, setting:egress.local_addresses, operation:page.goto
#[rstest]
/// Covers: setting:tls.accept_invalid_certs
#[case::relay_on(
    "bound",
    json!({ "tls": { "accept_invalid_certs": { "catalog.test": null } } }),
    true
)]
/// Covers: setting:security.raw_cdp_egress_proxy
#[case::switch_off(
    "bound-off",
    json!({ "security": { "raw_cdp_egress_proxy": false } }),
    false
)]
fn a_browser_session_with_a_local_address_reaches_its_site_from_it(
    #[case] run_id: &str,
    #[case] more: Value,
    #[case] secure: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut program = started(&env, &proving_ground, run_id, &bound(&more), &[]);
    let page = program.new_page(run_id);
    let port = proving_ground.address().port();

    let plain = program.goto(&page, &proving_ground.url("catalog.test", "/catalog/1"));
    let https =
        secure.then(|| program.goto(&page, &format!("https://catalog.test:{port}/catalog/2")));
    let socket = program.goto(
        &page,
        &proving_ground.url("access.test", "/request-kinds/websocket?to=access.test"),
    );
    settle(&mut program, &page);
    let observations = proving_ground.observations().unwrap();
    program.finish(run_id);

    let statuses: Vec<Value> = [Some(&plain), https.as_ref(), Some(&socket)]
        .into_iter()
        .flatten()
        .map(|answer| answer["result"]["response"]["status"].clone())
        .collect();
    assert_eq!(
        statuses,
        vec![json!(200); if secure { 3 } else { 2 }],
        "{plain} {https:?} {socket}"
    );
    let kinds = request_kinds(&observations);
    let expected = [("http/1.1", "request"), ("http/1.1", "websocket")];
    assert!(
        expected
            .iter()
            .all(|(protocol, kind)| kinds.contains(&((*protocol).to_owned(), (*kind).to_owned()))),
        "{kinds:?}"
    );
    assert!(
        !kinds.contains(&("h2".to_owned(), "request".to_owned())),
        "a host an exception lists is reached over HTTP/1.1 alone: {kinds:?}"
    );
    assert_eq!(
        site_sources(&observations),
        BTreeSet::from([BOUND.to_owned()])
    );
    assert_eq!(
        provenance(&env, run_id, "egress_relay")
            .iter()
            .map(|entry| entry["local_addresses"].clone())
            .collect::<Vec<_>>(),
        vec![json!({ "ipv4": BOUND, "ipv6": null })]
    );
}

/// The datagrams the Proving ground's UDP fixture received, each as its
/// source address.
fn datagram_sources(proving_ground: &ProvingGround) -> Vec<String> {
    proving_ground.observations().unwrap()["udp_datagrams"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|datagram| datagram["source"].as_str().map(str::to_owned))
        .collect()
}

/// A page whose peer connection names a STUN server and a TURN server over
/// UDP at the Proving ground's UDP fixture sends no UDP there, from the
/// Session's address or the machine's others: WebRTC is held to
/// `disable_non_proxied_udp`, so its only route is TCP through the relay.
/// The record's own control, a datagram the test sends, shows it records.
///
/// Covers: story:4-45, setting:egress.local_addresses, operation:page.goto
#[test]
fn webrtc_sends_no_udp_around_a_session_with_a_local_address() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let udp = proving_ground.description().fixtures.udp.clone();
    let turn: SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    let mut program = started(&env, &proving_ground, "webrtc", &bound(&json!({})), &[]);
    let page = program.new_page("webrtc");

    let control = UdpSocket::bind("127.0.0.1:0").unwrap();
    control.send_to(b"control", &udp).unwrap();
    let control = control.local_addr().unwrap().to_string();
    let deadline = Instant::now() + SETTLE;
    while datagram_sources(&proving_ground).is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    let loaded = program.goto(
        &page,
        &proving_ground.url(
            "access.test",
            &format!(
                "/request-kinds/webrtc?to=access.test&turn={}&stun={udp}",
                turn.port()
            ),
        ),
    );
    settle(&mut program, &page);
    let sources = datagram_sources(&proving_ground);
    let to_turn: BTreeSet<String> = proving_ground.observations().unwrap()["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|connection| connection["listener"] == json!("target"))
        .filter_map(|connection| connection["source"].as_str())
        .filter_map(|source| source.parse::<SocketAddr>().ok())
        .map(|source| source.ip().to_string())
        .collect();
    program.finish("webrtc");

    assert_eq!(
        &loaded["result"]["response"]["status"],
        &json!(200),
        "{loaded}"
    );
    assert_eq!(sources, vec![control]);
    assert!(to_turn.iter().all(|source| source == BOUND), "{to_turn:?}");
}

/// The local addresses the egress relay of each Session of the Run `run_id`
/// bound, by Session, as Provenance records them for every engine that held
/// it.
fn relay_addresses(env: &TestEnv, run_id: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut bound: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for entry in provenance(env, run_id, "egress_relay") {
        let session = entry["session_id"].as_str().unwrap_or_default().to_owned();
        let address = entry["local_addresses"]["ipv4"].to_string();
        bound.entry(session).or_default().insert(address);
    }
    bound
}

/// The part of the Session ID `session` after its Run's, such as
/// `session-1`, which marks its pages' visits.
fn session_mark(session: &Value) -> &str {
    session
        .as_str()
        .and_then(|session| session.rsplit_once('.'))
        .map_or("", |(_, mark)| mark)
}

/// Two Sessions a program opens at the same moment each take a position of
/// their own, and draw their proxy and their local address for it: under a
/// Proxy provider each sends a sticky session of its own from an address of
/// its own, and keeps both once its Run resumes in another engine (#882,
/// #891; ADR 0044).
///
/// Covers: story:8-89, story:4-45, setting:proxy.provider, setting:egress.local_addresses, operation:run.new_session, operation:run.resume
#[test]
fn two_sessions_opened_at_once_keep_their_own_proxy_exit_and_local_address_across_a_resume() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    env.user_configuration(&adapter_configuration(&proving_ground, "oxylabs"));
    let password = proving_ground
        .description()
        .fixtures
        .proxy_credentials
        .password
        .clone();
    let with_password = |command: &mut Command| {
        command.env(PASSWORD_VARIABLE, &password);
    };
    let run_id = "at-once";
    let settings: Map<String, Value> = json!({
        "proxy": { "provider": "oxylabs" },
        "egress": { "local_addresses": ["127.0.0.2", "127.0.0.4"] },
    })
    .as_object()
    .cloned()
    .unwrap_or_default();
    let visit = |engine: &mut Attached, id: i64, session: &Value, mark: &str| {
        let page = opened(&engine.ask(id, "session.new_page", &json!({ "session_id": session })));
        visit_marked(
            engine,
            &proving_ground,
            id + 1,
            &json!(page.page_id.0),
            mark,
        );
    };

    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        run_id,
        &settings,
        with_password,
    );
    let sessions: Vec<Value> = engine
        .at_once(&[
            (2, "run.new_session", json!({ "run_id": run_id })),
            (3, "run.new_session", json!({ "run_id": run_id })),
        ])
        .into_iter()
        .map(|opened| opened["result"]["session_id"].clone())
        .collect();
    for (id, session) in (10..).step_by(2).zip(&sessions) {
        let mark = format!("{}-before", session_mark(session));
        visit(&mut engine, id, session, &mark);
    }
    engine.kill();
    let mut resumed = resumed_with(
        &env,
        &cache,
        &proving_ground,
        run_id,
        &settings,
        with_password,
    );
    for (id, session) in (20..).step_by(2).zip(&sessions) {
        let mark = format!("{}-after", session_mark(session));
        visit(&mut resumed, id, session, &mark);
    }
    let _ = resumed.ask(50, "run.finish", &json!({ "run_id": run_id }));
    let _ = resumed.finish();

    let exits = |session: &str| -> BTreeSet<String> {
        sticky_relays(&proving_ground, &format!("mark={session}-"))
            .into_iter()
            .map(|(sticky, _)| sticky["session"].to_string())
            .collect()
    };
    let (first, second) = (exits("session-1"), exits("session-2"));
    let address = |address: &str| BTreeSet::from([json!(address).to_string()]);
    assert_eq!(
        (
            first.len(),
            second.len(),
            first.is_disjoint(&second),
            relay_addresses(&env, run_id),
        ),
        (
            1,
            1,
            true,
            BTreeMap::from([
                ("at-once.session-1".to_owned(), address("127.0.0.2")),
                ("at-once.session-2".to_owned(), address("127.0.0.4")),
            ]),
        ),
        "{sessions:?} {first:?} {second:?}"
    );
}
