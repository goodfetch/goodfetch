//! The transports of a Run's Sessions' local sources (ADR 0044; finding
//! 565-7): a source's transport is built once and shared, and the Run keeps
//! those used last, as many as the pool keeps Sessions open, so a Run whose
//! every Session takes its own address holds no transport of one long
//! closed.
//!
//! Covers: story:4-108

use std::net::Ipv4Addr;
use std::sync::Arc;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::tls::{CertificateExceptions, Trust};

use super::SessionTransports;
use crate::transports::{LocalSource, TransportConfig};

/// The Run's transport configuration.
fn config() -> TransportConfig {
    TransportConfig {
        trust: Trust::default_roots(),
        exceptions: CertificateExceptions::default(),
        client_certificates: None,
        proxy: None,
        resolve: HostMapping::default(),
        egress: Policy::new(&["*"], &[] as &[&str]).unwrap(),
        clock: Clock::system(),
        resolver: ResolverConfiguration::name_servers([]),
        local: None,
        delays: goodfetch_engine::happy_eyeballs::Delays::default(),
    }
}

/// The source of a Session holding `127.0.0.<last>`.
fn source(last: u8) -> LocalSource {
    LocalSource {
        ipv4: Some(Ipv4Addr::new(127, 0, 0, last)),
        ..LocalSource::default()
    }
}

/// With a pool of two Sessions, a source used again shares its transport,
/// a third source drops the transport used longest ago, and that source,
/// used again, gets a new one.
#[test]
fn a_runs_session_transports_keep_those_used_last_as_many_as_its_pool() {
    let sessions = SessionTransports::new(config(), None, 2);

    let first = sessions.of(&source(2)).unwrap().0;
    let second = sessions.of(&source(3)).unwrap().0;
    let first_again = sessions.of(&source(2)).unwrap().0;
    let third = sessions.of(&source(4)).unwrap().0;
    let kept = (
        Arc::ptr_eq(&first, &first_again),
        Arc::ptr_eq(&first, &sessions.of(&source(2)).unwrap().0),
        Arc::ptr_eq(&third, &sessions.of(&source(4)).unwrap().0),
        Arc::ptr_eq(&second, &sessions.of(&source(3)).unwrap().0),
    );

    assert_eq!(kept, (true, true, true, false));
}
