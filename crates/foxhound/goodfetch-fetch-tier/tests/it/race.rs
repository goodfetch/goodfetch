//! The connection race of the HTTP tier's transports (ADR 0118; RFC
//! 8305), reqwest's and `wreq`'s, with real sockets: a host whose IPv6
//! path drops packets is reached over IPv4 once the Session's
//! `resolver.attempt_delay` has passed. The Proving ground's IPv6 loopback
//! address on its port takes no connection, as a path that drops packets.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_fetch_tier::{Hop, Reuse, TransportConfig};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use url::Url;

use crate::{Stack, built, running_clock_runtime};

const SEED: u64 = 2326;
const NAME: &str = "dual.proving.test";
const IPV4: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
const IPV6: IpAddr = IpAddr::V6(Ipv6Addr::LOCALHOST);

/// The second family's attempt starts at the Session's attempt delay, a
/// second here, not at a delay of the library's own. The clock runs: a
/// paused one would race every deadline of the HTTP exchange.
///
/// Linux only: Linux drops the SYN at the Proving ground's full IPv6 queue,
/// as a path that drops packets, where macOS may complete the handshake
/// and leave the connection unanswered (`proving-ground`'s `blackhole`).
///
/// Covers: setting:resolver.attempt_delay
#[cfg(target_os = "linux")]
#[rstest]
fn a_host_whose_ipv6_path_drops_packets_is_reached_over_ipv4_after_the_sessions_attempt_delay(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let attempt = Duration::from_secs(1);
    let config = TransportConfig {
        resolve: HostMapping::new([(NAME, vec![IPV6, IPV4])]),
        egress: Policy::new(&["*"], &[] as &[&str])
            .unwrap()
            .with_private_ranges(&[NAME])
            .unwrap(),
        delays: Delays {
            attempt,
            ..Delays::default()
        },
        ..crate::config(&proving_ground, None, &[])
    };
    let clock = config.clock;
    let transport = built(stack, config);
    let url = Url::parse(&format!(
        "http://{NAME}:{}/",
        proving_ground.address().port()
    ))
    .unwrap();

    let (sent, waited) = running_clock_runtime().block_on(async {
        let begun = clock.now();
        let sent = transport.send(&Hop::get(url), Reuse::Never).await;
        (sent.map(|_| ()), clock.now().duration_since(begun))
    });

    assert_eq!(sent, Ok(()));
    assert!(
        attempt <= waited && waited < 2 * attempt,
        "it waited {waited:?}"
    );
}
