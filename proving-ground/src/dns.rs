//! The DNS fixture (ADR 0067: rebinding is tested against it, never a public
//! rebinding service), on its own UDP and TCP address, and the zone it answers
//! from, which the forward proxies resolve through too. An answer has a TTL of
//! 0, so a resolver asks again each time, unless its name declares one:
//!
//! - each host name the Proving ground serves resolves to its address;
//! - a name under `alternating.dns.test` answers the entries its labels
//!   declare in turn, one per query of each record type, as
//!   `93-184-215-14.127-0-0-1.alternating.dns.test` alternates between a
//!   public-looking address and a private one;
//! - a name under `mixed.dns.test` answers all of them at once.
//!
//! An entry is an address, `nx` (the name does not exist, `NXDOMAIN`) or
//! `servfail` (a server failure, `SERVFAIL`). An IPv4 label is its four
//! numbers joined by `-`; an IPv6 label is its 32 hexadecimal digits; an A
//! query takes the IPv4 addresses and an AAAA query the IPv6 ones, and both
//! take every `nx` and `servfail`. A label `ttl-<seconds>` gives the name's
//! answers that TTL, and its negative answers (RFC 2308) an SOA whose TTL
//! and minimum are that TTL, so a resolver may keep them as long. Any other
//! name does not exist.
//!
//! Every query is recorded in the connection record with the address it
//! came from, so a test sees which local address a resolver asked from.
#![expect(
    clippy::disallowed_types,
    reason = "the DNS fixture opens its own UDP and TCP sockets, outside the engine's transport (ADR 0060)"
)]

mod message;

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::observation::Observations;
use crate::sync::Mutex;

use self::message::{AAAA, Rcode, Refusal};

/// The zone whose names answer their addresses in turn.
const ALTERNATING: &str = ".alternating.dns.test";
/// The zone whose names answer all their addresses at once.
const MIXED: &str = ".mixed.dns.test";

/// The most alternating names whose turn is kept (STYLE.md: every buffer is
/// bounded); a name beyond them answers its first address each time.
const ROTATION_LIMIT: usize = 100_000;

/// The largest message read; a larger one is cut and read as malformed.
const MESSAGE_LIMIT: usize = 4096;

/// How many ephemeral TCP ports are tried while matching a free UDP port.
const BIND_ATTEMPTS: usize = 64;

/// How many times a UDP reply retries on `ErrorKind::Interrupted`.
const UDP_SEND_ATTEMPTS: usize = 4;

/// An address family, as a record type asks for one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Family {
    V4,
    V6,
}

impl Family {
    fn of(address: IpAddr) -> Self {
        if address.is_ipv4() {
            Self::V4
        } else {
            Self::V6
        }
    }
}

/// One entry a name under `alternating.dns.test` or `mixed.dns.test`
/// declares.
#[derive(Clone, Copy)]
enum Entry {
    Address(IpAddr),
    NameError,
    ServerFailure,
}

impl Entry {
    /// Whether a query of `family` takes this entry.
    fn answers(self, family: Family) -> bool {
        match self {
            Self::Address(address) => Family::of(address) == family,
            Self::NameError | Self::ServerFailure => true,
        }
    }
}

/// What the fixture answers one query of one family.
#[derive(Debug, PartialEq, Eq)]
enum Reply {
    /// These addresses, maybe none, each with this TTL.
    Addresses(Vec<IpAddr>, u32),
    /// The name does not exist, a negative answer kept for this TTL.
    NameError(u32),
    /// A server failure.
    ServerFailure,
}

/// The names the fixture answers.
pub(crate) struct Zone {
    /// The host names the Proving ground serves, in lowercase.
    served: BTreeSet<String>,
    /// The address they resolve to.
    site: IpAddr,
    /// Each alternating name's next turn, per address family.
    turns: Mutex<BTreeMap<(String, Family), usize>>,
}

