//! The Proving ground's certificate authority, made from its seed, and the
//! TLS configuration that serves each host name it serves with a leaf
//! certificate of its own, signed on the first handshake that asks for it
//! (spec #152, ADR 0059).
//!
//! The seed gives the authority's key and certificate byte for byte: the
//! keys are derived from the seed with SHA-256 and every signature is
//! RFC 6979's deterministic ECDSA, so a client that trusts one seed's
//! authority trusts it on every run. This is ADR 0059's one seeded
//! authority, since it never guards real traffic. Leaf certificates share
//! one key from the seed but are dated from the clock, valid for 397 days
//! from the day before the authority is made, because some clients'
//! platforms refuse a longer-lived server certificate. Signing a leaf for
//! every host name at once would cost each start of `serve` one signature per
//! name, and most runs ask for few of them. One leaf per host name, never a
//! wildcard, so an HTTP/2 client never reuses one host's connection for
//! another and each connection's host is the name it asked for, except for
//! the one leaf that also names a host the fetch edge's `421` page needs a
//! client to coalesce.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result};
use p256::ecdsa::signature::Signer as _;
use p256::pkcs8::EncodePrivateKey as _;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyIdMethod, KeyUsagePurpose, PublicKeyData, SerialNumber, SignatureAlgorithm,
};
use rustls::crypto::ring;
use rustls::pki_types::{DnsName, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::danger::ClientCertVerifier;
use rustls::server::{ClientHello, ResolvesServerCert, WebPkiClientVerifier};
use rustls::sign::{CertifiedKey, SigningKey};
use rustls::{RootCertStore, ServerConfig};
use sha2::{Digest as _, Sha256};

use crate::certificates::{Defect, MISMATCHED_NAME};
use crate::raw_site;

/// How long a leaf certificate is valid, within the 398 days the strictest
/// client platforms accept.
const LEAF_VALIDITY: Duration = Duration::from_hours(397 * 24);
const LEAF_BACKDATE: Duration = Duration::from_hours(24);
/// How long after the authority is made the leaf of
/// `not-yet-valid.certificates.test` becomes valid.
const NOT_YET_VALID_FOR: Duration = Duration::from_hours(30 * 24);

/// The certificate authority of one seed and the TLS configuration of the
/// host names it serves.
pub(crate) struct Authority {
    certificate_pem: String,
    server_config: Arc<ServerConfig>,
    http1_server_config: Arc<ServerConfig>,
    /// The client certificate the seed's client authority signed, and its
    /// key, as PEM.
    client_certificate_pem: String,
    client_key_pem: String,
    client_certificate_server_config: Arc<ServerConfig>,
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the tests count the leaves signed")
    )]
    leaves: Arc<Leaves>,
}

