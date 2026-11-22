//! The Egress canary (ADR 0043; GLOSSARY.md): once the deny is in place, the
//! product's side of the job tries the forbidden connection
//! `ci/egress/endpoints.toml` names, and the attempt passes only when it did
//! not connect and the deny's own record shows that the deny refused it: the
//! nftables `canary` counter on Linux and the firewall's log on Windows. At
//! the job's end, every other connection the deny refused fails the job,
//! named from its log.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::fmt;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::endpoints::Endpoints;
use super::rules::LOG_PREFIX;
use super::windows;
use crate::check::Finding;

/// How long the canary waits for its connection: a refusal is immediate.
const TIMEOUT: Duration = Duration::from_secs(10);

/// How the canary's connection attempt ended, as its outcome file holds it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Connected,
    Refused,
    /// Refused by a local policy before any packet left: a Windows Firewall
    /// block rule (`WSAEACCES`).
    Denied,
    TimedOut,
    Unreachable,
    Failed,
}

impl Outcome {
    const ALL: [Self; 6] = [
        Self::Connected,
        Self::Refused,
        Self::Denied,
        Self::TimedOut,
        Self::Unreachable,
        Self::Failed,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Refused => "refused",
            Self::Denied => "denied",
            Self::TimedOut => "timed-out",
            Self::Unreachable => "unreachable",
            Self::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|outcome| outcome.name() == text.trim())
            .with_context(|| format!("`{}` is not a canary outcome", text.trim()))
    }
}

/// Tries the canary's connection from where the product runs and writes how
/// it ended to `outcome`. It never fails on a refusal: `verdict` judges.
#[expect(
    clippy::disallowed_types,
    reason = "the Egress canary's one forbidden connection attempt is a socket by definition"
)]
pub(crate) fn attempt(root: &Path, outcome: &Path) -> Result<()> {
    let canary = Endpoints::read(root)?.canary;
    let ended = match std::net::TcpStream::connect_timeout(&canary, TIMEOUT) {
        Ok(_) => Outcome::Connected,
        Err(error) => match error.kind() {
            ErrorKind::ConnectionRefused | ErrorKind::ConnectionReset => Outcome::Refused,
            ErrorKind::PermissionDenied => Outcome::Denied,
            ErrorKind::TimedOut | ErrorKind::WouldBlock => Outcome::TimedOut,
            ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable => Outcome::Unreachable,
            _ => Outcome::Failed,
        },
    };
    std::fs::write(outcome, ended.name()).with_context(|| format!("writing {}", outcome.display()))
}

/// The deny's counters, from `nft --json list counters table inet goodfetch_egress`.
struct Counters {
    canary: u64,
    denied: u64,
}

