//! The resolved-address classification (ADRs 0017 and 0067): whether an
//! address a connection uses is one a page may reach. A page may not reach
//! an address of a block of IANA's special-purpose registries that is not
//! globally reachable ([`BLOCKS`], generated from the committed registries),
//! a cloud-metadata address, or an address outside global unicast (IPv4
//! multicast, and IPv6 outside `2000::/3`, which holds the deprecated
//! site-local and IPv4-compatible forms). A block the registries mark
//! neither way, a terminated one, is refused too: the check fails closed.
//!
//! IPv4-mapped, NAT64 (`64:ff9b::/96` and `64:ff9b:1::/48`), 6to4 and
//! Teredo addresses are judged by the IPv4 address they embed, so no
//! encoding hides a private one; under the local-use NAT64 prefix, whose
//! network picks the prefix length, every length RFC 6052 allows within it
//! is read, and the first reading a page may not reach decides. An IPv6
//! address with a zone ID names a link, never a host on the internet, and
//! is refused.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use goodfetch_protocol::special_purpose::{BLOCKS, Block};

/// What an address is to a request a page makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AddressClass {
    /// Globally reachable: a page may reach it.
    Global,
    /// A cloud-metadata address: `169.254.169.254`, `100.100.100.200` or
    /// `fd00:ec2::254`, in any form that embeds it.
    Metadata,
    /// In this block of IANA's special-purpose registries, the most
    /// specific that holds it, which is not globally reachable.
    SpecialPurpose(&'static Block),
    /// Outside the address space hosts are reached at: IPv4 multicast, or
    /// IPv6 outside global unicast (`2000::/3`).
    OutsideGlobalUnicast,
    /// An IPv6 address with a zone ID.
    ZoneId,
}

impl AddressClass {
    /// Whether a page may reach it.
    pub fn is_global(self) -> bool {
        self == Self::Global
    }
}

/// The cloud-metadata addresses: AWS's, Google Cloud's and Azure's IPv4
/// one, Alibaba Cloud's, and AWS's IPv6 one.
const METADATA: [IpAddr; 3] = [
    IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
    IpAddr::V4(Ipv4Addr::new(100, 100, 100, 200)),
    IpAddr::V6(Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254)),
];

/// Classifies `address`, as a connection to it would reach it.
pub fn classify(address: IpAddr) -> AddressClass {
    if METADATA.contains(&address) {
        return AddressClass::Metadata;
    }
    match address {
        IpAddr::V4(address) => classify_ipv4(address),
        IpAddr::V6(address) => classify_ipv6(address),
    }
}

/// Classifies the socket address a connection uses: an IPv6 one with a
/// zone ID is refused whatever its address.
pub fn classify_socket(address: SocketAddr) -> AddressClass {
    match address {
        SocketAddr::V6(address) if address.scope_id() != 0 => AddressClass::ZoneId,
        address => classify(address.ip()),
    }
}

fn classify_ipv4(address: Ipv4Addr) -> AddressClass {
    if let Some(class) = by_registry(IpAddr::V4(address)) {
        return class;
    }
    if address.is_multicast() {
        return AddressClass::OutsideGlobalUnicast;
    }
    AddressClass::Global
}

fn classify_ipv6(address: Ipv6Addr) -> AddressClass {
    let embedded = embedded_ipv4(address);
    if !embedded.is_empty() {
        return embedded
            .into_iter()
            .map(|address| classify(IpAddr::V4(address)))
            .find(|class| !class.is_global())
            .unwrap_or(AddressClass::Global);
    }
    if let Some(class) = by_registry(IpAddr::V6(address)) {
        return class;
    }
    if address.to_bits() >> 125 != 0b001 {
        return AddressClass::OutsideGlobalUnicast;
    }
    AddressClass::Global
}

/// The class the most specific block holding `address` gives, if one does.
fn by_registry(address: IpAddr) -> Option<AddressClass> {
    let block = BLOCKS
        .iter()
        .filter(|block| block.contains(address))
        .max_by_key(|block| block.prefix)?;
    Some(if block.globally_reachable == Some(true) {
        AddressClass::Global
    } else {
        AddressClass::SpecialPurpose(block)
    })
}

/// The IPv4 addresses an IPv6 address of a form that embeds one stands for,
/// in the order they are judged; none for any other address.
fn embedded_ipv4(address: Ipv6Addr) -> Vec<Ipv4Addr> {
    let bits = address.to_bits();
    // The `width` bits that end `shift` bits before the address's end.
    let field = |shift: u32, width: u32| -> u32 {
        u32::try_from((bits >> shift) & ((1 << width) - 1))
            .expect("invariant: a field of at most 32 bits")
    };
    if let Some(mapped) = address.to_ipv4_mapped() {
        return vec![mapped];
    }
    let ipv4 = Ipv4Addr::from_bits;
    match address.segments() {
        // NAT64's well-known prefix (RFC 6052).
        [0x64, 0xff9b, 0, 0, 0, 0, _, _] => vec![ipv4(field(0, 32))],
        // NAT64's local-use prefix (RFC 8215), read at the prefix lengths
        // 96, 64, 56 and 48, the octet at bits 64 to 71 skipped.
        [0x64, 0xff9b, 1, ..] => vec![
            ipv4(field(0, 32)),
            ipv4(field(24, 32)),
            ipv4(field(64, 8) << 24 | field(32, 24)),
            ipv4(field(64, 16) << 16 | field(40, 16)),
        ],
        // 6to4 (RFC 3056): the IPv4 address follows the prefix.
        [0x2002, ..] => vec![ipv4(field(80, 32))],
        // Teredo (RFC 4380): the server's address, then the client's,
        // written inverted.
        [0x2001, 0, ..] => vec![ipv4(field(64, 32)), ipv4(!field(0, 32))],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
