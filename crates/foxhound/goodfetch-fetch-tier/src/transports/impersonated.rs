//! `wreq`, the transport of HTTP-only `stealth` Sessions (spec #579; ADRs
//! 0020, 0042, 0060, 0118 and 0127): BoringSSL through `btls` and the
//! `http2` fork, which send its [`HttpProfile`]'s TLS ClientHello and
//! HTTP/2 settings, so a crawl over HTTP presents the client its browser
//! would. It fills the driver's `send(hop)` as reqwest does: the same
//! resolver, egress checks, connection marks, resends on a new connection
//! and HTTP/1.1 origins, and none of `wreq`'s redirects, retries or
//! content decoding.
//!
//! **Trust (ADR 0042, spec #565):** BoringSSL takes one store of
//! authorities per client, so the transport keeps one client per scope of
//! the engine's [`Trust`] and sends each hop to its host's: the profile is
//! the same in every scope, so a site cannot tell them apart. Mozilla's
//! roots come whole from `webpki-root-certs`. Each client verifies every
//! certificate during the handshake through Goodfetch's fork of `wreq`
//! (`ClientBuilder::tls_custom_verify`, BoringSSL's
//! `SSL_CTX_set_custom_verify`) with the engine's per-host decision, as the
//! egress relay does (ADR 0124): [`Examiner`] for the default roots and
//! `tls.trusted_cas`, and [`SharedTrust`] for the exceptions of
//! `tls.accept_invalid_certs` and their pins. A certificate that fails, or
//! whose host's exception has not decided yet, ends the handshake before
//! any byte of the request is sent; the verification leaves its verdict
//! for the transport, which ends the hop with `certificate_invalid`, or
//! takes the exception's decision and connects again, or ends it with
//! `certificate_changed`.
//!
//! **Client certificates:** its clients present none yet, so a hop to a
//! host a domain of `tls.client_certificates` covers is refused before it
//! connects, with `capability_unavailable` naming the setting, rather than
//! sent without the certificate the site asks for (step 7 review, finding
//! 563-565-3; the build that presents them goes with the `stealth`
//! Sessions that send through `wreq`).
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
use goodfetch_engine::CallError;
use goodfetch_engine::tls::{
    ClientCertificates, Decision, Examiner, Presented, SharedTrust, Verdict,
};
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};
use goodfetch_protocol::tls::{CertificateFailure, TLS_CLIENT_CERTIFICATES};
use http_body_util::BodyExt as _;
use url::Url;

use super::{
    AddressCheck, CONNECTED, DECISIONS, Http1Origins, MarkConnects, Resolver, TransportConfig,
    TransportUnavailable,
};
use crate::driver::{Body, CallFailure, Hop, Reuse, Sending, Sent, Transport, TransportFailure};
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
    /// The per-host decision of its clients' custom verification.
    decision: SharedTrust,
    /// The verdicts that verification left for the transport.
    verdicts: Verdicts,
    resolver: Arc<Resolver>,
    /// The client certificates of `tls.client_certificates`, which its
    /// clients cannot present yet: a hop to a host they cover is refused.
    client_certificates: Option<ClientCertificates>,
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
    /// [`TransportUnavailable`] when `wreq` refuses the proxy or an
    /// authority.
    pub fn new(
        config: TransportConfig,
        profile: HttpProfile,
    ) -> Result<Self, TransportUnavailable> {
        let resolver = Arc::new(Resolver::new(&config, None));
        let refused = |error: wreq::Error| TransportUnavailable::Client(error.to_string());
        let proxy = config
            .proxy
            .as_ref()
            .map(|proxy| wreq::Proxy::all(proxy.as_str()).map_err(refused))
            .transpose()?;
        let decision = SharedTrust::new(config.trust.clone(), config.exceptions.clone());
        let verdicts = Verdicts::default();
        let verify = custom_verify(
            decision.clone(),
            Arc::new(config.trust.examiner()),
            verdicts.clone(),
        );
        let client_certificates = config.client_certificates.clone();
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
                    .tls_custom_verify(verify.clone())
                    .dns_resolver(Arc::clone(&resolver) as Arc<dyn wreq::dns::Resolve>)
                    .redirect(wreq::redirect::Policy::none())
                    .retry(wreq::retry::Policy::never())
                    .connector_layer(MarkConnects);
                let builder = match &config.local {
                    Some(local) => bind_device(
                        builder.local_addresses(local.ipv4, local.ipv6),
                        local.interface.as_deref(),
                    ),
                    None => builder,
                };
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
            decision,
            verdicts,
            resolver,
            client_certificates,
        })
    }

    /// The address check of its connections, run ahead of a request.
    pub fn address_check(&self) -> AddressCheck {
        AddressCheck(Arc::clone(&self.resolver))
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

/// `builder` binding its sockets to `interface`'s device, where the OS
/// lets a socket bind one (ADR 0044).
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "illumos"))]
fn bind_device(builder: wreq::ClientBuilder, interface: Option<&str>) -> wreq::ClientBuilder {
    match interface {
        Some(interface) => builder.interface(interface.to_owned()),
        None => builder,
    }
}

