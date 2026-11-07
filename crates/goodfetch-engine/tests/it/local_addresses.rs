//! A Session's local addresses on the engine's own HTTP client (ADR 0044):
//! a site with addresses only in a family the Session holds no local
//! address of fails with `address_family_unavailable` before any
//! connection, and never leaves from another address.

use std::net::{Ipv6Addr, SocketAddr};

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{Dns, HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::http_client::{
    Failure, FetchError, LocalBinding, PageTraffic, Resolve, Waiter,
};
use goodfetch_engine::local_addresses::SessionAddresses;
use goodfetch_engine::tls::{CertificateExceptions, ClientCertificates, Trust};
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::json;

const SEED: u64 = 637;

/// Covers: story:4-46, error:address_family_unavailable
#[test]
fn an_ipv4_only_site_for_a_session_holding_only_ipv6_fails_with_address_family_unavailable() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let target: SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    let dns = Dns::new(
        HostMapping::default(),
        ResolverConfiguration::name_servers([]),
        Clock::system(),
    );
    let traffic = |ipv6: Option<Ipv6Addr>, ipv4| PageTraffic {
        egress: Policy::new(&["*"], &[] as &[&str])
            .unwrap()
            .with_private_ranges(&["127.0.0.0/8".to_owned()])
            .unwrap(),
        resolve: Resolve::new().with("ipv4-only.test", vec![target.ip()]),
        local: Some(LocalBinding::new(SessionAddresses { ipv4, ipv6 }, &dns)),
        proxy: None,
        trust: Trust::default_roots(),
        exceptions: CertificateExceptions::default(),
        client_certificates: ClientCertificates::default(),
        cookies: Vec::new(),
        clock: Clock::system(),
        delays: Delays::default(),
    };
    let url = format!("http://ipv4-only.test:{}/", target.port());
    let runtime = goodfetch_test_support::runtime();

    let only_ipv6 =
        runtime.block_on(traffic(Some(Ipv6Addr::LOCALHOST), None).get(&url, Waiter::Call));
    let after_refusal = target_connections(&proving_ground);
    let with_ipv4 = runtime.block_on(async {
        traffic(
            Some(Ipv6Addr::LOCALHOST),
            Some("127.0.0.1".parse().unwrap()),
        )
        .get(&url, Waiter::Call)
        .await
        .map(|response| response.status())
    });

    let Err(FetchError::Failed(Failure::AddressFamily(unavailable))) = only_ipv6 else {
        panic!("expected address_family_unavailable, found {only_ipv6:?}");
    };
    let error = CallError::from(unavailable);
    assert_eq!(
        (
            error.code(),
            error.details().cloned().map(serde_json::Value::Object),
            after_refusal,
            with_ipv4.ok()
        ),
        (
            "address_family_unavailable",
            Some(json!({ "name": "ipv4-only.test", "held": "ipv6" })),
            0,
            Some(200)
        )
    );
}

/// How many connections reached the watched target.
fn target_connections(proving_ground: &ProvingGround) -> usize {
    proving_ground.observations().unwrap()["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == "target")
        .count()
}
