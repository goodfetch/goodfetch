//! Certificate verification (ADRs 0016 and 0042): one per-host decision
//! for every certificate the engine's own HTTP client meets, taken in a
//! rustls verifier. A host's certificate must chain to Mozilla's roots or
//! to the certificate authorities trusted for its domain: those
//! `tls.trusted_cas` lists for an address a page can reach, or an
//! Operator-configured endpoint's own. Nothing turns verification off.
//!
//! A domain `tls.accept_invalid_certs` lists for trust on first use takes
//! [`exceptions`]' decision instead, once the handshake shows the key its
//! host presents: the pin, never the roots, decides from its first pin on.
//!
//! A domain `tls.client_certificates` lists presents its certificate when
//! its host asks for one ([`client_certificates`]). The handshake then goes
//! on past the server's certificate only when that certificate verifies and
//! the host has no pin, so the client certificate is never sent to a host
//! reached through an exception: a pinned host presenting another key is
//! stopped before it is sent.

mod client_certificates;
mod exceptions;
mod shared;

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

pub use client_certificates::{ClientCertificateFailure, ClientCertificates, IdentityFiles};
pub use exceptions::{
    CertificateChanged, CertificateExceptions, ExceptionUsed, PinBook, Presented,
};
pub use shared::{Decision, SharedTrust, Verdict, certificate_failure};

use goodfetch_protocol::tls::{
    CERTIFICATE_INVALID, CertificateFailure, CertificateInvalidDetails, KnownCertificateFailure,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{WebPkiServerVerifier, verify_server_cert_signed_by_trust_anchor};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::{self, PemObject as _};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::time_provider::TimeProvider;
use rustls::{CertificateError, DigitallySignedStruct, OtherError, RootCertStore, SignatureScheme};
use url::Host;

use crate::call_error::CallError;
use crate::clock::Clock;
use crate::known_certificates::KeyHash;

/// What a host's certificate may chain to beside Mozilla's roots.
#[derive(Clone, Debug, Default)]
pub struct Trust {
    /// Trusted for every host: an Operator-configured endpoint's own
    /// authorities, which it reaches alone.
    everywhere: Vec<CertificateDer<'static>>,
    /// Trusted for a domain and its subdomains, by the domain as the URL
    /// parser leaves a host.
    by_domain: BTreeMap<String, Vec<CertificateDer<'static>>>,
}

/// A host's certificate that failed verification, with no exception for
/// its domain: the request ends before anything is sent to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateInvalid {
    /// The host, as the request named it.
    pub host: String,
    /// How its certificate failed.
    pub failure: CertificateFailure,
}

impl CertificateInvalid {
    /// What `certificate_invalid` details: the host, the failure and the
    /// setting that makes an exception for its domain.
    pub fn details(&self) -> CertificateInvalidDetails {
        let host = &self.host;
        let fix = match self.failure {
            CertificateFailure::Known(KnownCertificateFailure::UnknownIssuer) => format!(
                "Trust the certificate authority of {host} for its domain under `tls.trusted_cas` in user or project configuration or the Operator policy, or list its domain under `tls.accept_invalid_certs` to pin its certificate on first use."
            ),
            _ => format!(
                "List the domain of {host} under `tls.accept_invalid_certs` to pin its certificate on first use, or give its pin there in user or project configuration or the Operator policy."
            ),
        };
        CertificateInvalidDetails {
            host: host.clone(),
            failure: self.failure.clone(),
            fix,
        }
    }

    /// The failure the call that waited for the request answers with.
    pub fn error(&self) -> CallError {
        CallError::new(&CERTIFICATE_INVALID, self.to_string(), &self.details())
    }
}

impl std::fmt::Display for CertificateInvalid {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "the certificate of {} failed verification ({})",
            self.host,
            self.failure.as_str()
        )
    }
}

impl std::error::Error for CertificateInvalid {}

/// One scope of a [`Trust`]: the hosts a listed domain covers, or those
/// none covers, and the authorities trusted for them beside Mozilla's
/// roots.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustScope {
    /// The listed domain, `None` for the hosts no listed domain covers.
    pub domain: Option<String>,
    /// The authorities trusted there beside Mozilla's roots: those trusted
    /// everywhere, then the domain's own, then its parents', the most
    /// specific first.
    pub authorities: Vec<CertificateDer<'static>>,
}