#[derive(Deserialize)]
struct Listing {
    nftables: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct Counter {
    name: String,
    packets: u64,
}

impl Counters {
    fn read(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let listing: Listing =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let counters: Vec<Counter> = listing
            .nftables
            .into_iter()
            .filter_map(|item| item.get("counter").cloned())
            .map(serde_json::from_value)
            .collect::<Result<_, _>>()
            .with_context(|| format!("parsing {}", path.display()))?;
        let packets = |name: &str| {
            counters
                .iter()
                .find(|counter| counter.name == name)
                .map(|counter| counter.packets)
                .with_context(|| format!("{} has no `{name}` counter", path.display()))
        };
        Ok(Self {
            canary: packets("canary")?,
            denied: packets("denied")?,
        })
    }
}

/// What shows that the deny refused the canary.
pub(crate) enum Record<'a> {
    /// The nftables counters before and after the attempt (Linux).
    Counters { before: &'a Path, after: &'a Path },
    /// The firewall's log of dropped packets (Windows).
    FirewallLog(&'a Path),
}

/// What lists the connections the deny refused, at the job's end.
pub(crate) enum Refusals<'a> {
    /// The nftables counters and the kernel log (Linux).
    Nftables { counters: &'a Path, log: &'a Path },
    /// The firewall's log of dropped packets (Windows).
    FirewallLog(&'a Path),
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// The canary passes when it did not connect and the deny recorded it.
pub(crate) fn verdict(root: &Path, outcome: &Path, record: &Record<'_>) -> Result<Vec<Finding>> {
    let canary = Endpoints::read(root)?.canary;
    let ended = Outcome::parse(&read(outcome)?)?;
    let counted = match record {
        Record::Counters { before, after } => {
            Counters::read(after)?.canary > Counters::read(before)?.canary
        }
        Record::FirewallLog(log) => windows::dropped(&read(log)?)?
            .iter()
            .any(|dropped| Destination::from(dropped).is_canary(canary)),
    };
    Ok(match (ended, counted) {
        (Outcome::Connected, _) => vec![Finding::new(
            "canary-reached",
            canary.to_string(),
            "the Egress canary connected: the deny is not in place",
        )],
        (_, false) => vec![Finding::new(
            "canary-unseen",
            canary.to_string(),
            format!(
                "the Egress canary's connection ended `{}` without reaching the deny, so it proves nothing",
                ended.name()
            ),
        )],
        (_, true) => Vec::new(),
    })
}

/// A refused connection's protocol and destination, from its log line.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Destination {
    protocol: String,
    address: IpAddr,
    port: Option<u16>,
}

impl Destination {
    fn is_canary(&self, canary: SocketAddr) -> bool {
        self.protocol == "tcp"
            && self.port.map(|port| SocketAddr::new(self.address, port)) == Some(canary)
    }
}

impl From<&windows::Dropped> for Destination {
    fn from(dropped: &windows::Dropped) -> Self {
        Self {
            protocol: dropped.protocol.clone(),
            address: dropped.address,
            port: dropped.port,
        }
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.port {
            Some(port) => write!(
                formatter,
                "{} {}",
                self.protocol,
                SocketAddr::new(self.address, port)
            ),
            None => write!(formatter, "{} {}", self.protocol, self.address),
        }
    }
}

/// Reads `… goodfetch-egress-denied: … DST=<address> … PROTO=<name> … DPT=<port> …`.
fn destination(line: &str) -> Result<Option<Destination>> {
    let Some((_, fields)) = line.split_once(LOG_PREFIX) else {
        return Ok(None);
    };
    let field = |key: &str| {
        fields
            .split_whitespace()
            .find_map(|token| token.strip_prefix(key)?.strip_prefix('='))
    };
    let address = field("DST")
        .with_context(|| format!("no DST in `{line}`"))?
        .parse()
        .with_context(|| format!("no address in DST of `{line}`"))?;
    let protocol = field("PROTO")
        .with_context(|| format!("no PROTO in `{line}`"))?
        .to_ascii_lowercase();
    let port = field("DPT")
        .map(|port| {
            port.parse()
                .with_context(|| format!("no port in DPT of `{line}`"))
        })
        .transpose()?;
    Ok(Some(Destination {
        protocol,
        address,
        port,
    }))
}

/// One finding per refused destination but the canary's, each once.
fn denied(destinations: impl IntoIterator<Item = Destination>, canary: SocketAddr) -> Vec<Finding> {
    let mut seen = BTreeSet::new();
    let mut findings = Vec::new();
    for destination in destinations {
        if destination.is_canary(canary) || seen.contains(&destination) {
            continue;
        }
        findings.push(Finding::new(
            "egress-denied",
            destination.to_string(),
            "the deny refused a connection outside the job's allowed endpoints (ADR 0043)",
        ));
        seen.insert(destination);
    }
    findings
}

/// Every connection the deny refused, but the canary's, fails the job: one
/// finding per destination in the deny's log, or, on Linux, one for the
/// counter when the log lost them.
pub(crate) fn report(root: &Path, refusals: &Refusals<'_>) -> Result<Vec<Finding>> {
    let canary = Endpoints::read(root)?.canary;
    match refusals {
        Refusals::FirewallLog(log) => {
            let dropped = windows::dropped(&read(log)?)?;
            Ok(denied(dropped.iter().map(Destination::from), canary))
        }
        Refusals::Nftables { counters, log } => {
            let counters = Counters::read(counters)?;
            if counters.denied < counters.canary {
                bail!(
                    "the deny's `denied` counter is below its `canary` counter, which its rules forbid"
                );
            }
            let mut destinations = Vec::new();
            for line in read(log)?.lines() {
                destinations.extend(destination(line)?);
            }
            let mut findings = denied(destinations, canary);
            if findings.is_empty() && counters.denied > counters.canary {
                findings.push(Finding::new(
                    "egress-denied-unlogged",
                    "counter denied",
                    format!(
                        "the deny refused {} connection packets besides the canary's, missing from its log",
                        counters.denied - counters.canary
                    ),
                ));
            }
            Ok(findings)
        }
    }
}
