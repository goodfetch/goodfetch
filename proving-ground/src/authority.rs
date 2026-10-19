//! The Proving ground's certificate authority, made from its seed, and the
//! TLS configuration that serves each host name it serves with a leaf
//! certificate of its own (spec #152, ADR 0059).
//!
//! The seed gives the authority's key and certificate byte for byte: the
//! keys are derived from the seed with SHA-256 and every signature is
//! RFC 6979's deterministic ECDSA, so a client that trusts one seed's
//! authority trusts it on every run. This is ADR 0059's one seeded
//! authority, since it never guards real traffic. Leaf certificates share
//! one key from the seed but are dated from the clock, valid for 397 days
//! from the day before they are made, because some clients' platforms refuse
//! a longer-lived server certificate. One leaf per host name, never a
//! wildcard, so an HTTP/2 client never reuses one host's connection for
//! another and each connection's host is the name it asked for.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use p256::ecdsa::signature::Signer as _;
use p256::pkcs8::EncodePrivateKey as _;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyIdMethod, KeyUsagePurpose, PublicKeyData, SerialNumber, SignatureAlgorithm,
};
use rustls::ServerConfig;
use rustls::crypto::ring;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::ResolvesServerCertUsingSni;
use rustls::sign::CertifiedKey;
use sha2::{Digest as _, Sha256};

/// How long a leaf certificate is valid, within the 398 days the strictest
/// client platforms accept.
const LEAF_VALIDITY: Duration = Duration::from_hours(397 * 24);
const LEAF_BACKDATE: Duration = Duration::from_hours(24);

/// The certificate authority of one seed and the TLS configuration of the
/// host names it serves.
pub(crate) struct Authority {
    certificate_pem: String,
    server_config: Arc<ServerConfig>,
}

impl Authority {
    /// Makes the seed's authority and a leaf certificate for each of
    /// `hosts`, dated from `now`.
    pub(crate) fn new<'a>(
        seed: u64,
        hosts: impl IntoIterator<Item = &'a str>,
        now: SystemTime,
    ) -> Result<Self> {
        let authority_key = SeededKey::derive(seed, "certificate authority");
        let mut params = CertificateParams::default();
        params.distinguished_name = name(&format!("Goodfetch Proving ground, seed {seed}"));
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.not_before = rcgen::date_time_ymd(2025, 1, 1);
        params.not_after = rcgen::date_time_ymd(2125, 1, 1);
        params.serial_number = Some(serial(seed, "certificate authority"));
        params.key_identifier_method = key_identifier(&authority_key);
        let certificate = params
            .self_signed(&authority_key)
            .context("signing the certificate authority")?;
        let issuer = Issuer::new(params, authority_key);

        let leaf_key = SeededKey::derive(seed, "leaf");
        let signing_key = ring::sign::any_ecdsa_type(&PrivateKeyDer::Pkcs8(
            PrivatePkcs8KeyDer::from(leaf_key.pkcs8()?),
        ))
        .context("loading the leaf key")?;
        let not_before = time::OffsetDateTime::from(now - LEAF_BACKDATE)
            .replace_nanosecond(0)
            .context("dating the leaf certificates")?;
        let mut resolver = ResolvesServerCertUsingSni::new();
        for host in hosts {
            let mut params = CertificateParams::new(vec![host.to_owned()])
                .with_context(|| format!("naming the leaf certificate of {host}"))?;
            params.distinguished_name = name(host);
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
            params.not_before = not_before;
            params.not_after = not_before + LEAF_VALIDITY;
            params.serial_number = Some(serial(seed, host));
            params.use_authority_key_identifier_extension = true;
            params.key_identifier_method = key_identifier(&leaf_key);
            let leaf = params
                .signed_by(&leaf_key, &issuer)
                .with_context(|| format!("signing the leaf certificate of {host}"))?;
            resolver
                .add(
                    host,
                    CertifiedKey::new(vec![leaf.der().clone()], Arc::clone(&signing_key)),
                )
                .with_context(|| format!("serving the leaf certificate of {host}"))?;
        }
        let mut server_config =
            ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
                .with_safe_default_protocol_versions()
                .context("choosing the TLS versions")?
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(resolver));
        server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
        Ok(Self {
            certificate_pem: certificate.pem(),
            server_config: Arc::new(server_config),
        })
    }

    /// The authority's certificate, as PEM.
    pub(crate) fn certificate_pem(&self) -> &str {
        &self.certificate_pem
    }

    /// The TLS configuration that serves every host name with its leaf.
    pub(crate) fn server_config(&self) -> Arc<ServerConfig> {
        Arc::clone(&self.server_config)
    }
}

fn name(common_name: &str) -> DistinguishedName {
    let mut name = DistinguishedName::new();
    name.push(DnType::CommonName, common_name);
    name
}

/// A positive 16-byte serial number from the seed and what it numbers.
fn serial(seed: u64, of: &str) -> SerialNumber {
    let mut bytes = derive(seed, &format!("serial of {of}"), 0);
    bytes[0] &= 0x7f;
    SerialNumber::from_slice(&bytes[..16])
}

/// RFC 5280's first method: the SHA-1-sized prefix of the key's hash, here
/// SHA-256 truncated to 20 bytes as rcgen's own method does.
fn key_identifier(key: &SeededKey) -> KeyIdMethod {
    KeyIdMethod::PreSpecified(Sha256::digest(key.subject_public_key_info())[..20].to_vec())
}

/// SHA-256 over the seed, a purpose and a counter.
fn derive(seed: u64, purpose: &str, counter: u32) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"goodfetch proving ground\0");
    hash.update(purpose.as_bytes());
    hash.update(b"\0");
    hash.update(seed.to_be_bytes());
    hash.update(counter.to_be_bytes());
    hash.finalize().into()
}

/// A P-256 key derived from the seed, which signs with RFC 6979's
/// deterministic ECDSA.
struct SeededKey {
    signing: p256::ecdsa::SigningKey,
    /// The uncompressed public point, as a certificate carries it.
    public: Vec<u8>,
}

impl SeededKey {
    fn derive(seed: u64, purpose: &str) -> Self {
        // A digest that is zero or not below the curve's order is no scalar;
        // the next counter is tried, which happens with probability 2^-32.
        let signing = (0..)
            .find_map(|counter| {
                p256::ecdsa::SigningKey::from_slice(&derive(seed, purpose, counter)).ok()
            })
            .expect("invariant: some counter gives a valid scalar");
        let public = signing
            .verifying_key()
            .to_encoded_point(false)
            .as_bytes()
            .to_vec();
        Self { signing, public }
    }

    /// The private key as PKCS #8, for rustls.
    fn pkcs8(&self) -> Result<Vec<u8>> {
        Ok(p256::SecretKey::from(self.signing.as_nonzero_scalar())
            .to_pkcs8_der()
            .context("encoding the leaf key")?
            .as_bytes()
            .to_vec())
    }
}

impl PublicKeyData for SeededKey {
    fn der_bytes(&self) -> &[u8] {
        &self.public
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &rcgen::PKCS_ECDSA_P256_SHA256
    }
}

impl rcgen::SigningKey for SeededKey {
    fn sign(&self, message: &[u8]) -> Result<Vec<u8>, rcgen::Error> {
        let signature: p256::ecdsa::Signature = self.signing.sign(message);
        Ok(signature.to_der().as_bytes().to_vec())
    }
}
