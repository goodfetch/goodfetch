//! A Group member's own identity on the peer link (ADR 0055): its name,
//! its key, the certificate the group's authority signed for it, that
//! authority and the revocation list it issued last, and the addresses
//! the Group member and its Lead Daemon listen at. The group store keeps
//! it, so a restart serves the peer link as before.

use std::time::SystemTime;

use rustls::pki_types::{
    CertificateDer, CertificateRevocationListDer, PrivateKeyDer, PrivatePkcs8KeyDer,
};

use super::{
    AuthorityError, GroupAuthority, GroupMemberKey, GroupMemberName, PeerIdentity, PeerTrust,
};

/// What a Group member keeps of itself.
pub(crate) struct KeptIdentity {
    /// Its Group member name.
    pub(crate) name: String,
    /// Its private key, which never leaves it.
    pub(crate) key: PrivatePkcs8KeyDer<'static>,
    /// Its certificate, which the group's authority signed.
    pub(crate) certificate: CertificateDer<'static>,
    /// The group's authority's certificate.
    pub(crate) authority: CertificateDer<'static>,
    /// The revocation list the authority issued last.
    pub(crate) revocations: CertificateRevocationListDer<'static>,
    /// The `host:port` it listens at for peer links.
    pub(crate) peer_address: String,
    /// The `host:port` its Lead Daemon listens at; its own on the Lead
    /// Daemon.
    pub(crate) lead_address: String,
}

impl std::fmt::Debug for KeptIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeptIdentity")
            .field("name", &self.name)
            .field("peer_address", &self.peer_address)
            .field("lead_address", &self.lead_address)
            .finish_non_exhaustive()
    }
}

impl KeptIdentity {
    /// The identity of the Lead Daemon `name` that forms the group of
    /// `authority` at `now`, listening at `peer_address`: a key of its own,
    /// the certificate the authority signs for it and the authority's
    /// first revocation list, which revokes nothing.
    ///
    /// # Errors
    ///
    /// [`AuthorityError`] when the OS generator gives nothing or rcgen
    /// cannot sign.
    pub(crate) fn lead(
        authority: &mut GroupAuthority,
        name: &GroupMemberName,
        peer_address: String,
        now: SystemTime,
    ) -> Result<Self, AuthorityError> {
        let key = GroupMemberKey::generate()?;
        let signed = authority.sign(name, &key.public_key(), now)?;
        let revocations = authority.revocation_list(&[], now)?.list;
        Ok(Self {
            name: name.as_str().to_owned(),
            key: key.kept(),
            certificate: signed.certificate,
            authority: authority.certificate().clone(),
            revocations,
            lead_address: peer_address.clone(),
            peer_address,
        })
    }

    /// What it presents on the peer link: its certificate, then the
    /// authority's, so a Daemon that joins finds the authority it pinned.
    pub(crate) fn presented(&self) -> PeerIdentity {
        PeerIdentity {
            chain: vec![self.certificate.clone(), self.authority.clone()],
            key: PrivateKeyDer::Pkcs8(self.key.clone_key()),
        }
    }

    /// What it trusts on the peer link.
    pub(crate) fn trust(&self) -> PeerTrust {
        PeerTrust {
            authority: self.authority.clone(),
            revocations: self.revocations.clone(),
        }
    }
}
