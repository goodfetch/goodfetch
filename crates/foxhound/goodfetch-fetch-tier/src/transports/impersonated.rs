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
//! **Client certificates:** BoringSSL takes one client identity per
//! client and picks it without the server name, so a host a listed domain
//! of `tls.client_certificates` covers goes out on clients of its own,
//! built for its trust scope and that domain when a host of the domain is
//! first reached, and presenting that domain's certificate (spec #563
//! story 34; step 7 review, finding 563-565-3). They present the same
//! profile, so a site cannot tell them from the others. A host reached
//! through its exception is sent no client certificate, as on reqwest.
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
use goodfetch_engine::run::secret_error;
use goodfetch_engine::tls::{
    ClientCertificateFailure, ClientCertificates, Decision, Examiner, IdentityFiles, Presented,
    SharedTrust, Verdict,
};
use goodfetch_protocol::tls::CertificateFailure;
use http_body_util::BodyExt as _;
use secrecy::ExposeSecret as _;
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
    /// The client certificates of `tls.client_certificates`.
    client_certificates: Option<ClientCertificates>,
    /// What builds its clients, kept for those of each listed domain of
    /// `tls.client_certificates`.
    builder: Arc<Builder>,
    /// Its clients presenting a listed domain's client certificate, by the
    /// trust scope and the listed domain of the hosts they serve, built
    /// when a host of the domain is first reached.
    presenting: Presenting,
}

/// What builds a [`Wreq`]'s clients: every one presents the same profile,
/// resolves, binds and goes through the proxy alike, and differs only in
/// the authorities it trusts and the client certificate it presents.
struct Builder {
    profile: HttpProfile,
    resolver: Arc<Resolver>,
    local: Option<super::LocalSource>,
    proxy: Option<wreq::Proxy>,
    /// The proxy's URL, as a failure of the proxy's own names it.
    through: Option<Url>,
    decision: SharedTrust,
    examiner: Arc<Examiner>,
    verdicts: Verdicts,
}

/// A trust scope's clients: offering HTTP/2 and HTTP/1.1, and HTTP/1.1
/// alone for the origins that asked.
struct ScopeClients {
    any: Clients,
    http1: Clients,
}

/// The clients presenting a client certificate, by trust scope and listed
/// domain. Clones share them.
#[derive(Clone, Default)]
#[expect(
    clippy::disallowed_types,
    reason = "the map is read or written in one step, never across an `.await` or another lock: a client is built outside it and the first one built is kept, so there is no lock order to test under loom (ADR 0060)"
)]
struct Presenting(Arc<std::sync::Mutex<PresentingClients>>);

/// The clients presenting a client certificate, by what they serve.
type PresentingClients = BTreeMap<Presents, Arc<ScopeClients>>;

/// What a set of presenting clients serves: the hosts of one trust scope
/// (`None` for the hosts no `tls.trusted_cas` domain covers) that one
/// listed domain of `tls.client_certificates` covers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Presents {
    trust_scope: Option<String>,
    listed: String,
}

