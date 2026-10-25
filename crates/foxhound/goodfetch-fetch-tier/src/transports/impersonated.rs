//! `wreq`, the transport of HTTP-only `stealth` Sessions (spec #579; ADRs
//! 0020, 0042, 0060, 0118 and 0127): BoringSSL through `btls` and the
//! `http2` fork, which send its [`HttpProfile`]'s TLS ClientHello and
//! HTTP/2 settings, so a crawl over HTTP presents the client its browser
//! would. It fills the driver's `send(hop)` as reqwest does: the same
//! resolver, egress checks, connection marks, resends on a new connection
//! and HTTP/1.1 origins, and none of `wreq`'s redirects, retries or
//! content decoding.
//!
//! **Trust (ADR 0042):** BoringSSL takes one store of authorities per
//! client, not a decision per host, so the transport keeps one client per
//! scope of the engine's [`Trust`] and sends each hop to its host's: the
//! profile is the same in every scope, so a site cannot tell them apart.
//! Mozilla's roots come whole from `webpki-root-certs`.
//!
//! **Translation point (ADR 0127):** [`classify`] is where `wreq`'s,
//! `wreq-proto`'s, `http2`'s, BoringSSL's and the OS's failures, and a
//! proxy's answer to the tunnel request, become a [`TransportFailure`].
//! It reads the forks' types and words, so the forks are pinned exactly
//! (the workspace's `Cargo.toml`), and the pin check of `tests.rs`, which
//! holds each fork's version and checksum beside the table of its cases,
//! fails the build when `Cargo.lock` moves one: the table is then read
//! again against the new sources.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::Arc;

use bytes::Bytes;
use goodfetch_engine::egress::{self, Judgment, Policy, Refused, RequestKind};
use http_body_util::BodyExt as _;
use url::Url;

use super::{
    CONNECTED, Http1Origins, MarkConnects, Resolver, TransportConfig, TransportUnavailable,
};
use crate::driver::{Body, Hop, Reuse, Sending, Sent, Transport, TransportFailure};
use crate::http_profile::HttpProfile;

/// `wreq`, the transport of HTTP-only `stealth` Sessions.
#[derive(Clone)]
pub struct Wreq {
    /// Its clients offering HTTP/2 and HTTP/1.1, by trust scope.
    any: Scopes,
    /// Its clients offering HTTP/1.1 alone, for the origins that asked.
    http1: Scopes,
    http1_origins: Http1Origins,
    trust: goodfetch_engine::tls::Trust,
    egress: Policy,
}

impl std::fmt::Debug for Wreq {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Wreq").finish_non_exhaustive()
    }
}

/// The clients of each trust scope, `None` for the hosts no listed domain
/// covers.
type Scopes = BTreeMap<Option<String>, Clients>;

/// A pooled client and one that opens a new connection for each request.
#[derive(Clone)]
struct Clients {
    pooled: wreq::Client,
    new: wreq::Client,
}

impl Wreq {
    /// A transport under `config` presenting `profile`, holding its own
    /// connection pools, so hops of one Run reuse connections.
    ///
    /// # Errors
    ///
    /// [`TransportUnavailable`] when the system's resolver configuration
    /// cannot be read, or `wreq` refuses the proxy or an authority.
    pub fn new(
        config: TransportConfig,
        profile: HttpProfile,
    ) -> Result<Self, TransportUnavailable> {
        let resolver = Arc::new(Resolver::new(&config)?);
        let refused = |error: wreq::Error| TransportUnavailable::Client(error.to_string());
        let proxy = config
            .proxy
            .as_ref()
            .map(|proxy| wreq::Proxy::all(proxy.as_str()).map_err(refused))
            .transpose()?;
        let mut any = Scopes::new();
        let mut http1 = Scopes::new();
        for scope in config.trust.scopes() {
            let client = |http1_only: bool, pooled: bool| {
                let store = wreq::tls::trust::CertStore::builder()
                    .add_der_certs(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter())
                    .add_der_certs(scope.authorities.iter())
                    .build()
                    .map_err(refused)?;
                let builder = wreq::Client::builder()
                    .emulation(profile.emulation())
                    // A folded header line is read as a space, as Chrome
                    // reads it.
                    .http1_options(
                        wreq::http1::Http1Options::builder()
                            .allow_obsolete_multiline_headers_in_responses(true)
                            .build(),
                    )
                    .tls_cert_store(store)
                    .dns_resolver(Arc::clone(&resolver) as Arc<dyn wreq::dns::Resolve>)
                    .redirect(wreq::redirect::Policy::none())
                    .retry(wreq::retry::Policy::never())
                    .connector_layer(MarkConnects);
                let builder = if pooled {
                    builder
                } else {
                    builder.pool_max_idle_per_host(0)
                };
                let builder = if http1_only {
                    builder.http1_only()
                } else {
                    builder
                };
                let builder = match &proxy {
                    Some(proxy) => builder.proxy(proxy.clone()),
                    None => builder.no_proxy(),
                };
                build(builder).map_err(refused)
            };
            let clients = |http1_only: bool| -> Result<Clients, TransportUnavailable> {
                Ok(Clients {
                    pooled: client(http1_only, true)?,
                    new: client(http1_only, false)?,
                })
            };
            any.insert(scope.domain.clone(), clients(false)?);
            http1.insert(scope.domain, clients(true)?);
        }
        Ok(Self {
            any,
            http1,
            http1_origins: Http1Origins::default(),
            trust: config.trust,
            egress: config.egress,
        })
    }

