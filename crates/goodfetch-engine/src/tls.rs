//! Certificate verification (ADRs 0016 and 0042): one per-host decision
//! for every certificate the engine's own HTTP client meets, taken in a
//! rustls verifier. A host's certificate must chain to Mozilla's roots or
//! to the certificate authorities trusted for its domain: those
//! `tls.trusted_cas` lists for an address a page can reach, or an
//! Operator-configured endpoint's own. Nothing turns verification off.

use std::collections::BTreeMap;
use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};
use url::Host;

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
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = Verifier::new(self, &provider);
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
        self.for_server(server_name).verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        )
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
