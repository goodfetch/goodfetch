//! Name resolution for the connections the engine opens itself: the
//! `resolve` mapping first, before any lookup, then the resolver the
//! Session's route calls for (ADRs 0024, 0044 and 0059).
//!
//! - **Linux** asks the engine's own resolver, `hickory-resolver`: each
//!   Product's static musl executable has no libc resolver to call. It reads
//!   the system's resolver configuration (`/etc/resolv.conf`) and hosts file
//!   and asks the name servers itself, so a name only an NSS module or
//!   multicast DNS resolves fails with `name_not_resolved`.
//! - **macOS and Windows** ask the system resolver, except for a Session with
//!   a local address, whose lookups go through the engine's own resolver on
//!   every OS (ADR 0044; binding its queries to that address is area 5's).
//! - **A Session behind a proxy** hands the name to the proxy, which
//!   resolves it, so neither the mapping nor a resolver sees it (ADR 0059).
//!
//! This module is the translation point of both resolvers' failures (ADR
//! 0127). `hickory-resolver` runs its name-server connections as Tokio tasks
//! of its own, outside any owner's tracker; they end with their
//! connections. The system resolver's `getaddrinfo` runs on Tokio's blocking
//! pool, since it cannot be cancelled once called.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use goodfetch_protocol::dns::{
    NAME_LOOKUP_FAILED, NAME_NOT_RESOLVED, NameLookupFailedDetails, NameNotResolvedDetails,
    NameResolver,
};
use hickory_resolver::TokioResolver;
use hickory_resolver::config::{
    ConnectionConfig, LookupIpStrategy, NameServerConfig, ResolveHosts, ResolverConfig,
    ResolverOpts,
};
use hickory_resolver::net::runtime::TokioRuntimeProvider;
use hickory_resolver::net::{DnsError, NetError, NoRecords};
use hickory_resolver::proto::op::ResponseCode;

use crate::composition::CallError;

/// The `resolve` setting's value: host names mapped to the addresses they
/// resolve to before any lookup (ADR 0059). A name matches whatever its case
/// and trailing dot.
#[derive(Clone, Debug, Default)]
pub struct HostMapping(BTreeMap<String, Vec<IpAddr>>);

impl HostMapping {
    /// The mapping of each name to its addresses.
    pub fn new<'name>(entries: impl IntoIterator<Item = (&'name str, Vec<IpAddr>)>) -> Self {
        Self(
            entries
                .into_iter()
                .map(|(name, addresses)| (canonical(name), addresses))
                .collect(),
        )
    }

    fn addresses(&self, name: &str) -> Option<&[IpAddr]> {
        self.0.get(&canonical(name)).map(Vec::as_slice)
    }
}

/// What the engine's own resolver reads: its name servers, search domains,
/// options and whether it reads the hosts file.
#[derive(Clone, Debug)]
pub struct ResolverConfiguration {
    config: ResolverConfig,
    options: ResolverOpts,
}

impl ResolverConfiguration {
    /// The system's resolver configuration: `/etc/resolv.conf` on Linux and
    /// macOS' own store on macOS, the adapters' settings on Windows, and the
    /// hosts file. It reads files and the OS's configuration store, so it is
    /// read once, before the engine serves.
    ///
    /// # Errors
    ///
    /// [`ConfigurationUnreadable`] when the system's configuration cannot be
    /// read, such as a container with no `/etc/resolv.conf`.
    pub fn system() -> Result<Self, ConfigurationUnreadable> {
        let (config, mut options) = hickory_resolver::system_conf::read_system_conf()
            .map_err(|error| ConfigurationUnreadable(error.to_string()))?;
        options.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        Ok(Self { config, options })
    }

