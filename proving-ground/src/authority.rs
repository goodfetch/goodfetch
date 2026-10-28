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
use rustls::ServerConfig;
use rustls::crypto::ring;
use rustls::pki_types::{DnsName, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::{CertifiedKey, SigningKey};
use sha2::{Digest as _, Sha256};

use crate::raw_site;

/// How long a leaf certificate is valid, within the 398 days the strictest
/// client platforms accept.
const LEAF_VALIDITY: Duration = Duration::from_hours(397 * 24);
const LEAF_BACKDATE: Duration = Duration::from_hours(24);

/// The certificate authority of one seed and the TLS configuration of the
/// host names it serves.
pub(crate) struct Authority {
    certificate_pem: String,
    server_config: Arc<ServerConfig>,
    http1_server_config: Arc<ServerConfig>,
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
        let mut by_name = HashMap::new();
        for host in hosts {
            let names = std::iter::once(host)
                .chain(raw_site::also_named(host).iter().copied())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            let mut params = CertificateParams::new(names)
                .with_context(|| format!("naming the leaf certificate of {host}"))?;
            params.distinguished_name = name(host);
            params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
            params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
            params.not_before = not_before;
            params.not_after = not_before + LEAF_VALIDITY;
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
                    certified: OnceLock::new(),
                },
            );
        }
        let leaves = Arc::new(Leaves {
            issuer,
            key: leaf_key,
            signing_key,
            by_name,
        });
        let config = |alpn: &[&[u8]]| -> Result<Arc<ServerConfig>> {
            let mut config =
                ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
                    .with_safe_default_protocol_versions()
                    .context("choosing the TLS versions")?
                    .with_no_client_auth()
                    .with_cert_resolver(Arc::clone(&leaves) as Arc<_>);
            config.alpn_protocols = alpn.iter().map(|name| name.to_vec()).collect();
            Ok(Arc::new(config))
        };
        Ok(Self {
            certificate_pem: certificate.pem(),
            server_config: config(&[b"h2", b"http/1.1"])?,
            http1_server_config: config(&[b"http/1.1"])?,
            leaves,
        })
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
    key: SeededKey,
    signing_key: Arc<dyn SigningKey>,
    by_name: HashMap<String, Leaf>,
}

/// A host name's leaf: its certificate's parameters, signed once.
struct Leaf {
    params: CertificateParams,
    /// Set by the first handshake for the name; one that arrives while it is
    /// being signed waits for that signature rather than making its own.
    certified: OnceLock<Arc<CertifiedKey>>,
}

impl Leaves {
    fn certified(&self, name: &str) -> Option<Arc<CertifiedKey>> {
        let leaf = self.by_name.get(name)?;
        let certified = leaf.certified.get_or_init(|| {
            let certificate = leaf
                .params
                .signed_by(&self.key, &self.issuer)
                .expect("invariant: a leaf whose names parsed signs with the seeded keys");
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
    use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
    use rustls::server::ServerConnection;
    use rustls::time_provider::TimeProvider;
    use rustls::{AlertDescription, ClientConfig, Connection, RootCertStore};

    use super::Authority;

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
        let mut roots = RootCertStore::empty();
        roots
            .add(
                CertificateDer::from_pem_slice(authority.certificate_pem().as_bytes())
                    .expect("the authority's PEM parses"),
            )
            .expect("the authority is a trust anchor");
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
