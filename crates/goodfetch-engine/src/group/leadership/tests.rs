//! Lead Daemon confirmation and fencing rules (ADR 0055): sole-member
//! immediate confirmation, strict majority across 2 and 3 members,
//! `group lead --confirm` override, and fencing on any higher lead epoch.

mod trace;

use goodfetch_protocol::group::{LeadEpoch, LeadEpochStaleDetails};

use super::{ExchangeOutcome, LeadRefusal, LeadSnapshot, Leadership};

#[test]
fn a_single_member_group_starts_confirmed() {
    let lead = Leadership::starting("lead", LeadEpoch::new(1), ["lead".to_owned()]);

    assert!(!lead.is_unconfirmed());
    assert_eq!(lead.can_act(), Ok(LeadEpoch::new(1)));
    assert_eq!(
        lead.status(),
        Ok(LeadSnapshot {
            lead_epoch: LeadEpoch::new(1),
            confirmed: true,
            confirmed_by_operator: false,
        })
    );
}

#[test]
fn a_three_member_lead_daemon_acts_only_after_reaching_one_other_member() {
    let mut lead = Leadership::starting(
        "lead",
        LeadEpoch::new(2),
        ["lead".to_owned(), "w1".to_owned(), "w2".to_owned()],
    );

    assert!(lead.is_unconfirmed());
    assert_eq!(
        lead.can_act(),
        Err(LeadRefusal::Unconfirmed {
            lead_epoch: LeadEpoch::new(2),
            confirmed_by: 1,
            total_members: 3,
        })
    );
    assert_eq!(
        lead.unconfirmed_peers(),
        vec!["w1".to_owned(), "w2".to_owned()]
    );

    // An unknown peer or stale epoch does not count toward majority.
    assert_eq!(
        lead.record_peer_exchange("stranger", LeadEpoch::new(2)),
        ExchangeOutcome::Pending
    );
    assert_eq!(
        lead.record_peer_exchange("w1", LeadEpoch::new(1)),
        ExchangeOutcome::Pending
    );
    assert!(lead.is_unconfirmed());

    // One known peer at the same epoch makes 2 of 3 -> strict majority.
    assert_eq!(
        lead.record_peer_exchange("w1", LeadEpoch::new(2)),
        ExchangeOutcome::Confirmed
    );
    assert!(!lead.is_unconfirmed());
    assert_eq!(lead.can_act(), Ok(LeadEpoch::new(2)));
    assert!(lead.unconfirmed_peers().is_empty());
}

#[test]
fn a_two_member_lead_daemon_needs_the_other_member_for_majority() {
    let mut lead = Leadership::starting(
        "lead",
        LeadEpoch::new(1),
        ["lead".to_owned(), "w1".to_owned()],
    );

    assert!(lead.is_unconfirmed());
    assert_eq!(
        lead.record_peer_exchange("w1", LeadEpoch::new(1)),
        ExchangeOutcome::Confirmed
    );
    assert_eq!(lead.can_act(), Ok(LeadEpoch::new(1)));
}

#[test]
fn operator_confirmation_lets_an_unconfirmed_lead_daemon_act() {
    let mut lead = Leadership::starting(
        "lead",
        LeadEpoch::new(3),
        ["lead".to_owned(), "w1".to_owned(), "w2".to_owned()],
    );

    let confirmed = lead.confirm_by_operator().unwrap();

    assert_eq!(
        confirmed,
        LeadSnapshot {
            lead_epoch: LeadEpoch::new(3),
            confirmed: true,
            confirmed_by_operator: true,
        }
    );
    assert_eq!(lead.can_act(), Ok(LeadEpoch::new(3)));
}

#[test]
fn a_higher_lead_epoch_fences_both_acting_and_operator_confirmation() {
    let mut lead = Leadership::starting(
        "lead",
        LeadEpoch::new(2),
        ["lead".to_owned(), "w1".to_owned(), "w2".to_owned()],
    );

    assert_eq!(
        lead.record_peer_exchange("w1", LeadEpoch::new(3)),
        ExchangeOutcome::Fenced(LeadEpoch::new(3))
    );

    let expected = LeadRefusal::Fenced(LeadEpochStaleDetails {
        lead_epoch: LeadEpoch::new(2),
        highest_lead_epoch: LeadEpoch::new(3),
    });
    assert_eq!(lead.can_act(), Err(expected.clone()));
    assert_eq!(lead.confirm_by_operator(), Err(expected.clone()));
    assert_eq!(lead.status(), Err(expected));
}

#[test]
fn a_non_lead_member_cannot_act_or_confirm() {
    let mut member = Leadership::not_lead("worker");

    assert_eq!(member.can_act(), Err(LeadRefusal::NotLead));
    assert_eq!(member.confirm_by_operator(), Err(LeadRefusal::NotLead));
    assert_eq!(member.status(), Err(LeadRefusal::NotLead));
}