/// A `tls.trusted_cas` entry that names no host.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{domain}` names no host or domain")]
pub struct InvalidDomain {
    /// The entry as written.
    pub domain: String,
}

impl Trust {
    /// Mozilla's roots alone.
    pub fn default_roots() -> Self {
        Self::default()
    }

    /// An Operator-configured endpoint's own certificate authorities,
    /// trusted beside Mozilla's roots for the endpoint, never a site.
    pub(crate) fn endpoint(authorities: &[CertificateDer<'static>]) -> Self {
        Self {
            everywhere: authorities.to_vec(),
            by_domain: BTreeMap::new(),
        }
    }

    /// Trusts `authorities` for `domain`, a host or a registrable domain
    /// with its subdomains, as an entry of `tls.trusted_cas` does.
    ///
    /// # Errors
    ///
    /// [`InvalidDomain`] when `domain` names no host.
    pub fn with_trusted_cas(
        mut self,
        domain: &str,
        authorities: Vec<CertificateDer<'static>>,
    ) -> Result<Self, InvalidDomain> {
        let key = domain_key(domain).ok_or_else(|| InvalidDomain {
            domain: domain.to_owned(),
        })?;
        self.by_domain.entry(key).or_default().extend(authorities);
        Ok(self)
    }

    /// The domains `host` is, or is a subdomain of, that list
    /// authorities: the most specific first.
    fn domains_of(&self, host: &str) -> Vec<&str> {
        let Some(mut name) = domain_key(host) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        if name.parse::<std::net::IpAddr>().is_ok() {
            // An address is listed only as itself.
            found.extend(
                self.by_domain
                    .get_key_value(name.as_str())
                    .map(|(key, _)| key.as_str()),
            );
            return found;
        }
        loop {
            if let Some((domain, _)) = self.by_domain.get_key_value(name.as_str()) {
                found.push(domain.as_str());
            }
            match name.split_once('.') {
                Some((_, parent)) => name = parent.to_owned(),
                None => return found,
            }
        }
    }

    /// The rustls configuration of a connection under this trust, offering
    /// HTTP/1.1 alone in ALPN; a transport that speaks HTTP/2 offers it
    /// itself (Foxhound's HTTP fetch tier). The handshake judges the
    /// certificate's dates at `clock`'s wall time.
    pub fn client_config(&self, clock: Clock) -> rustls::ClientConfig {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Verifier::new(self, &provider);
        let mut config =
            rustls::ClientConfig::builder_with_details(provider, OnClock::shared(clock))
                .with_safe_default_protocol_versions()
                .expect("invariant: ring supports rustls's default protocol versions")
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config
    }

    /// Each scope of this trust, for a TLS library that takes one store of
    /// authorities per client rather than a decision per host: BoringSSL
    /// in Foxhound's `wreq` transport, which keeps one client per scope
    /// and sends each host to [`Trust::scope_of`]'s (ADR 0042). The scope
    /// of no domain comes first, then each listed domain in order.
    pub fn scopes(&self) -> Vec<TrustScope> {
        let everywhere = TrustScope {
            domain: None,
            authorities: self.everywhere.clone(),
        };
        let listed = self.by_domain.keys().map(|domain| TrustScope {
            domain: Some(domain.clone()),
            authorities: self
                .everywhere
                .iter()
                .chain(
                    self.domains_of(domain)
                        .into_iter()
                        .flat_map(|listed| &self.by_domain[listed]),
                )
                .cloned()
                .collect(),
        });
        std::iter::once(everywhere).chain(listed).collect()
    }

    /// The listed domain whose scope `host` is in: the most specific that
    /// `host` is, or is a subdomain of; `None` for the scope of no domain.
    pub fn scope_of(&self, host: &str) -> Option<&str> {
        self.domains_of(host).first().copied()
    }
}

/// What `setting`, a resolved value of `tls.trusted_cas`, trusts beside
/// Mozilla's roots: each listed file read now on `owner`'s blocking work;
/// a file that cannot be read, or an entry that names no host, is logged
/// and trusts nothing.
///
/// # Cancel safety
///
/// Dropping the future loses only the answer.
pub async fn trusted_cas(setting: &serde_json::Value, owner: &crate::tasks::TaskOwner) -> Trust {
    let listed: Vec<(String, Vec<std::path::PathBuf>)> = setting
        .as_object()
        .into_iter()
        .flatten()
        .map(|(domain, files)| {
            let files = files
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|file| file.as_str().map(std::path::PathBuf::from))
                .collect();
            (domain.clone(), files)
        })
        .collect();
    let read = owner
        .blocking(move || {
            listed
                .into_iter()
                .map(|(domain, files)| {
                    (
                        domain,
                        files
                            .iter()
                            .filter_map(|file| authorities_in(file))
                            .flatten()
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .await;
    read.into_iter().fold(
        Trust::default_roots(),
        |trust, (domain, authorities)| match trust.clone().with_trusted_cas(&domain, authorities) {
            Ok(with) => with,
            Err(error) => {
                tracing::warn!(%error, "an entry of `tls.trusted_cas` names no host");
                trust
            }
        },
    )
}

/// The authorities `file` holds; none, logged, when it cannot be read.
fn authorities_in(file: &std::path::Path) -> Option<Vec<CertificateDer<'static>>> {
    #[expect(
        clippy::disallowed_methods,
        reason = "it runs on the tasks module's blocking policy"
    )]
    let bytes = std::fs::read(file);
    let read = bytes
        .map_err(|error| error.to_string())
        .and_then(|bytes| read_certificates(&bytes).map_err(|error| error.to_string()));
    match read {
        Ok(read) => Some(read),
        Err(error) => {
            tracing::warn!(%error, file = %file.display(), "a certificate authority of `tls.trusted_cas` cannot be read");
            None
        }
    }
}

/// The per-host decision for a TLS library other than rustls (ADR 0124):
/// the egress relay's BoringSSL hands each certificate chain it receives
/// to [`Examiner::examine`], which decides as the engine's own verifier
/// does, so one implementation decides for both.
#[derive(Debug)]
pub struct Examiner {
    verifier: Verifier,
    /// Whose wall time a certificate's dates are judged at.
    clock: Clock,
}

/// rustls's time source on the engine's [`Clock`] (ADR 0059): a handshake
/// judges a certificate's dates at the Clock's wall time, so a test's paused
/// Clock drives it.
#[derive(Debug)]
pub(crate) struct OnClock(Clock);

impl OnClock {
    pub(crate) fn shared(clock: Clock) -> Arc<dyn TimeProvider> {
        Arc::new(Self(clock))
    }
}

impl TimeProvider for OnClock {
    fn current_time(&self) -> Option<UnixTime> {
        Some(unix_time(&self.0))
    }
}

/// `clock`'s wall time as rustls reads it; a time before the epoch, which
/// no OS clock gives, reads as the epoch.
fn unix_time(clock: &Clock) -> UnixTime {
    UnixTime::since_unix_epoch(
        clock
            .wall()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

/// A certificate chain an [`Examiner`] could not read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the certificate chain of `{host}` cannot be read")]
pub struct Unreadable {
    /// The host that presented it.
    pub host: String,
}

impl Trust {
    /// The decision of this trust for chains received outside rustls,
    /// judging a certificate's dates at `clock`'s wall time.
    pub fn examiner(&self, clock: Clock) -> Examiner {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        Examiner {
            verifier: Verifier::new(self, &provider),
            clock,
        }
    }
}

impl Examiner {
    /// What `chain`, the DER certificates `host` presented, leaf first,
    /// shows: the key of its leaf, and how it fails verification now, if
    /// it does.
    ///
    /// # Errors
    ///
    /// [`Unreadable`] when there is no leaf, or it does not parse.
    pub fn examine(&self, host: &str, chain: &[Vec<u8>]) -> Result<Presented, Unreadable> {
        let unreadable = || Unreadable {
            host: host.to_owned(),
        };
        let (leaf, rest) = chain.split_first().ok_or_else(unreadable)?;
        let end_entity = CertificateDer::from(leaf.as_slice());
        let intermediates: Vec<CertificateDer<'_>> = rest
            .iter()
            .map(|der| CertificateDer::from(der.as_slice()))
            .collect();
        let parsed = ParsedCertificate::try_from(&end_entity).map_err(|_| unreadable())?;
        let key = KeyHash::of_public_key(parsed.subject_public_key_info().as_ref());
        let name = domain_key(host).unwrap_or_else(|| host.to_owned());
        let failure = match ServerName::try_from(name) {
            Ok(server_name) => self
                .verifier
                .failure(
                    &end_entity,
                    &intermediates,
                    &server_name,
                    &[],
                    unix_time(&self.clock),
                )
                .map_err(|_| unreadable())?,
            Err(_) => Some(CertificateFailure::Known(
                KnownCertificateFailure::NameMismatch,
            )),
        };
        Ok(Presented { key, failure })
    }
}

/// A host or domain as the URL parser leaves it, in lower case and
/// punycode, without the trailing dot that makes it fully qualified; an
/// IPv6 address without brackets.
fn domain_key(domain: &str) -> Option<String> {
    let parsed = if domain.contains(':') && !domain.starts_with('[') {
        Host::parse(&format!("[{domain}]")).ok()?
    } else {
        Host::parse(domain).ok()?
    };
    let key = match parsed {
        Host::Domain(name) => name.strip_suffix('.').unwrap_or(&name).to_owned(),
        Host::Ipv4(address) => address.to_string(),
        Host::Ipv6(address) => address.to_string(),
    };
    (!key.is_empty()).then_some(key)
}

/// `host` as the known-certificates store keys it: as the URL parser leaves
/// a host, without a trailing dot; none when it names no host.
pub(crate) fn host_key(host: &str) -> Option<String> {
    domain_key(host)
}

/// Whether `host` is, or is a subdomain of, a domain `listed` holds, each
/// as [`domain_key`] leaves it; an address is listed only as itself.
fn covered(host: &str, listed: impl Fn(&str) -> bool) -> bool {
    let Some(mut name) = domain_key(host) else {
        return false;
    };
    if name.parse::<std::net::IpAddr>().is_ok() {
        return listed(&name);
    }
    loop {
        if listed(&name) {
            return true;
        }
        match name.split_once('.') {
            Some((_, parent)) => name = parent.to_owned(),
            None => return false,
        }
    }
}

/// The per-host decision: a host's certificate is checked against
/// Mozilla's roots and the authorities trusted for it, by the verifier
/// built for its most specific listed domain.
#[derive(Debug)]
struct Verifier {
    /// For a host no listed domain covers.
    default: Arc<WebPkiServerVerifier>,
    /// For each listed domain: its authorities and those of every listed
    /// domain it is a subdomain of.
    by_domain: BTreeMap<String, Arc<WebPkiServerVerifier>>,
    trust: Trust,
    provider: Arc<CryptoProvider>,
}

/// How a certificate failed, carried from the verifier through rustls's
/// handshake error to the request that waited for it.
#[derive(Debug)]
struct Rejected(CertificateFailure);

impl std::fmt::Display for Rejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0.as_str())
    }
}

impl std::error::Error for Rejected {}

/// The failure a handshake that ended with `error` met on `host`'s
/// certificate; `None` when the handshake failed for another reason.
pub(crate) fn certificate_invalid(error: &io::Error, host: &str) -> Option<CertificateInvalid> {
    let rustls::Error::InvalidCertificate(CertificateError::Other(OtherError(other))) =
        error.get_ref()?.downcast_ref::<rustls::Error>()?
    else {
        return None;
    };
    let Rejected(failure) = other.downcast_ref::<Rejected>()?;
    Some(CertificateInvalid {
        host: host.to_owned(),
        failure: failure.clone(),
    })
}

/// The wire's name for how rustls's verifier refused a certificate
/// (ADR 0127's translation point): the failures the wire names, and any
/// other as its own name in text.
fn failure_of(error: &CertificateError) -> CertificateFailure {
    let known = |failure| CertificateFailure::Known(failure);
    let text = |name: &str| CertificateFailure::Unrecognized(name.to_owned());
    match error {
        CertificateError::Expired | CertificateError::ExpiredContext { .. } => {
            known(KnownCertificateFailure::Expired)
        }
        CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
            known(KnownCertificateFailure::NotYetValid)
        }
        CertificateError::UnknownIssuer => known(KnownCertificateFailure::UnknownIssuer),
        CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. } => {
            known(KnownCertificateFailure::NameMismatch)
        }
        CertificateError::Revoked => known(KnownCertificateFailure::Revoked),
        CertificateError::BadEncoding => text("bad_encoding"),
        CertificateError::BadSignature => text("bad_signature"),
        CertificateError::UnhandledCriticalExtension => text("unhandled_critical_extension"),
        CertificateError::InvalidPurpose | CertificateError::InvalidPurposeContext { .. } => {
            text("invalid_purpose")
        }
        CertificateError::UnknownRevocationStatus => text("unknown_revocation_status"),
        CertificateError::ExpiredRevocationList
        | CertificateError::ExpiredRevocationListContext { .. } => text("expired_revocation_list"),
        CertificateError::UnsupportedSignatureAlgorithmContext { .. }
        | CertificateError::UnsupportedSignatureAlgorithmForPublicKeyContext { .. } => {
            text("unsupported_signature_algorithm")
        }
        CertificateError::InvalidOcspResponse => text("invalid_ocsp_response"),
        CertificateError::ApplicationVerificationFailure => {
            text("application_verification_failure")
        }
        CertificateError::Other(other) => text(&other.to_string()),
        _ => text("invalid_certificate"),
    }
}

