//! The transports of a Run's Sessions' local sources (ADR 0044; finding
//! 565-7, #2369): a source's transport is built once and shared across open
//! Sessions on it, and dropped as soon as the last open Session holding that
//! source closes.
//!
//! Covers: story:4-108

use std::net::Ipv4Addr;
use std::sync::Arc;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::tls::{CertificateExceptions, Trust};
use goodfetch_protocol::page::SessionId;

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
        resolver: ResolverConfiguration::name_servers([]).into(),
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

fn session(id: &str) -> SessionId {
    SessionId(id.to_owned())
}

fn kept_count(sessions: &SessionTransports) -> usize {
    sessions
        .built
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .transports
        .len()
}

/// After the last Session holding a local source closes, the tier holds no
/// transport for that source, and a later Session given the same source
/// builds a new one.
#[test]
fn after_the_last_session_for_a_local_source_closes_its_transport_is_dropped_and_rebuilt() {
    let sessions = SessionTransports::new(config(), None);
    let local = source(2);

    let first = sessions.of(&session("s1"), &local).unwrap().0;
    let same_session = sessions.of(&session("s1"), &local).unwrap().0;
    assert!(Arc::ptr_eq(&first, &same_session));
    assert_eq!(kept_count(&sessions), 1);

    sessions.forget(&session("s1"));
    assert_eq!(kept_count(&sessions), 0);

    let rebuilt = sessions.of(&session("s2"), &local).unwrap().0;
    assert!(!Arc::ptr_eq(&first, &rebuilt));
    assert_eq!(kept_count(&sessions), 1);
}

/// A source still held by an open Session keeps its transport when another
/// Session on the same source closes, and drops it once the last open
/// Session closes.
#[test]
fn a_local_source_keeps_its_transport_while_another_open_session_holds_it() {
    let sessions = SessionTransports::new(config(), None);
    let local = source(3);

    let first = sessions.of(&session("s1"), &local).unwrap().0;
    let second = sessions.of(&session("s2"), &local).unwrap().0;
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(kept_count(&sessions), 1);

    sessions.forget(&session("s1"));
    let third = sessions.of(&session("s3"), &local).unwrap().0;
    assert!(Arc::ptr_eq(&first, &third));
    assert_eq!(kept_count(&sessions), 1);

    sessions.forget(&session("s2"));
    assert_eq!(kept_count(&sessions), 1);

    sessions.forget(&session("s3"));
    assert_eq!(kept_count(&sessions), 0);

    let fresh = sessions.of(&session("s4"), &local).unwrap().0;
    assert!(!Arc::ptr_eq(&first, &fresh));
}
