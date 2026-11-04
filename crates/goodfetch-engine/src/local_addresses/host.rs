//! What this host lets a Session send from: its interfaces' addresses as
//! `netdev` reads them on every OS, whether a socket binds an address, and,
//! on Linux, whether a prefix is routed locally with
//! `net.ipv6.ip_nonlocal_bind` on, read from `/proc`.

use std::net::IpAddr;

use super::{Host, Ipv6Prefix};

/// The OS's own answers.
#[derive(Clone, Copy, Debug, Default)]
pub struct System;

impl Host for System {
    #[expect(
        clippy::disallowed_types,
        reason = "a probe bind, which sends nothing, tells whether the host can send from an address (ADR 0044)"
    )]
    fn holds(&self, address: IpAddr) -> bool {
        std::net::UdpSocket::bind((address, 0)).is_ok()
    }

    fn interface(&self, name: &str) -> Option<Vec<IpAddr>> {
        netdev::get_interfaces()
            .into_iter()
            .find(|interface| {
                interface.name == name || interface.friendly_name.as_deref() == Some(name)
            })
            .map(|interface| addresses_of(&interface))
    }

    fn addresses(&self) -> Vec<IpAddr> {
        netdev::get_interfaces()
            .iter()
            .flat_map(addresses_of)
            .collect()
    }

    fn binds_any_of(&self, prefix: Ipv6Prefix) -> bool {
        any_of_prefix(prefix)
    }
}

/// The addresses `interface` holds, IPv4 first.
fn addresses_of(interface: &netdev::Interface) -> Vec<IpAddr> {
    interface
        .ipv4
        .iter()
        .map(|network| IpAddr::V4(network.addr()))
        .chain(
            interface
                .ipv6
                .iter()
                .map(|network| IpAddr::V6(network.addr())),
        )
        .collect()
}

/// Whether Linux binds any address of `prefix`: `ip_nonlocal_bind` on and
/// a local route covering it.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "two small kernel files read once per Session start, outside any Run's storage"
)]
fn any_of_prefix(prefix: Ipv6Prefix) -> bool {
    let nonlocal = std::fs::read_to_string("/proc/sys/net/ipv6/ip_nonlocal_bind")
        .is_ok_and(|value| value.trim() == "1");
    nonlocal
        && std::fs::read_to_string("/proc/net/ipv6_route")
            .is_ok_and(|routes| routed_locally(&routes, prefix))
}

/// macOS and Windows bind only the addresses assigned to the host.
#[cfg(not(target_os = "linux"))]
fn any_of_prefix(_prefix: Ipv6Prefix) -> bool {
    false
}

/// The `RTF_LOCAL` flag of a route in `/proc/net/ipv6_route`.
#[cfg(any(target_os = "linux", test))]
const LOCAL_ROUTE: u32 = 0x8000_0000;

/// Whether `routes`, `/proc/net/ipv6_route`'s text, holds a local route
/// covering `prefix`. Each line is the destination's 32 hexadecimal
/// digits, its length in hexadecimal, the source and its length, the next
/// hop, the metric, two counters, the flags and the device.
#[cfg(any(target_os = "linux", test))]
pub(crate) fn routed_locally(routes: &str, prefix: Ipv6Prefix) -> bool {
    routes.lines().any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let (Some(destination), Some(length), Some(flags)) =
            (fields.first(), fields.get(1), fields.get(8))
        else {
            return false;
        };
        let (Ok(destination), Ok(length), Ok(flags)) = (
            u128::from_str_radix(destination, 16),
            u8::from_str_radix(length, 16),
            u32::from_str_radix(flags, 16),
        ) else {
            return false;
        };
        let route = Ipv6Prefix {
            network: destination.into(),
            length,
        };
        flags & LOCAL_ROUTE != 0
            && length <= prefix.length
            && length <= 128
            && route.contains(prefix.network)
    })
}
