//! Name resolution through its interface, with no name server (the Quick
//! suite opens no socket): the `resolve` mapping before any lookup, a
//! Session behind a proxy, and the names a resolver answers itself. Lookups
//! from a name server are `tests/it`'s `dns` module.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use goodfetch_protocol::dns::NameResolver;
use rstest::rstest;

use super::{AnsweredBy, Dns, HostMapping, LookupError, Resolution, ResolverConfiguration, Route};
use crate::call_error::CallError;
use crate::clock::Clock;

const PINNED: IpAddr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 7));

/// A resolver with `mapping` and no name server, so any lookup that
/// reaches one fails.
fn dns(mapping: HostMapping) -> Dns {
    Dns::new(
        mapping,
        ResolverConfiguration::name_servers([]),
        Clock::system(),
    )
}

fn lookup(dns: &Dns, name: &str, route: Route) -> Result<Resolution, LookupError> {
    goodfetch_test_support::runtime().block_on(dns.lookup(name, route))
}

/// `localhost` is a name both resolvers answer themselves, so a mapping
/// that wins over it applied before either resolver ran.
///
/// Covers: story:3-134
#[rstest]
#[case::direct(Route::Direct)]
#[case::from_a_local_address(Route::LocalAddress)]
fn a_mapped_name_resolves_to_its_mapped_addresses_before_any_lookup(#[case] route: Route) {
    let dns = dns(HostMapping::new([("localhost", vec![PINNED])]));

    let resolution = lookup(&dns, "localhost", route).unwrap();

    assert_eq!(
        resolution,
        Resolution::Addresses {
            addresses: vec![PINNED],
            answered_by: AnsweredBy::Mapping,
        }
    );
}

/// Covers: story:3-134
#[rstest]
#[case::upper_case("Staging.Proving.TEST")]
#[case::trailing_dot("staging.proving.test.")]
fn a_mapping_matches_a_name_whatever_its_case_or_trailing_dot(#[case] name: &str) {
    let dns = dns(HostMapping::new([("staging.proving.test", vec![PINNED])]));

    let resolution = lookup(&dns, name, Route::Direct).unwrap();

    assert_eq!(
        resolution,
        Resolution::Addresses {
            addresses: vec![PINNED],
            answered_by: AnsweredBy::Mapping,
        }
    );
}

/// ADR 0059: the proxy resolves the name, so the mapping does not reach it.
///
/// Covers: story:3-134
#[test]
fn a_session_behind_a_proxy_hands_even_a_mapped_name_to_the_proxy() {
    let dns = dns(HostMapping::new([("staging.proving.test", vec![PINNED])]));

    let resolution = lookup(&dns, "staging.proving.test", Route::Proxy).unwrap();

    assert_eq!(resolution, Resolution::ByProxy);
}

/// ADR 0044: a Session with a local address asks the engine's own resolver
/// on every OS, which answers `localhost` itself (RFC 6761).
#[test]
fn a_session_with_a_local_address_asks_the_engines_own_resolver() {
    let dns = dns(HostMapping::default());

    let resolution = lookup(&dns, "localhost", Route::LocalAddress).unwrap();

    assert_eq!(
        resolution,
        Resolution::Addresses {
            addresses: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST)
            ],
            answered_by: AnsweredBy::Engine,
        }
    );
}

/// ADR 0024: the static musl executable asks the engine's own resolver.
#[cfg(target_os = "linux")]
#[test]
fn a_direct_lookup_on_linux_asks_the_engines_own_resolver() {
    let dns = dns(HostMapping::default());

    let resolution = lookup(&dns, "localhost", Route::Direct).unwrap();

    assert_eq!(
        resolution,
        Resolution::Addresses {
            addresses: vec![
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST)
            ],
            answered_by: AnsweredBy::Engine,
        }
    );
}

/// The Error code a Client sees, with the resolver that found nothing.
///
/// Covers: error:name_not_resolved
#[test]
fn a_name_not_resolved_answers_with_name_not_resolved() {
    let failure = LookupError::NameNotResolved {
        name: "printer.local".to_owned(),
        resolver: NameResolver::Engine,
    };

    let error = CallError::from(failure);

    insta::assert_debug_snapshot!(error);
}

/// Covers: error:name_lookup_failed
#[test]
fn a_lookup_its_name_servers_failed_answers_with_name_lookup_failed() {
    let failure = LookupError::LookupFailed {
        name: "staging.proving.test".to_owned(),
        resolver: NameResolver::Engine,
    };

    let error = CallError::from(failure);

    insta::assert_debug_snapshot!(error);
}

/// The system's configuration reads on the machine the tests run on.
#[test]
fn the_systems_resolver_configuration_reads() {
    let configuration = ResolverConfiguration::system();

    assert!(configuration.is_ok(), "{configuration:?}");
}

/// The system resolver's lookup never waits for a thread of Tokio's
/// blocking pool (#2870): with the runtime's one blocking thread held,
/// `localhost` answers with a loopback address while it is still held.
#[test]
fn a_system_lookup_never_waits_for_a_blocking_thread() {
    let (found, held_throughout) = goodfetch_test_support::runtime_with_one_blocking_thread()
        .block_on(goodfetch_test_support::with_the_blocking_pool_held(
            super::system_lookup("localhost"),
        ));

    let loopback = found.is_ok_and(|found| found.iter().any(IpAddr::is_loopback));
    assert_eq!((loopback, held_throughout), (true, true));
}