/// Whether `end_entity` is its own issuer: trusted as an authority of its
/// own, it chains, whatever its dates.
fn self_signed(end_entity: &CertificateDer<'_>, now: UnixTime, provider: &CryptoProvider) -> bool {
    let Ok(parsed) = ParsedCertificate::try_from(end_entity) else {
        return false;
    };
    let mut itself = RootCertStore::empty();
    if itself.add(end_entity.clone().into_owned()).is_err() {
        return false;
    }
    match verify_server_cert_signed_by_trust_anchor(
        &parsed,
        &itself,
        &[],
        now,
        provider.signature_verification_algorithms.all,
    ) {
        Ok(()) => true,
        Err(rustls::Error::InvalidCertificate(error)) => matches!(
            error,
            CertificateError::Expired
                | CertificateError::ExpiredContext { .. }
                | CertificateError::NotValidYet
                | CertificateError::NotValidYetContext { .. }
        ),
        Err(_) => false,
    }
}

impl Verifier {
    fn new(trust: &Trust, provider: &Arc<CryptoProvider>) -> Self {
        let build = |extra: &[&CertificateDer<'static>]| {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            for certificate in trust.everywhere.iter().chain(extra.iter().copied()) {
                // A certificate that is no CA certificate trusts nothing;
                // it is the setting's to refuse, not the request's.
                let _ = roots.add(certificate.clone());
            }
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::clone(provider))
                .build()
                .expect("invariant: Mozilla's roots make a verifier")
        };
        let by_domain = trust
            .by_domain
            .keys()
            .map(|domain| {
                let extra: Vec<&CertificateDer<'static>> = trust
                    .domains_of(domain)
                    .into_iter()
                    .flat_map(|listed| &trust.by_domain[listed])
                    .collect();
                (domain.clone(), build(&extra))
            })
            .collect();
        Self {
            default: build(&[]),
            by_domain,
            trust: trust.clone(),
            provider: Arc::clone(provider),
        }
    }

    /// How `end_entity`, presented with `intermediates` by `server_name`,
    /// fails verification: none when it verifies.
    fn failure(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<Option<CertificateFailure>, rustls::Error> {
        match self.for_server(server_name).verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(_) => Ok(None),
            Err(rustls::Error::InvalidCertificate(error)) => Ok(Some(
                if matches!(error, CertificateError::UnknownIssuer)
                    && self_signed(end_entity, now, &self.provider)
                {
                    CertificateFailure::Known(KnownCertificateFailure::SelfSigned)
                } else {
                    failure_of(&error)
                },
            )),
            Err(other) => Err(other),
        }
    }

    /// The verifier for `server_name`.
    fn for_server(&self, server_name: &ServerName<'_>) -> &WebPkiServerVerifier {
        let host = server_name.to_str();
        self.trust
            .domains_of(&host)
            .first()
            .and_then(|domain| self.by_domain.get(*domain))
            .unwrap_or(&self.default)
    }
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.failure(end_entity, intermediates, server_name, ocsp_response, now)? {
            None => Ok(ServerCertVerified::assertion()),
            Some(failure) => Err(rustls::Error::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(Rejected(failure))),
            ))),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.default
            .verify_tls12_signature(message, certificate, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.default
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.default.supported_verify_schemes()
    }
}

