//! The reqwest transport's translation point (ADR 0127), in process: a
//! proxy's refusal of the tunnel is its own variant (ADR 0053), and so is
//! its demand for credentials and a proxy nothing answers on.

use goodfetch_fetch_tier::{Connection, Hop, Transport as _, TransportFailure};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use url::Url;

use crate::{reqwest, running_clock_runtime};

/// Any seed: the proxies and the Catalog family are served on every one.
const SEED: u64 = 759;

/// Covers: story:5-128
#[rstest]
#[case::a_proxy_refusing_the_tunnel("refusing_tunnels", TransportFailure::TunnelRefused)]
#[case::a_proxy_asking_for_credentials("datacenter", TransportFailure::ProxyAuthenticationRequired)]
#[case::a_proxy_nothing_answers_on("down", TransportFailure::ProxyUnreachable)]
fn a_proxy_s_answer_to_the_tunnel_request_is_its_own_failure(
    #[case] proxy: &str,
    #[case] expected: TransportFailure,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let proxy_address = &proving_ground.description().fixtures.proxies[proxy];
    let proxy_url = Url::parse(&format!("http://{proxy_address}")).unwrap();
    let transport = reqwest(&proving_ground, Some(proxy_url));
    let hop = Hop::get(
        Url::parse(&format!(
            "https://catalog.test:{}/catalog/1",
            proving_ground.address().port()
        ))
        .unwrap(),
    );

    let failure = running_clock_runtime()
        .block_on(transport.send(&hop, Connection::Pooled))
        .unwrap_err();

    assert_eq!(failure, expected);
}
