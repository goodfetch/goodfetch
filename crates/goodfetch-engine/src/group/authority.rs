//! A Daemon group's certificate authority (ADR 0055): `group init` creates
//! it on the Lead Daemon, which signs each Group member's certificate for
//! the peer link's mutual TLS and the revocation list that removes one.
//!
//! Its key, every Group member's key and every serial number come from the
//! OS generator, never from `Entropy` (ADR 0059): rcgen makes the P-256
//! keys through ring, which reads the OS generator. A Group member makes its
//! own key and sends only its public key, so no private key crosses the
//! peer link. Dates are readings of the engine's Clock, passed in.

use std::time::{Duration, SystemTime};

use rcgen::{
    BasicConstraints, CertificateParams, CertificateRevocationListParams, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyIdMethod, KeyPair, KeyUsagePurpose,
    PKCS_ECDSA_P256_SHA256, PublicKeyData, RevokedCertParams, SerialNumber, SignatureAlgorithm,
};
use rustls::pki_types::{
    CertificateDer, CertificateRevocationListDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName,
};
use time::OffsetDateTime;

/// How long the group's certificate authority is valid: 87,600 hours, 10
/// years **(shape)**, Consul's built-in authority's default `root_cert_ttl`.
pub const AUTHORITY_LIFETIME: Duration = Duration::from_hours(87_600);

/// How long a Group member's certificate is valid: 72 hours **(shape)**,
/// Consul's default `leaf_cert_ttl`. A Group member renews it before then.
pub const CERTIFICATE_LIFETIME: Duration = Duration::from_hours(72);

/// How long before its signing a certificate or revocation list is valid
/// from, so a Group member whose clock runs a little behind takes it: one
/// minute **(shape)**.
const BACKDATE: Duration = Duration::from_secs(60);

/// The DNS suffix of a Group member's name in its certificate. The
/// reserved `.invalid` top-level domain (RFC 6761) resolves nowhere, so the
/// name stands for the Group member only within its group.
const NAME_SUFFIX: &str = "group.goodfetch.invalid";

/// How many bytes of the OS generator a serial number or a group's
/// identifier takes: 128 bits.
const RANDOM_BYTES: usize = 16;

/// Why the group's certificate authority could not do what it was asked.
#[derive(Debug, thiserror::Error)]
pub enum AuthorityError {
    /// The OS generator gave nothing.
    #[error("the OS generator gave no random bytes: {0}")]
    NoRandomness(#[from] getrandom::Error),
    /// rcgen could not make the key, the certificate or the list.
    #[error("the certificate cannot be made: {0}")]
    Certificate(#[from] rcgen::Error),
}

/// A Group member's name: one DNS label of lowercase letters, digits and
/// hyphens, such as a pod's name, which its certificate carries and the
/// Group member connecting to it checks.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct GroupMemberName(String);

/// A Group member's name that is no DNS label.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{0}` cannot name a Group member: use 1 to 63 lowercase letters, digits or hyphens, \
     starting and ending with a letter or digit"
)]
pub struct GroupMemberNameInvalid(pub String);

impl GroupMemberName {
    /// `name`, when it is one DNS label (RFC 1123) in lowercase.
    ///
    /// # Errors
    ///
    /// [`GroupMemberNameInvalid`] otherwise.
    pub fn new(name: &str) -> Result<Self, GroupMemberNameInvalid> {
        let allowed = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
        let bytes = name.as_bytes();
        let label = (1..=63).contains(&bytes.len())
            && bytes.first().copied().is_some_and(allowed)
            && bytes.last().copied().is_some_and(allowed)
            && bytes.iter().all(|&byte| allowed(byte) || byte == b'-');
        if label {
            Ok(Self(name.to_owned()))
        } else {
            Err(GroupMemberNameInvalid(name.to_owned()))
        }
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name a Group member connecting to this one expects its
    /// certificate to carry.
    pub fn server_name(&self) -> ServerName<'static> {
        ServerName::try_from(self.dns_name())
            .expect("invariant: a DNS label under the group's suffix is a DNS name")
    }

    fn dns_name(&self) -> String {
        format!("{}.{NAME_SUFFIX}", self.0)
    }
}

/// A Group member's own key pair, P-256, made from the OS generator.
pub struct GroupMemberKey(KeyPair);

impl std::fmt::Debug for GroupMemberKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GroupMemberKey")
            .finish_non_exhaustive()
    }
}

impl GroupMemberKey {
    /// A new key pair.
    ///
    /// # Errors
    ///
    /// [`AuthorityError::Certificate`] when the OS generator gives nothing.
    pub fn generate() -> Result<Self, AuthorityError> {
        Ok(Self(KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?))
    }

    /// Its public key, which the Group member sends the Lead Daemon to sign.
    pub fn public_key(&self) -> GroupMemberPublicKey {
        GroupMemberPublicKey(self.0.public_key_raw().to_vec())
    }

    /// Its private key, for the peer link's TLS; it never leaves the Group
    /// member.
    pub fn private_key(&self) -> PrivateKeyDer<'static> {
        PrivatePkcs8KeyDer::from(self.0.serialize_der()).into()
    }
}

/// A Group member's P-256 public key, as an uncompressed point (SEC 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMemberPublicKey(Vec<u8>);

/// Public key bytes that are no uncompressed P-256 point.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("a Group member's public key must be an uncompressed P-256 point of 65 bytes")]
pub struct GroupMemberPublicKeyInvalid;

