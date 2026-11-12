//! Each transport's translation point (ADR 0127), reqwest's and `wreq`'s: a
//! proxy's refusal of the tunnel is its own variant (ADR 0053), and so is
//! its demand for credentials and a proxy nothing answers on.

use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::response_limits::Limits;
use goodfetch_fetch_tier::{FetchBounds, Hop, Reuse, TransportFailure, fetch};
use goodfetch_protocol::page::proxy::{PROXY_AUTH_FAILED, PROXY_UNAVAILABLE};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::json;
use url::Url;

use crate::{Stack, running_clock_runtime, transport};

/// Any seed: the proxies and the Catalog family are served on every one.
const SEED: u64 = 759;

/// Covers: story:5-128, story:5-122
#[rstest]
#[case::a_proxy_refusing_the_tunnel("refusing_tunnels", TransportFailure::TunnelRefused)]
#[case::a_proxy_asking_for_credentials("datacenter", TransportFailure::ProxyAuthenticationRequired)]
#[case::a_proxy_nothing_answers_on("down", TransportFailure::ProxyUnreachable)]
fn a_proxy_s_answer_to_the_tunnel_request_is_its_own_failure(
    #[case] proxy: &str,
    #[case] expected: TransportFailure,
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let proxy_address = &proving_ground.description().fixtures.proxies[proxy];
    let proxy_url = Url::parse(&format!("http://{proxy_address}")).unwrap();
    let transport = transport(stack, &proving_ground, Some(proxy_url));
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

    assert_eq!(failure, expected);
}

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
fn a_fetch_its_proxy_failed_ends_with_the_proxys_error_code(
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