    /// Only these name servers, over UDP and then TCP, with no search domain
    /// and no hosts file.
    pub fn name_servers(addresses: impl IntoIterator<Item = SocketAddr>) -> Self {
        let name_servers = addresses
            .into_iter()
            .map(|address| {
                let connections = [ConnectionConfig::udp(), ConnectionConfig::tcp()]
                    .into_iter()
                    .map(|mut connection| {
                        connection.port = address.port();
                        connection
                    })
                    .collect();
                NameServerConfig::new(address.ip(), true, connections)
            })
            .collect();
        let mut options = ResolverOpts::default();
        options.ip_strategy = LookupIpStrategy::Ipv4AndIpv6;
        options.use_hosts_file = ResolveHosts::Never;
        Self {
            config: ResolverConfig::from_name_servers(name_servers),
            options,
        }
    }
}

/// The system's resolver configuration could not be read.
#[derive(Debug, thiserror::Error)]
#[error("the system's resolver configuration cannot be read: {0}")]
pub struct ConfigurationUnreadable(String);

/// How a Session's connections leave the machine, which decides who
/// resolves its host names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// Straight from the machine's own addresses.
    Direct,
    /// From a local address the Session binds (ADR 0044).
    LocalAddress,
    /// Through a proxy, which resolves the names it is given.
    Proxy,
}

/// Who gave a name its addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnsweredBy {
    /// The `resolve` mapping.
    Mapping,
    /// The engine's own resolver.
    Engine,
    /// The operating system's resolver.
    System,
}

/// What a lookup gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// The name's addresses, in the resolver's order.
    Addresses {
        /// Its addresses, never empty.
        addresses: Vec<IpAddr>,
        /// Who gave them.
        answered_by: AnsweredBy,
    },
    /// The name goes to the Session's proxy as it is.
    ByProxy,
}

/// Why a lookup gave no address.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LookupError {
    /// The resolver found no address for the name.
    #[error("{resolver:?} resolver found no address for {name}")]
    NameNotResolved {
        /// The name asked for.
        name: String,
        /// The resolver asked.
        resolver: NameResolver,
    },
    /// The resolver's name servers gave no usable answer.
    #[error("the name servers of the {resolver:?} resolver gave no usable answer for {name}")]
    LookupFailed {
        /// The name asked for.
        name: String,
        /// The resolver asked.
        resolver: NameResolver,
    },
}

impl From<LookupError> for CallError {
    fn from(error: LookupError) -> Self {
        match error {
            LookupError::NameNotResolved { name, resolver } => {
                let multicast_dns_name = is_multicast_dns_name(&name);
                let message = match (resolver, multicast_dns_name) {
                    (NameResolver::Engine, true) => format!(
                        "{name} is a .local name, which multicast DNS answers; the engine's own resolver asks only the name servers of the system's resolver configuration. Map it with the `resolve` setting"
                    ),
                    (NameResolver::Engine, false) => format!(
                        "the engine's own resolver found no address for {name} in the hosts file or from the system's name servers; it asks no NSS module, so a name only one resolves needs the `resolve` setting"
                    ),
                    (NameResolver::System, _) => {
                        format!("the system resolver found no address for {name}")
                    }
                };
                Self::new(
                    &NAME_NOT_RESOLVED,
                    message,
                    &NameNotResolvedDetails {
                        name,
                        resolver,
                        multicast_dns_name,
                    },
                )
            }
            LookupError::LookupFailed { name, resolver } => Self::new(
                &NAME_LOOKUP_FAILED,
                format!("the name servers gave no usable answer for {name}"),
                &NameLookupFailedDetails { name, resolver },
            ),
        }
    }
}

/// The engine's name resolution: the `resolve` mapping and the engine's own
/// resolver, cheap to clone.
#[derive(Clone)]
pub struct Dns {
    mapping: Arc<HostMapping>,
    engine: TokioResolver,
}