impl Authority {
    /// Makes the seed's authority and readies a leaf certificate for each of
    /// `hosts`, dated from `now`.
    pub(crate) fn new<'a>(
        seed: u64,
        hosts: impl IntoIterator<Item = &'a str>,
        now: SystemTime,
    ) -> Result<Self> {
        let (certificate, issuer) = authority(
            seed,
            "certificate authority",
            &format!("Goodfetch Proving ground, seed {seed}"),
        )?;
        // Another authority from the seed, which signs the leaf of
        // `unknown-issuer.certificates.test` and is never published.
        let (_, unknown_issuer) = authority(
            seed,
            "unknown issuer",
            &format!("Goodfetch Proving ground unknown issuer, seed {seed}"),
        )?;

        let leaf_key = SeededKey::derive(seed, "leaf");
        let signing_key = ring::sign::any_ecdsa_type(&PrivateKeyDer::Pkcs8(
            PrivatePkcs8KeyDer::from(leaf_key.pkcs8()?),
        ))
        .context("loading the leaf key")?;
        let not_before = time::OffsetDateTime::from(now - LEAF_BACKDATE)
            .replace_nanosecond(0)
            .context("dating the leaf certificates")?;
        let mut by_name = HashMap::new();
        for host in hosts {
            let defect = Defect::of(host);
            let subject = match defect {
                Some(Defect::NameMismatch) => MISMATCHED_NAME,
                _ => host,
            };
            let names = std::iter::once(subject)
                .chain(raw_site::also_named(host).iter().copied())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut params = CertificateParams::new(names)
                .with_context(|| format!("naming the leaf certificate of {host}"))?;
            params.distinguished_name = name(subject);
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
            let valid_from = match defect {
                Some(Defect::Expired) => not_before - LEAF_VALIDITY,
                Some(Defect::NotYetValid) => not_before + NOT_YET_VALID_FOR,
                _ => not_before,
            };
            params.not_before = valid_from;
            params.not_after = valid_from + LEAF_VALIDITY;
            params.serial_number = Some(serial(seed, host));
            params.use_authority_key_identifier_extension = true;
            params.key_identifier_method = key_identifier(&leaf_key);
            // Keyed as rustls's `ResolvesServerCertUsingSni` keys a name, so
            // a client's SNI finds the same leaf it would there.
            let name = DnsName::try_from(host)
                .map(|name| name.to_lowercase_owned())
                .with_context(|| format!("serving the leaf certificate of {host}"))?;
            by_name.insert(
                name.as_ref().to_owned(),
                Leaf {
                    params,
                    signer: match defect {
                        Some(Defect::UnknownIssuer) => Signer::UnknownIssuer,
                        Some(Defect::SelfSigned) => Signer::Itself,
                        _ => Signer::Authority,
                    },
                    certified: OnceLock::new(),
                },
            );
        }
        let leaves = Arc::new(Leaves {
            issuer,
            unknown_issuer,
            key: leaf_key,
            signing_key,
            by_name,
        });
        let ClientIdentity {
            verifier: client_verifier,
            certificate_pem: client_certificate_pem,
            key_pem: client_key_pem,
        } = client_identity(seed)?;
        let config = |alpn: &[&[u8]],
                      clients: Option<Arc<dyn ClientCertVerifier>>|
         -> Result<Arc<ServerConfig>> {
            let builder = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
                .with_safe_default_protocol_versions()
                .context("choosing the TLS versions")?;
            let builder = match clients {
                Some(verifier) => builder.with_client_cert_verifier(verifier),
                None => builder.with_no_client_auth(),
            };
            let mut config = builder.with_cert_resolver(Arc::clone(&leaves) as Arc<_>);
            config.alpn_protocols = alpn.iter().map(|name| name.to_vec()).collect();
            Ok(Arc::new(config))
        };
        Ok(Self {
            certificate_pem: certificate.pem(),
            server_config: config(&[b"h2", b"http/1.1"], None)?,
            http1_server_config: config(&[b"http/1.1"], None)?,
            client_certificate_pem,
            client_key_pem,
            client_certificate_server_config: config(&[b"h2", b"http/1.1"], Some(client_verifier))?,
            leaves,
        })
    }

    /// The client certificate the seed's client authority signed, as PEM.
    pub(crate) fn client_certificate_pem(&self) -> &str {
        &self.client_certificate_pem
    }

    /// The private key of that client certificate, as PKCS #8 PEM.
    pub(crate) fn client_key_pem(&self) -> &str {
        &self.client_key_pem
    }

    /// The TLS configuration of the host names that require a client
    /// certificate the seed's client authority signed.
    pub(crate) fn client_certificate_server_config(&self) -> Arc<ServerConfig> {
        Arc::clone(&self.client_certificate_server_config)
    }

    /// The TLS configuration of the host names that offer HTTP/1.1 alone.
    pub(crate) fn http1_server_config(&self) -> Arc<ServerConfig> {
        Arc::clone(&self.http1_server_config)
    }

    /// The authority's certificate, as PEM.
    pub(crate) fn certificate_pem(&self) -> &str {
        &self.certificate_pem
    }

    /// The TLS configuration that serves every host name with its leaf.
    pub(crate) fn server_config(&self) -> Arc<ServerConfig> {
        Arc::clone(&self.server_config)
    }

    /// How many host names' leaves the handshakes so far have signed.
    #[cfg(test)]
    fn leaves_signed(&self) -> usize {
        self.leaves
            .by_name
            .values()
            .filter(|leaf| leaf.certified.get().is_some())
            .count()
    }
}

