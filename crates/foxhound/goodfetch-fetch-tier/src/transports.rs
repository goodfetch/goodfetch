//! The HTTP transports (spec #579; ADRs 0020, 0060, 0118, 0127), each
//! filling the driver's `send(hop)`: reqwest in `standard` and
//! `identified` Sessions, here, and `wreq` in HTTP-only `stealth` Sessions
//! (`impersonated`), which sends its Identity profile's TLS and HTTP/2
//! fingerprint. Each opens its own sockets (ADR 0060's one exception for a
//! transport), names hosts through the engine's [`Dns`] (the
//! `network.resolve` mapping, then the engine's resolver) and verifies
//! certificates under the engine's [`Trust`]. Neither follows a redirect,
//! resends anything, decodes a content coding or sets a timeout of its
//! own: the driver does each. In a Session that may move to the browser,
//! browser-fetch (`in_browser`) takes their place, sending from inside the
//! Session's browser context (ADR 0020).
//!
//! **Connections (ADR 0118):** it keeps a pool of idle connections for
//! [`Reuse::Allowed`] hops and opens a new one, kept for nothing, for
//! each [`Reuse::Never`] hop, a resend's. It tells a reused connection
//! from a new one by a mark its connector sets on the hop's task when the
//! hop opens one. An origin that answered `HTTP_1_1_REQUIRED` is sent
//! HTTP/1.1 alone from then on.
//!
//! **Translation point (ADR 0127):** [`classify`] is where reqwest's,
//! hyper's, rustls's and the OS's failures, and a proxy's answer to the
//! tunnel request, become a [`TransportFailure`]; any failure it does not
//! know is the generic [`TransportFailure::Other`] (ADR 0118). A proxy's
//! refusal of the tunnel is read from hyper-util's tunnel error, whose type
//! is private, by its text: reqwest keeps no status of the proxy's answer,
//! so ADR 0053's ceiling learning cannot tell a 429 from a 502 there.
//!
//! **The resolved-address check (ADR 0017):** a URL the egress policy
//! refuses opens no connection, nor does the address a URL names in place
//! of a host, in any spelling, which no resolver answers (the driver's
//! `judged`). Going direct, every address the resolver answers is judged in
//! the connector, on each connection's own lookup, before the connection
//! tries any of them, so neither a name answering a public and a private
//! address nor one whose answer turns private between connections reaches
//! the private one. Through a proxy, which would resolve the name again
//! itself, the transport resolves and judges it before sending anything to
//! the proxy. [`AddressCheck`] runs the same check ahead of a crawl
//! request. The `network.resolve` mapping's addresses are judged as the
//! resolver's are, as on the browser path: a Run reaching a mapped host on
//! a private address lists it in `egress.private_ranges`.
//!
//! **Local addresses (ADR 0044):** a Session holding local addresses
//! ([`LocalSource`]) sends every connection from its address of the
//! target's family, and every DNS query through the engine's resolver from
//! its address of the name server's family; a name answering only in a
//! family it holds no address of fails `address_family_unavailable`. An
//! interface name also binds the socket to its device on Linux, macOS and
//! illumos (`SO_BINDTODEVICE`, `IP_BOUND_IF`), and only its address on
//! Windows. reqwest binds one local address per client, so a reqwest
//! Session holding both families keeps a client of each, a hop taking the
//! family of its host's first answer, IPv4 first as the engine's resolver
//! orders them; `wreq` binds both on one client.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{Dns, HostMapping, Resolution, ResolverConfiguration, Route};
use goodfetch_engine::egress::{self, Policy, Refused};
use goodfetch_engine::local_addresses::SessionAddresses;
use goodfetch_engine::tls::Trust;
use goodfetch_protocol::dns::IpFamily;
use url::Url;

use crate::driver::{Body, Hop, Reuse, Sending, Sent, Transport, TransportFailure, judged};

mod impersonated;
mod in_browser;

pub use impersonated::Wreq;
pub(crate) use in_browser::BrowserFetch;

