//! The connection record (ADR 0067's oracle of "no connection reached the
//! target"): every connection a listener of the Proving ground accepted
//! (its site's, each forward proxy's and the target's), and every relay a
//! forward proxy carried out or refused, with the connection it attempted
//! for it. Each entry carries the time step it came at, its source and its
//! destination. A connection a proxy opened names its relay: the proxy
//! binds its side before connecting, so the relay is recorded before the
//! connection it opens is accepted.
//!
//! It also holds every query the DNS fixture received, with the address it
//! came from, so a test sees which local address a resolver asked from,
//! and every datagram the UDP fixture received, so a test sees that no UDP
//! left from another address than a Session's (ADR 0044).
//!
//! At most [`ENTRY_LIMIT`] connections, as many relays, queries and
//! datagrams are kept; later ones are counted in `unrecorded_connections`,
//! `unrecorded_relays`, `unrecorded_dns_queries` and
//! `unrecorded_udp_datagrams`.

use std::collections::BTreeMap;
use std::net::SocketAddr;

use serde::Serialize;

use crate::sync::Mutex;

/// The most connections, and the most relays, kept (STYLE.md: every buffer
/// is bounded).
const ENTRY_LIMIT: usize = 100_000;

/// When an entry came: the Proving ground's time step then, and the
/// microseconds since `serve` started.
#[derive(Clone, Copy, Serialize)]
pub(crate) struct Stamp {
    pub(crate) time_step: u64,
    pub(crate) at_microseconds: u64,
}

/// The record of one `serve`.
#[derive(Default)]
pub(crate) struct ConnectionRecord {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// The number of connections accepted, recorded or not: the next one's
    /// ID.
    accepted: u64,
    connections: Vec<Accepted>,
    unrecorded_connections: u64,
    relays: Vec<Relay>,
    unrecorded_relays: u64,
    dns_queries: Vec<DnsQuery>,
    unrecorded_dns_queries: u64,
    udp_datagrams: Vec<UdpDatagram>,
    unrecorded_udp_datagrams: u64,
    /// The latest relay whose connection left from each proxy-side address.
    relay_sources: BTreeMap<SocketAddr, usize>,
}

/// A connection a listener accepted.
#[derive(Serialize)]
struct Accepted {
    id: u64,
    #[serde(flatten)]
    stamp: Stamp,
    listener: &'static str,
    source: SocketAddr,
    destination: SocketAddr,
    /// The relay that opened it, when a forward proxy did.
    relay: Option<usize>,
}

/// A query the DNS fixture received.
#[derive(Serialize)]
struct DnsQuery {
    #[serde(flatten)]
    stamp: Stamp,
    /// The address and port it came from.
    source: SocketAddr,
    /// The name asked for, in lowercase, without its trailing dot.
    name: String,
    /// Its record type: `A`, `AAAA`, or the type's number.
    record_type: String,
}

/// A datagram the UDP fixture received.
#[derive(Serialize)]
struct UdpDatagram {
    #[serde(flatten)]
    stamp: Stamp,
    /// The address and port it came from.
    source: SocketAddr,
    /// The fixture's address it reached.
    destination: SocketAddr,
    /// Its length in bytes.
    length: usize,
}

/// What a forward proxy did with one request.
#[derive(Serialize)]
pub(crate) struct Relay {
    id: usize,
    #[serde(flatten)]
    stamp: Stamp,
    #[serde(flatten)]
    request: RelayRequest,
    /// The address the proxy connected to or would have, once resolved.
    destination: Option<SocketAddr>,
    /// The proxy's side of the connection it attempted.
    source: Option<SocketAddr>,
    outcome: Outcome,
    bytes: Carried,
}

/// What a proxy received from its client and sent to it on one relay, its
/// own answer included.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Carried {
    pub(crate) received: u64,
    pub(crate) sent: u64,
}

/// The request a relay is for, as the proxy read it.
#[derive(Serialize)]
pub(crate) struct RelayRequest {
    /// The proxy's name, as `serve`'s output line reports it.
    pub(crate) proxy: &'static str,
    /// The proxy kind it serves, if it is one of a kind's.
    pub(crate) kind: Option<&'static str>,
    /// The ID of the client's connection to the proxy.
    pub(crate) connection: u64,
    pub(crate) method: String,
    /// The request target as sent: `host:port` for `CONNECT`, an absolute
    /// URL otherwise.
    pub(crate) target: String,
    /// The task of the target's host, when the Proving ground serves it.
    pub(crate) task: Option<String>,
    /// The username of its `Proxy-Authorization`, never its password.
    pub(crate) username: Option<String>,
    pub(crate) sticky: Option<Sticky>,
    /// The exit the proxy relayed through.
    pub(crate) exit: Option<String>,
}

/// The sticky session a provider's username asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Sticky {
    /// The Proxy provider adapter whose format the username follows.
    pub(crate) provider: &'static str,
    pub(crate) session: Option<String>,
    pub(crate) country: Option<String>,
    pub(crate) lifetime_minutes: Option<u64>,
}

