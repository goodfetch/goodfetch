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
use goodfetch_engine::tls::Trust;
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
    /// Its clients offering HTTP/2 and HTTP/1.1.
    any: Clients,
    /// Its clients offering HTTP/1.1 alone, for the origins that asked.
    http1: Clients,
    http1_origins: Http1Origins,
    resolver: Arc<Resolver>,
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
        let resolver = Arc::new(Resolver::new(&config));
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
                    .connector_layer(MarkConnects);
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
        Ok(Self {
            any: clients(&[b"h2", b"http/1.1"])?,
            http1: clients(&[b"http/1.1"])?,
            http1_origins: Http1Origins::default(),
            resolver,
        })
    }

    /// The address check of its connections, run ahead of a request.
    pub fn address_check(&self) -> AddressCheck {
        AddressCheck(Arc::clone(&self.resolver))
    }

    /// The client `hop` goes out on, as `reuse` allows.
    fn client(&self, hop: &Hop, reuse: Reuse) -> &reqwest::Client {
        let clients = if self.http1_origins.contains(&hop.url) {
            &self.http1
        } else {
            &self.any
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

impl Transport for Reqwest {
    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            self.resolver.before_sending(&hop.url).await?;
            let mut request = self
                .client(hop, reuse)
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
    #[error("{0}")]
    Egress(Refused),
}

impl From<ResolveFailure> for TransportFailure {
    fn from(failure: ResolveFailure) -> Self {
        match failure {
            ResolveFailure::NotResolved => Self::NameNotResolved,
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
}

impl Resolver {
    /// The resolver of a transport under `config`.
    fn new(config: &TransportConfig) -> Self {
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
        Self {
            dns,
            egress: config.egress.clone(),
            direct: config.proxy.is_none(),
        }
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
        lookup(self.dns.clone(), Some(self.egress.clone()), name.to_owned())
            .await
            .map(|_| ())
            .map_err(|failure| match failure {
                ResolveFailure::Egress(refused) => ResolveFailure::Egress(Refused {
                    url: url.to_string(),
                    ..refused
                }),
                failure @ ResolveFailure::NotResolved => failure,
            })
    }
}

/// The addresses `dns` gives `name`, the `network.resolve` mapping's as the
/// resolver's, judged by `judged` when given.
async fn lookup(
    dns: Dns,
    judged: Option<Policy>,
    name: String,
) -> Result<Vec<SocketAddr>, ResolveFailure> {
    let Ok(Resolution::Addresses { addresses, .. }) = dns.lookup(&name, Route::Direct).await else {
        return Err(ResolveFailure::NotResolved);
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
            Ok(()) | Err(ResolveFailure::NotResolved) => Ok(()),
            Err(ResolveFailure::Egress(refused)) => Err(refused),
        }
    }

    /// The addresses a connection to `url`'s host may use, each judged as
    /// the transport's own connections are: the address the URL names in
    /// place of a host, or those its name resolves to. The light-browser
    /// bridge connects to these alone (ADR 0017).
    ///
    /// # Errors
    ///
    /// `Some` refusal by the domain policy or the address check; `None`
    /// for a name that resolves to no address.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future drops its lookup.
    pub(crate) async fn connectable(&self, url: &Url) -> Result<Vec<IpAddr>, Option<Refused>> {
        judged(url, &self.0.egress).map_err(Some)?;
        match url.host() {
            Some(url::Host::Ipv4(address)) => Ok(vec![IpAddr::V4(address)]),
            Some(url::Host::Ipv6(address)) => Ok(vec![IpAddr::V6(address)]),
            Some(url::Host::Domain(name)) => lookup(
                self.0.dns.clone(),
                Some(self.0.egress.clone()),
                name.to_owned(),
            )
            .await
            .map(|addresses| addresses.iter().map(SocketAddr::ip).collect())
            .map_err(|failure| match failure {
                ResolveFailure::Egress(refused) => Some(refused),
                ResolveFailure::NotResolved => None,
            }),
            None => Err(None),
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
