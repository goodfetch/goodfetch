//! The nftables ruleset a job's VM loads to deny outbound traffic (ADR 0043).
//! It accepts loopback, replies, the VM's own link upkeep (DHCP, IPv6
//! neighbor discovery, and the multicast listener reports the kernel sends
//! when an interface comes up, which never leave the link) and the job's
//! allowed endpoints at the addresses resolved before the deny. Anything else
//! is refused; when the job's user (the rootless container's, so the
//! product's) sent it, it is counted in `denied` and logged, and the Egress
//! canary's address is also counted in `canary`, so the end-of-job check can
//! tell the canary from any other refused connection.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use anyhow::{Context as _, Result, bail};

use super::endpoints::{Endpoint, Endpoints, JobKey};

/// The kernel log prefix of a refused connection, which `report` reads back.
pub(crate) const LOG_PREFIX: &str = "goodfetch-egress-denied: ";

/// A user ID inside the job's VM.
#[derive(Clone, Copy)]
pub(crate) struct Uid(pub(crate) u32);

/// One `<host> <port> <address>` line of the resolved list: an allowed
/// endpoint at an address the VM resolved before the deny.
fn parse_resolved(line: &str, allowed: &[Endpoint], canary: SocketAddr) -> Result<(IpAddr, u16)> {
    let mut fields = line.split_whitespace();
    let (Some(host), Some(port), Some(address), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        bail!("`{line}` is not `<host> <port> <address>`");
    };
    let endpoint = Endpoint::try_from(format!("{host}:{port}"))?;
    if !allowed.contains(&endpoint) {
        bail!("{endpoint} is not an endpoint the job may reach");
    }
    let address: IpAddr = address
        .parse()
        .with_context(|| format!("`{address}` is not an IP address"))?;
    if SocketAddr::new(address, endpoint.port) == canary {
        bail!("{endpoint} resolves to the Egress canary's address {canary}");
    }
    Ok((address, endpoint.port))
}

fn set(name: &str, kind: &str, elements: &[String]) -> String {
    let mut text = format!("\tset {name} {{\n\t\ttype {kind} . inet_service\n");
    if !elements.is_empty() {
        let _ = writeln!(text, "\t\telements = {{ {} }}", elements.join(", "));
    }
    text.push_str("\t}\n");
    text
}

/// The job's allowed endpoints at the addresses resolved before the deny,
/// refusing any line that names another endpoint or the canary.
pub(crate) fn resolved_addresses(
    endpoints: &Endpoints,
    job: &JobKey,
    resolved: &Path,
) -> Result<BTreeSet<(IpAddr, u16)>> {
    let allowed = endpoints.allowed(job)?;
    let text = std::fs::read_to_string(resolved)
        .with_context(|| format!("reading {}", resolved.display()))?;
    let mut addresses = BTreeSet::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        addresses.insert(
            parse_resolved(line, &allowed, endpoints.canary)
                .with_context(|| format!("reading {}", resolved.display()))?,
        );
    }
    Ok(addresses)
}

pub(crate) fn rules(root: &Path, job: &JobKey, resolved: &Path, uid: Uid) -> Result<String> {
    let endpoints = Endpoints::read(root)?;
    let addresses = resolved_addresses(&endpoints, job, resolved)?;
    let element = |(address, port): &(IpAddr, u16)| format!("{address} . {port}");
    let ipv4: Vec<String> = addresses
        .iter()
        .filter(|(address, _)| address.is_ipv4())
        .map(element)
        .collect();
    let ipv6: Vec<String> = addresses
        .iter()
        .filter(|(address, _)| address.is_ipv6())
        .map(element)
        .collect();
    let canary_family = if endpoints.canary.is_ipv4() {
        "ip"
    } else {
        "ip6"
    };

    let mut ruleset = format!(
        "# ADR 0043's outbound-traffic deny for {job} (xtask egress rules).\n\
         table inet goodfetch_egress\n\
         delete table inet goodfetch_egress\n\
         table inet goodfetch_egress {{\n\
         \tcounter canary {{\n\t}}\n\
         \tcounter denied {{\n\t}}\n\
         \tcounter background {{\n\t}}\n"
    );
    ruleset.push_str(&set("allowed_ipv4", "ipv4_addr", &ipv4));
    ruleset.push_str(&set("allowed_ipv6", "ipv6_addr", &ipv6));
    let _ = write!(
        ruleset,
        "\tchain output {{\n\
         \t\ttype filter hook output priority filter; policy accept;\n\
         \t\toifname \"lo\" accept\n\
         \t\tct state established,related accept\n\
         \t\tudp sport 68 udp dport 67 accept\n\
         \t\ticmpv6 type {{ nd-router-solicit, nd-neighbor-solicit, nd-neighbor-advert, mld-listener-report, mld-listener-done, mld2-listener-report }} accept\n\
         \t\tip daddr . tcp dport @allowed_ipv4 accept\n\
         \t\tip6 daddr . tcp dport @allowed_ipv6 accept\n\
         \t\tmeta skuid != {uid} counter name \"background\" reject\n\
         \t\t{canary_family} daddr {canary_address} tcp dport {canary_port} counter name \"canary\"\n\
         \t\tcounter name \"denied\" log prefix \"{LOG_PREFIX}\"\n\
         \t\tmeta l4proto tcp reject with tcp reset\n\
         \t\treject with icmpx admin-prohibited\n\
         \t}}\n\
         }}\n",
        uid = uid.0,
        canary_address = endpoints.canary.ip(),
        canary_port = endpoints.canary.port(),
    );
    Ok(ruleset)
}
