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
    /// Its Lead Daemon's Group member name; its own on the Lead Daemon.
    pub(crate) lead_name: String,
    /// When its certificate stops being valid.
    pub(crate) certificate_expires_at: SystemTime,
    /// When it asks for the next certificate, by [`renew_at`].
    pub(crate) renew_at: SystemTime,
}

/// When a Group member that received at `received` a certificate valid
/// until `expires` asks for the next one: two thirds of the way, which
/// leaves a third of its life for retries, as cert-manager renews by
/// default (`renewBefore`, shape).
pub(crate) fn renew_at(received: SystemTime, expires: SystemTime) -> SystemTime {
    let life = expires.duration_since(received).unwrap_or_default();
    received + life * 2 / 3
}

/// A certificate a Group member received in place of the one it had.
pub(crate) struct Renewed {
    /// The new key, whose public half the certificate is for.
    pub(crate) key: PrivatePkcs8KeyDer<'static>,
    pub(crate) certificate: CertificateDer<'static>,
    pub(crate) expires_at: SystemTime,
    /// The authority's latest revocation list.
    pub(crate) revocations: CertificateRevocationListDer<'static>,
    /// When it was received.
    pub(crate) received_at: SystemTime,
}

impl std::fmt::Debug for Renewed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Renewed")
            .field("expires_at", &self.expires_at)
            .field("received_at", &self.received_at)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for KeptIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeptIdentity")
            .field("name", &self.name)
            .field("peer_address", &self.peer_address)
            .field("lead_address", &self.lead_address)
            .field("lead_name", &self.lead_name)
            .field("certificate_expires_at", &self.certificate_expires_at)
            .field("renew_at", &self.renew_at)
            .finish_non_exhaustive()
    }
}

impl Clone for KeptIdentity {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            key: self.key.clone_key(),
            certificate: self.certificate.clone(),
            authority: self.authority.clone(),
            revocations: self.revocations.clone(),
            peer_address: self.peer_address.clone(),
            lead_address: self.lead_address.clone(),
            lead_name: self.lead_name.clone(),
            certificate_expires_at: self.certificate_expires_at,
            renew_at: self.renew_at,
        }
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
            lead_name: name.as_str().to_owned(),
            certificate_expires_at: signed.not_after,
            renew_at: renew_at(now, signed.not_after),
        })
    }

    /// Whether it is its group's Lead Daemon, which renews its own
    /// certificate rather than asking for one.
    pub(crate) fn is_lead(&self) -> bool {
        self.lead_name == self.name
    }

    /// It, holding `renewed` in place of its certificate.
    pub(crate) fn with(self, renewed: Renewed) -> Self {
        Self {
            key: renewed.key,
            certificate: renewed.certificate,
            revocations: renewed.revocations,
            certificate_expires_at: renewed.expires_at,
            renew_at: renew_at(renewed.received_at, renewed.expires_at),
            ..self
        }
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
