//! The light-browser bridge's egress boundary (ADR 0017; finding 579-5),
//! in process over loopback sockets: a request without the bridge's token,
//! or with a wrong one, is answered `407`; a request or a `CONNECT` to a
//! host or an address the Run's egress policy refuses is answered `403`;
//! and in each the site, a listener of the test's own, receives no
//! connection. The positive control: with the token, a request to an
//! allowed host reaches the site in origin form, its proxy headers dropped.
//! A client that sends no request head is given up at the head deadline.
//!
//! Covers: story:5-120
#![expect(
    clippy::disallowed_types,
    reason = "the test's own loopback sockets stand for Lightpanda and the site (ADR 0060)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on Tokio's own test runtime (ADR 0060)"
)]

use std::fmt::Write as _;

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use data_encoding::BASE64;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::tls::{CertificateExceptions, Trust};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

use super::relay;
use crate::transports::{AddressCheck, Reqwest, TransportConfig};

/// The bridge's token in these tests.
const TOKEN: &str = "a-token-of-the-test";

/// The host the Run's egress policy lets reach the loopback site.
const ALLOWED: &str = "allowed.test";

/// A host resolving to the same site that the policy refuses: loopback is
/// a private address it does not list.
const REFUSED: &str = "refused.test";

/// The `Proxy-Authorization` value the bridge expects.
fn expected() -> String {
    format!(
        "Basic {}",
        BASE64.encode(format!("goodfetch:{TOKEN}").as_bytes())
    )
}

/// The address check of the HTTP tier of a Run whose policy allows every
/// domain, mapping both hosts to loopback and listing only [`ALLOWED`] in
/// its private ranges.
fn check() -> AddressCheck {
    let loopback = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
    Reqwest::new(TransportConfig {
        trust: Trust::default_roots(),
        exceptions: CertificateExceptions::default(),
        client_certificates: None,
        proxy: None,
        resolve: HostMapping::new([(ALLOWED, loopback.clone()), (REFUSED, loopback)]),
        egress: Policy::new(&["*"], &[] as &[&str])
            .unwrap()
            .with_private_ranges(&[ALLOWED])
            .unwrap(),
        clock: Clock::system(),
        resolver: ResolverConfiguration::name_servers([]),
        local: None,
    })
    .unwrap()
    .address_check()
}

/// What the bridge answers `request`, sent as Lightpanda sends it, and
/// whether the site, listening on `site`, received a connection.
async fn relayed(site: &TcpListener, request: String) -> (String, bool) {
    let bridge = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut lightpanda = TcpStream::connect(bridge.local_addr().unwrap())
        .await
        .unwrap();
    let (accepted, _) = bridge.accept().await.unwrap();
    lightpanda.write_all(request.as_bytes()).await.unwrap();
    let check = check();
    let expected = expected();
    let (relayed, answer) = tokio::join!(
        relay(accepted, &check, &expected, &Clock::system()),
        async {
            let mut answer = Vec::new();
            lightpanda.read_to_end(&mut answer).await.unwrap();
            answer
        }
    );
    relayed.unwrap();
    let reached = tokio::time::timeout(Duration::from_millis(200), site.accept())
        .await
        .is_ok();
    (String::from_utf8(answer).unwrap(), reached)
}

/// A request of `method` to `target` with `authorization` and the site's
/// `Host`.
fn request(method: &str, target: &str, authorization: Option<&str>) -> String {
    let mut head = format!("{method} {target} HTTP/1.1\r\nHost: {ALLOWED}\r\n");
    if let Some(authorization) = authorization {
        let _ = write!(head, "Proxy-Authorization: {authorization}\r\n");
    }
    head.push_str("\r\n");
    head
}

/// The status line of `answer`.
fn status(answer: &str) -> &str {
    answer.lines().next().unwrap_or_default()
}