/// What a transport reaches and trusts.
#[derive(Clone, Debug)]
pub struct TransportConfig {
    /// What each host's certificate may chain to beside Mozilla's roots.
    pub trust: Trust,
    /// The Session's proxy, whose connections reach it in place of the
    /// site, `None` to connect directly.
    pub proxy: Option<Url>,
    /// The `network.resolve` mapping: host names and their addresses.
    pub resolve: Vec<(String, Vec<IpAddr>)>,
    /// The Session's egress policy.
    pub egress: Policy,
    /// What the resolver's cache keeps time on.
    pub clock: Clock,
    /// What the engine's own resolver reads: the system's configuration,
    /// or the name servers a test gives.
    pub resolver: ResolverConfiguration,
    /// The Session's local addresses (`egress.local_addresses`), `None`
    /// to let the OS choose.
    pub local: Option<LocalSource>,
}

/// What a Session's connections and DNS queries leave from (ADR 0044): its
/// local addresses, and the interface they came from when
/// `egress.local_addresses` names one, whose device the socket binds where
/// the OS allows.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LocalSource {
    /// Its IPv4 address.
    pub ipv4: Option<std::net::Ipv4Addr>,
    /// Its IPv6 address.
    pub ipv6: Option<std::net::Ipv6Addr>,
    /// The interface the addresses came from.
    pub interface: Option<String>,
}

impl LocalSource {
    /// The source of a Session holding `addresses`, of `interface` when
    /// one gave them.
    pub fn new(addresses: SessionAddresses, interface: Option<String>) -> Self {
        Self {
            ipv4: addresses.ipv4,
            ipv6: addresses.ipv6,
            interface,
        }
    }

    /// The families it holds an address of, IPv4 first.
    fn families(&self) -> Vec<IpFamily> {
        let mut families = Vec::new();
        if self.ipv4.is_some() {
            families.push(IpFamily::Ipv4);
        }
        if self.ipv6.is_some() {
            families.push(IpFamily::Ipv6);
        }
        families
    }

    /// Its address of `family`.
    fn address(&self, family: IpFamily) -> Option<IpAddr> {
        match family {
            IpFamily::Ipv4 => self.ipv4.map(IpAddr::V4),
            IpFamily::Ipv6 => self.ipv6.map(IpAddr::V6),
        }
    }
}

/// The family of `address`.
fn family_of(address: IpAddr) -> IpFamily {
    match address {
        IpAddr::V4(_) => IpFamily::Ipv4,
        IpAddr::V6(_) => IpFamily::Ipv6,
    }
}

/// Why a transport could not be built.
#[derive(Debug, thiserror::Error)]
pub enum TransportUnavailable {
    /// Its library refused the configuration, such as a proxy URL it does
    /// not take.
    #[error("the HTTP transport could not be built: {0}")]
    Client(String),
}

/// reqwest, the transport of `standard` and `identified` Sessions.
#[derive(Clone, Debug)]
pub struct Reqwest {
    /// Its clients, one set for each family the Session binds a local
    /// address of, or one set, `None`, letting the OS choose.
    bound: Vec<(Option<IpFamily>, Bound)>,
    http1_origins: Http1Origins,
    resolver: Arc<Resolver>,
}

/// The clients of one local binding.
#[derive(Clone, Debug)]
struct Bound {
    /// Its clients offering HTTP/2 and HTTP/1.1.
    any: Clients,
    /// Its clients offering HTTP/1.1 alone, for the origins that asked.
    http1: Clients,
}

/// A pooled client and one that opens a new connection for each request.
#[derive(Clone, Debug)]
struct Clients {
    pooled: reqwest::Client,
    new: reqwest::Client,
}

