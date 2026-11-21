//! Each transport's translation point (ADR 0127), reqwest's and `wreq`'s: a
//! proxy's refusal of the tunnel is its own variant (ADR 0053), and so is
//! its demand for credentials and a proxy nothing answers on.
//! What a site's own connection does (a refusal, a reset, a close before
//! any answer, plain text for a TLS hello) is read from raw listeners on
//! loopback beside the Proving ground, which serves no such fault.
#![expect(
    clippy::disallowed_types,
    clippy::disallowed_methods,
    reason = "raw loopback listeners on the test's own threads, as sites outside the engine, never an async worker (ADR 0060)"
)]

use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::response_limits::Limits;
use goodfetch_fetch_tier::{FetchBounds, Hop, Reuse, Transport, TransportFailure, fetch};
use goodfetch_protocol::page::proxy::{PROXY_AUTH_FAILED, PROXY_UNAVAILABLE};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::json;
use url::Url;

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};

use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_fetch_tier::FetchFailure;

use crate::{Stack, built, config, running_clock_runtime, transport};

/// Any seed: the proxies and the Catalog family are served on every one.
const SEED: u64 = 759;

/// A fetch a proxy of its own failed ends with the proxy's Error code,
/// naming the proxy without its credentials, never with
/// `navigation_failed` (ADRs 0053 and 0017; issue 2232), and keeps the
/// transport's failure for whoever paces or retries on it.
/// Covers: story:5-128
#[rstest]
#[case::a_proxy_refusing_the_tunnel(
    "refusing_tunnels",
    PROXY_UNAVAILABLE.name,
    TransportFailure::TunnelRefused
)]
#[case::a_proxy_asking_for_credentials(
    "datacenter",
    PROXY_AUTH_FAILED.name,
    TransportFailure::ProxyAuthenticationRequired
)]
#[case::a_proxy_nothing_answers_on("down", PROXY_UNAVAILABLE.name, TransportFailure::ProxyUnreachable)]
fn a_fetch_its_proxy_failed_ends_with_the_proxy_s_error_code(
    #[case] proxy: &str,
    #[case] code: &str,
    #[case] met: TransportFailure,
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let proxy_address = &proving_ground.description().fixtures.proxies[proxy];
    let proxy_url = Url::parse(&format!("http://user:secret@{proxy_address}")).unwrap();
    let transport = transport(stack, &proving_ground, Some(proxy_url));
    let url = format!(
        "https://catalog.test:{}/catalog/1",
        proving_ground.address().port()
    );
    let hop = Hop::get(Url::parse(&url).unwrap());
    let bounds = FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    };

    let failure = running_clock_runtime()
        .block_on(fetch(transport.as_ref(), &hop, &bounds, &Clock::system()))
        .unwrap_err();

    assert_eq!(failure.transport(), Some(&met));
    let error = failure.error();
    assert_eq!(error.code(), code);
    let details = error.details().unwrap();
    assert_eq!(details["proxy"], json!(format!("http://{proxy_address}")));
    assert_eq!(details["url"], json!(url));
    assert!(!error.message().contains("secret"), "{}", error.message());
}

/// `timeout.first_byte` and `timeout.navigation` at 30 s each.
fn bounds() -> FetchBounds {
    FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    }
}

/// The failure of a fetch of `url` through `transport`.
fn failed(transport: &dyn Transport, url: &str) -> FetchFailure {
    let hop = Hop::get(Url::parse(url).unwrap());
    running_clock_runtime()
        .block_on(fetch(transport, &hop, &bounds(), &Clock::system()))
        .unwrap_err()
}

/// What a raw listener does with each connection it accepts.
#[derive(Clone, Copy, Debug)]
enum Raw {
    /// Nothing listens: its port was bound, then closed.
    Refuses,
    /// Closes with the request unread, which resets the connection.
    Resets,
    /// Reads the request's head, then closes with no answer.
    Closes,
    /// Answers a TLS hello in plain text.
    AnswersPlainText,
}

