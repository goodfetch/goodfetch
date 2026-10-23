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

mod exceptions;

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, OnceLock};

pub(crate) use exceptions::Presented;
pub use exceptions::{
    CertificateChanged, CertificateExceptions, ExceptionKind, ExceptionUsed, PinBook,
};

use goodfetch_protocol::tls::{
    CERTIFICATE_INVALID, CertificateFailure, CertificateInvalidDetails, KnownCertificateFailure,
};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{WebPkiServerVerifier, verify_server_cert_signed_by_trust_anchor};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::{self, PemObject as _};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::server::ParsedCertificate;
use rustls::{CertificateError, DigitallySignedStruct, OtherError, RootCertStore, SignatureScheme};
use url::Host;

use crate::composition::CallError;
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

    /// The rustls configuration of a connection under this trust.
    pub(crate) fn client_config(&self) -> rustls::ClientConfig {
        self.config(None)
    }

    /// The rustls configuration of a connection to a host listed for trust
    /// on first use: its handshake goes on whatever its certificate, and
    /// `seen` keeps the key it presented and how it failed verification,
    /// for [`CertificateExceptions`] to decide before anything is sent. The
    /// handshake's signature is still checked against that key.
    pub(crate) fn client_config_presenting(&self, seen: Arc<Seen>) -> rustls::ClientConfig {
        self.config(Some(seen))
    }

    fn config(&self, seen: Option<Arc<Seen>>) -> rustls::ClientConfig {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Verifier::new(self, &provider, seen);
        let mut config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("invariant: ring supports rustls's default protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        config
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

/// What one connection's handshake showed of a listed host's certificate,
/// kept by its verifier.
#[derive(Debug, Default)]
pub(crate) struct Seen(OnceLock<Presented>);

impl Seen {
    /// What the handshake showed; none before it reached the certificate.
    pub(crate) fn take(&self) -> Option<Presented> {
        self.0.get().cloned()
    }

    /// Keeps what the handshake showed: a connection verifies one
    /// certificate, so a second is never kept.
    fn keep(&self, presented: Presented) {
        let _ = self.0.set(presented);
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
    /// For a host listed for trust on first use: where the key it presents
    /// is kept, its certificate let through to the exception's decision.
    seen: Option<Arc<Seen>>,
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
    fn new(trust: &Trust, provider: &Arc<CryptoProvider>, seen: Option<Arc<Seen>>) -> Self {
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
            seen,
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
        let failure = match self.for_server(server_name).verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(verified) if self.seen.is_none() => return Ok(verified),
            Ok(_) => None,
            Err(rustls::Error::InvalidCertificate(error)) => Some(
                if matches!(error, CertificateError::UnknownIssuer)
                    && self_signed(end_entity, now, &self.provider)
                {
                    CertificateFailure::Known(KnownCertificateFailure::SelfSigned)
                } else {
                    failure_of(&error)
                },
            ),
            Err(other) => return Err(other),
        };
        let Some(seen) = &self.seen else {
            let failure = failure.expect("invariant: a verified certificate returned above");
            return Err(rustls::Error::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(Rejected(failure))),
            )));
        };
        let parsed = ParsedCertificate::try_from(end_entity)?;
        seen.keep(Presented {
            key: KeyHash::of_public_key(parsed.subject_public_key_info().as_ref()),
            failure,
        });
        Ok(ServerCertVerified::assertion())
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