impl Reqwest {
    /// A transport under `config`, holding its own connection pool, so
    /// hops of one Run reuse connections.
    ///
    /// # Errors
    ///
    /// [`TransportUnavailable`] when reqwest refuses the proxy.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "both transports take their configuration alike, and `Wreq::new` consumes it"
    )]
    pub fn new(config: TransportConfig) -> Result<Self, TransportUnavailable> {
        let resolver = Arc::new(Resolver::new(&config, None));
        let families: Vec<Option<IpFamily>> = match &config.local {
            Some(local) => local.families().into_iter().map(Some).collect(),
            None => vec![None],
        };
        let bound = families
            .into_iter()
            .map(|family| Ok((family, Self::bound(&config, family)?)))
            .collect::<Result<_, TransportUnavailable>>()?;
        Ok(Self {
            bound,
            http1_origins: Http1Origins::default(),
            resolver,
        })
    }

    /// The clients that bind the Session's local address of `family`, or
    /// none.
    fn bound(
        config: &TransportConfig,
        family: Option<IpFamily>,
    ) -> Result<Bound, TransportUnavailable> {
        let resolver = Arc::new(Resolver::new(config, family));
        let local = family.and_then(|family| {
            config
                .local
                .as_ref()
                .and_then(|local| local.address(family))
        });
        let interface = config
            .local
            .as_ref()
            .and_then(|local| local.interface.clone());
        let clients = |alpn: &[&[u8]]| -> Result<Clients, TransportUnavailable> {
            let mut tls = config.trust.client_config();
            tls.alpn_protocols = alpn.iter().map(|protocol| protocol.to_vec()).collect();
            let client = |pooled: bool| {
                let builder = reqwest::Client::builder()
                    .tls_backend_preconfigured(tls.clone())
                    .dns_resolver(Arc::clone(&resolver))
                    // A folded header line is read as a space, as Chrome
                    // reads it.
                    .http1_allow_obsolete_multiline_headers_in_responses(true)
                    .redirect(reqwest::redirect::Policy::none())
                    .retry(reqwest::retry::never())
                    .connector_layer(MarkConnects)
                    .local_address(local);
                let builder = bind_device(builder, interface.as_deref());
                let builder = if pooled {
                    builder
                } else {
                    builder.pool_max_idle_per_host(0)
                };
                let builder = if alpn.contains(&&b"h2"[..]) {
                    builder
                } else {
                    builder.http1_only()
                };
                let builder = match &config.proxy {
                    Some(proxy) => builder.proxy(
                        reqwest::Proxy::all(proxy.as_str())
                            .map_err(|error| TransportUnavailable::Client(error.to_string()))?,
                    ),
                    None => builder.no_proxy(),
                };
                build(builder).map_err(|error| TransportUnavailable::Client(error.to_string()))
            };
            Ok(Clients {
                pooled: client(true)?,
                new: client(false)?,
            })
        };
        Ok(Bound {
            any: clients(&[b"h2", b"http/1.1"])?,
            http1: clients(&[b"http/1.1"])?,
        })
    }

    /// The address check of its connections, run ahead of a request.
    pub fn address_check(&self) -> AddressCheck {
        AddressCheck(Arc::clone(&self.resolver))
    }

    /// The clients of the local binding `hop` goes out from: the only
    /// one, or, for a Session holding both families, that of its host's
    /// first answer.
    async fn binding(&self, hop: &Hop) -> Result<&Bound, TransportFailure> {
        let first = &self.bound[0].1;
        if self.bound.len() == 1 {
            return Ok(first);
        }
        let family = self.resolver.first_family(&hop.url).await?;
        Ok(self
            .bound
            .iter()
            .find(|(bound, _)| *bound == family)
            .map_or(first, |(_, bound)| bound))
    }

    /// The client `hop` goes out on from `bound`, as `reuse` allows.
    fn client<'b>(&self, bound: &'b Bound, hop: &Hop, reuse: Reuse) -> &'b reqwest::Client {
        let clients = if self.http1_origins.contains(&hop.url) {
            &bound.http1
        } else {
            &bound.any
        };
        match reuse {
            Reuse::Allowed => &clients.pooled,
            Reuse::Never => &clients.new,
        }
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the HTTP fetch tier's transport opens its own sockets (ADR 0060)"
)]
fn build(builder: reqwest::ClientBuilder) -> reqwest::Result<reqwest::Client> {
    builder.build()
}