/// Why a certificate file reads as no certificate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CertificateFileError {
    /// It holds no certificate, in PEM or DER.
    #[error("the file holds no certificate, in PEM or DER")]
    NoCertificate,
    /// A certificate it holds does not parse.
    #[error("a certificate in the file does not parse")]
    Malformed,
}

/// Why a key file reads as no private key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum KeyFileError {
    /// It holds no private key, in PEM or DER.
    #[error("the file holds no private key, in PEM or DER")]
    NoKey,
    /// The key it holds does not parse.
    #[error("the private key in the file does not parse")]
    Malformed,
}

/// The certificates of a certificate file a person named, such as an
/// authority of `tls.trusted_cas`: every `CERTIFICATE` block of a PEM
/// file in order, or the one certificate of a DER file.
///
/// # Errors
///
/// [`CertificateFileError`] when it holds no certificate, or one that does
/// not parse.
pub fn read_certificates(
    file: &[u8],
) -> Result<Vec<CertificateDer<'static>>, CertificateFileError> {
    let certificates = if is_pem(file) {
        CertificateDer::pem_slice_iter(file)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| CertificateFileError::Malformed)?
    } else if file.is_empty() {
        Vec::new()
    } else {
        let certificate = CertificateDer::from(file.to_vec());
        if ParsedCertificate::try_from(&certificate).is_err() {
            return Err(CertificateFileError::NoCertificate);
        }
        vec![certificate]
    };
    if certificates.is_empty() {
        return Err(CertificateFileError::NoCertificate);
    }
    if certificates
        .iter()
        .any(|certificate| ParsedCertificate::try_from(certificate).is_err())
    {
        return Err(CertificateFileError::Malformed);
    }
    Ok(certificates)
}

