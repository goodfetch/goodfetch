//! `ci/egress/endpoints.toml`, parsed: the Egress canary's address and each
//! job's declaration under the deny, outside it, or under it with named
//! endpoints (ADR 0043). The pinned artifact mirror is no endpoint: every
//! pinned download, the test V8 build and Goodfetch's own releases
//! (`compat/releases.toml`, ADR 0066) included, comes before the deny, so a
//! job under it reaches only the fixtures, local to it (owner decision of
//! 2026-10-19).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

pub(crate) const ENDPOINTS: &str = "ci/egress/endpoints.toml";

/// A job of a workflow, written `<workflow file>#<job id>`.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct JobKey {
    workflow: String,
    job: String,
}

impl JobKey {
    pub(crate) fn new(workflow: &str, job: &str) -> Self {
        Self {
            workflow: workflow.to_owned(),
            job: job.to_owned(),
        }
    }
}

impl TryFrom<String> for JobKey {
    type Error = anyhow::Error;

    fn try_from(text: String) -> Result<Self> {
        match text.split_once('#') {
            Some((workflow, job))
                if !workflow.is_empty() && !job.is_empty() && !job.contains('#') =>
            {
                Ok(Self::new(workflow, job))
            }
            _ => bail!("`{text}` is not `<workflow file>#<job id>`"),
        }
    }
}

impl fmt::Display for JobKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}#{}", self.workflow, self.job)
    }
}

/// A host name or IP address and a TCP port the deny lets a job reach.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub(crate) struct Endpoint {
    pub(crate) host: String,
    pub(crate) port: u16,
}

impl Endpoint {
    fn parse(host: &str, port: &str) -> Result<Self> {
        let host = host
            .strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(host);
        let is_name = !host.is_empty()
            && host.chars().all(|character| {
                character.is_ascii_alphanumeric() || character == '.' || character == '-'
            });
        if !is_name && host.parse::<IpAddr>().is_err() {
            bail!("`{host}` is neither a host name nor an IP address");
        }
        let port: u16 = port
            .parse()
            .with_context(|| format!("`{port}` is not a port"))?;
        if port == 0 {
            bail!("port 0 is not a port");
        }
        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
        })
    }
}

impl TryFrom<String> for Endpoint {
    type Error = anyhow::Error;

    fn try_from(text: String) -> Result<Self> {
        let (host, port) = text
            .rsplit_once(':')
            .with_context(|| format!("`{text}` is not `<host>:<port>`"))?;
        Self::parse(host, port)
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(formatter, "[{}]:{}", self.host, self.port)
        } else {
            write!(formatter, "{}:{}", self.host, self.port)
        }
    }
}

/// The jobs ADR 0043 lets reach named endpoints, and only those.
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Exception {
    /// The nightly real software-as-a-service Sink accounts (ADR 0026).
    SaasSinks,
    /// The release-candidate benchmark's online Proving ground (ADRs 0025 and 0035).
    Benchmark,
    /// The nightly public testnets for Paid fetch proofs (ADR 0045).
    Testnets,
    /// The online conformance against real vendors (ADR 0033).
    VendorConformance,
    /// The nightly build of the Clean-up rules' daily channel (ADR 0047).
    CleanUpRulesDaily,
    /// The release-candidate Cloud Run deploy (ADR 0049).
    CloudRunDeploy,
    /// ADR 0065's model jobs.
    Model,
    /// ADR 0070's Integration account job.
    IntegrationAccount,
    /// ADR 0070's nightly checks of unpinned releases.
    UnpinnedReleases,
    /// A fixture's licence endpoint.
    FixtureLicence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DenyEntry {
    job: JobKey,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OutsideEntry {
    job: JobKey,
    /// Why the job runs none of the product.
    #[serde(rename = "reason")]
    _reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NamedEntry {
    job: JobKey,
    exception: Exception,
    endpoints: Vec<Endpoint>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    canary: SocketAddr,
    #[serde(default)]
    deny: Vec<DenyEntry>,
    #[serde(default)]
    outside: Vec<OutsideEntry>,
    #[serde(default)]
    named: Vec<NamedEntry>,
}

/// How a job stands to the deny.
pub(crate) enum Rule {
    /// It runs the product, under the deny: the fixtures only.
    Deny,
    /// It runs none of the product, outside the deny.
    Outside,
    /// It runs under the deny and may also reach its named endpoints, as
    /// the exception it is.
    Named {
        exception: Exception,
        endpoints: Vec<Endpoint>,
    },
}

pub(crate) struct Declaration {
    pub(crate) job: JobKey,
    pub(crate) rule: Rule,
}

pub(crate) struct Endpoints {
    /// The forbidden address the Egress canary tries.
    pub(crate) canary: SocketAddr,
    pub(crate) declarations: Vec<Declaration>,
}

impl Endpoints {
    pub(crate) fn read(root: &Path) -> Result<Self> {
        let path = root.join(ENDPOINTS);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let file: File =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if !is_public(file.canary.ip()) {
            bail!(
                "the canary {} in {ENDPOINTS} is not a public address, so refusing it proves nothing",
                file.canary
            );
        }
        let declarations = file
            .deny
            .into_iter()
            .map(|entry| Declaration {
                job: entry.job,
                rule: Rule::Deny,
            })
            .chain(file.outside.into_iter().map(|entry| Declaration {
                job: entry.job,
                rule: Rule::Outside,
            }))
            .chain(file.named.into_iter().map(|entry| Declaration {
                job: entry.job,
                rule: Rule::Named {
                    exception: entry.exception,
                    endpoints: entry.endpoints,
                },
            }))
            .collect();
        Ok(Self {
            canary: file.canary,
            declarations,
        })
    }

    pub(crate) fn declaration(&self, job: &JobKey) -> Option<&Declaration> {
        self.declarations
            .iter()
            .find(|declaration| &declaration.job == job)
    }

    /// Everything a job under the deny may reach beyond its fixtures: its
    /// named endpoints, when ADR 0043 names it, and nothing otherwise. A job
    /// outside the deny or not declared has no such list.
    pub(crate) fn allowed(&self, job: &JobKey) -> Result<Vec<Endpoint>> {
        let named: &[Endpoint] = match self.declaration(job).map(|declaration| &declaration.rule) {
            None => bail!("{job} is not declared in {ENDPOINTS}"),
            Some(Rule::Outside) => bail!("{job} runs outside the deny, so it has no allowed list"),
            Some(Rule::Deny) => &[],
            Some(Rule::Named { endpoints, .. }) => endpoints,
        };
        let mut allowed = named.to_vec();
        allowed.sort();
        allowed.dedup();
        Ok(allowed)
    }
}

/// Whether an address lies on the public internet, where a refused
/// connection shows the deny at work rather than a local refusal.
fn is_public(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            !(address.is_loopback()
                || address.is_private()
                || address.is_link_local()
                || address.is_unspecified()
                || address.is_broadcast()
                || address.is_multicast())
        }
        IpAddr::V6(address) => {
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_unique_local()
                || address.is_unicast_link_local())
        }
    }
}
