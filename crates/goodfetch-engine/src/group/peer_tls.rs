//! The peer link's mutual TLS (ADR 0055): both sides present a certificate
//! the group's certificate authority signed, and each refuses the other's
//! when another authority signed it, when it is out of its dates at the
//! engine's Clock (ADR 0059), or when the group's revocation list names it.
//! The connecting side also checks that the accepting side's certificate
//! carries the Group member name it dialled. TLS 1.3 only: both sides are
//! Goodfetch.
//!
//! A Daemon that joins has no certificate yet: it opens its link with
//! [`joining`], which presents none and trusts the Lead Daemon's
//! certificate only when the chain it shows holds the authority whose
//! public key it pinned and that authority signed it, as `kubeadm join`
//! pins its cluster's authority. The accepting side lets such a link
//! through, and the peer link answers it nothing but `group.join`.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::{WebPkiServerVerifier, verify_server_cert_signed_by_trust_anchor};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{
    CertificateDer, CertificateRevocationListDer, PrivateKeyDer, ServerName, UnixTime,
};
use rustls::server::{ParsedCertificate, VerifierBuilderError, WebPkiClientVerifier};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, ServerConfig,
    SignatureScheme,
};

use crate::clock::Clock;
use crate::known_certificates::KeyHash;
use crate::tls::OnClock;

/// What a Group member trusts on the peer link: the group's certificate
/// authority, and the revocation list it issued last.
#[derive(Clone, Debug)]
pub struct PeerTrust {
    /// The group's certificate authority's certificate.
    pub authority: CertificateDer<'static>,
    /// The certificates it revoked.
    pub revocations: CertificateRevocationListDer<'static>,
}

/// What a Group member presents on the peer link: its certificate, which
/// the group's certificate authority signed, and its own key.
pub struct PeerIdentity {
    /// Its certificate, first, and any other of its chain.
    pub chain: Vec<CertificateDer<'static>>,
    /// Its private key.
    pub key: PrivateKeyDer<'static>,
}

impl std::fmt::Debug for PeerIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PeerIdentity")
            .field("chain", &self.chain.len())
            .finish_non_exhaustive()
    }
}

/// Why a side of the peer link could not be configured.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PeerTlsError {
    /// The group's certificate authority's certificate is no certificate
    /// rustls can trust.
    #[error("the group's certificate authority cannot be trusted: {0}")]
    Authority(rustls::Error),
    /// The verifier refused the authority or the revocation list.
    #[error("the peer link's verifier cannot be built: {0}")]
    Verifier(#[from] VerifierBuilderError),
    /// The Group member's own certificate and key do not go together.
    #[error("the Group member's certificate and key cannot be presented: {0}")]
    Identity(rustls::Error),
}

/// The configuration of the side that accepts peer links: it asks every
/// connecting Group member for its certificate and refuses one the
/// group's authority did not sign; a link without one is a Daemon that
/// joins, which it lets through for `group.join` alone.
///
/// # Errors
///
/// [`PeerTlsError`] when the trust or the identity is unusable.
pub fn accepting(
    identity: PeerIdentity,
    trust: &PeerTrust,
    clock: Clock,
) -> Result<ServerConfig, PeerTlsError> {
    let provider = provider();
    let verifier =
        WebPkiClientVerifier::builder_with_provider(roots(trust)?, Arc::clone(&provider))
            .with_crls([trust.revocations.clone()])
            .allow_unauthenticated()
            .build()?;
    ServerConfig::builder_with_details(provider, OnClock::shared(clock))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("invariant: ring supports TLS 1.3")
        .with_client_cert_verifier(verifier)
        .with_single_cert(identity.chain, identity.key)
        .map_err(PeerTlsError::Identity)
}

/// The configuration of the side that opens a peer link, which presents
/// its certificate when the accepting side asks.
///
/// # Errors
///
/// [`PeerTlsError`] when the trust or the identity is unusable.
pub fn connecting(
    identity: PeerIdentity,
    trust: &PeerTrust,
    clock: Clock,
) -> Result<ClientConfig, PeerTlsError> {
    let provider = provider();
    let verifier =
        WebPkiServerVerifier::builder_with_provider(roots(trust)?, Arc::clone(&provider))
            .with_crls([trust.revocations.clone()])
            .build()?;
    ClientConfig::builder_with_details(provider, OnClock::shared(clock))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("invariant: ring supports TLS 1.3")
        .with_webpki_verifier(verifier)
        .with_client_auth_cert(identity.chain, identity.key)
        .map_err(PeerTlsError::Identity)
}

/// The configuration of a Daemon that joins the group whose authority's
/// public key hashes to `pin`: it presents no certificate, and trusts the
/// Lead Daemon's when the chain it shows holds that authority and that
/// authority signed it, at the engine's Clock. A chain without it fails
/// the handshake as an unknown issuer.
pub fn joining(pin: KeyHash, clock: Clock) -> ClientConfig {
    let provider = provider();
    ClientConfig::builder_with_details(Arc::clone(&provider), OnClock::shared(clock))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .expect("invariant: ring supports TLS 1.3")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { pin, provider }))
        .with_no_client_auth()
}

/// The SHA-256 of `certificate`'s public key, which a Daemon that joins
/// pins for the group's certificate authority; none when it is no
/// certificate.
pub(crate) fn key_hash_of(certificate: &CertificateDer<'_>) -> Option<KeyHash> {
    ParsedCertificate::try_from(certificate)
        .ok()
        .map(|parsed| KeyHash::of_public_key(parsed.subject_public_key_info().as_ref()))
}

/// The verifier of a Daemon that joins: the authority it pinned, found in
/// the chain the Lead Daemon shows, is its only root.
#[derive(Debug)]
struct Pinned {
    pin: KeyHash,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let pinned = intermediates
            .iter()
            .find(|certificate| key_hash_of(certificate).as_ref() == Some(&self.pin))
            .ok_or(rustls::Error::InvalidCertificate(
                CertificateError::UnknownIssuer,
            ))?;
        let mut roots = RootCertStore::empty();
        roots.add(pinned.clone().into_owned())?;
        verify_server_cert_signed_by_trust_anchor(
            &ParsedCertificate::try_from(end_entity)?,
            &roots,
            &[],
            now,
            self.provider.signature_verification_algorithms.all,
        )?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The group's certificate authority as rustls's only root.
fn roots(trust: &PeerTrust) -> Result<Arc<RootCertStore>, PeerTlsError> {
    let mut roots = RootCertStore::empty();
    roots
        .add(trust.authority.clone())
        .map_err(PeerTlsError::Authority)?;
    Ok(Arc::new(roots))
}

#[cfg(test)]
mod tests;