impl Presenting {
    fn clients(&self) -> std::sync::MutexGuard<'_, PresentingClients> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
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
        let resolver = Arc::new(Resolver::new(&config));
        let refused = |error: wreq::Error| TransportUnavailable::Client(error.to_string());
        let proxy = config
            .proxy
            .as_ref()
            .map(|route| {
                super::through(route, wreq::Proxy::all, wreq::Proxy::basic_auth).map_err(refused)
            })
            .transpose()?;
        let decision = SharedTrust::new(
            config.trust.clone(),
            config.exceptions.clone(),
            config.clock,
        );
        let verdicts = Verdicts::default();
        let builder = Arc::new(Builder {
            profile,
            resolver: Arc::clone(&resolver),
            local: config.local.clone(),
            proxy,
            through: config.proxy.as_ref().and_then(super::proxy_named),
            decision: decision.clone(),
            examiner: Arc::new(config.trust.examiner(config.clock)),
            verdicts: verdicts.clone(),
        });
        let mut any = Scopes::new();
        let mut http1 = Scopes::new();
        for scope in config.trust.scopes() {
            let clients = builder
                .scope_clients(&scope.authorities, None)
                .map_err(refused)?;
            any.insert(scope.domain.clone(), clients.any);
            http1.insert(scope.domain, clients.http1);
        }
        Ok(Self {
            any,
            http1,
            http1_origins: Http1Origins::default(),
            trust: config.trust,
            decision,
            verdicts,
            resolver,
            client_certificates: config.client_certificates,
            builder,
            presenting: Presenting::default(),
        })
    }

    /// The address check of its connections, run ahead of a request.
    pub fn address_check(&self) -> AddressCheck {
        AddressCheck(Arc::clone(&self.resolver))
    }

    /// The clients of `host`'s trust scope that present the client
    /// certificate of its listed domain of `tls.client_certificates`, built
    /// when a host of the domain is first reached: none when no listed
    /// domain covers it. Every client presents the same profile, so a site
    /// cannot tell which one served it.
    async fn presenting_for(
        &self,
        host: &str,
    ) -> Result<Option<Arc<ScopeClients>>, TransportFailure> {
        let Some(certificates) = &self.client_certificates else {
            return Ok(None);
        };
        let Some(domain) = certificates.scope_of(host) else {
            return Ok(None);
        };
        let key = Presents {
            trust_scope: self.trust.scope_of(host).map(str::to_owned),
            listed: domain,
        };
        if let Some(clients) = self.presenting.clients().get(&key) {
            return Ok(Some(Arc::clone(clients)));
        }
        let Some(files) = certificates
            .files_for_host(host)
            .await
            .map_err(|failure| super::client_certificate_failure(&failure))?
        else {
            return Ok(None);
        };
        let unusable = |reason: String| {
            super::client_certificate_failure(&ClientCertificateFailure::Unusable {
                domain: files.domain.clone(),
                layer: certificates.layer(),
                reason,
            })
        };
        let identity = identity_of(&files).map_err(unusable)?;
        let authorities = self
            .trust
            .scopes()
            .into_iter()
            .find(|scope| scope.domain == key.trust_scope)
            .map(|scope| scope.authorities)
            .unwrap_or_default();
        let built = Arc::new(
            self.builder
                .scope_clients(&authorities, Some(&identity))
                .map_err(|_| TransportFailure::Other)?,
        );
        Ok(Some(Arc::clone(
            self.presenting.clients().entry(key).or_insert(built),
        )))
    }

    /// The client `hop` goes out on, as its host's trust scope and `reuse`
    /// allow: one of `presenting`, the clients that present its host's
    /// client certificate, when it has one.
    fn client<'c>(
        &'c self,
        hop: &Hop,
        reuse: Reuse,
        presenting: Option<&'c ScopeClients>,
    ) -> Option<&'c wreq::Client> {
        let http1_only = self.http1_origins.contains(&hop.url);
        let clients = match presenting {
            Some(presenting) if http1_only => &presenting.http1,
            Some(presenting) => &presenting.any,
            None => {
                let scopes = if http1_only { &self.http1 } else { &self.any };
                let scope = hop
                    .url
                    .host_str()
                    .and_then(|host| self.trust.scope_of(host))
                    .map(str::to_owned);
                scopes.get(&scope)?
            }
        };
        Some(match reuse {
            Reuse::Allowed => &clients.pooled,
            Reuse::Never => &clients.new,
        })
    }
}

/// The client identity `files` hold, as BoringSSL takes it: the key in
/// PKCS #8, whichever PEM form its reference gave.
fn identity_of(files: &IdentityFiles) -> Result<wreq::tls::trust::Identity, String> {
    let key = btls::pkey::PKey::private_key_from_pem(files.key.expose_secret().as_bytes())
        .and_then(|key| key.private_key_to_pem_pkcs8())
        .map_err(|_| "the value of its key's reference is no PEM private key".to_owned())?;
    let key = secrecy::zeroize::Zeroizing::new(key);
    wreq::tls::trust::Identity::from_pkcs8_pem(&files.certificate, &key)
        .map_err(|_| "its key is not the key of its certificate".to_owned())
}

