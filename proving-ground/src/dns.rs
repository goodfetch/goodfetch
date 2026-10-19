//! The DNS fixture (ADR 0067: rebinding is tested against it, never a public
//! rebinding service), on its own UDP address, and the zone it answers from,
//! which the forward proxies resolve through too. Every answer has a TTL of
//! 0, so a resolver asks again each time:
//!
//! - each host name the Proving ground serves resolves to its address;
//! - a name under `alternating.dns.test` answers the addresses its labels
//!   declare in turn, one per query of each record type, as
//!   `93-184-215-14.127-0-0-1.alternating.dns.test` alternates between a
//!   public-looking address and a private one;
//! - a name under `mixed.dns.test` answers all of them at once.
//!
//! An IPv4 label is its four numbers joined by `-`; an IPv6 label is its 32
//! hexadecimal digits. Any other name does not exist (`NXDOMAIN`).
#![expect(
    clippy::disallowed_types,
    reason = "the DNS fixture opens its own UDP socket, outside the engine's transport (ADR 0060)"
)]

mod message;

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use tokio::net::UdpSocket;

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

    /// The addresses `name` answers to a query of one family, which take
    /// their turn; `None` when the name does not exist.
    fn lookup(&self, name: &str, family: Family) -> Option<Vec<IpAddr>> {
        let name = name.trim_end_matches('.').to_ascii_lowercase();
        if self.served.contains(&name) {
            return Some(
                Some(self.site)
                    .filter(|address| Family::of(*address) == family)
                    .into_iter()
                    .collect(),
            );
        }
        let (labels, alternating) = match (name.strip_suffix(ALTERNATING), name.strip_suffix(MIXED))
        {
            (Some(labels), _) => (labels, true),
            (None, Some(labels)) => (labels, false),
            (None, None) => return None,
        };
        let declared = labels.split('.').map(address).collect::<Option<Vec<_>>>()?;
        let addresses: Vec<_> = declared
            .into_iter()
            .filter(|address| Family::of(*address) == family)
            .collect();
        if !alternating || addresses.is_empty() {
            return Some(addresses);
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
        Some(vec![addresses[turn % addresses.len()]])
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
            .find_map(|family| self.lookup(host, family)?.into_iter().next())
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
        let answer = match family {
            Some(family) => self.lookup(&query.name, family),
            // A name that exists answers no record of another type.
            None => self.lookup(&query.name, Family::V4).map(|_| Vec::new()),
        };
        Some(match answer {
            Some(addresses) => message::answer(&query, Rcode::NoError, &addresses),
            None => message::answer(&query, Rcode::NameError, &[]),
        })
    }
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

/// The DNS fixture's bound socket.
pub(crate) struct DnsServer {
    socket: UdpSocket,
    zone: Arc<Zone>,
}

impl DnsServer {
    /// Binds the address; port 0 binds a free port.
    pub(crate) async fn bind(address: SocketAddr, zone: Arc<Zone>) -> Result<Self> {
        let socket = UdpSocket::bind(address)
            .await
            .with_context(|| format!("binding the DNS fixture on {address}"))?;
        Ok(Self { socket, zone })
    }

    /// The bound address.
    pub(crate) fn address(&self) -> Result<SocketAddr> {
        self.socket
            .local_addr()
            .context("reading the DNS fixture's address")
    }

    /// Answers every query until dropped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future between messages loses none; one dropped while
    /// its reply is sent is lost, as UDP may lose it anyway.
    pub(crate) async fn run(self) {
        let mut buffer = vec![0; MESSAGE_LIMIT];
        loop {
            let Ok((length, client)) = self.socket.recv_from(&mut buffer).await else {
                // A failed read concerns only the client that sent it.
                continue;
            };
            if let Some(reply) = self.zone.respond(&buffer[..length]) {
                let _ = self.socket.send_to(&reply, client).await;
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