/// `builder` as it is: Windows binds the interface's address alone, which
/// its strong host model sends from that interface (ADR 0044).
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "illumos")))]
fn bind_device(builder: wreq::ClientBuilder, _interface: Option<&str>) -> wreq::ClientBuilder {
    builder
}

impl Transport for Wreq {
    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            self.resolver.before_sending(&hop.url).await?;
            match hop.url.host_str() {
                Some(host) if hop.url.scheme() == "https" => {
                    self.send_to(&tls_host(host), hop, reuse).await
                }
                _ => self
                    .send_once(hop, reuse)
                    .await
                    .map_err(|(failure, _)| failure),
            }
        })
    }
}

impl Wreq {
    /// Sends `hop` over TLS to `host`, its certificate decided per host
    /// during each handshake: sent again on a new connection each time its
    /// handshake stopped for the exception's decision, which
    /// [`SharedTrust::decide_presented`] then took.
    async fn send_to(&self, host: &str, hop: &Hop, reuse: Reuse) -> Result<Sent, TransportFailure> {
        if let Some(unpresented) = self.unpresented(host) {
            return Err(unpresented);
        }
        self.decision
            .before(host)
            .await
            .map_err(TransportFailure::CertificateChanged)?;
        for _ in 0..DECISIONS {
            let presented = match self.send_once(hop, reuse).await {
                Ok(mut sent) => {
                    sent.certificate_exception = self.decision.exception(host);
                    return Ok(sent);
                }
                Err((failure, None)) => return Err(failure),
                Err((_, Some(presented))) => presented,
            };
            match self
                .decision
                .decide_presented(host, *presented, false)
                .await
            {
                Decision::Again => {}
                Decision::Changed(changed) => {
                    return Err(TransportFailure::CertificateChanged(changed));
                }
                // Its clients present no client certificate.
                Decision::Excepted => return Err(TransportFailure::Tls),
            }
        }
        Err(TransportFailure::Tls)
    }

    /// The refusal of a hop to `host` when a domain of
    /// `tls.client_certificates` covers it: its clients would connect
    /// without the certificate the setting lists for it.
    fn unpresented(&self, host: &str) -> Option<TransportFailure> {
        let certificates = self.client_certificates.as_ref()?;
        let domain = certificates.scope_of(host)?;
        Some(TransportFailure::ClientCertificate(CallFailure(
            CallError::new(
                &CAPABILITY_UNAVAILABLE,
                format!(
                    "`tls.client_certificates` lists a client certificate for `{domain}`, which a `stealth` Session's HTTP transport cannot present yet, so the request to `{host}` was not sent."
                ),
                &CapabilityUnavailableDetails {
                    key: TLS_CLIENT_CERTIFICATES.name.to_owned(),
                    layer: certificates.layer().unwrap_or(SettingsLayer::Run),
                    kind: "stealth_client_certificate".to_owned(),
                    fix: "an `identified` Session for the domain".to_owned(),
                },
            ),
        )))
    }

