//! The local addresses a Session's connections leave from (ADR 0044):
//! `egress.local_addresses` read at a Run's start, which [`Rotation::check`]
//! refuses with `local_address_unavailable` when it yields no address the
//! host can send from, then one address of each family for each Session at
//! its start, [`Rotation::next_session`], new Sessions spread round-robin
//! across the blocks `egress.rotation_block.ipv4` and `.ipv6` size.
//!
//! - **A list** is the host's addresses, each of which the host must hold.
//! - **An interface** gives the addresses it holds when each Session starts;
//!   IPv6 link-local ones are left out, since a socket binds them only with
//!   their zone.
//! - **An IPv6 prefix** gives any address of it where the host lets a
//!   socket bind one (Linux, with `net.ipv6.ip_nonlocal_bind` on and the
//!   prefix routed locally), else the host's addresses inside it.
//!
//! What the host holds is read through [`Host`]: the OS's own in
//! [`System`], a table in tests.
#![expect(
    clippy::disallowed_types,
    reason = "a Run's count of the Sessions it bound, an atomic counter nothing else synchronizes on, which no loom test reaches"
)]

mod host;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicU64, Ordering};

use goodfetch_protocol::egress::{
    EGRESS_LOCAL_ADDRESSES, EGRESS_ROTATION_BLOCK_IPV4, EGRESS_ROTATION_BLOCK_IPV6,
    LOCAL_ADDRESS_UNAVAILABLE, LocalAddressFailure, LocalAddressUnavailableDetails,
};
pub use goodfetch_protocol::egress::{Ipv6Prefix, LocalAddresses};

pub use self::host::System;
use crate::CallError;
use crate::settings::Resolved;

/// What the host lets a Session send from.
pub trait Host: Send + Sync {
    /// Whether a socket can bind `address` on this host.
    fn holds(&self, address: IpAddr) -> bool;
    /// The addresses the interface `name` holds now; `None` when the host
    /// has no interface of that name.
    fn interface(&self, name: &str) -> Option<Vec<IpAddr>>;
    /// Every address the host's interfaces hold now.
    fn addresses(&self) -> Vec<IpAddr>;
    /// Whether a socket can bind any address of `prefix`, assigned or not.
    fn binds_any_of(&self, prefix: Ipv6Prefix) -> bool;
}

/// The local addresses one Session holds for its life: one of each family
/// at most.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionAddresses {
    /// The IPv4 address its connections bind.
    pub ipv4: Option<Ipv4Addr>,
    /// The IPv6 address its connections bind.
    pub ipv6: Option<Ipv6Addr>,
}

/// The sizes of the blocks Sessions are spread across, as prefix lengths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockSizes {
    /// `egress.rotation_block.ipv4`, from 0 to 32.
    pub ipv4: u8,
    /// `egress.rotation_block.ipv6`, from 0 to 128.
    pub ipv6: u8,
}

/// The catalog's defaults: a /24 and a /64.
impl Default for BlockSizes {
    fn default() -> Self {
        Self {
            ipv4: prefix_length(&(EGRESS_ROTATION_BLOCK_IPV4.default)()),
            ipv6: prefix_length(&(EGRESS_ROTATION_BLOCK_IPV6.default)()),
        }
    }
}

impl BlockSizes {
    /// The sizes `settings` give.
    pub fn of(settings: &Resolved) -> Self {
        Self {
            ipv4: prefix_length(settings.value(&EGRESS_ROTATION_BLOCK_IPV4)),
            ipv6: prefix_length(settings.value(&EGRESS_ROTATION_BLOCK_IPV6)),
        }
    }
}

/// A block size's value, which its shape holds to a prefix length of its
/// family.
fn prefix_length(value: &serde_json::Value) -> u8 {
    value
        .as_u64()
        .and_then(|size| u8::try_from(size).ok())
        .expect("invariant: a block size's shape holds a prefix length")
}

/// Why `egress.local_addresses` yields no address: `local_address_unavailable`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    /// The address, interface or prefix that yields nothing.
    pub entry: String,
    /// Why.
    pub failure: LocalAddressFailure,
}