impl GroupMemberPublicKey {
    /// The key whose uncompressed point is `raw`. Only its form is checked:
    /// a certificate signed for a point off the curve serves no handshake.
    ///
    /// # Errors
    ///
    /// [`GroupMemberPublicKeyInvalid`] when `raw` is not 65 bytes starting
    /// with `0x04`.
    pub fn from_raw(raw: Vec<u8>) -> Result<Self, GroupMemberPublicKeyInvalid> {
        if raw.len() == 65 && raw.first() == Some(&0x04) {
            Ok(Self(raw))
        } else {
            Err(GroupMemberPublicKeyInvalid)
        }
    }

    /// The uncompressed point.
    pub fn raw(&self) -> &[u8] {
        &self.0
    }
}

impl PublicKeyData for GroupMemberPublicKey {
    fn der_bytes(&self) -> &[u8] {
        &self.0
    }

    fn algorithm(&self) -> &'static SignatureAlgorithm {
        &PKCS_ECDSA_P256_SHA256
    }
}

/// A certificate the group's certificate authority signed for a Group
/// member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupMemberCertificate {
    /// The certificate.
    pub certificate: CertificateDer<'static>,
    /// Its serial number, which a revocation list names to revoke it.
    pub serial: SerialNumber,
    /// When it stops being valid.
    pub not_after: SystemTime,
}

/// The group's certificate authority: its key and its certificate.
pub struct GroupAuthority {
    issuer: Issuer<'static, KeyPair>,
    certificate: CertificateDer<'static>,
}

impl std::fmt::Debug for GroupAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GroupAuthority")
            .finish_non_exhaustive()
    }
}

impl GroupAuthority {
    /// A new certificate authority for one group, created at `now`: its
    /// name carries a random identifier, so no two groups share one.
    ///
    /// # Errors
    ///
    /// [`AuthorityError`] when the OS generator gives nothing or rcgen
    /// cannot make the certificate.
    pub fn create(now: SystemTime) -> Result<Self, AuthorityError> {
        let key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)?;
        let mut params = CertificateParams::default();
        params.distinguished_name.push(
            DnType::CommonName,
            format!(
                "Goodfetch Daemon group {}",
                data_encoding::HEXLOWER.encode(&random()?)
            ),
        );
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.serial_number = Some(serial()?);
        params.not_before = date(backdated(now));
        params.not_after = date(now + AUTHORITY_LIFETIME);
        let certificate = params.self_signed(&key)?.der().clone();
        Ok(Self {
            issuer: Issuer::new(params, key),
            certificate,
        })
    }

    /// Its certificate, which every Group member trusts for the peer link.
    pub fn certificate(&self) -> &CertificateDer<'static> {
        &self.certificate
    }

    /// The certificate of the Group member `name`, for `key`, signed at
    /// `now` and valid for [`CERTIFICATE_LIFETIME`], for both sides of the
    /// peer link.
    ///
    /// # Errors
    ///
    /// [`AuthorityError`] when the OS generator gives nothing or rcgen
    /// cannot sign it.
    pub fn sign(
        &self,
        name: &GroupMemberName,
        key: &GroupMemberPublicKey,
        now: SystemTime,
    ) -> Result<GroupMemberCertificate, AuthorityError> {
        let mut params = CertificateParams::new(vec![name.dns_name()])?;
        params
            .distinguished_name
            .push(DnType::CommonName, name.as_str());
        params.is_ca = IsCa::ExplicitNoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![
            ExtendedKeyUsagePurpose::ServerAuth,
            ExtendedKeyUsagePurpose::ClientAuth,
        ];
        params.use_authority_key_identifier_extension = true;
        let serial = serial()?;
        params.serial_number = Some(serial.clone());
        let not_after = now + CERTIFICATE_LIFETIME;
        params.not_before = date(backdated(now));
        params.not_after = date(not_after);
        let certificate = params.signed_by(key, &self.issuer)?.der().clone();
        Ok(GroupMemberCertificate {
            certificate,
            serial,
            not_after,
        })
    }

    /// The revocation list, issued at `now`, of the certificates whose
    /// serial numbers are `revoked`: a Group member refuses each on the
    /// peer link. Its number is `now` in seconds, so a later list has a
    /// higher one.
    ///
    /// # Errors
    ///
    /// [`AuthorityError::Certificate`] when rcgen cannot sign it.
    pub fn revocation_list(
        &self,
        revoked: &[SerialNumber],
        now: SystemTime,
    ) -> Result<CertificateRevocationListDer<'static>, AuthorityError> {
        let seconds = now
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let params = CertificateRevocationListParams {
            this_update: date(backdated(now)),
            next_update: date(now + CERTIFICATE_LIFETIME),
            crl_number: SerialNumber::from(seconds),
            issuing_distribution_point: None,
            revoked_certs: revoked
                .iter()
                .map(|serial| RevokedCertParams {
                    serial_number: serial.clone(),
                    revocation_time: date(now),
                    reason_code: None,
                    invalidity_date: None,
                })
                .collect(),
            key_identifier_method: KeyIdMethod::Sha256,
        };
        Ok(params.signed_by(&self.issuer)?.into())
    }
}

/// `now` less [`BACKDATE`], or the epoch for a reading before it.
fn backdated(now: SystemTime) -> SystemTime {
    now.checked_sub(BACKDATE).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// `at` as rcgen writes a date.
fn date(at: SystemTime) -> OffsetDateTime {
    OffsetDateTime::from(at)
}

/// [`RANDOM_BYTES`] from the OS generator.
fn random() -> Result<[u8; RANDOM_BYTES], getrandom::Error> {
    let mut bytes = [0; RANDOM_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(bytes)
}

/// A serial number from the OS generator, positive as RFC 5280 requires.
fn serial() -> Result<SerialNumber, getrandom::Error> {
    let mut bytes = random()?;
    bytes[0] &= 0x7f;
    Ok(SerialNumber::from_slice(&bytes))
}

#[cfg(test)]
mod tests;