/// The leaf certificate of each host name, by its lowercased name.
struct Leaves {
    issuer: Issuer<'static, SeededKey>,
    unknown_issuer: Issuer<'static, SeededKey>,
    key: SeededKey,
    signing_key: Arc<dyn SigningKey>,
    by_name: HashMap<String, Leaf>,
}

/// A host name's leaf: its certificate's parameters, signed once.
struct Leaf {
    params: CertificateParams,
    signer: Signer,
    /// Set by the first handshake for the name; one that arrives while it is
    /// being signed waits for that signature rather than making its own.
    certified: OnceLock<Arc<CertifiedKey>>,
}

/// Who signs a leaf.
#[derive(Clone, Copy, Debug)]
enum Signer {
    /// The seed's authority, which the Proving ground publishes.
    Authority,
    /// The seed's other authority, which it never publishes.
    UnknownIssuer,
    /// The leaf's own key.
    Itself,
}

impl Leaves {
    fn certified(&self, name: &str) -> Option<Arc<CertifiedKey>> {
        let leaf = self.by_name.get(name)?;
        let certified = leaf.certified.get_or_init(|| {
            let signed = match leaf.signer {
                Signer::Authority => leaf.params.signed_by(&self.key, &self.issuer),
                Signer::UnknownIssuer => leaf.params.signed_by(&self.key, &self.unknown_issuer),
                Signer::Itself => leaf.params.self_signed(&self.key),
            };
            let certificate =
                signed.expect("invariant: a leaf whose names parsed signs with the seeded keys");
            Arc::new(CertifiedKey::new(
                vec![certificate.der().clone()],
                Arc::clone(&self.signing_key),
            ))
        });
        Some(Arc::clone(certified))
    }
}

impl ResolvesServerCert for Leaves {
    /// As rustls's `ResolvesServerCertUsingSni`: no SNI, or a name with no
    /// leaf, gets no certificate.
    fn resolve(&self, client_hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        self.certified(client_hello.server_name()?)
    }
}

impl fmt::Debug for Leaves {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Leaves")
            .field("host_names", &self.by_name.len())
            .finish_non_exhaustive()
    }
}

/// A certificate authority from the seed and `purpose`, named `common_name`,
/// valid from 2025 to 2125: its certificate and what it signs with.
fn authority(
    seed: u64,
    purpose: &str,
    common_name: &str,
) -> Result<(rcgen::Certificate, Issuer<'static, SeededKey>)> {
    let key = SeededKey::derive(seed, purpose);
    let mut params = CertificateParams::default();
    params.distinguished_name = name(common_name);
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    params.not_before = rcgen::date_time_ymd(2025, 1, 1);
    params.not_after = rcgen::date_time_ymd(2125, 1, 1);
    params.serial_number = Some(serial(seed, purpose));
    params.key_identifier_method = key_identifier(&key);
    let certificate = params
        .self_signed(&key)
        .with_context(|| format!("signing the {purpose}"))?;
    Ok((certificate, Issuer::new(params, key)))
}

/// The verifier of the seed's client authority, which the host names that
/// require mutual TLS check a client with, and the client certificate it
/// signed with its key, which a test hands its client.
struct ClientIdentity {
    verifier: Arc<dyn ClientCertVerifier>,
    certificate_pem: String,
    key_pem: String,
}

/// The client certificate of `seed`, byte for byte the same on every run:
/// valid from 2025 to 2125, for client authentication only.
fn client_identity(seed: u64) -> Result<ClientIdentity> {
    let (authority, issuer) = authority(
        seed,
        "client certificate authority",
        &format!("Goodfetch Proving ground client authority, seed {seed}"),
    )?;
    let key = SeededKey::derive(seed, "client certificate");
    let mut params = CertificateParams::default();
    params.distinguished_name = name(&format!("Goodfetch Proving ground client, seed {seed}"));
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    params.not_before = rcgen::date_time_ymd(2025, 1, 1);
    params.not_after = rcgen::date_time_ymd(2125, 1, 1);
    params.serial_number = Some(serial(seed, "client certificate"));
    params.use_authority_key_identifier_extension = true;
    params.key_identifier_method = key_identifier(&key);
    let certificate = params
        .signed_by(&key, &issuer)
        .context("signing the client certificate")?;
    let mut roots = RootCertStore::empty();
    roots
        .add(authority.der().clone())
        .context("trusting the client authority")?;
    let verifier = WebPkiClientVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(ring::default_provider()),
    )
    .build()
    .context("building the client certificate verifier")?;
    Ok(ClientIdentity {
        verifier,
        certificate_pem: certificate.pem(),
        key_pem: pem("PRIVATE KEY", &key.pkcs8()?),
    })
}