impl From<Unavailable> for CallError {
    fn from(unavailable: Unavailable) -> Self {
        let entry = &unavailable.entry;
        let message = match unavailable.failure {
            LocalAddressFailure::NotHeld => {
                format!(
                    "this host holds no local address {entry}: assign it, or remove it from `egress.local_addresses`"
                )
            }
            LocalAddressFailure::UnknownInterface => {
                format!("this host has no network interface `{entry}`")
            }
            LocalAddressFailure::InterfaceWithoutAddress => {
                format!("the network interface `{entry}` holds no address a Session can send from")
            }
            LocalAddressFailure::PrefixUnusable => format!(
                "the prefix {entry} holds none of this host's addresses; on Linux, route it locally and set `net.ipv6.ip_nonlocal_bind` to 1 to send from any of its addresses"
            ),
            LocalAddressFailure::NotLaunchedHere => format!(
                "`egress.local_addresses` ({entry}) binds only the browsers this engine launches on this machine, never a Borrowed browser, a third-party CDP browser or a Browser host's: open the Session on a browser the engine launches, or leave `egress.local_addresses` unset for its Run"
            ),
            LocalAddressFailure::ProxyNotRelayed => format!(
                "`egress.local_addresses` ({entry}) cannot hold a Session behind an `https://` proxy, which the egress relay does not chain through, so its browser would reach the proxy from an address the OS chooses: use an `http://` or `socks5://` proxy, or leave `egress.local_addresses` unset"
            ),
        };
        Self::new(
            &LOCAL_ADDRESS_UNAVAILABLE,
            message,
            &LocalAddressUnavailableDetails {
                entry: unavailable.entry,
                failure: unavailable.failure,
                setting: EGRESS_LOCAL_ADDRESSES.name.to_owned(),
            },
        )
    }
}

/// A Run's local addresses and the Sessions it has bound: each new Session
/// takes the next block of each family.
#[derive(Debug)]
pub struct Rotation {
    addresses: LocalAddresses,
    blocks: BlockSizes,
    sessions: AtomicU64,
}

/// Where a family's addresses come from at one Session's start.
enum Pool {
    /// These addresses, sorted, each once.
    Listed(Vec<IpAddr>),
    /// Any address of the prefix.
    Any(Ipv6Prefix),
}

impl Rotation {
    /// The rotation `settings` ask for; none without `egress.local_addresses`.
    pub fn of(settings: &Resolved) -> Option<Self> {
        let addresses = LocalAddresses::parse(settings.value(&EGRESS_LOCAL_ADDRESSES)).ok()?;
        Some(Self::new(addresses, BlockSizes::of(settings)))
    }

    /// A rotation over `addresses` in blocks of `blocks`.
    pub fn new(addresses: LocalAddresses, blocks: BlockSizes) -> Self {
        Self {
            addresses,
            blocks,
            sessions: AtomicU64::new(0),
        }
    }

    /// Whether the addresses give a Session an address on `host`, as a Run
    /// checks at its start: every listed address held, the interface
    /// present with an address, or the prefix usable.
    ///
    /// # Errors
    ///
    /// [`Unavailable`] for the first entry that yields nothing.
    pub fn check(&self, host: &dyn Host) -> Result<(), Unavailable> {
        if let LocalAddresses::Addresses(listed) = &self.addresses
            && let Some(missing) = listed.iter().find(|address| !host.holds(**address))
        {
            return Err(Unavailable {
                entry: missing.to_string(),
                failure: LocalAddressFailure::NotHeld,
            });
        }
        self.pools(host).map(drop)
    }

    /// The addresses the next Session holds for its life: the next block of
    /// each family the addresses give on `host` now, an interface's read
    /// now.
    ///
    /// # Errors
    ///
    /// [`Unavailable`] when they give no address now, such as an interface
    /// that lost its addresses since the Run started.
    pub fn next_session(&self, host: &dyn Host) -> Result<SessionAddresses, Unavailable> {
        let pools = self.pools(host)?;
        let session = self.sessions.fetch_add(1, Ordering::Relaxed);
        Ok(Self::pick(&pools, self.blocks, session))
    }

    /// The addresses the `ordinal`-th Session of the Run holds for its
    /// life, counted from 0, as [`Self::next_session`] would give it: the
    /// same Session drawn again, such as one reopened from run state, takes
    /// the same addresses while the host holds them.
    ///
    /// # Errors
    ///
    /// [`Unavailable`] when they give no address now.
    pub fn session(&self, host: &dyn Host, ordinal: u64) -> Result<SessionAddresses, Unavailable> {
        let pools = self.pools(host)?;
        Ok(Self::pick(&pools, self.blocks, ordinal))
    }

    /// The addresses of each family's pool the `session`-th Session takes.
    fn pick(
        (ipv4, ipv6): &(Option<Pool>, Option<Pool>),
        blocks: BlockSizes,
        session: u64,
    ) -> SessionAddresses {
        let pick = |pool: &Pool, block: u8| match pool {
            Pool::Listed(addresses) => spread(addresses, block, session),
            Pool::Any(prefix) => Some(IpAddr::V6(in_prefix(*prefix, block, session))),
        };
        SessionAddresses {
            ipv4: ipv4
                .as_ref()
                .and_then(|pool| pick(pool, blocks.ipv4))
                .and_then(|address| match address {
                    IpAddr::V4(address) => Some(address),
                    IpAddr::V6(_) => None,
                }),
            ipv6: ipv6
                .as_ref()
                .and_then(|pool| pick(pool, blocks.ipv6))
                .and_then(|address| match address {
                    IpAddr::V6(address) => Some(address),
                    IpAddr::V4(_) => None,
                }),
        }
    }