impl Dns {
    /// Resolution with `mapping` and the engine's own resolver reading
    /// `configuration`. It opens no socket until a lookup needs one.
    pub fn new(mapping: HostMapping, configuration: ResolverConfiguration) -> Self {
        let engine = TokioResolver::builder_with_config(
            configuration.config,
            TokioRuntimeProvider::default(),
        )
        .with_options(configuration.options)
        .build()
        .expect("invariant: a resolver built from a parsed configuration and no TLS builds");
        Self {
            mapping: Arc::new(mapping),
            engine,
        }
    }

    /// The addresses of `name` for a Session whose connections take `route`.
    ///
    /// # Errors
    ///
    /// [`LookupError::NameNotResolved`] when the resolver has no address for
    /// it, and [`LookupError::LookupFailed`] when the engine's own
    /// resolver's name servers gave no usable answer.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only this lookup; a system lookup already
    /// running finishes on its blocking thread and is dropped.
    pub async fn lookup(&self, name: &str, route: Route) -> Result<Resolution, LookupError> {
        if route == Route::Proxy {
            return Ok(Resolution::ByProxy);
        }
        if let Some(addresses) = self.mapping.addresses(name) {
            return Ok(Resolution::Addresses {
                addresses: addresses.to_vec(),
                answered_by: AnsweredBy::Mapping,
            });
        }
        if route == Route::LocalAddress || cfg!(target_os = "linux") {
            self.engine_lookup(name).await
        } else {
            system_lookup(name).await
        }
    }

    async fn engine_lookup(&self, name: &str) -> Result<Resolution, LookupError> {
        match self.engine.lookup_ip(name).await {
            Ok(lookup) => {
                let addresses = unique(lookup.iter());
                if addresses.is_empty() {
                    Err(not_resolved(name, NameResolver::Engine))
                } else {
                    Ok(Resolution::Addresses {
                        addresses,
                        answered_by: AnsweredBy::Engine,
                    })
                }
            }
            Err(error) if is_absent(&error) => Err(not_resolved(name, NameResolver::Engine)),
            Err(_) => Err(LookupError::LookupFailed {
                name: name.to_owned(),
                resolver: NameResolver::Engine,
            }),
        }
    }
}

/// A lookup through the operating system's resolver. Its failures are not
/// told apart: the standard library reports `getaddrinfo`'s codes only as
/// text, which ADR 0127 keeps unread.
async fn system_lookup(name: &str) -> Result<Resolution, LookupError> {
    match tokio::net::lookup_host((name, 0)).await {
        Ok(found) => {
            let addresses = unique(found.map(|address| address.ip()));
            if addresses.is_empty() {
                Err(not_resolved(name, NameResolver::System))
            } else {
                Ok(Resolution::Addresses {
                    addresses,
                    answered_by: AnsweredBy::System,
                })
            }
        }
        Err(_) => Err(not_resolved(name, NameResolver::System)),
    }
}

/// Whether the name servers said the name has no address: it does not
/// exist, or exists with no address record. Any other answer, a server
/// failure or a refusal included, is no answer.
fn is_absent(error: &NetError) -> bool {
    matches!(
        error,
        NetError::Dns(DnsError::NoRecordsFound(NoRecords {
            response_code: ResponseCode::NXDomain | ResponseCode::NoError,
            ..
        }))
    )
}

fn not_resolved(name: &str, resolver: NameResolver) -> LookupError {
    LookupError::NameNotResolved {
        name: name.to_owned(),
        resolver,
    }
}

/// The addresses in their first order, each once.
fn unique(addresses: impl Iterator<Item = IpAddr>) -> Vec<IpAddr> {
    let mut unique: Vec<IpAddr> = Vec::new();
    for address in addresses {
        if !unique.contains(&address) {
            unique.push(address);
        }
    }
    unique
}

/// A name's lookup key: lowercase, without its trailing dot.
fn canonical(name: &str) -> String {
    name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase()
}

/// Whether `name` sits under `.local`, multicast DNS's domain (RFC 6762).
fn is_multicast_dns_name(name: &str) -> bool {
    canonical(name).rsplit('.').next() == Some("local")
}

#[cfg(test)]
mod tests;
