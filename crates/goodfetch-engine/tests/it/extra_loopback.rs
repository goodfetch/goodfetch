//! Tests that need the extra loopback addresses `127.0.0.2` to `127.0.0.9`
//! (TESTING.md, Storage, network and OS services): a host without them skips
//! this module.
//!
//! A Session bound to a local address (ADR 0044), against the Proving
//! ground: its binding's lookup and connection leave from that address, and
//! its DNS queries leave from it through the engine's own resolver, on every
//! OS. The Proving ground's connection record and its DNS fixture's record
//! of queries name each source.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{
    AnsweredBy, Dns, HostMapping, Resolution, ResolverConfiguration, Route,
};
use goodfetch_engine::http_client::{LocalBinding, Resolve};
use goodfetch_engine::local_addresses::SessionAddresses;
use goodfetch_test_support::{ProvingGround, TestEnv};
use serde_json::Value;

const SEED: u64 = 637;

/// The extra loopback address the Session binds.
const BOUND: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 3);

/// The engine's resolution, asking only the Proving ground's DNS fixture.
fn dns(proving_ground: &ProvingGround) -> Dns {
    let fixture: SocketAddr = proving_ground.description().fixtures.dns.parse().unwrap();
    Dns::new(
        HostMapping::default(),
        ResolverConfiguration::name_servers([fixture]),
        Clock::system(),
    )
}

/// The source address of each connection the watched target accepted.
fn target_sources(observations: &Value) -> Vec<IpAddr> {
    observations["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == "target")
        .map(|connection| {
            connection["source"]
                .as_str()
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
        })
        .collect()
}

/// A Session's binding as the egress relay takes it for a browser Session
/// (ADR 0044): its lookup's queries leave from the Session's address
/// through the engine's resolver, and its connection to an answer leaves
/// from it too; a connection to a family it holds no address of is refused
/// before any is opened.
/// Covers: story:4-45
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a real name server's socket stays off the paused clock, so the test runs on the real-time runtime `tokio::test` builds (TESTING.md, Time)"
)]
async fn a_bindings_lookup_and_connection_leave_from_the_sessions_address() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let target: SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    let binding = LocalBinding::new(
        SessionAddresses {
            ipv4: Some(BOUND),
            ipv6: None,
        },
        &dns(&proving_ground),
    );

    let found = binding
        .lookup("3.catalog.test", target.port(), &Resolve::new())
        .await
        .unwrap();
    let connected = binding.connect(found[0]).await.map(|_| ());
    let other_family = binding
        .connect(SocketAddr::new(
            IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
            target.port(),
        ))
        .await
        .map(|_| ())
        .map_err(|error| error.kind());
    let observations = proving_ground.observations().unwrap();

    assert_eq!(
        (
            found,
            connected.is_ok(),
            target_sources(&observations),
            ProvingGround::dns_query_sources(&observations, "3.catalog.test")
                .into_iter()
                .all(|source| source == IpAddr::V4(BOUND)),
            ProvingGround::dns_query_sources(&observations, "3.catalog.test").is_empty(),
            other_family,
        ),
        (
            vec![SocketAddr::new(
                proving_ground.address().ip(),
                target.port()
            )],
            true,
            vec![IpAddr::V4(BOUND)],
            true,
            false,
            Err(std::io::ErrorKind::AddrNotAvailable),
        )
    );
}

/// The resolver alone: with no local address its queries leave from the
/// address the OS chooses, bound to one they leave from it.
/// Covers: story:4-44
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a real name server's socket stays off the paused clock, so the test runs on the real-time runtime `tokio::test` builds (TESTING.md, Time)"
)]
async fn the_engines_resolver_sends_a_sessions_queries_from_its_local_address() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let unbound = dns(&proving_ground);
    let bound = unbound.from_local_addresses(Some(BOUND), None);

    let from_bound = bound.lookup("1.catalog.test", Route::LocalAddress).await;
    let from_unbound = unbound.lookup("2.catalog.test", Route::LocalAddress).await;
    let observations = proving_ground.observations().unwrap();

    let site = proving_ground.address().ip();
    assert_eq!(
        (
            from_bound,
            ProvingGround::dns_query_sources(&observations, "1.catalog.test")
                .into_iter()
                .all(|source| source == IpAddr::V4(BOUND)),
            from_unbound.is_ok(),
            ProvingGround::dns_query_sources(&observations, "2.catalog.test")
                .contains(&IpAddr::V4(BOUND)),
        ),
        (
            Ok(Resolution::Addresses {
                addresses: vec![site],
                answered_by: AnsweredBy::Engine,
            }),
            true,
            true,
            false,
        )
    );
}