    /// Each family's pool on `host` now; none for a family without
    /// addresses, and an error when neither has one.
    fn pools(&self, host: &dyn Host) -> Result<(Option<Pool>, Option<Pool>), Unavailable> {
        let listed = match &self.addresses {
            LocalAddresses::Addresses(listed) => listed.clone(),
            LocalAddresses::Interface(name) => {
                let Some(held) = host.interface(name) else {
                    return Err(Unavailable {
                        entry: name.clone(),
                        failure: LocalAddressFailure::UnknownInterface,
                    });
                };
                let usable: Vec<IpAddr> = held.into_iter().filter(|a| !link_local(*a)).collect();
                if usable.is_empty() {
                    return Err(Unavailable {
                        entry: name.clone(),
                        failure: LocalAddressFailure::InterfaceWithoutAddress,
                    });
                }
                usable
            }
            LocalAddresses::Prefix(prefix) => {
                if host.binds_any_of(*prefix) {
                    return Ok((None, Some(Pool::Any(*prefix))));
                }
                let inside: Vec<IpAddr> = host
                    .addresses()
                    .into_iter()
                    .filter(|address| match address {
                        IpAddr::V6(address) => prefix.contains(*address),
                        IpAddr::V4(_) => false,
                    })
                    .filter(|address| !link_local(*address) && host.holds(*address))
                    .collect();
                if inside.is_empty() {
                    return Err(Unavailable {
                        entry: prefix.to_string(),
                        failure: LocalAddressFailure::PrefixUnusable,
                    });
                }
                inside
            }
        };
        let of = |ipv4: bool| {
            let mut family: Vec<IpAddr> = listed
                .iter()
                .copied()
                .filter(|address| address.is_ipv4() == ipv4)
                .collect();
            family.sort_unstable();
            family.dedup();
            (!family.is_empty()).then_some(Pool::Listed(family))
        };
        Ok((of(true), of(false)))
    }
}

/// Whether `address` is an IPv6 link-local one, which binds only with its
/// zone.
fn link_local(address: IpAddr) -> bool {
    matches!(address, IpAddr::V6(address) if address.segments()[0] & 0xffc0 == 0xfe80)
}

/// `address` as a 128-bit number and its width in bits.
fn bits_of(address: IpAddr) -> (u128, u32) {
    match address {
        IpAddr::V4(address) => (u128::from(u32::from(address)), 32),
        IpAddr::V6(address) => (u128::from(address), 128),
    }
}

/// The block of `block` bits `address` is in: its first `block` bits.
pub fn block_of(address: IpAddr, block: u8) -> u128 {
    let (bits, width) = bits_of(address);
    let kept = width.saturating_sub(u32::from(block));
    bits.checked_shr(kept).unwrap_or(0)
}

/// The address the `session`-th Session takes of `addresses`, sorted and
/// each once, spread across blocks of `block` bits: the blocks in turn,
/// each block's addresses in turn on each round. None without an address.
pub fn spread(addresses: &[IpAddr], block: u8, session: u64) -> Option<IpAddr> {
    let mut blocks: BTreeMap<u128, Vec<IpAddr>> = BTreeMap::new();
    for address in addresses {
        blocks
            .entry(block_of(*address, block))
            .or_default()
            .push(*address);
    }
    let count = u64::try_from(blocks.len()).ok()?;
    if count == 0 {
        return None;
    }
    let index = usize::try_from(session % count).ok()?;
    let members = blocks.into_values().nth(index)?;
    let round = session / count;
    let member = usize::try_from(round % u64::try_from(members.len()).ok()?).ok()?;
    members.get(member).copied()
}

/// The address the `session`-th Session takes of `prefix`, spread across
/// its blocks of `block` bits: the blocks in turn, and in each block its
/// addresses from the first after the block's own, round by round. A block
/// no longer than the prefix is the prefix itself.
pub fn in_prefix(prefix: Ipv6Prefix, block: u8, session: u64) -> Ipv6Addr {
    let block = u32::from(block.max(prefix.length));
    let network = u128::from(prefix.network);
    let block_bits = block - u32::from(prefix.length);
    let host_bits = 128 - block;
    let session = u128::from(session);
    let (index, round) = if block_bits >= 64 {
        (session, 0)
    } else {
        let blocks = 1_u128 << block_bits;
        (session % blocks, session / blocks)
    };
    let offset = match host_bits {
        0 => 0,
        bits => {
            // A block's own address, all host bits zero, is its Subnet-Router
            // anycast address on IPv6: each round starts past it.
            let size = 1_u128.checked_shl(bits).unwrap_or(0).wrapping_sub(1);
            if size == 0 { 0 } else { round % size + 1 }
        }
    };
    let index_shifted = index.checked_shl(host_bits).unwrap_or(0);
    Ipv6Addr::from(network | index_shifted | offset)
}

#[cfg(test)]
mod tests;