impl Zone {
    pub(crate) fn new<'a>(served: impl IntoIterator<Item = &'a String>, site: IpAddr) -> Self {
        Self {
            served: served
                .into_iter()
                .map(|name| name.to_ascii_lowercase())
                .collect(),
            site,
            turns: Mutex::default(),
        }
    }

    /// What `name` answers to a query of one family, which takes its turn.
    fn lookup(&self, name: &str, family: Family) -> Reply {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        if self.served.contains(&name) {
            return Reply::Addresses(
                Some(self.site)
                    .filter(|address| Family::of(*address) == family)
                    .into_iter()
                    .collect(),
                0,
            );
        }
        let (labels, alternating) = match (name.strip_suffix(ALTERNATING), name.strip_suffix(MIXED))
        {
            (Some(labels), _) => (labels, true),
            (None, Some(labels)) => (labels, false),
            (None, None) => return Reply::NameError(0),
        };
        let Some((entries, ttl)) = declared(labels) else {
            return Reply::NameError(0);
        };
        let entries: Vec<_> = entries
            .into_iter()
            .filter(|entry| entry.answers(family))
            .collect();
        if !alternating || entries.is_empty() {
            return mixed(&entries, ttl);
        }
        let turn = {
            let mut turns = self.turns.lock();
            let key = (name, family);
            if turns.len() >= ROTATION_LIMIT && !turns.contains_key(&key) {
                0
            } else {
                let turn = turns.entry(key).or_default();
                *turn += 1;
                *turn - 1
            }
        };
        mixed(&entries[turn % entries.len()..][..1], ttl)
    }

    /// The address a forward proxy connects to for `host`: the host itself
    /// when it is an address, else its first IPv4 answer, else its first
    /// IPv6 one. Each lookup takes its turn.
    pub(crate) fn resolve(&self, host: &str) -> Option<IpAddr> {
        let literal = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(address) = literal.parse() {
            return Some(address);
        }
        [Family::V4, Family::V6]
            .into_iter()
            .find_map(|family| match self.lookup(host, family) {
                Reply::Addresses(addresses, _) => addresses.into_iter().next(),
                Reply::NameError(_) | Reply::ServerFailure => None,
            })
    }

    /// The reply to one message, if it gets one.
    fn respond(&self, message: &[u8]) -> Option<Vec<u8>> {
        let query = match message::parse(message) {
            Ok(query) => query,
            Err(Refusal::Ignored) => return None,
            Err(Refusal::Answered { id, rcode }) => return Some(message::refusal(id, rcode)),
        };
        let family = match query.record_type {
            message::A => Some(Family::V4),
            AAAA => Some(Family::V6),
            _ => None,
        };
        let reply = match family {
            Some(family) => self.lookup(&query.name, family),
            // A name that exists answers no record of another type.
            None => match self.lookup(&query.name, Family::V4) {
                Reply::Addresses(_, ttl) => Reply::Addresses(Vec::new(), ttl),
                other => other,
            },
        };
        Some(match reply {
            Reply::Addresses(addresses, ttl) => {
                message::answer(&query, Rcode::NoError, &addresses, ttl)
            }
            Reply::NameError(ttl) => message::answer(&query, Rcode::NameError, &[], ttl),
            Reply::ServerFailure => message::answer(&query, Rcode::ServerFailure, &[], 0),
        })
    }
}

/// The entries and the TTL `labels` declare; `None` when a label is none of
/// an entry or a TTL.
fn declared(labels: &str) -> Option<(Vec<Entry>, u32)> {
    let mut entries = Vec::new();
    let mut ttl = 0;
    for label in labels.split('.') {
        match label {
            "nx" => entries.push(Entry::NameError),
            "servfail" => entries.push(Entry::ServerFailure),
            _ => match label.strip_prefix("ttl-") {
                Some(seconds) => ttl = seconds.parse().ok()?,
                None => entries.push(Entry::Address(address(label)?)),
            },
        }
    }
    Some((entries, ttl))
}

/// The reply that answers `entries` at once: a server failure if one is a
/// failure, else `NXDOMAIN` if one is, else every address.
fn mixed(entries: &[Entry], ttl: u32) -> Reply {
    if entries
        .iter()
        .any(|entry| matches!(entry, Entry::ServerFailure))
    {
        return Reply::ServerFailure;
    }
    if entries
        .iter()
        .any(|entry| matches!(entry, Entry::NameError))
    {
        return Reply::NameError(ttl);
    }
    Reply::Addresses(
        entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Address(address) => Some(*address),
                Entry::NameError | Entry::ServerFailure => None,
            })
            .collect(),
        ttl,
    )
}

/// An address label: four numbers joined by `-`, or 32 hexadecimal digits.
fn address(label: &str) -> Option<IpAddr> {
    if label.len() == 32 {
        let value = u128::from_str_radix(label, 16).ok()?;
        return Some(IpAddr::V6(Ipv6Addr::from(value)));
    }
    label
        .replace('-', ".")
        .parse::<Ipv4Addr>()
        .ok()
        .filter(|_| label.contains('-'))
        .map(IpAddr::V4)
}

/// The DNS fixture's bound UDP socket and TCP listener on the same address.
pub(crate) struct DnsServer {
    udp: UdpSocket,
    tcp: TcpListener,
    zone: Arc<Zone>,
    observations: Arc<Observations>,
}

