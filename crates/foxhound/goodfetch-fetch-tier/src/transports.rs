//! The HTTP transports (spec #579; ADRs 0060, 0118, 0127): reqwest in
//! `standard` and `identified` Sessions, filling the driver's `send(hop)`.
//! It opens its own sockets (ADR 0060's one exception for a transport),
//! names hosts through the engine's [`Dns`] (the `network.resolve` mapping,
//! then the engine's resolver) and verifies certificates under the
//! engine's [`Trust`]. It follows no redirect, resends nothing, decodes no
//! content coding and sets no timeout of its own: the driver does each.
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
//! refuses opens no connection, and the resolver's answers are judged
//! before any connection. The `network.resolve` mapping's addresses are
//! not judged, as on the browser path, until `egress.private_ranges` is a
//! setting a Run can declare (#622, #623). Through a proxy, which resolves
//! the site's name itself, only the URL is judged.

use std::error::Error;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use bytes::Bytes;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::{
    AnsweredBy, ConfigurationUnreadable, Dns, HostMapping, Resolution, ResolverConfiguration, Route,
};
use goodfetch_engine::egress::{self, Judgment, Policy, Refused, RequestKind};
use goodfetch_engine::tls::Trust;
use url::Url;

use crate::driver::{Body, Hop, Sending, Sent, Transport, TransportFailure};

/// What a reqwest transport reaches and trusts.
#[derive(Clone, Debug)]
pub struct ReqwestConfig {
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
}

/// Why a reqwest transport could not be built.
#[derive(Debug, thiserror::Error)]
pub enum ReqwestUnavailable {
    /// The system's resolver configuration could not be read.
    #[error(transparent)]
    Resolver(#[from] ConfigurationUnreadable),
    /// reqwest refused the configuration, such as a proxy URL it does not
    /// take.
    #[error("the HTTP transport could not be built: {0}")]
    Client(String),
}

/// reqwest, the transport of `standard` and `identified` Sessions.
#[derive(Clone, Debug)]
pub struct Reqwest {
    client: reqwest::Client,
    egress: Policy,
}

impl Reqwest {
    /// A transport under `config`, holding its own connection pool, so
    /// hops of one Run reuse connections (#763).
    ///
    /// # Errors
    ///
    /// [`ReqwestUnavailable`] when the system's resolver configuration
    /// cannot be read or reqwest refuses the proxy.
    pub fn new(config: ReqwestConfig) -> Result<Self, ReqwestUnavailable> {
        let dns = Dns::new(
            HostMapping::new(
                config
                    .resolve
                    .iter()
                    .map(|(host, addresses)| (host.as_str(), addresses.clone())),
            ),
            ResolverConfiguration::system()?,
            config.clock,
        );
        let mut tls = config.trust.client_config();
        tls.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        let resolver = Resolver {
            dns,
            judged: config.proxy.is_none().then(|| config.egress.clone()),
        };
        let builder = reqwest::Client::builder()
            .tls_backend_preconfigured(tls)
            .dns_resolver(Arc::new(resolver))
            // A folded header line is read as a space, as Chrome reads it.
            .http1_allow_obsolete_multiline_headers_in_responses(true)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never());
        let builder = match &config.proxy {
            Some(proxy) => builder.proxy(
                reqwest::Proxy::all(proxy.as_str())
                    .map_err(|error| ReqwestUnavailable::Client(error.to_string()))?,
            ),
            None => builder.no_proxy(),
        };
        let client =
            build(builder).map_err(|error| ReqwestUnavailable::Client(error.to_string()))?;
        Ok(Self {
            client,
            egress: config.egress,
        })
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
    fn send<'a>(&'a self, hop: &'a Hop) -> Sending<'a, Sent> {
        Box::pin(async move {
            let request = egress::Request {
                url: hop.url.as_str(),
                kind: RequestKind::Fetch,
            };
            if let Judgment::Refused(refused) = egress::judge(&request, &self.egress) {
                return Err(TransportFailure::Egress(refused));
            }
            let mut request = self.client.request(hop.method.clone(), hop.url.clone());
            for (name, value) in &hop.headers {
                request = request.header(name, value);
            }
            if let Some(body) = &hop.body {
                request = request.body(body.clone());
            }
            let response = request
                .send()
                .await
                .map_err(|error| failure_of(&error, &hop.url))?;
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
            })
        })
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

/// The transport's resolver: the engine's [`Dns`], whose answers the
/// Session's egress policy judges when the connection goes direct.
struct Resolver {
    dns: Dns,
    judged: Option<Policy>,
}

impl reqwest::dns::Resolve for Resolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let dns = self.dns.clone();
        let judged = self.judged.clone();
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let Ok(Resolution::Addresses {
                addresses,
                answered_by,
            }) = dns.lookup(&name, Route::Direct).await
            else {
                return Err(ResolveFailure::NotResolved.into());
            };
            let addresses: Vec<SocketAddr> = addresses
                .into_iter()
                .map(|address| SocketAddr::new(address, 0))
                .collect();
            if let Some(policy) = judged.filter(|_| answered_by != AnsweredBy::Mapping) {
                egress::judge_addresses(&name, &addresses, &policy)
                    .map_err(ResolveFailure::Egress)?;
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The failure `error` is, read down its chain of sources: the first one
/// a variant names decides; anything else is the generic case.
pub(crate) fn classify(error: &(dyn Error + 'static)) -> TransportFailure {
    let mut next = Some(error);
    while let Some(error) = next {
        if let Some(failure) = named(error) {
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

/// The failure one error of a chain names, if it names one.
fn named(error: &(dyn Error + 'static)) -> Option<TransportFailure> {
    if let Some(resolve) = error.downcast_ref::<ResolveFailure>() {
        return Some(match resolve {
            ResolveFailure::NotResolved => TransportFailure::NameNotResolved,
            ResolveFailure::Egress(refused) => TransportFailure::Egress(refused.clone()),
        });
    }
    if let Some(tunnel) = tunnel_failure(&error.to_string()) {
        return Some(tunnel);
    }
    if let Some(tls) = error.downcast_ref::<rustls::Error>() {
        return Some(match tls {
            rustls::Error::InvalidCertificate(_) => TransportFailure::CertificateInvalid,
            _ => TransportFailure::Tls,
        });
    }
    if let Some(hyper) = error.downcast_ref::<hyper::Error>() {
        if hyper.is_parse() {
            return Some(TransportFailure::InvalidResponse);
        }
        if hyper.is_incomplete_message() {
            return Some(TransportFailure::ConnectionClosed);
        }
    }
    if let Some(io) = error.downcast_ref::<std::io::Error>()
        && io.get_ref().is_none()
    {
        return match io.kind() {
            std::io::ErrorKind::ConnectionRefused => Some(TransportFailure::ConnectionRefused),
            std::io::ErrorKind::TimedOut => Some(TransportFailure::ConnectionTimedOut),
            std::io::ErrorKind::ConnectionReset => Some(TransportFailure::ConnectionReset),
            std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::ConnectionAborted => {
                Some(TransportFailure::ConnectionClosed)
            }
            _ => None,
        };
    }
    None
}

/// The proxy's answer to the tunnel request, as hyper-util's private
/// tunnel error writes it.
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