#[rstest::rstest]
#[case::no_token(None)]
#[case::a_wrong_token(Some("Basic Z29vZGZldGNoOm5vdC10aGUtdG9rZW4="))]
#[tokio::test]
async fn a_request_without_the_bridges_token_is_answered_407_and_relays_nothing(
    #[case] authorization: Option<&str>,
) {
    let site = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = site.local_addr().unwrap().port();

    let (answer, reached) = relayed(
        &site,
        request("GET", &format!("http://{ALLOWED}:{port}/"), authorization),
    )
    .await;

    assert_eq!(
        (status(&answer), reached),
        ("HTTP/1.1 407 Proxy Authentication Required", false)
    );
}

#[rstest::rstest]
#[case::a_refused_host("GET", "http://refused.test:{port}/")]
#[case::a_refused_address("GET", "http://127.0.0.1:{port}/")]
#[case::a_connect_to_a_refused_host("CONNECT", "refused.test:{port}")]
#[case::a_connect_to_a_refused_address("CONNECT", "127.0.0.1:{port}")]
#[tokio::test]
async fn a_request_the_egress_policy_refuses_is_answered_403_and_relays_nothing(
    #[case] method: &str,
    #[case] target: &str,
) {
    let site = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = site.local_addr().unwrap().port();
    let target = target.replace("{port}", &port.to_string());

    let (answer, reached) = relayed(&site, request(method, &target, Some(&expected()))).await;

    assert_eq!(
        (status(&answer), reached),
        ("HTTP/1.1 403 Forbidden", false)
    );
}

/// The positive control: the token and an allowed host reach the site,
/// which receives the request in origin form without the proxy's header.
#[tokio::test]
async fn a_request_with_the_token_to_an_allowed_host_reaches_the_site() {
    let site = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = site.local_addr().unwrap().port();
    let bridge = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut lightpanda = TcpStream::connect(bridge.local_addr().unwrap())
        .await
        .unwrap();
    let (accepted, _) = bridge.accept().await.unwrap();
    let sent = request(
        "GET",
        &format!("http://{ALLOWED}:{port}/page?x=1"),
        Some(&expected()),
    );
    lightpanda.write_all(sent.as_bytes()).await.unwrap();
    lightpanda.shutdown().await.unwrap();
    let (check, expected) = (check(), expected());

    let (relayed, received, answer) = tokio::join!(
        relay(accepted, &check, &expected, &Clock::system()),
        async {
            let (mut connection, _) = site.accept().await.unwrap();
            let mut received = vec![0; 4096];
            let count = connection.read(&mut received).await.unwrap();
            received.truncate(count);
            connection
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            connection.shutdown().await.unwrap();
            String::from_utf8(received).unwrap()
        },
        async {
            let mut answer = Vec::new();
            lightpanda.read_to_end(&mut answer).await.unwrap();
            String::from_utf8(answer).unwrap()
        },
    );

    relayed.unwrap();
    assert!(
        received.starts_with("GET /page?x=1 HTTP/1.1\r\n"),
        "{received}"
    );
    assert!(
        !received
            .to_ascii_lowercase()
            .contains("proxy-authorization"),
        "{received}"
    );
    assert_eq!(status(&answer), "HTTP/1.1 200 OK");
}

/// A client that connects and sends no request head holds no bridge task
/// past the bridge's head deadline: the relay gives it up, on the Run's
/// clock, as the egress relay does (deep review finding 577-8).
#[tokio::test(start_paused = true)]
async fn a_client_that_sends_no_request_head_is_given_up_at_the_head_deadline() {
    let bridge = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let _lightpanda = TcpStream::connect(bridge.local_addr().unwrap())
        .await
        .unwrap();
    let (accepted, _) = bridge.accept().await.unwrap();
    let (check, expected) = (check(), expected());

    let ended = tokio::time::timeout(
        Duration::from_secs(60),
        relay(accepted, &check, &expected, &Clock::system()),
    )
    .await;

    assert!(
        matches!(&ended, Ok(Err(error)) if error.kind() == std::io::ErrorKind::TimedOut),
        "{ended:?}"
    );
}