/// `builder` binding its sockets to `interface`'s device, where the OS
/// lets a socket bind one (ADR 0044).
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "illumos"))]
fn bind_device(builder: reqwest::ClientBuilder, interface: Option<&str>) -> reqwest::ClientBuilder {
    match interface {
        Some(interface) => builder.interface(interface),
        None => builder,
    }
}

/// `builder` as it is: Windows binds the interface's address alone, which
/// its strong host model sends from that interface (ADR 0044).
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "illumos")))]
fn bind_device(
    builder: reqwest::ClientBuilder,
    _interface: Option<&str>,
) -> reqwest::ClientBuilder {
    builder
}

impl Transport for Reqwest {
    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            self.resolver.before_sending(&hop.url).await?;
            let bound = self.binding(hop).await?;
            let mut request = self
                .client(bound, hop, reuse)
                .request(hop.method.clone(), hop.url.clone());
            for (name, value) in &hop.headers {
                request = request.header(name, value);
            }
            if let Some(body) = &hop.body {
                request = request.body(body.clone());
            }
            let (response, connected) = CONNECTED
                .scope(Cell::new(false), async {
                    let response = request.send().await;
                    (response, CONNECTED.with(Cell::get))
                })
                .await;
            let reused = reuse == Reuse::Allowed && !connected;
            let response = response.map_err(|error| match failure_of(&error, &hop.url) {
                TransportFailure::ConnectionClosed | TransportFailure::ConnectionReset
                    if reused =>
                {
                    TransportFailure::ClosedAtReuse
                }
                TransportFailure::Http11Required => {
                    self.http1_origins.insert(&hop.url);
                    TransportFailure::Http11Required
                }
                failure => failure,
            })?;
            let headers = response
                .headers()
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        String::from_utf8_lossy(value.as_bytes()).into_owned(),
                    )
                })
                .collect();
            Ok(Sent {
                status: response.status().as_u16(),
                headers,
                body: Box::new(ReqwestBody {
                    response,
                    url: hop.url.clone(),
                }),
                reused,
                decoded: false,
            })
        })
    }
}

tokio::task_local! {
    /// Whether the hop sent on this task opened a connection: set by
    /// [`MarkConnects`] when reqwest's connector is asked for one, which
    /// reqwest does on the hop's own task, and only when no idle
    /// connection is ready for it.
    static CONNECTED: Cell<bool>;
}

/// The connector layer that marks the hop's task when it opens a
/// connection, so a hop that opened none is known to have reused one.
#[derive(Clone, Copy, Debug)]
struct MarkConnects;

impl<S> tower_layer::Layer<S> for MarkConnects {
    type Service = Marking<S>;

    fn layer(&self, inner: S) -> Self::Service {
        Marking(inner)
    }
}

/// reqwest's connector, behind [`MarkConnects`].
#[derive(Clone, Debug)]
struct Marking<S>(S);

impl<S, Request> tower_service::Service<Request> for Marking<S>
where
    S: tower_service::Service<Request>,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = S::Future;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.0.poll_ready(context)
    }

    fn call(&mut self, request: Request) -> Self::Future {
        // A connection opened outside a hop's scope, which no hop reads.
        let _ = CONNECTED.try_with(|connected| connected.set(true));
        self.0.call(request)
    }
}

/// The origins that answered `HTTP_1_1_REQUIRED`, which keep HTTP/1.1 for
/// the transport's life (ADR 0118). Clones share them.
#[derive(Clone, Debug, Default)]
#[expect(
    clippy::disallowed_types,
    reason = "the set is read or written in one step, never across an `.await` or another lock, so there is no lock order to test under loom (ADR 0060)"
)]
struct Http1Origins(Arc<std::sync::Mutex<BTreeSet<String>>>);

impl Http1Origins {
    fn origins(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn contains(&self, url: &Url) -> bool {
        self.origins().contains(&url.origin().ascii_serialization())
    }

    fn insert(&self, url: &Url) {
        self.origins().insert(url.origin().ascii_serialization());
    }
}

/// A reqwest answer's body.
struct ReqwestBody {
    response: reqwest::Response,
    url: Url,
}

impl Body for ReqwestBody {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(async move {
            self.response
                .chunk()
                .await
                .map_err(|error| failure_of(&error, &self.url))
        })
    }
}