    /// The client `hop` goes out on, as its host's trust scope and `reuse`
    /// allow.
    fn client(&self, hop: &Hop, reuse: Reuse) -> Option<&wreq::Client> {
        let scopes = if self.http1_origins.contains(&hop.url) {
            &self.http1
        } else {
            &self.any
        };
        let scope = hop
            .url
            .host_str()
            .and_then(|host| self.trust.scope_of(host))
            .map(str::to_owned);
        let clients = scopes.get(&scope)?;
        Some(match reuse {
            Reuse::Allowed => &clients.pooled,
            Reuse::Never => &clients.new,
        })
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the stealth HTTP fetch tier's transport opens its own sockets (ADR 0060)"
)]
fn build(builder: wreq::ClientBuilder) -> wreq::Result<wreq::Client> {
    builder.build()
}

impl Transport for Wreq {
    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            let request = egress::Request {
                url: hop.url.as_str(),
                kind: RequestKind::Fetch,
            };
            if let Judgment::Refused(refused) = egress::judge(&request, &self.egress) {
                return Err(TransportFailure::Egress(refused));
            }
            // Every scope has its clients, so a host always finds one.
            let client = self.client(hop, reuse).ok_or(TransportFailure::Other)?;
            let mut request = client.request(hop.method.clone(), hop.url.as_str());
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
                body: Box::new(WreqBody {
                    response,
                    url: hop.url.clone(),
                }),
                reused,
                decoded: false,
            })
        })
    }
}

/// A `wreq` answer's body.
struct WreqBody {
    response: wreq::Response,
    url: Url,
}

impl Body for WreqBody {
    fn chunk(&mut self) -> Sending<'_, Option<Bytes>> {
        Box::pin(async move {
            loop {
                let Some(frame) = self.response.frame().await else {
                    return Ok(None);
                };
                let frame = frame.map_err(|error| failure_of(&error, &self.url))?;
                // Trailers carry no bytes of the body.
                if let Ok(data) = frame.into_data() {
                    return Ok(Some(data));
                }
            }
        })
    }
}

impl wreq::dns::Resolve for Resolver {
    fn resolve(&self, name: wreq::dns::Name) -> wreq::dns::Resolving {
        let addresses = self.addresses(name.as_str().to_owned());
        Box::pin(async move { Ok(Box::new(addresses.await?.into_iter()) as wreq::dns::Addrs) })
    }
}

/// `error`'s failure, an egress refusal naming the hop's URL.
fn failure_of(error: &wreq::Error, url: &Url) -> TransportFailure {
    match classify(error) {
        TransportFailure::Egress(refused) => TransportFailure::Egress(Refused {
            url: url.to_string(),
            ..refused
        }),
        failure => failure,
    }
}

/// The failure `error` is, read down its chain of sources: the first one
/// a variant names decides; anything else is the generic case. `wreq`'s
/// translation point.
pub(crate) fn classify(error: &(dyn Error + 'static)) -> TransportFailure {
    super::walk(error, named)
}

/// BoringSSL's reason for a certificate that failed verification.
const CERTIFICATE_VERIFY_FAILED: &str = "CERTIFICATE_VERIFY_FAILED";

/// The failure one error of `wreq`'s chain names, if it names one:
/// BoringSSL's, `http2`'s and `wreq-proto`'s.
fn named(error: &(dyn Error + 'static)) -> Option<TransportFailure> {
    if let Some(ssl) = error.downcast_ref::<btls::ssl::Error>() {
        // A failure of the socket under TLS is the OS's, further down.
        return ssl.ssl_error().map(stack_failure);
    }
    if let Some(stack) = error.downcast_ref::<btls::error::ErrorStack>() {
        return Some(stack_failure(stack));
    }
    if let Some(h2) = error.downcast_ref::<http2::Error>() {
        return match h2.reason() {
            Some(http2::Reason::REFUSED_STREAM) => Some(TransportFailure::RefusedStream),
            Some(http2::Reason::HTTP_1_1_REQUIRED) => Some(TransportFailure::Http11Required),
            // A graceful `GOAWAY` fails only the streams above its last
            // one, which the site never processed (RFC 9113, section 6.8).
            Some(http2::Reason::NO_ERROR) if h2.is_go_away() && h2.is_remote() => {
                Some(TransportFailure::RefusedStream)
            }
            _ => None,
        };
    }
    if let Some(proto) = error.downcast_ref::<wreq_proto::Error>() {
        if proto.is_parse() {
            return Some(TransportFailure::InvalidResponse);
        }
        if proto.is_incomplete_message() {
            return Some(TransportFailure::ConnectionClosed);
        }
    }
    None
}

/// The failure BoringSSL's error stack names: a certificate that failed
/// verification, else any other TLS failure.
fn stack_failure(stack: &btls::error::ErrorStack) -> TransportFailure {
    if stack
        .errors()
        .iter()
        .any(|error| error.reason() == Some(CERTIFICATE_VERIFY_FAILED))
    {
        TransportFailure::CertificateInvalid
    } else {
        TransportFailure::Tls
    }
}

#[cfg(test)]
mod tests;