    /// Sends `hop` once: its answer, or its failure and, when its handshake
    /// stopped for its host's exception to decide, the certificate the host
    /// presented.
    async fn send_once(
        &self,
        hop: &Hop,
        reuse: Reuse,
    ) -> Result<Sent, (TransportFailure, Option<Box<Presented>>)> {
        let host = hop.url.host_str().map(tls_host).unwrap_or_default();
        let undecided = |failure| (failure, None);
        {
            // Every scope has its clients, so a host always finds one.
            let client = self
                .client(hop, reuse)
                .ok_or((TransportFailure::Other, None))?;
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
                    undecided(TransportFailure::ClosedAtReuse)
                }
                TransportFailure::Http11Required => {
                    self.http1_origins.insert(&hop.url);
                    undecided(TransportFailure::Http11Required)
                }
                // The custom verification refused the certificate, and
                // left its verdict.
                TransportFailure::CertificateInvalid(failure) => match self.verdicts.take(&host) {
                    Some(Verdict::Rejected(failure)) => {
                        undecided(TransportFailure::CertificateInvalid(failure))
                    }
                    Some(Verdict::Undecided(presented)) => (
                        TransportFailure::CertificateInvalid(failure),
                        Some(Box::new(presented)),
                    ),
                    Some(Verdict::Passes) | None => {
                        undecided(TransportFailure::CertificateInvalid(failure))
                    }
                },
                failure => undecided(failure),
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
                certificate_exception: None,
            })
        }
    }
}

/// `host`, a URL's, as `wreq` names the host of its connections: an IPv6
/// address without its brackets.
fn tls_host(host: &str) -> String {
    host.strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host)
        .to_owned()
}

/// The verdicts the custom verification left for the transport, by host:
/// each refused certificate's, read once its handshake failed. Clones
/// share them.
#[derive(Clone, Debug, Default)]
#[expect(
    clippy::disallowed_types,
    reason = "BoringSSL's verification callback is synchronous and cannot await: the map is read or written in one step, never across an `.await` or another lock, so there is no lock order to test under loom (ADR 0060)"
)]
struct Verdicts(Arc<std::sync::Mutex<BTreeMap<String, Verdict>>>);

impl Verdicts {
    fn verdicts(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Verdict>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn leave(&self, host: &str, verdict: Verdict) {
        self.verdicts().insert(host.to_owned(), verdict);
    }

    fn take(&self, host: &str) -> Option<Verdict> {
        self.verdicts().remove(host)
    }
}

/// The custom verification of every client of one transport: the engine's
/// decision on the chain each host presents, during the handshake. A
/// refused chain ends the handshake, its verdict left in `verdicts`.
fn custom_verify(
    decision: SharedTrust,
    examiner: Arc<Examiner>,
    verdicts: Verdicts,
) -> wreq::tls::CustomVerify {
    wreq::tls::CustomVerify::new(move |host, ssl| {
        let chain: Vec<Vec<u8>> = ssl
            .peer_cert_chain()
            .map(|chain| {
                chain
                    .iter()
                    .filter_map(|certificate| certificate.to_der().ok())
                    .collect()
            })
            .unwrap_or_default();
        let verdict = match examiner.examine(host, &chain) {
            // Its clients present no client certificate.
            Ok(presented) => decision.verdict(host, presented, false),
            Err(_) => Verdict::Rejected(CertificateFailure::Unrecognized(
                "unreadable_certificate".to_owned(),
            )),
        };
        if verdict == Verdict::Passes {
            return Ok(());
        }
        verdicts.leave(host, verdict);
        Err(wreq::tls::SslVerifyError::Invalid(
            wreq::tls::SslAlert::BAD_CERTIFICATE,
        ))
    })
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

/// `error`'s failure, as a hop to `url` reports it.
fn failure_of(error: &wreq::Error, url: &Url) -> TransportFailure {
    classify(error).of_hop(url)
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
            Some(http2::Reason::ENHANCE_YOUR_CALM) => Some(TransportFailure::EnhanceYourCalm),
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
        // BoringSSL names no failure here: the `wreq` transport's own
        // verification names each (spec #565).
        TransportFailure::CertificateInvalid(CertificateFailure::Unrecognized(
            CERTIFICATE_VERIFY_FAILED.to_ascii_lowercase(),
        ))
    } else {
        TransportFailure::Tls
    }
}

#[cfg(test)]
mod tests;
