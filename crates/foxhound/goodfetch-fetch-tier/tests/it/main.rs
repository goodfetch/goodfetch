//! `goodfetch-fetch-tier` through its public interface, against the
//! Proving ground (ADR 0062): each HTTP transport's failures, the
//! transport conformance gate (pitfall H13) and the fingerprint an
//! HTTP-only `stealth` Session presents.
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod addresses;
mod body_limits;
mod certificates;
mod chain;
mod conformance_gate;
mod extra_loopback;
mod fetch_edge;
mod fingerprint;
mod light_browser;
mod transport_failures;
mod tunnels;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::tls::{CertificateExceptions, Trust, read_certificates};
use goodfetch_fetch_tier::{HttpProfile, Reqwest, Transport, TransportConfig, Wreq};
use goodfetch_page::ProxyRoute;
use goodfetch_test_support::ProvingGround;
use url::Url;

/// A runtime whose clock runs: the transport waits on real sockets, which
/// a paused clock would race with every deadline.
fn running_clock_runtime() -> tokio::runtime::Runtime {
    #[expect(
        clippy::disallowed_methods,
        reason = "test helpers build the tests' runtime (ADR 0060)"
    )]
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// The requests the Proving ground received on `host` for `path`.
fn requests_for(proving_ground: &ProvingGround, host: &str, path: &str) -> Vec<serde_json::Value> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host.split(':').next() == Some(host) && request.target == path)
        .map(|request| serde_json::to_value(request).unwrap())
        .collect()
}

/// The HTTP transports each test runs on.
#[derive(Clone, Copy, Debug)]
enum Stack {
    /// reqwest, of `standard` and `identified` Sessions.
    Reqwest,
    /// `wreq`, of HTTP-only `stealth` Sessions, presenting the default
    /// HTTP profile.
    Wreq,
}

/// What a transport reaching every host the Proving ground serves at its
/// address takes, through `proxy` when one is given, trusting its
/// certificate authority for the fetch edge hosts and `trusted`, so they
/// are reached over TLS and HTTP/2 too. Its egress policy lists those
/// hosts in its private ranges, the Proving ground serving on loopback.
fn config(proving_ground: &ProvingGround, proxy: Option<Url>, trusted: &[&str]) -> TransportConfig {
    let resolve: Vec<(String, Vec<std::net::IpAddr>)> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), vec![address.parse().unwrap()]))
        .collect();
    let hosts: Vec<&str> = resolve.iter().map(|(host, _)| host.as_str()).collect();
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper reads the authority's file before any runtime starts"
    )]
    let authority = std::fs::read(&proving_ground.description().ca).unwrap();
    let trust =
        ["fetch-edge.test"]
            .iter()
            .chain(trusted)
            .fold(Trust::default_roots(), |trust, domain| {
                trust
                    .with_trusted_cas(domain, read_certificates(&authority).unwrap())
                    .unwrap()
            });
    TransportConfig {
        trust,
        exceptions: CertificateExceptions::default(),
        client_certificates: None,
        proxy: proxy.map(|url| ProxyRoute {
            url: url.to_string(),
            credentials: None,
        }),
        egress: Policy::new(&["*"], &[] as &[&str])
            .unwrap()
            .with_private_ranges(&hosts)
            .unwrap(),
        resolve: HostMapping::new(resolve),
        clock: Clock::system(),
        resolver: ResolverConfiguration::system().unwrap(),
        local: None,
    }
}

/// reqwest, under [`config`].
fn reqwest(proving_ground: &ProvingGround, proxy: Option<Url>) -> Reqwest {
    Reqwest::new(config(proving_ground, proxy, &[])).unwrap()
}

/// `stack`'s transport under `config`.
fn built(stack: Stack, config: TransportConfig) -> Box<dyn Transport> {
    match stack {
        Stack::Reqwest => Box::new(Reqwest::new(config).unwrap()),
        Stack::Wreq => Box::new(Wreq::new(config, HttpProfile::default()).unwrap()),
    }
}

/// `stack`'s transport, under [`config`].
fn transport(
    stack: Stack,
    proving_ground: &ProvingGround,
    proxy: Option<Url>,
) -> Box<dyn Transport> {
    built(stack, config(proving_ground, proxy, &[]))
}