/// `error`'s failure, an egress refusal naming the hop's URL.
fn failure_of(error: &reqwest::Error, url: &Url) -> TransportFailure {
    match classify(error) {
        TransportFailure::Egress(refused) => TransportFailure::Egress(Refused {
            url: url.to_string(),
            ..refused
        }),
        failure => failure,
    }
}

/// Why the transport's resolver gave no address.
#[derive(Debug, thiserror::Error)]
enum ResolveFailure {
    #[error("no address for the name")]
    NotResolved,
    #[error("{name} has no address of the family the Session holds")]
    Family { name: String, held: IpFamily },
    #[error("{0}")]
    Egress(Refused),
}

impl From<ResolveFailure> for TransportFailure {
    fn from(failure: ResolveFailure) -> Self {
        match failure {
            ResolveFailure::NotResolved => Self::NameNotResolved,
            ResolveFailure::Family { name, held } => Self::AddressFamily { name, held },
            ResolveFailure::Egress(refused) => Self::Egress(refused),
        }
    }
}

/// The transport's resolver: the engine's [`Dns`], whose answers, the
/// `network.resolve` mapping's included, the Session's egress policy
/// judges: in the connector when the connection goes direct, so
/// every address a connection may try is judged on its own lookup before
/// it tries one; ahead of the request when it goes through a proxy, which
/// would resolve the name again itself.
struct Resolver {
    dns: Dns,
    egress: Policy,
    /// Whether its connections go to the site directly, not through a
    /// proxy.
    direct: bool,
    /// The families its connections may reach, those the Session holds a
    /// local address of; every family when it holds none.
    families: Option<Vec<IpFamily>>,
}

impl Resolver {
    /// The resolver of a transport under `config`, its connections
    /// reaching `family` alone when given, else every family the Session
    /// holds.
    fn new(config: &TransportConfig, family: Option<IpFamily>) -> Self {
        let dns = Dns::new(
            HostMapping::new(
                config
                    .resolve
                    .iter()
                    .map(|(host, addresses)| (host.as_str(), addresses.clone())),
            ),
            config.resolver.clone(),
            config.clock,
        );
        let dns = match &config.local {
            Some(local) => dns.from_local_addresses(local.ipv4, local.ipv6),
            None => dns,
        };
        let families = config.local.as_ref().map(|local| match family {
            Some(family) => vec![family],
            None => local.families(),
        });
        Self {
            dns,
            egress: config.egress.clone(),
            direct: config.proxy.is_none(),
            families,
        }
    }

    /// The family of the first address `url`'s host answers among those
    /// the Session holds, IPv4 first; `None` for no host.
    async fn first_family(&self, url: &Url) -> Result<Option<IpFamily>, ResolveFailure> {
        let name = match url.host() {
            Some(url::Host::Domain(name)) => name.to_owned(),
            Some(url::Host::Ipv4(_)) => return Ok(Some(IpFamily::Ipv4)),
            Some(url::Host::Ipv6(_)) => return Ok(Some(IpFamily::Ipv6)),
            None => return Ok(None),
        };
        let addresses = self.addresses(name).await?;
        Ok(addresses.first().map(|address| family_of(address.ip())))
    }

    /// The addresses of `name` for the connector, judged when the
    /// connection goes direct.
    fn addresses(
        &self,
        name: String,
    ) -> impl Future<Output = Result<Vec<SocketAddr>, ResolveFailure>> + Send + 'static {
        lookup(
            self.dns.clone(),
            self.direct.then(|| self.egress.clone()),
            self.families.clone(),
            name,
        )
    }

    /// Judges a hop to `url` before it is sent: its URL and any address it
    /// names in place of a host, and, through a proxy, the addresses its
    /// name resolves to, the proxy's own lookup being out of the
    /// transport's reach (ADR 0017).
    async fn before_sending(&self, url: &Url) -> Result<(), ResolveFailure> {
        judged(url, &self.egress).map_err(ResolveFailure::Egress)?;
        if self.direct {
            return Ok(());
        }
        self.resolved(url).await
    }

    /// Judges the addresses `url`'s name resolves to, whatever the route.
    async fn resolved(&self, url: &Url) -> Result<(), ResolveFailure> {
        let Some(url::Host::Domain(name)) = url.host() else {
            return Ok(());
        };
        lookup(
            self.dns.clone(),
            Some(self.egress.clone()),
            None,
            name.to_owned(),
        )
        .await
        .map(|_| ())
        .map_err(|failure| match failure {
            ResolveFailure::Egress(refused) => ResolveFailure::Egress(Refused {
                url: url.to_string(),
                ..refused
            }),
            failure @ (ResolveFailure::NotResolved | ResolveFailure::Family { .. }) => failure,
        })
    }
}

