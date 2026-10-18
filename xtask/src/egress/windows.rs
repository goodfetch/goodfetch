//! The deny on the Release suite's Windows cell, a GitHub-hosted runner
//! whose own agent must keep its connection: Windows Firewall rules block
//! the product's programs (ADR 0043) from every address but loopback and the
//! job's allowed endpoints, and the firewall's log of dropped packets is the
//! record the canary's verdict and the end-of-job check read.

use std::collections::{BTreeMap, BTreeSet};

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use anyhow::{Context as _, Result, bail};

/// One block rule's remote side, as `ci/egress/windows.ps1` applies it to
/// each program: `<protocol> <remote addresses> <remote ports>`, where `*`
/// is any port.
fn rule(protocol: &str, addresses: &str, ports: &str) -> String {
    format!("{protocol} {addresses} {ports}\n")
}

fn range_text<T: PartialEq + std::fmt::Display>(start: &T, end: &T) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

/// The ranges of `0..=max` outside the sorted, merged `excluded` intervals.
fn complement(excluded: &BTreeSet<(u128, u128)>, max: u128) -> Vec<(u128, u128)> {
    let mut ranges = Vec::new();
    let mut next = Some(0_u128);
    for &(start, end) in excluded {
        let Some(from) = next else {
            break;
        };
        if start > from {
            ranges.push((from, start - 1));
        }
        if end >= from {
            next = end.checked_add(1).filter(|value| *value <= max);
        }
    }
    if let Some(from) = next {
        ranges.push((from, max));
    }
    ranges
}

fn port_complement(allowed: &BTreeSet<u16>) -> String {
    let excluded: BTreeSet<(u128, u128)> = allowed
        .iter()
        .map(|port| (u128::from(*port), u128::from(*port)))
        .collect();
    complement(&excluded, u128::from(u16::MAX))
        .into_iter()
        .filter_map(|(start, end)| {
            // Port 0 is no port a connection can name.
            let start = start.max(1);
            (start <= end).then(|| range_text(&start, &end))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The block rules of a job's deny on Windows, from its allowed endpoints at
/// their resolved addresses: every protocol to every address but loopback
/// and the allowed ones, and the allowed addresses' other TCP ports and UDP.
pub(crate) fn rules(allowed: &BTreeSet<(IpAddr, u16)>) -> String {
    let mut by_address: BTreeMap<IpAddr, BTreeSet<u16>> = BTreeMap::new();
    for (address, port) in allowed {
        by_address.entry(*address).or_default().insert(*port);
    }
    let mut ipv4 = BTreeSet::from([(
        u128::from(u32::from(Ipv4Addr::new(127, 0, 0, 0))),
        u128::from(u32::from(Ipv4Addr::new(127, 255, 255, 255))),
    )]);
    let mut ipv6 = BTreeSet::from([(
        u128::from(Ipv6Addr::LOCALHOST),
        u128::from(Ipv6Addr::LOCALHOST),
    )]);
    for address in by_address.keys() {
        match address {
            IpAddr::V4(v4) => ipv4.insert((u128::from(u32::from(*v4)), u128::from(u32::from(*v4)))),
            IpAddr::V6(v6) => ipv6.insert((u128::from(*v6), u128::from(*v6))),
        };
    }
    let mut addresses: Vec<String> = complement(&ipv4, u128::from(u32::MAX))
        .into_iter()
        .map(|(start, end)| {
            let address = |value: u128| Ipv4Addr::from(u32::try_from(value).unwrap_or(u32::MAX));
            range_text(&address(start), &address(end))
        })
        .collect();
    addresses.extend(
        complement(&ipv6, u128::MAX)
            .into_iter()
            .map(|(start, end)| range_text(&Ipv6Addr::from(start), &Ipv6Addr::from(end))),
    );
    let mut text = rule("any", &addresses.join(","), "*");
    for (address, ports) in &by_address {
        text.push_str(&rule("tcp", &address.to_string(), &port_complement(ports)));
        text.push_str(&rule("udp", &address.to_string(), "*"));
    }
    text
}

/// An outbound packet the firewall dropped, from its log.
pub(crate) struct Dropped {
    pub(crate) protocol: String,
    pub(crate) address: IpAddr,
    pub(crate) port: Option<u16>,
}

/// Every outbound (`SEND`) drop in the firewall's log, read by the fields
/// its `#Fields:` header names.
pub(crate) fn dropped(log: &str) -> Result<Vec<Dropped>> {
    let mut fields: Option<Vec<&str>> = None;
    let mut dropped = Vec::new();
    for line in log.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if let Some(header) = line.strip_prefix("#Fields:") {
            fields = Some(header.split_whitespace().collect());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let names = fields
            .as_ref()
            .context("the firewall log has no `#Fields:` header before its entries")?;
        let values: Vec<&str> = line.split_whitespace().collect();
        let field = |name: &str| {
            names
                .iter()
                .position(|candidate| *candidate == name)
                .and_then(|index| values.get(index).copied())
                .with_context(|| format!("no `{name}` in `{line}`"))
        };
        if field("action")? != "DROP" || field("path")? != "SEND" {
            continue;
        }
        let port = match field("dst-port")? {
            "-" => None,
            port => Some(
                port.parse()
                    .with_context(|| format!("no port in dst-port of `{line}`"))?,
            ),
        };
        dropped.push(Dropped {
            protocol: field("protocol")?.to_ascii_lowercase(),
            address: field("dst-ip")?
                .parse()
                .with_context(|| format!("no address in dst-ip of `{line}`"))?,
            port,
        });
    }
    if fields.is_none() && !log.trim().is_empty() {
        bail!("the firewall log has no `#Fields:` header");
    }
    Ok(dropped)
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn ports_around_two_allowed_ones_leave_them_out() {
        assert_eq!(
            port_complement(&BTreeSet::from([443, 8443])),
            "1-442,444-8442,8444-65535"
        );
    }

    #[test]
    fn an_allowed_last_address_ends_the_complement_before_it() {
        let excluded = BTreeSet::from([(10, 10), (255, 255)]);

        let ranges = complement(&excluded, 255);

        let mut text = String::new();
        for (start, end) in ranges {
            let _ = write!(text, "{start}-{end} ");
        }
        assert_eq!(text, "0-9 11-254 ");
    }
}
