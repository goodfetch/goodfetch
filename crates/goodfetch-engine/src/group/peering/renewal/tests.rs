//! Certificate renewal through the Group member's interface, a Lead Daemon
//! serving over loopback on real time: a Group member due to renew keeps
//! the certificate the Lead Daemon signs for a new key, presents it, and
//! opens the next peer link with it; the Lead Daemon renews its own; one
//! that asks under another name, or is no Group member, is refused; a
//! renewal that fails is tried again after a tenth of the time left; an
//! expired certificate is not renewed.

use std::sync::Arc;
use std::time::Duration;

use goodfetch_test_support::real_time_runtime;
use rstest::rstest;
use rustls::pki_types::CertificateDer;
use tokio_rustls::TlsConnector;

use super::super::Peering;
use super::super::loopback::{clock_at, closed_address, joined, lead, name, now, stored};
use crate::group::identity::KeptIdentity;
use crate::group::{CERTIFICATE_LIFETIME, GroupMemberName, connecting};

/// How far apart two readings of the same moment may be: the time a test
/// takes.
const SLACK: Duration = Duration::from_mins(1);

/// The certificate `peering`'s listener presents to the Group member
/// `client`, which dials it as `server`.
async fn presented(
    peering: &Peering,
    client: &KeptIdentity,
    server: &GroupMemberName,
) -> CertificateDer<'static> {
    let (near, far) = tokio::io::duplex(64 * 1024);
    let config = connecting(client.presented(), &client.trust(), peering.clock).unwrap();
    let connector = TlsConnector::from(Arc::new(config));
    let acceptor = peering.presenting().await.unwrap().acceptor;
    let (connected, accepted) = tokio::join!(
        connector.connect(server.server_name(), near),
        acceptor.accept(far)
    );
    accepted.unwrap();
    let connected = connected.unwrap();
    connected.get_ref().1.peer_certificates().unwrap()[0].clone()
}

/// Asserts that `wait` is two thirds of a certificate's life, less the
/// test's time.
fn two_thirds_of_a_life(wait: Duration) {
    let expected = CERTIFICATE_LIFETIME * 2 / 3;
    assert!(
        wait <= expected && wait + SLACK > expected,
        "{wait:?}, not about {expected:?}"
    );
}

#[test]
fn a_group_member_due_keeps_and_presents_the_certificate_the_lead_daemon_signs() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let before = lead.admitted("worker", now()).await;
        let member = joined(&before, clock).await;

        let wait = member.peering.renew_when_due().await.unwrap();

        let after = stored(&member.store).await;
        assert_ne!(after.certificate, before.certificate);
        assert_ne!(after.key.secret_pkcs8_der(), before.key.secret_pkcs8_der());
        assert!(after.certificate_expires_at > before.certificate_expires_at);
        two_thirds_of_a_life(wait);
        assert_eq!(
            presented(&member.peering, &lead.kept, &name("worker")).await,
            after.certificate
        );
        member
            .peering
            .renewed(after, clock.wall())
            .await
            .expect("the renewed certificate opens the next peer link");
    });
}

#[test]
fn the_lead_daemon_renews_its_own_certificate() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now()).await;

        let wait = lead.peering.renew_when_due().await.unwrap();

        let after = stored(&lead.store).await;
        assert_ne!(after.certificate, lead.kept.certificate);
        two_thirds_of_a_life(wait);
        assert_eq!(
            presented(
                &lead.peering,
                &lead.member("worker", now()).0,
                &name("lead")
            )
            .await,
            after.certificate
        );
    });
}

#[rstest]
#[case::under_another_members_name("worker", true, "other", "invalid_params")]
#[case::that_is_no_group_member("stranger", false, "stranger", "invalid_request")]
fn the_lead_daemon_refuses_a_renewal(
    #[case] certified: &str,
    #[case] admitted: bool,
    #[case] asks_as: &str,
    #[case] code: &str,
) {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let (certificate, recorded) = lead.member(certified, now());
        if admitted {
            lead.admit(certified, recorded).await;
        }
        let member = joined(&certificate, clock).await;
        let asking = KeptIdentity {
            name: asks_as.to_owned(),
            ..certificate
        };

        let refused = member.peering.renewal_from_lead(&asking).await.unwrap_err();

        assert_eq!(refused.code(), code, "{refused}");
    });
}

#[test]
fn a_renewal_that_fails_is_tried_again_after_a_tenth_of_the_time_left() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let before = KeptIdentity {
            lead_address: closed_address().await,
            ..lead.member("worker", now()).0
        };
        let member = joined(&before, clock).await;

        let retry = member.peering.renew_when_due().await.unwrap();

        let expected = CERTIFICATE_LIFETIME / 10;
        assert!(
            retry <= expected && retry + SLACK > expected,
            "{retry:?}, not about {expected:?}"
        );
        assert_eq!(stored(&member.store).await.certificate, before.certificate);
    });
}

#[test]
fn an_expired_certificate_is_not_renewed() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now() + CERTIFICATE_LIFETIME + Duration::from_secs(1));
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let before = lead.admitted("worker", now()).await;
        let member = joined(&before, clock).await;

        assert_eq!(member.peering.renew_when_due().await, None);
        assert_eq!(stored(&member.store).await.certificate, before.certificate);
    });
}
