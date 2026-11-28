//! The engine's HTTP transports' connection step, `http_client::connect`,
//! with real sockets (ADR 0118; RFC 8305): a host whose IPv6 path drops
//! packets is reached over IPv4 after the attempt delay, a name whose every
//! address failed is resolved again on the next attempt, a Session holding
//! a local address of one family never tries the other, and behind a proxy
//! the race is to the proxy's address. The Proving ground's IPv6 loopback
//! address on its port takes no connection, as a path that drops packets.
//!
//! The race runs on the paused `Clock`: while an attempt waits on its
//! socket the runtime is idle, so Tokio moves the clock to the race's next
//! timer, which is the moment the attempt delay ends. A test that waits on
//! the DNS fixture holds the clock still meanwhile.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::SystemTime;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{Dns, HostMapping, IpFamily, ResolverConfiguration};
use goodfetch_engine::happy_eyeballs::{Delays, FamilyUnavailable, RaceError};
use goodfetch_engine::http_client::{Connecting, Egress, connect};
use goodfetch_protocol::dns::NameResolver;
use goodfetch_test_support::{ProvingGround, TestEnv};
use tokio::net::TcpListener;

use crate::held_clock::held;

const SEED: u64 = 1542;
const NAME: &str = "dual.proving.test";
const IPV4: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const IPV6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);

/// How a test's connections resolve: `mapping`, then the engine's own
/// resolver asking the Proving ground's DNS fixture, on `clock`.
fn connecting(proving_ground: &ProvingGround, mapping: HostMapping, clock: Clock) -> Connecting {
    let fixture = proving_ground.description().fixtures.dns.parse().unwrap();
    Connecting {
        dns: Dns::new(
            mapping,
            ResolverConfiguration::name_servers([fixture]),
            clock,
        ),
        clock,
        delays: Delays::default(),
    }
}

/// A Session bound to IPv4 loopback, whose lookups go through the engine's
/// own resolver on every OS (ADR 0044).
fn bound_to_ipv4() -> Egress {
    Egress {
        local_ipv4: Some(Ipv4Addr::LOCALHOST),
        ..Egress::default()
    }
}

/// Linux only: Linux drops the SYN at the Proving ground's full IPv6 queue,
/// as a path that drops packets, where macOS may complete the handshake
/// and leave the connection unanswered (`proving-ground`'s `blackhole`).
/// The race's moments on every OS are `happy_eyeballs`' unit tests.
///
/// Covers: story:3-148, setting:resolver.attempt_delay
#[cfg(target_os = "linux")]
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the race runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
fn a_host_whose_ipv6_path_drops_packets_is_reached_over_ipv4_after_the_attempt_delay() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = proving_ground.address().port();

    let (peer, waited) = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let connecting = connecting(
            &proving_ground,
            HostMapping::new([(NAME, vec![IPV6, IPV4])]),
            clock,
        );
        let begun = clock.now();
        let connected = connect(NAME, port, &Egress::default(), &connecting)
            .await
            .unwrap();
        (connected.peer, clock.now().duration_since(begun))
    });

    assert_eq!(peer, SocketAddr::new(IPV4, port));
    assert!(waited <= Delays::default().attempt, "it waited {waited:?}");
}

/// RFC 8305 section 9. The name answers loopback, then `NXDOMAIN`, each
/// for a minute: nothing listens on the port of the Proving ground's `down`
/// proxy, so the first connection fails at every address, and the second
/// lookup asks the fixture again instead of reusing the answer.
///
/// Covers: story:3-147
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the race runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
fn a_name_whose_every_address_failed_is_resolved_again_on_the_next_attempt() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let down: SocketAddr = proving_ground.description().fixtures.proxies["down"]
        .parse()
        .unwrap();
    let name = "ttl-60.127-0-0-1.nx.alternating.dns.test";

    let outcomes = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let connecting = connecting(&proving_ground, HostMapping::default(), clock);
        let mut outcomes = Vec::new();
        for _ in 0..2 {
            let outcome = held(connect(name, down.port(), &bound_to_ipv4(), &connecting)).await;
            outcomes.push(outcome.map(|connected| connected.peer));
        }
        outcomes
    });

    assert_eq!(
        outcomes,
        [
            Err(RaceError::Unreachable {
                tried: vec![SocketAddr::new(IPV4, down.port())],
            }),
            Err(RaceError::Lookup(
                goodfetch_engine::dns::LookupError::NameNotResolved {
                    name: name.to_owned(),
                    resolver: NameResolver::Engine,
                }
            )),
        ]
    );
}