/// The private key of a key file a person named, such as a client
/// certificate's: the first private key of a PEM file (PKCS #8, PKCS #1 or
/// SEC 1), or a DER file's.
///
/// # Errors
///
/// [`KeyFileError`] when it holds no private key, or one that does not
/// parse.
pub fn read_private_key(file: &[u8]) -> Result<PrivateKeyDer<'static>, KeyFileError> {
    let key = if is_pem(file) {
        PrivateKeyDer::from_pem_slice(file).map_err(|error| match error {
            pem::Error::NoItemsFound => KeyFileError::NoKey,
            _ => KeyFileError::Malformed,
        })?
    } else {
        PrivateKeyDer::try_from(file.to_vec()).map_err(|_| KeyFileError::NoKey)?
    };
    rustls::crypto::ring::default_provider()
        .key_provider
        .load_private_key(key.clone_key())
        .map_err(|_| KeyFileError::Malformed)?;
    Ok(key)
}

/// Whether `file` holds a PEM block.
fn is_pem(file: &[u8]) -> bool {
    file.windows(PEM_BEGIN.len())
        .any(|window| window == PEM_BEGIN)
}

const PEM_BEGIN: &[u8] = b"-----BEGIN ";

/// The fuzz target `certificate_file`'s body (ADR 0067): any bytes read as
/// a certificate file and as a key file, which a person named in a setting,
/// and as a pin row's key bytes in the known-certificates store. Each
/// certificate read reads back from its own bytes as itself, its pin from
/// its key bytes; it panics only on a bug.
pub fn fuzz_certificate_file(data: &[u8]) {
    if let Ok(certificates) = read_certificates(data) {
        for certificate in &certificates {
            assert_eq!(
                read_certificates(certificate).as_deref(),
                Ok(std::slice::from_ref(certificate)),
                "a certificate read reads back as another"
            );
            if let Ok(parsed) = ParsedCertificate::try_from(certificate) {
                let key = KeyHash::of_public_key(parsed.subject_public_key_info().as_ref());
                assert_eq!(
                    KeyHash::from_stored(key.stored()),
                    Some(key),
                    "a certificate's pin does not read back from its row's key bytes"
                );
            }
        }
    }
    // The same bytes as a pin row's key bytes in the known-certificates
    // store, which a damaged disk or a newer minor may hold: a key read
    // stores as those very bytes, and any other length reads as no key.
    match KeyHash::from_stored(data) {
        Some(key) => assert_eq!(
            key.stored(),
            data,
            "a pin row's key bytes read as another key"
        ),
        None => assert_ne!(data.len(), 32, "32 key bytes read as no key"),
    }
    if let Ok(key) = read_private_key(data) {
        assert!(
            read_private_key(key.secret_der()).is_ok(),
            "a key read does not read back from its own bytes"
        );
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod certificate_file {
    goodfetch_test_support::fuzz_replay!(
        crate::tls::fuzz_certificate_file,
        "fuzz/certificate_file"
    );
}