/// `der` as a PEM block labelled `label`.
fn pem(label: &str, der: &[u8]) -> String {
    let encoded = data_encoding::BASE64.encode(der);
    let mut text = format!("-----BEGIN {label}-----\n");
    for line in encoded.as_bytes().chunks(64) {
        text.push_str(std::str::from_utf8(line).expect("invariant: base64 is ASCII"));
        text.push('\n');
    }
    text.push_str("-----END ");
    text.push_str(label);
    text.push_str("-----\n");
    text
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
            .to_sec1_point(false)
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

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "concurrent handshakes run on threads of the test's own"
)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, SystemTime};

    use rstest::rstest;
    use rustls::client::ClientConnection;
    use rustls::crypto::ring;
    use rustls::pki_types::pem::PemObject as _;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
    use rustls::server::ServerConnection;
    use rustls::time_provider::TimeProvider;
    use rustls::{AlertDescription, CertificateError, ClientConfig, Connection, RootCertStore};

    use super::Authority;
    use crate::certificates::{CLIENT_CERTIFICATE_HOST, Defect};

    /// 2026-10-01T00:00:00Z, the authority's "now" and the client's clock.
    const NOW: Duration = Duration::from_hours(20_727 * 24);

    fn authority() -> Authority {
        Authority::new(
            7,
            ["catalog.test", "shop.test"],
            SystemTime::UNIX_EPOCH + NOW,
        )
        .expect("the authority is made")
    }

    #[derive(Debug)]
    struct AtNow;

    impl TimeProvider for AtNow {
        fn current_time(&self) -> Option<UnixTime> {
            Some(UnixTime::since_unix_epoch(NOW))
        }
    }

    /// Hands what `from` has to send to `to`, which reads it all.
    fn pass(from: &mut Connection, to: &mut Connection) -> Result<(), rustls::Error> {
        let mut bytes = Vec::new();
        while from.wants_write() {
            from.write_tls(&mut bytes)
                .expect("writing to memory succeeds");
        }
        let mut unread = bytes.as_slice();
        while !unread.is_empty() {
            to.read_tls(&mut unread)
                .expect("reading from memory succeeds");
            to.process_new_packets()?;
        }
        Ok(())
    }

    /// The leaf certificate a client that trusts the authority receives when
    /// it asks for `server_name`, through a TLS handshake in memory, or the
    /// error the client sees.
    fn handshake(
        authority: &Authority,
        server_name: &str,
    ) -> Result<CertificateDer<'static>, rustls::Error> {
        let certificate = CertificateDer::from_pem_slice(authority.certificate_pem().as_bytes())
            .expect("the authority's PEM parses");
        handshake_trusting(authority, &certificate, server_name)
    }

    /// As [`handshake`], for a client that trusts `anchor` alone.
    fn handshake_trusting(
        authority: &Authority,
        anchor: &CertificateDer<'static>,
        server_name: &str,
    ) -> Result<CertificateDer<'static>, rustls::Error> {
        let mut roots = RootCertStore::empty();
        roots
            .add(anchor.clone())
            .expect("the certificate is a trust anchor");
        let config =
            ClientConfig::builder_with_details(Arc::new(ring::default_provider()), Arc::new(AtNow))
                .with_safe_default_protocol_versions()
                .expect("the provider offers the default versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
        let server_name =
            ServerName::try_from(server_name.to_owned()).expect("the server name is valid");
        let mut client = Connection::Client(
            ClientConnection::new(Arc::new(config), server_name).expect("the client starts"),
        );
        let mut server = Connection::Server(
            ServerConnection::new(authority.server_config()).expect("the server starts"),
        );
        while client.is_handshaking() {
            // A refusing server's own error is not what the client sees; the
            // alert it sends is.
            let _refused = pass(&mut client, &mut server);
            assert!(server.wants_write(), "the server answers each flight");
            pass(&mut server, &mut client)?;
        }
        Ok(client
            .peer_certificates()
            .and_then(|chain| chain.first())
            .expect("the server sent a leaf")
            .clone()
            .into_owned())
    }

    /// The family's hosts and one host whose certificate is valid.
    fn authority_with_defects() -> Authority {
        let hosts = Defect::ALL
            .into_iter()
            .map(Defect::host)
            .chain(["catalog.test"]);
        Authority::new(7, hosts, SystemTime::UNIX_EPOCH + NOW).expect("the authority is made")
    }

    /// How a client that trusts the authority refuses `server_name`'s
    /// certificate, or `valid` when it accepts it.
    fn verdict(authority: &Authority, server_name: &str) -> &'static str {
        match handshake(authority, server_name) {
            Ok(_) => "valid",
            Err(rustls::Error::InvalidCertificate(error)) => match error {
                CertificateError::Expired | CertificateError::ExpiredContext { .. } => "expired",
                CertificateError::NotValidYet | CertificateError::NotValidYetContext { .. } => {
                    "not_yet_valid"
                }
                CertificateError::UnknownIssuer => "unknown_issuer",
                CertificateError::NotValidForName
                | CertificateError::NotValidForNameContext { .. } => "name_mismatch",
                other => panic!("an unexpected refusal of {server_name}: {other:?}"),
            },
            Err(other) => panic!("an unexpected failure for {server_name}: {other:?}"),
        }
    }

    /// The leaf the authority serves for `host`.
    fn served_leaf(authority: &Authority, host: &str) -> CertificateDer<'static> {
        authority
            .leaves
            .certified(host)
            .expect("the authority serves the host")
            .cert[0]
            .clone()
    }

    /// Pitfall H03's fixtures: each host's certificate fails in its one way,
    /// and the same authority's certificate for another host passes.
    #[rstest]
    #[case::expired(Defect::Expired, "expired")]
    #[case::not_yet_valid(Defect::NotYetValid, "not_yet_valid")]
    #[case::unknown_issuer(Defect::UnknownIssuer, "unknown_issuer")]
    #[case::name_mismatch(Defect::NameMismatch, "name_mismatch")]
    #[case::self_signed(Defect::SelfSigned, "unknown_issuer")]
    fn each_certificates_host_serves_a_leaf_that_fails_in_its_one_way(
        #[case] defect: Defect,
        #[case] refused_as: &str,
    ) {
        let authority = authority_with_defects();

        assert_eq!(
            (
                verdict(&authority, defect.host()),
                verdict(&authority, "catalog.test")
            ),
            (refused_as, "valid")
        );
    }

    /// A self-signed leaf is its own issuer: a client trusting the leaf
    /// itself accepts it, which it does not for an unknown issuer's leaf.
    #[rstest]
    #[case::self_signed(Defect::SelfSigned, true)]
    #[case::unknown_issuer(Defect::UnknownIssuer, false)]
    fn only_the_self_signed_leaf_is_its_own_issuer(#[case] defect: Defect, #[case] own: bool) {
        let authority = authority_with_defects();
        let leaf = served_leaf(&authority, defect.host());

        assert_eq!(
            handshake_trusting(&authority, &leaf, defect.host()).is_ok(),
            own
        );
    }

    #[test]
    fn making_the_authority_signs_no_leaf_certificate() {
        let authority = authority();

        assert_eq!(authority.leaves_signed(), 0);
    }

    #[test]
    fn a_host_names_leaf_is_signed_on_its_first_handshake_and_served_again() {
        let authority = authority();

        let leaves = (
            handshake(&authority, "catalog.test").expect("the first handshake succeeds"),
            handshake(&authority, "catalog.test").expect("the second handshake succeeds"),
        );

        assert_eq!(leaves.0, leaves.1);
        assert_eq!(authority.leaves_signed(), 1);
    }

    #[test]
    fn concurrent_first_handshakes_for_one_host_name_sign_its_leaf_once() {
        let authority = authority();

        let leaves = std::thread::scope(|scope| {
            let handshakes = [(); 4].map(|()| scope.spawn(|| handshake(&authority, "shop.test")));
            handshakes.map(|handshake| {
                handshake
                    .join()
                    .expect("no handshake panics")
                    .expect("each handshake succeeds")
            })
        });

        assert_eq!(authority.leaves_signed(), 1);
        assert!(
            leaves.iter().all(|leaf| *leaf == leaves[0]),
            "one leaf is served"
        );
    }

    /// The client certificate and key `authority` hands a client.
    fn client_identity(
        authority: &Authority,
    ) -> (Vec<CertificateDer<'static>>, PrivateKeyDer<'static>) {
        let chain = CertificateDer::pem_slice_iter(authority.client_certificate_pem().as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .expect("the client certificate's PEM parses");
        let key = PrivateKeyDer::from_pem_slice(authority.client_key_pem().as_bytes())
            .expect("the client key's PEM parses");
        (chain, key)
    }

    /// A handshake in memory with the host that requires a client
    /// certificate, by a client that trusts the authority and presents
    /// `identity`, if any: the server's verdict.
    fn mutual_handshake(
        authority: &Authority,
        identity: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
    ) -> Result<(), rustls::Error> {
        let mut roots = RootCertStore::empty();
        roots
            .add(
                CertificateDer::from_pem_slice(authority.certificate_pem().as_bytes())
                    .expect("the authority's PEM parses"),
            )
            .expect("the authority is a trust anchor");
        let builder =
            ClientConfig::builder_with_details(Arc::new(ring::default_provider()), Arc::new(AtNow))
                .with_safe_default_protocol_versions()
                .expect("the provider offers the default versions")
                .with_root_certificates(roots);
        let config = match identity {
            Some((chain, key)) => builder
                .with_client_auth_cert(chain, key)
                .expect("the client key signs"),
            None => builder.with_no_client_auth(),
        };
        let server_name = ServerName::try_from(CLIENT_CERTIFICATE_HOST.to_owned())
            .expect("the server name is valid");
        let mut client = Connection::Client(
            ClientConnection::new(Arc::new(config), server_name).expect("the client starts"),
        );
        let mut server = Connection::Server(
            ServerConnection::new(authority.client_certificate_server_config())
                .expect("the server starts"),
        );
        while server.is_handshaking() {
            pass(&mut client, &mut server)?;
            pass(&mut server, &mut client).expect("the client accepts the server's leaf");
        }
        Ok(())
    }

    /// The host that requires a client certificate refuses a client that
    /// presents none, or one another seed's client authority signed, and
    /// accepts the one its own seed's client authority signed.
    #[test]
    fn the_client_certificate_host_takes_only_its_seeds_client_certificate() {
        let at = SystemTime::UNIX_EPOCH + NOW;
        let authority =
            Authority::new(7, [CLIENT_CERTIFICATE_HOST], at).expect("the authority is made");
        let another_seed =
            Authority::new(8, [CLIENT_CERTIFICATE_HOST], at).expect("the authority is made");

        let none = mutual_handshake(&authority, None);
        let foreign = mutual_handshake(&authority, Some(client_identity(&another_seed)));
        let own = mutual_handshake(&authority, Some(client_identity(&authority)));

        assert!(
            matches!(none, Err(rustls::Error::NoCertificatesPresented)),
            "{none:?}"
        );
        assert!(
            matches!(
                foreign,
                Err(rustls::Error::InvalidCertificate(
                    CertificateError::UnknownIssuer
                ))
            ),
            "{foreign:?}"
        );
        assert_eq!(own, Ok(()));
    }

    #[rstest]
    #[case::a_host_name_it_does_not_serve("unknown.test")]
    #[case::no_host_name("127.0.0.1")]
    fn a_handshake_for_no_served_host_name_is_refused_and_signs_nothing(#[case] server_name: &str) {
        let authority = authority();

        let refused = handshake(&authority, server_name);

        assert!(
            matches!(
                refused,
                Err(rustls::Error::AlertReceived(AlertDescription::AccessDenied))
            ),
            "{refused:?}"
        );
        assert_eq!(authority.leaves_signed(), 0);
    }
}