/// The addresses `dns` gives `name`, the `network.resolve` mapping's as the
/// resolver's, judged by `judged` when given, and only those of
/// `families` when given: a name answering in none of them, with a
/// Session holding one family, fails with the family held.
async fn lookup(
    dns: Dns,
    judged: Option<Policy>,
    families: Option<Vec<IpFamily>>,
    name: String,
) -> Result<Vec<SocketAddr>, ResolveFailure> {
    let Ok(Resolution::Addresses { addresses, .. }) = dns.lookup(&name, Route::Direct).await else {
        return Err(ResolveFailure::NotResolved);
    };
    let addresses: Vec<IpAddr> = match families {
        Some(families) => {
            let held: Vec<IpAddr> = addresses
                .iter()
                .copied()
                .filter(|address| families.contains(&family_of(*address)))
                .collect();
            match (held.is_empty(), families.as_slice()) {
                (true, [family]) if !addresses.is_empty() => {
                    return Err(ResolveFailure::Family {
                        name,
                        held: *family,
                    });
                }
                (true, _) => return Err(ResolveFailure::NotResolved),
                (false, _) => held,
            }
        }
        None => addresses,
    };
    let addresses: Vec<SocketAddr> = addresses
        .into_iter()
        .map(|address| SocketAddr::new(address, 0))
        .collect();
    if let Some(policy) = judged {
        // The policy reads the host a URL names, so a host listed in
        // `egress.private_ranges` opens its own connections; the hop's
        // URL replaces this one in a refusal.
        egress::judge_addresses(&format!("http://{name}/"), &addresses, &policy)
            .map_err(ResolveFailure::Egress)?;
    }
    Ok(addresses)
}

/// The address check of a transport's connections, run ahead of a crawl
/// request, so one whose host stands for an address the Session's egress
/// policy refuses ends before anything is sent (ADR 0017): the URL, any
/// address it names in place of a host, and the addresses its name
/// resolves to through the transport's own resolver, whose cache it
/// shares.
#[derive(Clone, Debug)]
pub struct AddressCheck(Arc<Resolver>);

impl fmt::Debug for Resolver {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Resolver")
            .field("direct", &self.direct)
            .finish_non_exhaustive()
    }
}

impl AddressCheck {
    /// Judges `url` as the transport's connection to it would be judged.
    /// A name that resolves to no address passes: its fetch then fails
    /// with `name_not_resolved`.
    ///
    /// # Errors
    ///
    /// [`Refused`] by the domain policy or the address check.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops its lookup.
    pub async fn check(&self, url: &Url) -> Result<(), Refused> {
        judged(url, &self.0.egress)?;
        match self.0.resolved(url).await {
            Ok(()) | Err(ResolveFailure::NotResolved | ResolveFailure::Family { .. }) => Ok(()),
            Err(ResolveFailure::Egress(refused)) => Err(refused),
        }
    }
}

impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let addresses = self.addresses(name.as_str().to_owned());
        Box::pin(async move { Ok(Box::new(addresses.await?.into_iter()) as reqwest::dns::Addrs) })
    }
}