/// The address of a listener beside the Proving ground's that treats each
/// connection as `raw` says, for as long as the test's process lives.
fn raw(proving_ground: &ProvingGround, raw: Raw) -> SocketAddr {
    let listener = TcpListener::bind((proving_ground.address().ip(), 0)).unwrap();
    let address = listener.local_addr().unwrap();
    if matches!(raw, Raw::Refuses) {
        return address;
    }
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            treat(stream, raw);
        }
    });
    address
}

/// Treats one accepted connection as `raw` says.
fn treat(mut stream: TcpStream, raw: Raw) {
    match raw {
        Raw::Refuses => {}
        // Data left unread at the close makes the kernel send a reset.
        Raw::Resets => {
            let _ = stream.peek(&mut [0; 1]);
        }
        Raw::Closes => {
            let mut head = Vec::new();
            let mut byte = [0; 1];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).is_ok_and(|read| read == 1)
            {
                head.push(byte[0]);
            }
        }
        Raw::AnswersPlainText => {
            let _ = stream.peek(&mut [0; 1]);
            let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n");
            let _ = stream.read(&mut [0; 1024]);
        }
    }
}

/// What a site's connection does to a fetch is its own transport failure,
/// on reqwest and on `wreq`, read from a real socket: the failure a crawl
/// classes its attempt by (issue 2286).
/// Covers: story:5-122
#[rstest]
#[case::nothing_listens(Raw::Refuses, "http", TransportFailure::ConnectionRefused)]
#[case::a_reset(Raw::Resets, "http", TransportFailure::ConnectionReset)]
#[case::a_close_before_any_answer(Raw::Closes, "http", TransportFailure::ConnectionClosed)]
#[case::plain_text_for_a_tls_hello(Raw::AnswersPlainText, "https", TransportFailure::Tls)]
fn what_a_sites_connection_does_is_its_own_failure(
    #[case] does: Raw,
    #[case] scheme: &str,
    #[case] expected: TransportFailure,
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let address = raw(&proving_ground, does);

    let failure = failed(
        transport.as_ref(),
        &format!("{scheme}://catalog.test:{}/", address.port()),
    );

    assert_eq!(
        failure.transport(),
        Some(&expected),
        "{:?}",
        failure.error()
    );
    assert_eq!(failure.error().code(), "navigation_failed");
}

/// An HTTP/2 stream the site resets with `ENHANCE_YOUR_CALM` is that
/// failure, never sent again (ADR 0118).
/// Covers: story:5-122
#[rstest]
fn a_stream_reset_with_enhance_your_calm_is_its_own_failure(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);

    let failure = failed(
        transport.as_ref(),
        &format!(
            "https://fetch-edge.test:{}/calm",
            proving_ground.address().port()
        ),
    );

    assert_eq!(
        failure.transport(),
        Some(&TransportFailure::EnhanceYourCalm),
        "{:?}",
        failure.error()
    );
}

/// A name the resolver says does not exist is its own failure.
/// Covers: story:5-122
#[rstest]
fn a_name_that_does_not_exist_is_its_own_failure(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let fixture: SocketAddr = proving_ground.description().fixtures.dns.parse().unwrap();
    let transport = built(
        stack,
        goodfetch_fetch_tier::TransportConfig {
            resolve: HostMapping::default(),
            resolver: ResolverConfiguration::name_servers([fixture]),
            ..config(&proving_ground, None, &[])
        },
    );

    let failure = failed(
        transport.as_ref(),
        &format!(
            "http://nx.mixed.dns.test:{}/",
            proving_ground.address().port()
        ),
    );

    assert_eq!(
        failure.transport(),
        Some(&TransportFailure::NameNotResolved),
        "{:?}",
        failure.error()
    );
}

/// A host whose certificate chains to no authority its trust scope holds
/// fails verification: rustls's refusal and BoringSSL's alike.
/// Covers: story:5-122
#[rstest]
fn a_certificate_no_trusted_authority_issued_is_refused(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let hop = Hop::get(
        Url::parse(&format!(
            "https://catalog.test:{}/catalog/1",
            proving_ground.address().port()
        ))
        .unwrap(),
    );

    let failure = running_clock_runtime()
        .block_on(transport.send(&hop, Reuse::Allowed))
        .unwrap_err();

    assert!(
        matches!(failure, TransportFailure::CertificateInvalid(_)),
        "{failure:?}"
    );
}
