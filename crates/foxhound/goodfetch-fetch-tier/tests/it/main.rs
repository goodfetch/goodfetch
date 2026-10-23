//! `goodfetch-fetch-tier` through its public interface, against the
//! Proving ground (ADR 0062): the HTTP transport's failures and the
//! transport conformance gate (pitfall H13).
#![expect(
    clippy::unwrap_used,
    reason = "test code, where STYLE.md allows unwrap; Clippy's test exemption does not reach integration-test helpers"
)]

mod conformance_gate;
mod transport_failures;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::tls::Trust;
use goodfetch_fetch_tier::{Reqwest, ReqwestConfig};
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

/// reqwest reaching every host the Proving ground serves at its address,
/// through `proxy` when one is given.
fn reqwest(proving_ground: &ProvingGround, proxy: Option<Url>) -> Reqwest {
    let resolve = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), vec![address.parse().unwrap()]))
        .collect();
    Reqwest::new(ReqwestConfig {
        trust: Trust::default_roots(),
        proxy,
        resolve,
        egress: Policy::new(&["*"], &[] as &[&str]).unwrap(),
        clock: Clock::system(),
    })
    .unwrap()
}
