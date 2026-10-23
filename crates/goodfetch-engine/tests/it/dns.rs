//! Name resolution with a real name server on loopback (spec #213's module
//! interface of the resolver): which resolver a Session's route asks, what
//! the `resolve` mapping keeps from every name server, and how a name the
//! engine's own resolver cannot resolve fails (ADRs 0024, 0044 and 0059).
#![expect(
    clippy::disallowed_methods,
    reason = "a real name server's socket stays off the paused clock, so each test runs on the real-time runtime `tokio::test` builds (TESTING.md, Time)"
)]

use std::net::{IpAddr, Ipv4Addr};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{
    AnsweredBy, Dns, HostMapping, LookupError, Resolution, ResolverConfiguration, Route,
};
use goodfetch_protocol::dns::NameResolver;

use crate::name_server::{Answer, NameServer};

const STAGING: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 5);

fn dns(server: &NameServer, mapping: HostMapping) -> Dns {
    Dns::new(
        mapping,
        ResolverConfiguration::name_servers([server.address()]),
        Clock::system(),
    )
}

/// The lookup, while `server` answers.
async fn lookup(
    server: &mut NameServer,
    dns: &Dns,
    name: &str,
    route: Route,
) -> Result<Resolution, LookupError> {
    tokio::select! {
        resolution = dns.lookup(name, route) => resolution,
        () = server.serve() => unreachable!("the name server serves until the lookup ends"),
    }
}

/// ADR 0044: on every OS.
#[tokio::test]
async fn a_session_with_a_local_address_asks_the_engines_own_resolvers_name_servers() {
    let mut server =
        NameServer::start([("staging.proving.test", Answer::Addresses(vec![STAGING]))]).await;
    let dns = dns(&server, HostMapping::default());

    let resolution = lookup(
        &mut server,
        &dns,
        "staging.proving.test",
        Route::LocalAddress,
    )
    .await;

    assert_eq!(
        resolution,
        Ok(Resolution::Addresses {
            addresses: vec![IpAddr::V4(STAGING)],
            answered_by: AnsweredBy::Engine,
        })
    );
}

/// ADR 0024: each Product's static musl executable.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_direct_lookup_on_linux_asks_the_engines_own_resolvers_name_servers() {
    let mut server =
        NameServer::start([("staging.proving.test", Answer::Addresses(vec![STAGING]))]).await;
    let dns = dns(&server, HostMapping::default());

    let resolution = lookup(&mut server, &dns, "staging.proving.test", Route::Direct).await;

    assert_eq!(
        resolution,
        Ok(Resolution::Addresses {
            addresses: vec![IpAddr::V4(STAGING)],
            answered_by: AnsweredBy::Engine,
        })
    );
}

/// ADR 0024: macOS and Windows keep the system resolver for a Session with
/// no local address. `localhost` needs no network from it.
#[cfg(not(target_os = "linux"))]
#[tokio::test]
async fn a_direct_lookup_on_macos_and_windows_asks_the_system_resolver() {
    let mut server = NameServer::start([]).await;
    let dns = dns(&server, HostMapping::default());

    let resolution = lookup(&mut server, &dns, "localhost", Route::Direct).await;

    let Ok(Resolution::Addresses { answered_by, .. }) = resolution else {
        panic!("the system resolver did not resolve localhost: {resolution:?}");
    };
    assert_eq!((answered_by, server.asked()), (AnsweredBy::System, &[][..]));
}

/// ADR 0059: the mapping applies before any lookup, so no name server
/// hears of the name.
///
/// Covers: story:3-134
#[tokio::test]
async fn a_mapped_name_reaches_no_name_server() {
    let mut server =
        NameServer::start([("staging.proving.test", Answer::Addresses(vec![STAGING]))]).await;
    let pinned = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 9));
    let dns = dns(
        &server,
        HostMapping::new([("staging.proving.test", vec![pinned])]),
    );

    let resolution = lookup(
        &mut server,
        &dns,
        "staging.proving.test",
        Route::LocalAddress,
    )
    .await;

    assert_eq!(
        (resolution, server.asked()),
        (
            Ok(Resolution::Addresses {
                addresses: vec![pinned],
                answered_by: AnsweredBy::Mapping,
            }),
            &[][..]
        )
    );
}

/// A name only an NSS module knows looks, to the engine's own resolver,
/// like a name that does not exist.
#[tokio::test]
async fn a_name_the_name_servers_do_not_have_is_not_resolved_by_the_engines_own_resolver() {
    let mut server = NameServer::start([("intranet.corp", Answer::NoSuchName)]).await;
    let dns = dns(&server, HostMapping::default());

    let resolution = lookup(&mut server, &dns, "intranet.corp", Route::LocalAddress).await;

    assert_eq!(
        resolution,
        Err(LookupError::NameNotResolved {
            name: "intranet.corp".to_owned(),
            resolver: NameResolver::Engine,
        })
    );
}

/// The engine's own resolver asks the name servers for a `.local` name,
/// which a cluster's DNS may serve, and never multicast DNS.
#[tokio::test]
async fn a_local_name_only_multicast_dns_answers_is_not_resolved_by_the_engines_own_resolver() {
    let mut server = NameServer::start([]).await;
    let dns = dns(&server, HostMapping::default());

    let resolution = lookup(&mut server, &dns, "printer.local", Route::LocalAddress).await;

    assert_eq!(
        resolution,
        Err(LookupError::NameNotResolved {
            name: "printer.local".to_owned(),
            resolver: NameResolver::Engine,
        })
    );
}

/// A cluster's `.local` names still resolve through its name servers.
#[tokio::test]
async fn a_local_name_the_name_servers_serve_resolves() {
    let mut server = NameServer::start([(
        "api.default.svc.cluster.local",
        Answer::Addresses(vec![STAGING]),
    )])
    .await;
    let dns = dns(&server, HostMapping::default());

    let resolution = lookup(
        &mut server,
        &dns,
        "api.default.svc.cluster.local",
        Route::LocalAddress,
    )
    .await;

    assert_eq!(
        resolution,
        Ok(Resolution::Addresses {
            addresses: vec![IpAddr::V4(STAGING)],
            answered_by: AnsweredBy::Engine,
        })
    );
}

#[tokio::test]
async fn a_name_server_failure_is_a_failed_lookup() {
    let mut server = NameServer::start([("staging.proving.test", Answer::ServerFailure)]).await;
    let dns = dns(&server, HostMapping::default());

    let resolution = lookup(
        &mut server,
        &dns,
        "staging.proving.test",
        Route::LocalAddress,
    )
    .await;

    assert_eq!(
        resolution,
        Err(LookupError::LookupFailed {
            name: "staging.proving.test".to_owned(),
            resolver: NameResolver::Engine,
        })
    );
}