/// The control: a connection that succeeded keeps the answer for its TTL.
///
/// Covers: story:3-147
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the race runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
fn a_name_whose_address_connected_keeps_its_answer() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let port = proving_ground.address().port();
    let name = "ttl-60.127-0-0-1.nx.alternating.dns.test";

    let peers = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let connecting = connecting(&proving_ground, HostMapping::default(), clock);
        let mut peers = Vec::new();
        for _ in 0..2 {
            let connected = held(connect(name, port, &bound_to_ipv4(), &connecting)).await;
            peers.push(connected.map(|connected| connected.peer));
        }
        peers
    });

    assert_eq!(
        peers,
        [
            Ok(SocketAddr::new(IPV4, port)),
            Ok(SocketAddr::new(IPV4, port))
        ]
    );
}

/// Two listeners on one port, IPv6 and IPv4 loopback, both accepting: a
/// race that tried IPv6, its first family, would connect there.
async fn both_families() -> (TcpListener, TcpListener) {
    loop {
        let ipv4 = TcpListener::bind((IPV4, 0)).await.unwrap();
        let port = ipv4.local_addr().unwrap().port();
        if let Ok(ipv6) = TcpListener::bind((IPV6, port)).await {
            return (ipv6, ipv4);
        }
    }
}

/// ADR 0044 through ADR 0118.
///
/// Covers: story:3-148
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the race runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
fn a_session_with_a_local_address_of_one_family_never_tries_the_other() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let (peer, listening) = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let (_ipv6, ipv4) = both_families().await;
        let listening = ipv4.local_addr().unwrap();
        let connecting = connecting(
            &proving_ground,
            HostMapping::new([(NAME, vec![IPV6, IPV4])]),
            clock,
        );
        let connected = connect(NAME, listening.port(), &bound_to_ipv4(), &connecting)
            .await
            .unwrap();
        (connected.peer, listening)
    });

    assert_eq!(peer, listening);
}

/// Covers: story:3-148, error:address_family_unavailable
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the race runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
fn a_session_bound_to_ipv6_cannot_reach_a_host_of_ipv4_addresses_alone() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let egress = Egress {
        local_ipv6: Some(Ipv6Addr::LOCALHOST),
        ..Egress::default()
    };

    let outcome = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let connecting = connecting(
            &proving_ground,
            HostMapping::new([(NAME, vec![IPV4])]),
            clock,
        );
        connect(NAME, proving_ground.address().port(), &egress, &connecting)
            .await
            .map(|connected| connected.peer)
    });

    assert_eq!(
        outcome,
        Err(RaceError::FamilyUnavailable(FamilyUnavailable {
            name: NAME.to_owned(),
            held: IpFamily::Ipv6,
        }))
    );
}

/// ADR 0118: the proxy resolves the site, so the race is to the proxy's
/// own addresses. The site's name is one the DNS fixture does not know:
/// resolving it would fail the connection.
///
/// Covers: story:3-148
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the race runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
fn behind_a_proxy_the_race_is_to_the_address_of_the_proxy() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let proxy: SocketAddr = proving_ground.description().fixtures.proxies["open"]
        .parse()
        .unwrap();
    let egress = Egress {
        proxy: Some(("proxy.proving.test".to_owned(), proxy.port())),
        ..bound_to_ipv4()
    };

    let peer = goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let connecting = connecting(
            &proving_ground,
            HostMapping::new([("proxy.proving.test", vec![proxy.ip()])]),
            clock,
        );
        held(connect("unknown.proving.test", 443, &egress, &connecting))
            .await
            .map(|connected| connected.peer)
    });

    assert_eq!(peer, Ok(proxy));
}
