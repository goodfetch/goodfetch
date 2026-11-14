//! Covers: story:5-128

use std::net::Ipv4Addr;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{HostMapping, ResolverConfiguration, Route};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::tls::{CertificateExceptions, Trust};
use url::Url;

use super::{HTTP1_ORIGINS, Http1Origins, LocalSource, Resolver, TransportConfig, classify};
use crate::driver::TransportFailure;

/// A failure no library named.
#[derive(Debug, thiserror::Error)]
#[error("something else went wrong")]
struct Unknown;

#[test]
fn an_unknown_failure_maps_to_the_generic_case() {
    let failure = classify(&Unknown);

    assert_eq!(failure, TransportFailure::Other);
}

#[test]
fn an_unknown_failure_wrapped_in_an_io_error_maps_to_the_generic_case() {
    let wrapped = std::io::Error::other(Unknown);

    let failure = classify(&wrapped);

    assert_eq!(failure, TransportFailure::Other);
}

/// Covers: story:5-127
#[test]
fn a_refused_http2_stream_is_the_unprocessed_refusal() {
    let failure = classify(&h2::Error::from(h2::Reason::REFUSED_STREAM));

    assert_eq!(failure, TransportFailure::RefusedStream);
}

/// Covers: story:5-127
#[test]
fn http_1_1_required_wrapped_in_an_io_error_is_its_own_failure() {
    let wrapped = std::io::Error::other(h2::Error::from(h2::Reason::HTTP_1_1_REQUIRED));

    let failure = classify(&wrapped);

    assert_eq!(failure, TransportFailure::Http11Required);
}

/// A site's HTTP/2 `ENHANCE_YOUR_CALM`, a reset or a `GOAWAY`, is its own
/// failure, which backs its domain off (ADR 0118).
///
/// Covers: story:5-132
#[test]
fn enhance_your_calm_is_its_own_failure_named_as_chrome_names_it() {
    let failure = classify(&h2::Error::from(h2::Reason::ENHANCE_YOUR_CALM));

    assert_eq!(
        (failure.clone(), failure.network_error()),
        (
            TransportFailure::EnhanceYourCalm,
            "net::ERR_HTTP2_PROTOCOL_ERROR"
        )
    );
}

/// Covers: story:5-127
#[test]
fn another_http2_error_is_the_generic_case() {
    let failure = classify(&h2::Error::from(h2::Reason::INTERNAL_ERROR));

    assert_eq!(failure, TransportFailure::Other);
}

/// The origins kept on HTTP/1.1 are bounded: past [`HTTP1_ORIGINS`], the
/// one that answered `HTTP_1_1_REQUIRED` longest ago is forgotten first.
///
/// Covers: story:5-127
#[test]
fn the_origins_kept_on_http_1_1_forget_the_oldest_past_their_bound() {
    let origins = Http1Origins::default();
    let origin = |index: usize| Url::parse(&format!("https://site{index}.test/")).unwrap();
    for index in 0..=HTTP1_ORIGINS {
        origins.insert(&origin(index));
    }
    origins.insert(&origin(HTTP1_ORIGINS));

    assert_eq!(
        (
            origins.contains(&origin(0)),
            origins.contains(&origin(1)),
            origins.contains(&origin(HTTP1_ORIGINS)),
        ),
        (false, true, true)
    );
}

/// The configuration of a transport of a Session holding `local`.
fn holding(local: Option<LocalSource>) -> TransportConfig {
    TransportConfig {
        trust: Trust::default_roots(),
        exceptions: CertificateExceptions::default(),
        client_certificates: None,
        proxy: None,
        resolve: HostMapping::default(),
        egress: Policy::new(&["*"], &[] as &[&str]).unwrap(),
        clock: Clock::system(),
        resolver: ResolverConfiguration::name_servers([]),
        local,
    }
}

/// ADR 0044: a Session holding a local address resolves through the
/// engine's own resolver, from that address, on every OS: its lookups take
/// the local-address route, which asks the engine's resolver wherever the
/// direct route would ask the system's (macOS and Windows; finding 565-2).
/// A Session holding none takes the direct route.
#[rstest::rstest]
#[case::a_local_address(Some(LocalSource {
    ipv4: Some(Ipv4Addr::new(127, 0, 0, 3)),
    ..LocalSource::default()
}), Route::LocalAddress)]
#[case::none(None, Route::Direct)]
fn a_sessions_lookups_take_the_engines_resolver_when_it_holds_a_local_address(
    #[case] local: Option<LocalSource>,
    #[case] route: Route,
) {
    let resolver = Resolver::new(&holding(local));

    assert_eq!(resolver.route, route);
}