/// The failure `error` is, read down its chain of sources: the first one
/// a variant names decides; anything else is the generic case. reqwest's
/// translation point.
pub(crate) fn classify(error: &(dyn Error + 'static)) -> TransportFailure {
    walk(error, named)
}

/// The failure `error` is, read down its chain of sources with
/// `library`'s names for its own errors beside those every transport
/// shares: the first one a variant names decides; anything else is the
/// generic case.
fn walk(
    error: &(dyn Error + 'static),
    library: fn(&(dyn Error + 'static)) -> Option<TransportFailure>,
) -> TransportFailure {
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(failure) = shared(error)
            .or_else(|| library(error))
            .or_else(|| io_failure(error))
        {
            return failure;
        }
        next = match error.downcast_ref::<std::io::Error>() {
            // An `io::Error`'s `source` skips the error it wraps.
            Some(io) => io.get_ref().map(|inner| inner as &(dyn Error + 'static)),
            None => error.source(),
        };
    }
    TransportFailure::Other
}

/// The failure one error of a chain names among those every transport
/// meets: its resolver's, and a proxy's answer to the tunnel request.
fn shared(error: &(dyn Error + 'static)) -> Option<TransportFailure> {
    if let Some(resolve) = error.downcast_ref::<ResolveFailure>() {
        return Some(match resolve {
            ResolveFailure::NotResolved => TransportFailure::NameNotResolved,
            ResolveFailure::Family { name, held } => TransportFailure::AddressFamily {
                name: name.clone(),
                held: *held,
            },
            ResolveFailure::Egress(refused) => TransportFailure::Egress(refused.clone()),
        });
    }
    tunnel_failure(&error.to_string())
}

/// The failure one error of reqwest's chain names, if it names one:
/// rustls's, h2's and hyper's.
fn named(error: &(dyn Error + 'static)) -> Option<TransportFailure> {
    if let Some(tls) = error.downcast_ref::<rustls::Error>() {
        return Some(match tls {
            rustls::Error::InvalidCertificate(_) => TransportFailure::CertificateInvalid,
            _ => TransportFailure::Tls,
        });
    }
    if let Some(h2) = error.downcast_ref::<h2::Error>() {
        return match h2.reason() {
            Some(h2::Reason::REFUSED_STREAM) => Some(TransportFailure::RefusedStream),
            Some(h2::Reason::HTTP_1_1_REQUIRED) => Some(TransportFailure::Http11Required),
            // A graceful `GOAWAY` fails only the streams above its last
            // one, which the site never processed (RFC 9113, section 6.8).
            Some(h2::Reason::NO_ERROR) if h2.is_go_away() && h2.is_remote() => {
                Some(TransportFailure::RefusedStream)
            }
            _ => None,
        };
    }
    if let Some(hyper) = error.downcast_ref::<hyper::Error>() {
        if hyper.is_parse() {
            return Some(TransportFailure::InvalidResponse);
        }
        if hyper.is_incomplete_message() {
            return Some(TransportFailure::ConnectionClosed);
        }
    }
    None
}

/// The failure an OS error of a chain names, if it names one.
fn io_failure(error: &(dyn Error + 'static)) -> Option<TransportFailure> {
    let io = error.downcast_ref::<std::io::Error>()?;
    if io.get_ref().is_some() {
        return None;
    }
    match io.kind() {
        std::io::ErrorKind::ConnectionRefused => Some(TransportFailure::ConnectionRefused),
        std::io::ErrorKind::TimedOut => Some(TransportFailure::ConnectionTimedOut),
        std::io::ErrorKind::ConnectionReset => Some(TransportFailure::ConnectionReset),
        std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionAborted => {
            Some(TransportFailure::ConnectionClosed)
        }
        _ => None,
    }
}

/// The proxy's answer to the tunnel request, as hyper-util's private
/// tunnel error writes it, and `wreq`'s, which writes the same words.
fn tunnel_failure(text: &str) -> Option<TransportFailure> {
    match text.strip_prefix("tunnel error: ")? {
        "unsuccessful" => Some(TransportFailure::TunnelRefused),
        "proxy authorization required" => Some(TransportFailure::ProxyAuthenticationRequired),
        "failed to create underlying connection" => Some(TransportFailure::ProxyUnreachable),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
