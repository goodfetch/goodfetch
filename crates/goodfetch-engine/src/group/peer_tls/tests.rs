use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use rcgen::SerialNumber;
use rustls::pki_types::ServerName;

use crate::clock::Clock;
use crate::group::authority::{
    CERTIFICATE_LIFETIME, GroupAuthority, GroupMemberCertificate, GroupMemberKey, GroupMemberName,
};
use crate::tls::certificate_failure;

use super::{PeerIdentity, PeerTrust, accepting, connecting};

fn signed_at() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_hours(500_000)
}

/// A Group member whose certificate `authority` signed at [`signed_at`].
struct GroupMember {
    name: GroupMemberName,
    key: GroupMemberKey,
    certificate: GroupMemberCertificate,
}

impl GroupMember {
    fn signed(authority: &GroupAuthority, name: &str) -> Self {
        let name = GroupMemberName::new(name).unwrap();
        let key = GroupMemberKey::generate().unwrap();
        let certificate = authority
            .sign(&name, &key.public_key(), signed_at())
            .unwrap();
        Self {
            name,
            key,
            certificate,
        }
    }

    fn identity(&self) -> PeerIdentity {
        PeerIdentity {
            chain: vec![self.certificate.certificate.clone()],
            key: self.key.private_key(),
        }
    }
}

/// The trust of a Group member of `authority`'s group, whose revocation
/// list names `revoked`.
fn trust(authority: &GroupAuthority, revoked: &[SerialNumber]) -> PeerTrust {
    PeerTrust {
        authority: authority.certificate().clone(),
        revocations: authority.revocation_list(revoked, signed_at()).unwrap(),
    }
}

/// The test Clock, paused at `at`; made on the runtime it runs on.
#[expect(
    clippy::disallowed_methods,
    reason = "a test drives the engine's time through the test Clock (ADR 0059)"
)]
fn paused_at(at: SystemTime) -> Clock {
    Clock::paused(at)
}

/// A peer link's handshake at `at`, from `client` dialling `dialled` to
/// `server`, each with its trust: the certificate failure of the side that
/// refused the other's certificate.
fn hand_shake(
    client: (&GroupMember, &PeerTrust),
    server: (&GroupMember, &PeerTrust),
    dialled: &GroupMemberName,
    at: SystemTime,
) -> Result<(), Option<CertificateFailure>> {
    goodfetch_test_support::runtime()
        .block_on(async {
            let clock = paused_at(at);
            let connecting = connecting(client.0.identity(), client.1, clock).unwrap();
            let accepting = accepting(server.0.identity(), server.1, clock).unwrap();
            exchange(connecting, accepting, dialled.server_name())
        })
        .map_err(|error| certificate_failure(&error))
}

/// Runs a handshake in memory until both sides finish or one refuses.
fn exchange(
    client: rustls::ClientConfig,
    server: rustls::ServerConfig,
    dialled: ServerName<'static>,
) -> Result<(), rustls::Error> {
    let mut client = rustls::ClientConnection::new(Arc::new(client), dialled).unwrap();
    let mut server = rustls::ServerConnection::new(Arc::new(server)).unwrap();
    while client.is_handshaking() || server.is_handshaking() {
        let mut bytes = Vec::new();
        while client.wants_write() {
            client.write_tls(&mut bytes).unwrap();
        }
        let mut sent = bytes.as_slice();
        while !sent.is_empty() {
            server.read_tls(&mut sent).unwrap();
            server.process_new_packets()?;
        }
        let mut bytes = Vec::new();
        while server.wants_write() {
            server.write_tls(&mut bytes).unwrap();
        }
        let mut sent = bytes.as_slice();
        while !sent.is_empty() {
            client.read_tls(&mut sent).unwrap();
            client.process_new_packets()?;
        }
    }
    Ok(())
}