impl Builder {
    /// The clients of a trust scope whose authorities, beside Mozilla's
    /// roots, are `authorities`, presenting `identity` when a server asks
    /// for a client certificate.
    fn scope_clients(
        &self,
        authorities: &[rustls::pki_types::CertificateDer<'static>],
        identity: Option<&wreq::tls::trust::Identity>,
    ) -> wreq::Result<ScopeClients> {
        let clients = |http1_only: bool| -> wreq::Result<Clients> {
            Ok(Clients {
                pooled: self.client(authorities, identity, http1_only, true)?,
                new: self.client(authorities, identity, http1_only, false)?,
            })
        };
        Ok(ScopeClients {
            any: clients(false)?,
            http1: clients(true)?,
        })
    }

    /// One client: offering HTTP/1.1 alone when `http1_only`, and keeping
    /// its connections when `pooled`.
    fn client(
        &self,
        authorities: &[rustls::pki_types::CertificateDer<'static>],
        identity: Option<&wreq::tls::trust::Identity>,
        http1_only: bool,
        pooled: bool,
    ) -> wreq::Result<wreq::Client> {
        let store = wreq::tls::trust::CertStore::builder()
            .add_der_certs(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter())
            .add_der_certs(authorities.iter())
            .build()?;
        let verify = custom_verify(
            self.decision.clone(),
            Arc::clone(&self.examiner),
            self.verdicts.clone(),
            identity.is_some(),
        );
        let builder = wreq::Client::builder()
            .emulation(self.profile.emulation())
            // A folded header line is read as a space, as Chrome reads it.
            .http1_options(
                wreq::http1::Http1Options::builder()
                    .allow_obsolete_multiline_headers_in_responses(true)
                    .build(),
            )
            .tls_cert_store(store)
            .tls_custom_verify(verify)
            .dns_resolver(Arc::clone(&self.resolver) as Arc<dyn wreq::dns::Resolve>)
            .redirect(wreq::redirect::Policy::none())
            .retry(wreq::retry::Policy::never())
            .connector_layer(MarkConnects);
        let builder = match identity {
            Some(identity) => builder.tls_identity(identity.clone()),
            None => builder,
        };
        let builder = match &self.local {
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
        let builder = match &self.proxy {
            Some(proxy) => builder.proxy(proxy.clone()),
            None => builder.no_proxy(),
        };
        build(builder)
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
    fn proxy(&self) -> Option<Url> {
        self.builder.through.clone()
    }

    fn send<'a>(&'a self, hop: &'a Hop, reuse: Reuse) -> Sending<'a, Sent> {
        Box::pin(async move {
            self.resolver.before_sending(&hop.url).await?;
            let sent = match hop.url.host_str() {
                Some(host) if hop.url.scheme() == "https" => {
                    self.send_to(&tls_host(host), hop, reuse).await
                }
                _ => self
                    .send_once(hop, reuse)
                    .await
                    .map_err(|(failure, _)| failure),
            };
            if let Err(failure) = &sent {
                self.resolver.forget_unreached(&hop.url, failure).await;
            }
            sent
        })
    }
}

impl Wreq {
    /// Sends `hop` over TLS to `host`, its certificate decided per host
    /// during each handshake: sent again on a new connection each time its
    /// handshake stopped for the exception's decision, which
    /// [`SharedTrust::decide_presented`] then took. A host a listed domain
    /// of `tls.client_certificates` covers goes out on the clients that
    /// present its certificate.
    async fn send_to(&self, host: &str, hop: &Hop, reuse: Reuse) -> Result<Sent, TransportFailure> {
        self.decision
            .before(host)
            .await
            .map_err(TransportFailure::CertificateChanged)?;
        let presenting = self.presenting_for(host).await?;
        for _ in 0..DECISIONS {
            let presented = match self.send_on(presenting.as_deref(), hop, reuse).await {
                Ok(mut sent) => {
                    sent.certificate_exception = self.decision.exception(host);
                    return Ok(sent);
                }
                Err((failure, None)) => return Err(failure),
                Err((_, Some(presented))) => presented,
            };
            match self
                .decision
                .decide_presented(host, *presented, presenting.is_some())
                .await
            {
                Decision::Again => {}
                Decision::Changed(changed) => {
                    return Err(TransportFailure::CertificateChanged(changed));
                }
                // A client certificate never goes to a host reached
                // through its exception.
                Decision::Excepted => {
                    return Err(self
                        .client_certificates
                        .as_ref()
                        .and_then(|certificates| certificates.refused(host))
                        .map_or(TransportFailure::Tls, |refused| {
                            TransportFailure::ClientCertificate(CallFailure(secret_error(&refused)))
                        }));
                }
            }
        }
        Err(TransportFailure::Tls)
    }

    /// Sends `hop` once, on its host's clients that present no client
    /// certificate.
    async fn send_once(
        &self,
        hop: &Hop,
        reuse: Reuse,
    ) -> Result<Sent, (TransportFailure, Option<Box<Presented>>)> {
        self.send_on(None, hop, reuse).await
    }

    /// Sends `hop` once, on one of `presenting` when its host has clients
    /// that present a client certificate: its answer, or its failure and,
    /// when its handshake stopped for its host's exception to decide, the
    /// certificate the host presented.
    async fn send_on(
        &self,
        presenting: Option<&ScopeClients>,
        hop: &Hop,
        reuse: Reuse,
    ) -> Result<Sent, (TransportFailure, Option<Box<Presented>>)> {
        let host = hop.url.host_str().map(tls_host).unwrap_or_default();
        let undecided = |failure| (failure, None);
        {
            // Every scope has its clients, so a host always finds one.
            let client = self
                .client(hop, reuse, presenting)
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
    presents: bool,
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
            Ok(presented) => decision.verdict(host, presented, presents),
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