impl DnsServer {
    /// Binds both TCP and UDP on `address`, port 0 a free port shared by both;
    /// every query it receives is recorded into `observations` with the
    /// address it came from.
    pub(crate) async fn bind(
        address: SocketAddr,
        zone: Arc<Zone>,
        observations: Arc<Observations>,
    ) -> Result<Self> {
        if address.port() != 0 {
            let tcp = TcpListener::bind(address)
                .await
                .with_context(|| format!("binding the DNS fixture's TCP listener on {address}"))?;
            let udp = UdpSocket::bind(address)
                .await
                .with_context(|| format!("binding the DNS fixture's UDP socket on {address}"))?;
            return Ok(Self {
                udp,
                tcp,
                zone,
                observations,
            });
        }
        for _ in 0..BIND_ATTEMPTS {
            let tcp = TcpListener::bind(address)
                .await
                .with_context(|| format!("binding the DNS fixture's TCP listener on {address}"))?;
            let bound = tcp
                .local_addr()
                .context("reading the DNS fixture's TCP address")?;
            match UdpSocket::bind(bound).await {
                Ok(udp) => {
                    return Ok(Self {
                        udp,
                        tcp,
                        zone,
                        observations,
                    });
                }
                Err(error) if error.kind() == ErrorKind::AddrInUse => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("binding the DNS fixture's UDP socket on {bound}")
                    });
                }
            }
        }
        bail!(
            "binding the DNS fixture on {address}: no shared TCP and UDP port free after {BIND_ATTEMPTS} attempts"
        );
    }

    /// The bound address.
    pub(crate) fn address(&self) -> Result<SocketAddr> {
        self.udp
            .local_addr()
            .context("reading the DNS fixture's address")
    }

    /// Answers every UDP and TCP query until `stop` is cancelled, spawning
    /// each TCP connection on `tracker`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future between messages loses none; one dropped while
    /// its UDP reply is sent is lost, as UDP may lose it anyway.
    pub(crate) async fn run(self, tracker: TaskTracker, stop: CancellationToken) {
        let mut buffer = vec![0; MESSAGE_LIMIT];
        loop {
            tokio::select! {
                () = stop.cancelled() => return,
                received = self.udp.recv_from(&mut buffer) => {
                    let Ok((length, client)) = received else {
                        // A failed read concerns only the client that sent it.
                        continue;
                    };
                    if let Some(reply) =
                        answer_query(&self.zone, &self.observations, client, &buffer[..length])
                    {
                        send_udp_reply(&self.udp, &reply, client).await;
                    }
                }
                accepted = self.tcp.accept() => {
                    let Ok((stream, client)) = accepted else {
                        continue;
                    };
                    let _ = stream.set_nodelay(true);
                    tracker.spawn(serve_tcp(
                        stream,
                        client,
                        Arc::clone(&self.zone),
                        Arc::clone(&self.observations),
                        stop.clone(),
                    ));
                }
            }
        }
    }
}

/// Records `message` if it parses as a query and returns the zone's reply.
fn answer_query(
    zone: &Zone,
    observations: &Observations,
    client: SocketAddr,
    message: &[u8],
) -> Option<Vec<u8>> {
    if let Ok(query) = message::parse(message) {
        let record_type = match query.record_type {
            message::A => "A".to_owned(),
            AAAA => "AAAA".to_owned(),
            other => other.to_string(),
        };
        observations.connection_record().dns_query(
            observations.stamp(),
            client,
            &query.name,
            &record_type,
        );
    }
    zone.respond(message)
}

/// Sends a UDP reply, retrying a signal interruption.
async fn send_udp_reply(socket: &UdpSocket, reply: &[u8], client: SocketAddr) {
    for _ in 0..UDP_SEND_ATTEMPTS {
        match socket.send_to(reply, client).await {
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Ok(_) | Err(_) => return,
        }
    }
}

/// Serves RFC 1035 length-prefixed DNS queries on one TCP connection until
/// EOF, a malformed frame, or `stop`.
async fn serve_tcp(
    mut stream: TcpStream,
    client: SocketAddr,
    zone: Arc<Zone>,
    observations: Arc<Observations>,
    stop: CancellationToken,
) {
    let mut length_bytes = [0_u8; 2];
    let mut buffer = Vec::new();
    loop {
        let read_length = tokio::select! {
            () = stop.cancelled() => return,
            result = stream.read_exact(&mut length_bytes) => result,
        };
        if read_length.is_err() {
            return;
        }
        let length = usize::from(u16::from_be_bytes(length_bytes));
        if length == 0 || length > MESSAGE_LIMIT {
            return;
        }
        buffer.resize(length, 0);
        let read_body = tokio::select! {
            () = stop.cancelled() => return,
            result = stream.read_exact(&mut buffer) => result,
        };
        if read_body.is_err() {
            return;
        }
        if let Some(reply) = answer_query(&zone, &observations, client, &buffer) {
            let Ok(reply_length) = u16::try_from(reply.len()) else {
                return;
            };
            let mut framed = Vec::with_capacity(2 + reply.len());
            framed.extend_from_slice(&reply_length.to_be_bytes());
            framed.extend_from_slice(&reply);
            let wrote = tokio::select! {
                () = stop.cancelled() => return,
                result = stream.write_all(&framed) => result,
            };
            if wrote.is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::ipv4("127-0-0-1", Some("127.0.0.1"))]
    #[case::ipv6("00000000000000000000ffff7f000001", Some("::ffff:127.0.0.1"))]
    #[case::dotted("127.0.0.1", None)]
    #[case::short("127-0-1", None)]
    #[case::word("public", None)]
    fn an_address_label_reads_as_its_address(#[case] label: &str, #[case] expected: Option<&str>) {
        assert_eq!(address(label), expected.map(|text| text.parse().unwrap()));
    }
}