#[test]
fn two_group_members_of_one_group_open_a_peer_link() {
    let authority = GroupAuthority::create(signed_at()).unwrap();
    let lead = GroupMember::signed(&authority, "lead");
    let crawler = GroupMember::signed(&authority, "crawler-1");
    let trust = trust(&authority, &[]);

    let shaken = hand_shake((&crawler, &trust), (&lead, &trust), &lead.name, signed_at());

    assert_eq!(shaken, Ok(()));
}

#[test]
fn a_group_member_refuses_a_server_another_group_signed() {
    let ours = GroupAuthority::create(signed_at()).unwrap();
    let theirs = GroupAuthority::create(signed_at()).unwrap();
    let crawler = GroupMember::signed(&ours, "crawler-1");
    let stranger = GroupMember::signed(&theirs, "lead");

    let shaken = hand_shake(
        (&crawler, &trust(&ours, &[])),
        (&stranger, &trust(&theirs, &[])),
        &stranger.name,
        signed_at(),
    );

    assert_eq!(
        shaken,
        Err(Some(CertificateFailure::Known(
            KnownCertificateFailure::UnknownIssuer
        )))
    );
}

#[test]
fn a_group_member_refuses_a_client_another_group_signed() {
    let ours = GroupAuthority::create(signed_at()).unwrap();
    let theirs = GroupAuthority::create(signed_at()).unwrap();
    let lead = GroupMember::signed(&ours, "lead");
    // The stranger trusts our authority, so only the lead's check refuses.
    let stranger = GroupMember::signed(&theirs, "crawler-1");

    let shaken = hand_shake(
        (&stranger, &trust(&ours, &[])),
        (&lead, &trust(&ours, &[])),
        &lead.name,
        signed_at(),
    );

    assert_eq!(
        shaken,
        Err(Some(CertificateFailure::Known(
            KnownCertificateFailure::UnknownIssuer
        )))
    );
}

#[test]
fn a_revoked_group_member_is_refused() {
    let authority = GroupAuthority::create(signed_at()).unwrap();
    let lead = GroupMember::signed(&authority, "lead");
    let removed = GroupMember::signed(&authority, "crawler-1");
    let revoking = trust(
        &authority,
        std::slice::from_ref(&removed.certificate.serial),
    );

    let shaken = hand_shake(
        (&removed, &trust(&authority, &[])),
        (&lead, &revoking),
        &lead.name,
        signed_at(),
    );

    assert_eq!(
        shaken,
        Err(Some(CertificateFailure::Known(
            KnownCertificateFailure::Revoked
        )))
    );
}

#[test]
fn a_certificate_past_its_lifetime_is_refused() {
    let authority = GroupAuthority::create(signed_at()).unwrap();
    let lead = GroupMember::signed(&authority, "lead");
    let crawler = GroupMember::signed(&authority, "crawler-1");
    let trust = trust(&authority, &[]);

    let shaken = hand_shake(
        (&crawler, &trust),
        (&lead, &trust),
        &lead.name,
        lead.certificate.not_after + Duration::from_secs(1),
    );

    assert_eq!(
        lead.certificate.not_after,
        signed_at() + CERTIFICATE_LIFETIME
    );
    assert_eq!(
        shaken,
        Err(Some(CertificateFailure::Known(
            KnownCertificateFailure::Expired
        )))
    );
}

#[test]
fn a_group_member_answering_for_another_name_is_refused() {
    let authority = GroupAuthority::create(signed_at()).unwrap();
    let lead = GroupMember::signed(&authority, "lead");
    let crawler = GroupMember::signed(&authority, "crawler-1");
    let impostor = GroupMember::signed(&authority, "crawler-2");
    let trust = trust(&authority, &[]);

    let shaken = hand_shake(
        (&crawler, &trust),
        (&impostor, &trust),
        &lead.name,
        signed_at(),
    );

    assert_eq!(
        shaken,
        Err(Some(CertificateFailure::Known(
            KnownCertificateFailure::NameMismatch
        )))
    );
}
