//! The relay's certificate authority (ADR 0042): made in memory when a
//! Run's first browser launches with a certificate exception, never written
//! anywhere, and name-constrained to the domains the Run lists, so its key,
//! were it ever read, could not forge a certificate another site's client
//! accepts. Chromium trusts it by the SHA-256 of its public key
//! (`--ignore-certificate-errors-spki-list`); the relay presents each
//! listed host a leaf it signs, made once per host.

#![expect(
    clippy::disallowed_types,
    reason = "the authority's leaves are made one at a time, never across another lock or an `.await` (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};

use data_encoding::BASE64;
use goodfetch_engine::known_certificates::KeyHash;
use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair, KeyUsagePurpose,
    NameConstraints, PublicKeyData as _,
};

/// The authority, its key held only here.
pub(crate) struct Authority {
    key: KeyPair,
    params: CertificateParams,
    certificate: Vec<u8>,
    spki_sha256: String,
    domains: Vec<String>,
    leaves: Mutex<BTreeMap<String, Arc<Leaf>>>,
}

/// A host's leaf: its chain, leaf first, then the authority, and its key
/// in PKCS #8.
pub(crate) struct Leaf {
    pub(crate) chain: [Vec<u8>; 2],
    pub(crate) key: Vec<u8>,
}

/// Why the authority or a leaf could not be made.
#[derive(Debug, thiserror::Error)]
#[error("the relay's certificate authority could not sign: {0}")]
pub(crate) struct Unsigned(#[from] rcgen::Error);

impl std::fmt::Debug for Authority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Authority")
            .field("spki_sha256", &self.spki_sha256)
            .field("domains", &self.domains)
            .finish_non_exhaustive()
    }
}

impl Authority {
    /// A new authority constrained to `domains`, each a host or a domain
    /// with its subdomains as the URL parser leaves it; an address is
    /// permitted as itself alone.
    ///
    /// # Errors
    ///
    /// [`Unsigned`] when the key or the certificate cannot be made.
    pub(crate) fn new(domains: Vec<String>) -> Result<Self, Unsigned> {
        let key = KeyPair::generate()?;
        let mut params = CertificateParams::new(Vec::<String>::new())?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, "Goodfetch egress relay");
        params.distinguished_name = name;
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.name_constraints = Some(NameConstraints {
            permitted_subtrees: domains.iter().map(|domain| subtree(domain)).collect(),
            excluded_subtrees: Vec::new(),
        });
        let certificate = params.self_signed(&key)?.der().to_vec();
        let spki_sha256 =
            BASE64.encode(KeyHash::of_public_key(&key.subject_public_key_info()).stored());
        Ok(Self {
            key,
            params,
            certificate,
            spki_sha256,
            domains,
            leaves: Mutex::new(BTreeMap::new()),
        })
    }

    /// The SHA-256 of its public key in base64, as Chromium's
    /// `--ignore-certificate-errors-spki-list` takes it.
    pub(crate) fn spki_sha256(&self) -> &str {
        &self.spki_sha256
    }

    /// The leaf it signs for `host`, made on first use. It signs for any
    /// host, but a host outside its domains fails every verifier that
    /// reads its name constraints.
    ///
    /// # Errors
    ///
    /// [`Unsigned`] when the leaf cannot be made.
    pub(crate) fn leaf(&self, host: &str) -> Result<Arc<Leaf>, Unsigned> {
        let mut leaves = self.leaves.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(leaf) = leaves.get(host) {
            return Ok(Arc::clone(leaf));
        }
        let key = KeyPair::generate()?;
        let mut params = CertificateParams::new(vec![host.to_owned()])?;
        let mut name = DistinguishedName::new();
        name.push(DnType::CommonName, host);
        params.distinguished_name = name;
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let issuer = Issuer::from_params(&self.params, &self.key);
        let certificate = params.signed_by(&key, &issuer)?;
        let leaf = Arc::new(Leaf {
            chain: [certificate.der().to_vec(), self.certificate.clone()],
            key: key.serialize_der(),
        });
        leaves.insert(host.to_owned(), Arc::clone(&leaf));
        Ok(leaf)
    }
}

/// The permitted subtree of `domain`: an address alone, or a name with
/// its subdomains.
fn subtree(domain: &str) -> GeneralSubtree {
    match domain.parse::<IpAddr>() {
        Ok(address @ IpAddr::V4(_)) => {
            GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(address, 32))
        }
        Ok(address @ IpAddr::V6(_)) => {
            GeneralSubtree::IpAddress(CidrSubnet::from_addr_prefix(address, 128))
        }
        Err(_) => GeneralSubtree::DnsName(domain.to_owned()),
    }
}
