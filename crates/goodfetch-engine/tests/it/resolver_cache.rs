//! The engine's resolver cache against the Proving ground's DNS fixture, on
//! the paused `Clock` (ADR 0118; RFC 2308): an answer is reused for its TTL
//! and asked again after it, an `NXDOMAIN` is kept for its negative TTL, and
//! a server failure is never kept. Each name alternates its entries, so a
//! lookup that reached the fixture gets the next one and a kept answer
//! repeats the last. The engine's own resolver answers on every OS for a
//! Session with a local address (ADR 0044); each lookup holds the clock
//! still while the fixture answers.

use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{
    Dns, HostMapping, IpFamily, LookupError, ResolverConfiguration, Route,
};
use goodfetch_protocol::dns::NameResolver;
use goodfetch_test_support::{ProvingGround, TestEnv};

use crate::held_clock::held;

const SEED: u64 = 1542;

/// What a lookup gives.
type Looked = Result<Vec<IpAddr>, LookupError>;

fn loopback(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(127, 0, 0, last))
}

fn not_resolved(name: &str) -> Looked {
    Err(LookupError::NameNotResolved {
        name: name.to_owned(),
        resolver: NameResolver::Engine,
    })
}

/// Looks `name`'s IPv4 addresses up at the moments `after` the first, each
/// measured from the lookup before it, through the fixture of
/// `proving_ground`.
#[expect(
    clippy::disallowed_methods,
    reason = "the lookups run on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
)]
fn lookups(proving_ground: &ProvingGround, name: &str, after: &[Duration]) -> Vec<Looked> {
    let fixture = proving_ground.description().fixtures.dns.parse().unwrap();
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let dns = Dns::new(
            HostMapping::default(),
            ResolverConfiguration::name_servers([fixture]),
            clock,
        );
        let mut looked = Vec::new();
        for wait in std::iter::once(&Duration::ZERO).chain(after) {
            clock.sleep(*wait).await;
            looked.push(held(dns.addresses(name, IpFamily::Ipv4, Route::LocalAddress)).await);
        }
        looked
    })
}

/// Covers: story:3-147
#[test]
fn an_answer_is_reused_for_its_ttl_and_asked_again_after_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let looked = lookups(
        &proving_ground,
        "ttl-60.127-0-0-1.127-0-0-2.alternating.dns.test",
        &[Duration::from_secs(59), Duration::from_secs(1)],
    );

    assert_eq!(
        looked,
        [
            Ok(vec![loopback(1)]),
            Ok(vec![loopback(1)]),
            Ok(vec![loopback(2)]),
        ]
    );
}

/// RFC 2308: the SOA's TTL and minimum, 60 s.
///
/// Covers: story:3-147
#[test]
fn an_nxdomain_is_kept_for_its_negative_ttl() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let name = "ttl-60.nx.127-0-0-1.alternating.dns.test";

    let looked = lookups(
        &proving_ground,
        name,
        &[Duration::from_secs(59), Duration::from_secs(1)],
    );

    assert_eq!(
        looked,
        [
            not_resolved(name),
            not_resolved(name),
            Ok(vec![loopback(1)])
        ]
    );
}

/// The engine's resolver asks three times before a server failure fails a
/// lookup (`hickory-resolver`'s two retries), so the name declares three
/// failures before its address.
///
/// Covers: story:3-147
#[test]
fn a_server_failure_is_never_kept() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let name = "ttl-60.servfail.servfail.servfail.127-0-0-1.alternating.dns.test";

    let looked = lookups(&proving_ground, name, &[Duration::ZERO]);

    assert_eq!(
        looked,
        [
            Err(LookupError::LookupFailed {
                name: name.to_owned(),
                resolver: NameResolver::Engine,
            }),
            Ok(vec![loopback(1)]),
        ]
    );
}

/// On Linux a direct lookup goes through the engine's own resolver, which
/// reads the system's resolver configuration (ADR 0024): run in a
/// `DnsNamespace`, whose `/etc/resolv.conf` names the DNS fixture, it keeps
/// the fixture's answer for its TTL. The namespace's child is this test
/// binary, running the module's ignored test alone.
#[cfg(target_os = "linux")]
mod linux {
    use std::net::IpAddr;
    use std::time::{Duration, SystemTime};

    use goodfetch_engine::clock::Clock;
    use goodfetch_engine::dns::{Dns, HostMapping, IpFamily, ResolverConfiguration, Route};
    use goodfetch_test_support::{DnsNamespace, ProvingGround, TestEnv};

    use super::{SEED, loopback};
    use crate::held_clock::held;

    const NAME: &str = "ttl-60.127-0-0-1.127-0-0-2.alternating.dns.test";
    const CHILD: &str =
        "resolver_cache::linux::in_the_namespace_a_direct_lookup_keeps_its_answer_for_its_ttl";

    /// Covers: story:3-147
    #[test]
    fn on_linux_a_direct_lookup_keeps_the_fixtures_answer_for_its_ttl() {
        let env = TestEnv::new();
        let proving_ground = ProvingGround::start(&env, SEED).unwrap();
        let fixture = proving_ground.description().fixtures.dns.parse().unwrap();
        let namespace = DnsNamespace::enter(&env, fixture).unwrap();

        let output = namespace
            .output(
                &env,
                std::env::current_exe().unwrap(),
                [CHILD, "--exact", "--include-ignored"],
            )
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{output:?}"
        );
    }

    /// The child, which runs only inside the namespace.
    #[test]
    #[ignore = "run as a child by resolver_cache::linux::on_linux_a_direct_lookup_keeps_the_fixtures_answer_for_its_ttl"]
    #[expect(
        clippy::disallowed_methods,
        reason = "the lookups run on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
    )]
    fn in_the_namespace_a_direct_lookup_keeps_its_answer_for_its_ttl() {
        let looked: Vec<Vec<IpAddr>> = goodfetch_test_support::runtime().block_on(async {
            let clock = Clock::paused(SystemTime::UNIX_EPOCH);
            let dns = Dns::new(
                HostMapping::default(),
                ResolverConfiguration::system().unwrap(),
                clock,
            );
            let mut looked = Vec::new();
            for wait in [
                Duration::ZERO,
                Duration::from_secs(59),
                Duration::from_secs(1),
            ] {
                clock.sleep(wait).await;
                let addresses = held(dns.addresses(NAME, IpFamily::Ipv4, Route::Direct)).await;
                looked.push(addresses.unwrap());
            }
            looked
        });

        assert_eq!(
            looked,
            [vec![loopback(1)], vec![loopback(1)], vec![loopback(2)]]
        );
    }
}