/// How a relay ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    /// The proxy connected and relayed the request or the tunnel.
    Relayed,
    /// The request carried no credentials the proxy accepts (`407`).
    ProxyAuthenticationRequired,
    /// The proxy refuses `CONNECT` (`403`).
    TunnelRefused,
    /// The request was not one the proxy reads (`400`).
    Malformed,
    /// The target's host is neither served nor declared (`502`).
    Unresolved,
    /// The target resolved outside this host's loopback (`403`), so no test
    /// ever reaches a public service.
    DestinationProhibited,
    /// Nothing accepted the connection (`502`).
    ConnectionRefused,
    /// The proxy relayed another request already, past its plan (`429`).
    TooManyRequests,
}

/// A relay's place in the record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RelayId(usize);

/// What `GET /observations` serves of the record.
#[derive(Serialize)]
pub(crate) struct Document<'a> {
    connections: &'a [Accepted],
    unrecorded_connections: u64,
    relays: &'a [Relay],
    unrecorded_relays: u64,
    dns_queries: &'a [DnsQuery],
    unrecorded_dns_queries: u64,
    udp_datagrams: &'a [UdpDatagram],
    unrecorded_udp_datagrams: u64,
}

impl ConnectionRecord {
    /// Records a connection `listener` accepted and returns its ID; one a
    /// forward proxy opened names its relay.
    pub(crate) fn accepted(
        &self,
        stamp: Stamp,
        listener: &'static str,
        source: SocketAddr,
        destination: SocketAddr,
    ) -> u64 {
        let mut state = self.state.lock();
        let id = state.accepted;
        state.accepted += 1;
        if state.connections.len() >= ENTRY_LIMIT {
            state.unrecorded_connections += 1;
            return id;
        }
        let relay = state.relay_sources.get(&source).copied();
        state.connections.push(Accepted {
            id,
            stamp,
            listener,
            source,
            destination,
            relay,
        });
        id
    }

    /// Records a relay, with its outcome so far.
    pub(crate) fn relay(
        &self,
        stamp: Stamp,
        request: RelayRequest,
        destination: Option<SocketAddr>,
        source: Option<SocketAddr>,
        outcome: Outcome,
    ) -> Option<RelayId> {
        let mut state = self.state.lock();
        if state.relays.len() >= ENTRY_LIMIT {
            state.unrecorded_relays += 1;
            return None;
        }
        let id = state.relays.len();
        if let Some(source) = source {
            state.relay_sources.insert(source, id);
        }
        state.relays.push(Relay {
            id,
            stamp,
            request,
            destination,
            source,
            outcome,
            bytes: Carried::default(),
        });
        Some(RelayId(id))
    }

    /// Records how a relay ended and the bytes it carried.
    pub(crate) fn ended(&self, relay: Option<RelayId>, outcome: Outcome, bytes: Carried) {
        let Some(RelayId(id)) = relay else {
            return;
        };
        if let Some(relay) = self.state.lock().relays.get_mut(id) {
            relay.outcome = outcome;
            relay.bytes = bytes;
        }
    }

    /// Records a query the DNS fixture received from `source`.
    pub(crate) fn dns_query(
        &self,
        stamp: Stamp,
        source: SocketAddr,
        name: &str,
        record_type: &str,
    ) {
        let mut state = self.state.lock();
        if state.dns_queries.len() >= ENTRY_LIMIT {
            state.unrecorded_dns_queries += 1;
            return;
        }
        state.dns_queries.push(DnsQuery {
            stamp,
            source,
            name: name.to_owned(),
            record_type: record_type.to_owned(),
        });
    }

    /// Records a datagram of `length` bytes the UDP fixture at `destination`
    /// received from `source`.
    pub(crate) fn udp_datagram(
        &self,
        stamp: Stamp,
        source: SocketAddr,
        destination: SocketAddr,
        length: usize,
    ) {
        let mut state = self.state.lock();
        if state.udp_datagrams.len() >= ENTRY_LIMIT {
            state.unrecorded_udp_datagrams += 1;
            return;
        }
        state.udp_datagrams.push(UdpDatagram {
            stamp,
            source,
            destination,
            length,
        });
    }

    /// Serializes the record into `GET /observations`'s document.
    pub(crate) fn with_document<T>(&self, write: impl FnOnce(&Document<'_>) -> T) -> T {
        let state = self.state.lock();
        write(&Document {
            connections: &state.connections,
            unrecorded_connections: state.unrecorded_connections,
            relays: &state.relays,
            unrecorded_relays: state.unrecorded_relays,
            dns_queries: &state.dns_queries,
            unrecorded_dns_queries: state.unrecorded_dns_queries,
            udp_datagrams: &state.udp_datagrams,
            unrecorded_udp_datagrams: state.unrecorded_udp_datagrams,
        })
    }
}
