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
//!   every OS (ADR 0044), its queries sent from that address
//!   ([`Dns::from_local_addresses`]).
//! - **A Session behind a proxy** hands the name to the proxy, which
//!   resolves it, so neither the mapping nor a resolver sees it (ADR 0059).
//!
//! The engine's own resolver keeps its answers in [`cache`] as the records
//! say, read on the [`Clock`], and asks for each IP family on its own, so a
//! Happy Eyeballs race can start on the first family's answer (ADR 0118);
//! `hickory-resolver`'s own cache is off. The system resolver keeps what
//! the OS keeps.
//!
//! This module is the translation point of both resolvers' failures (ADR
//! 0127). `hickory-resolver` runs its name-server connections as Tokio tasks
//! of its own, outside any owner's tracker; they end with their
//! connections. The system resolver's `getaddrinfo` runs on Tokio's blocking
//! pool, since it cannot be cancelled once called.

mod cache;

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

pub use goodfetch_protocol::dns::IpFamily;
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
use hickory_resolver::proto::rr::{RData, RecordType};
use tokio::sync::Mutex;

use self::cache::{AnswerCache, Kept, Outcome};
use crate::clock::Clock;
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

/// The family of `address`.
pub fn family_of(address: IpAddr) -> IpFamily {
    match address {
        IpAddr::V4(_) => IpFamily::Ipv4,
        IpAddr::V6(_) => IpFamily::Ipv6,
    }
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
/// resolver with its cache, cheap to clone; every clone shares the cache.
#[derive(Clone)]
pub struct Dns {
    mapping: Arc<HostMapping>,
    configuration: Arc<ResolverConfiguration>,
    engine: TokioResolver,
    clock: Clock,
    cache: Arc<Mutex<AnswerCache>>,
}

impl Dns {
    /// Resolution with `mapping` and the engine's own resolver reading
    /// `configuration`, whose answers are kept as their records say on
    /// `clock`. It opens no socket until a lookup needs one.
    pub fn new(mapping: HostMapping, configuration: ResolverConfiguration, clock: Clock) -> Self {
        Self {
            mapping: Arc::new(mapping),
            engine: engine_resolver(configuration.clone()),
            configuration: Arc::new(configuration),
            clock,
            cache: Arc::new(Mutex::new(AnswerCache::default())),
        }
    }

    /// The same resolution for a Session that holds these local addresses
    /// (ADR 0044): its engine resolver sends each query from the Session's
    /// address of the name server's family, and asks no name server of a
    /// family it holds no address of, so no query leaves from another
    /// address. It shares the mapping and the cache.
    #[must_use]
    pub fn from_local_addresses(&self, ipv4: Option<Ipv4Addr>, ipv6: Option<Ipv6Addr>) -> Self {
        let mut configuration = (*self.configuration).clone();
        let (domain, search, name_servers) = configuration.config.clone().into_parts();
        let name_servers = name_servers
            .into_iter()
            .filter_map(|mut name_server| {
                let local = match name_server.ip {
                    IpAddr::V4(_) => ipv4.map(IpAddr::V4),
                    IpAddr::V6(_) => ipv6.map(IpAddr::V6),
                }?;
                for connection in &mut name_server.connections {
                    connection.bind_addr = Some(SocketAddr::new(local, 0));
                }
                Some(name_server)
            })
            .collect();
        configuration.config = ResolverConfig::from_parts(domain, search, name_servers);
        Self {
            mapping: Arc::clone(&self.mapping),
            engine: engine_resolver(configuration.clone()),
            configuration: Arc::new(configuration),
            clock: self.clock,
            cache: Arc::clone(&self.cache),
        }
    }

    /// The addresses of `name` for a Session whose connections take `route`,
    /// IPv4 first.
    ///
    /// # Errors
    ///
    /// [`LookupError::NameNotResolved`] when the resolver has no address for
    /// it, and [`LookupError::LookupFailed`] when the engine's own
    /// resolver's name servers gave no usable answer for either family.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only this lookup; a system lookup already
    /// running finishes on its blocking thread and is dropped.
    pub async fn lookup(&self, name: &str, route: Route) -> Result<Resolution, LookupError> {
        if route == Route::Proxy {
            return Ok(Resolution::ByProxy);
        }
        let (ipv4, ipv6) = tokio::join!(
            self.addresses(name, IpFamily::Ipv4, route),
            self.addresses(name, IpFamily::Ipv6, route),
        );
        let answered_by = if self.mapping.addresses(name).is_some() {
            AnsweredBy::Mapping
        } else if asks_the_engine(route) {
            AnsweredBy::Engine
        } else {
            AnsweredBy::System
        };
        match (ipv4, ipv6) {
            (Err(ipv4), Err(ipv6)) => Err(ipv4.or(ipv6)),
            (ipv4, ipv6) => Ok(Resolution::Addresses {
                addresses: ipv4
                    .unwrap_or_default()
                    .into_iter()
                    .chain(ipv6.unwrap_or_default())
                    .collect(),
                answered_by,
            }),
        }
    }

    /// The addresses of `name` in `family`, never empty, for a connection
    /// that takes `route`. The engine's own resolver answers from its cache
    /// while the records allow. Under [`Route::Proxy`], `name` is the
    /// proxy's own, which the connection reaches directly.
    ///
    /// # Errors
    ///
    /// [`LookupError::NameNotResolved`] when the name has no address in
    /// `family`, and [`LookupError::LookupFailed`] when the engine's own
    /// resolver's name servers gave no usable answer.
    ///
    /// # Cancel safety
    ///
    /// As [`Dns::lookup`]'s.
    pub async fn addresses(
        &self,
        name: &str,
        family: IpFamily,
        route: Route,
    ) -> Result<Vec<IpAddr>, LookupError> {
        let resolver = if asks_the_engine(route) {
            NameResolver::Engine
        } else {
            NameResolver::System
        };
        let found = if let Ok(literal) = name.parse::<IpAddr>() {
            of_family(std::iter::once(literal), family)
        } else if let Some(mapped) = self.mapping.addresses(name) {
            of_family(mapped.iter().copied(), family)
        } else if resolver == NameResolver::Engine {
            return self.engine_addresses(name, family).await;
        } else {
            of_family(system_lookup(name).await?.into_iter(), family)
        };
        if found.is_empty() {
            Err(not_resolved(name, resolver))
        } else {
            Ok(found)
        }
    }

    /// Forgets what the engine's own resolver keeps for `name`, so the next
    /// lookup asks its name servers again: every address it gave failed to
    /// connect (RFC 8305 section 9).
    pub async fn forget(&self, name: &str) {
        self.cache.lock().await.forget(name);
    }

    async fn engine_addresses(
        &self,
        name: &str,
        family: IpFamily,
    ) -> Result<Vec<IpAddr>, LookupError> {
        let kept = self.cache.lock().await.kept(name, family, self.clock.now());
        match kept {
            Some(Kept::Addresses(addresses)) => return Ok(addresses),
            Some(Kept::Absent) => return Err(not_resolved(name, NameResolver::Engine)),
            None => {}
        }
        let record_type = match family {
            IpFamily::Ipv4 => RecordType::A,
            IpFamily::Ipv6 => RecordType::AAAA,
        };
        let answered = self.engine.lookup(name, record_type).await;
        self.engine.clear_lookup_cache(name, record_type);
        let outcome = match answered {
            Ok(lookup) => {
                let addresses =
                    unique(
                        lookup
                            .answers()
                            .iter()
                            .filter_map(|record| match &record.data {
                                RData::A(address) => Some(IpAddr::V4(address.0)),
                                RData::AAAA(address) => Some(IpAddr::V6(address.0)),
                                _ => None,
                            }),
                    );
                let ttl = lookup.answers().iter().map(|record| record.ttl).min();
                match ttl {
                    Some(ttl) if !addresses.is_empty() => Outcome::Addresses {
                        addresses,
                        ttl: Duration::from_secs(u64::from(ttl)),
                    },
                    _ => Outcome::Absent { negative_ttl: None },
                }
            }
            Err(NetError::Dns(DnsError::NoRecordsFound(NoRecords {
                response_code: ResponseCode::NXDomain | ResponseCode::NoError,
                negative_ttl,
                ..
            }))) => Outcome::Absent {
                negative_ttl: negative_ttl.map(|ttl| Duration::from_secs(u64::from(ttl))),
            },
            Err(_) => Outcome::Failure,
        };
        self.cache
            .lock()
            .await
            .keep(name, family, outcome.clone(), self.clock.now());
        match outcome {
            Outcome::Addresses { addresses, .. } => Ok(addresses),
            Outcome::Absent { .. } => Err(not_resolved(name, NameResolver::Engine)),
            Outcome::Failure => Err(LookupError::LookupFailed {
                name: name.to_owned(),
                resolver: NameResolver::Engine,
            }),
        }
    }
}

impl LookupError {
    /// The error two families' failed lookups make: a failure of the name
    /// servers wins over a name with no address, which a retry would not
    /// change.
    pub(crate) fn or(self, other: Self) -> Self {
        match (&self, &other) {
            (Self::NameNotResolved { .. }, Self::LookupFailed { .. }) => other,
            _ => self,
        }
    }
}

/// The engine's own resolver reading `configuration`, its own cache off.
fn engine_resolver(configuration: ResolverConfiguration) -> TokioResolver {
    let mut options = configuration.options;
    // The cache `Dns` holds keeps answers as ADR 0118 says; hickory's would
    // also keep server failures.
    options.cache_size = 0;
    TokioResolver::builder_with_config(configuration.config, TokioRuntimeProvider::default())
        .with_options(options)
        .build()
        .expect("invariant: a resolver built from a parsed configuration and no TLS builds")
}

/// Whether a connection that takes `route` resolves through the engine's own
/// resolver: on Linux, and from a local address on every OS.
fn asks_the_engine(route: Route) -> bool {
    route == Route::LocalAddress || cfg!(target_os = "linux")
}

/// The addresses of `addresses` in `family`.
fn of_family(addresses: impl Iterator<Item = IpAddr>, family: IpFamily) -> Vec<IpAddr> {
    addresses
        .filter(|address| family_of(*address) == family)
        .collect()
}

/// A lookup through the operating system's resolver, of both families.
/// Its failures are not told apart: the standard library reports
/// `getaddrinfo`'s codes only as text, which ADR 0127 keeps unread.
async fn system_lookup(name: &str) -> Result<Vec<IpAddr>, LookupError> {
    match tokio::net::lookup_host((name, 0)).await {
        Ok(found) => Ok(unique(found.map(|address| address.ip()))),
        Err(_) => Err(not_resolved(name, NameResolver::System)),
    }
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
