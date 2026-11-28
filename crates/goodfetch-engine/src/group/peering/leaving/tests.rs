//! A Group member leaving its group through its interface, a Lead Daemon
//! serving over loopback on real time: the Lead Daemon revokes the one
//! that leaves and refuses its certificate at the next peer link; one
//! revoked already leaves too; one that asks under another Group member's
//! name is refused; one whose Lead Daemon is out of reach is refused.

use goodfetch_protocol::group::LeadEpoch;
use goodfetch_test_support::real_time_runtime;

use super::super::loopback::{clock_at, closed_address, identity, joined, lead, now};
use super::ask_to_leave;
use crate::group::CERTIFICATE_LIFETIME;
use crate::group::identity::KeptIdentity;
use crate::group::store::{self, Revocation};

#[test]
fn a_group_member_that_leaves_is_revoked_and_refused_at_its_next_link() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let worker = lead.admitted("worker", now()).await;
        let member = joined(&worker, clock).await;

        ask_to_leave(&worker, LeadEpoch::new(1), &identity(), clock)
            .await
            .unwrap();

        assert_eq!(
            store::revoke(&lead.store, "worker".to_owned(), clock.wall())
                .await
                .unwrap(),
            Revocation::Unknown
        );
        let refused = member.peering.renewal_from_lead(&worker).await.unwrap_err();
        assert_eq!(refused.code(), "group_member_unknown", "{refused}");
    });
}

#[test]
fn a_revoked_group_member_leaves_too() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let worker = lead.admitted("worker", now()).await;
        assert!(matches!(
            store::revoke(&lead.store, "worker".to_owned(), clock.wall())
                .await
                .unwrap(),
            Revocation::Revoked(_)
        ));

        ask_to_leave(&worker, LeadEpoch::new(1), &identity(), clock)
            .await
            .expect("a Lead Daemon that revoked a Group member removed it already");
    });
}

#[test]
fn a_group_member_cannot_leave_under_another_ones_name() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let worker = lead.admitted("worker", now()).await;
        lead.admitted("other", now()).await;
        let asking = KeptIdentity {
            name: "other".to_owned(),
            ..worker
        };

        let refused = ask_to_leave(&asking, LeadEpoch::new(1), &identity(), clock)
            .await
            .unwrap_err();

        assert_eq!(refused.code(), "invalid_params", "{refused}");
    });
}

#[test]
fn a_group_member_whose_lead_daemon_is_out_of_reach_cannot_leave() {
    real_time_runtime().block_on(async {
        let clock = clock_at(now());
        let lead = lead(clock, now() + CERTIFICATE_LIFETIME).await;
        let worker = KeptIdentity {
            lead_address: closed_address().await,
            ..lead.admitted("worker", now()).await
        };

        let refused = ask_to_leave(&worker, LeadEpoch::new(1), &identity(), clock)
            .await
            .unwrap_err();

        assert_eq!(refused.code(), "group_lead_unreachable", "{refused}");
    });
}
