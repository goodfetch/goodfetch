//! IANA's IPv4 and IPv6 special-purpose address registries as a table, the
//! source of the resolved-address classification (ADRs 0017 and 0067).
//! `xtask codegen` writes [`BLOCKS`] from the registries committed in
//! `iana/` with their date, and fails when the two drift apart.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// One block of a registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    /// Its first address.
    pub address: IpAddr,
    /// Its prefix length.
    pub prefix: u8,
    /// The registry's name for it.
    pub name: &'static str,
    /// Its "Globally Reachable" column: `None` where the registry writes
    /// `N/A`, or nothing for a terminated block.
    pub globally_reachable: Option<bool>,
}

impl Block {
    /// Whether it holds `address`, of the same family.
    pub fn contains(&self, address: IpAddr) -> bool {
        match (self.address, address) {
            (IpAddr::V4(block), IpAddr::V4(address)) => {
                let mask = u32::MAX
                    .checked_shl(32 - u32::from(self.prefix))
                    .unwrap_or(0);
                u32::from(block) & mask == u32::from(address) & mask
            }
            (IpAddr::V6(block), IpAddr::V6(address)) => {
                let mask = u128::MAX
                    .checked_shl(128 - u32::from(self.prefix))
                    .unwrap_or(0);
                u128::from(block) & mask == u128::from(address) & mask
            }
            _ => false,
        }
    }
}

/// A committed registry's dates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registry {
    /// When IANA last updated it.
    pub updated: &'static str,
    /// When it was fetched and committed.
    pub committed: &'static str,
}

include!("../generated/special_purpose.rs");
