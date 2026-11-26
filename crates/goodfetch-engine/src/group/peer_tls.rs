//! The peer link's mutual TLS (ADR 0055): both sides present a certificate
//! the group's certificate authority signed, and each refuses the other's
//! when another authority signed it, when it is out of its dates at the
//! engine's Clock (ADR 0059), or when the group's revocation list names it.
//! The connecting side also checks that the accepting side's certificate
//! carries the Group member name it dialled. TLS 1.3 only: both sides are
//! Goodfetch.

use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer};
use rustls::server::{VerifierBuilderError, WebPkiClientVerifier};
use rustls::{ClientConfig, RootCertStore, ServerConfig};

use crate::clock::Clock;
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
/// connecting Group member for its certificate and refuses a connection
/// without one.
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
