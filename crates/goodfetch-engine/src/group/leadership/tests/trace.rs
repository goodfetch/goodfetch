//! The group module's lead-epoch transitions as `tracing` events (ADR 0060):
//! each one the lead-epoch model names (`test-support/models/lead-epoch/`),
//! under [`TARGET`], in the order the group took them, which TLC validates a
//! trace against.

use goodfetch_protocol::group::LeadEpoch;
use goodfetch_test_support::{CapturedLog, model_trace};
use tracing_subscriber::layer::SubscriberExt as _;

use crate::group::{ExchangeOutcome, LeadEpochs, Leadership, TARGET};

/// The trace of a three-member group across formation, membership change,
/// promotion to epoch 2, stale-epoch refusal, fencing, and operator
/// confirmation. `just tlc-validate lead-epoch` validates it against
/// `TraceLeadEpoch.tla`.
const THREE_MEMBERS: &str = include_str!("trace/three-members.ndjson");

/// Covers: story:3-106, story:3-108
#[test]
fn the_group_traces_each_transition_the_lead_epoch_model_names() {
    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber =
        tracing_subscriber::registry().with(model_trace(&[TARGET], move || writer.clone()));
    let _traced = tracing::subscriber::set_default(subscriber);

    // 1. Lead Daemon forms a 1-member group at epoch 1 and adds two members.
    let mut lead = Leadership::starting("lead", LeadEpoch::new(1), ["lead".to_owned()]);
    lead.on_member_added("worker-a".to_owned());
    lead.on_member_added("worker-b".to_owned());

    // 2. worker-a is promoted at epoch 2 and exchanges epochs with worker-b.
    let mut worker_b_epochs = LeadEpochs::for_member("worker-b", LeadEpoch::new(1));
    let mut promoted = Leadership::starting(
        "worker-a",
        LeadEpoch::new(2),
        [
            "lead".to_owned(),
            "worker-a".to_owned(),
            "worker-b".to_owned(),
        ],
    );
    worker_b_epochs.admit(LeadEpoch::new(2)).unwrap();
    assert_eq!(
        promoted.record_peer_exchange("worker-b", LeadEpoch::new(2)),
        ExchangeOutcome::Confirmed
    );

    // 3. The former Lead Daemon contacts worker-b with stale epoch 1, is
    //    refused, and fences itself.
    assert!(worker_b_epochs.admit(LeadEpoch::new(1)).is_err());
    assert_eq!(
        lead.record_peer_exchange("worker-b", LeadEpoch::new(2)),
        ExchangeOutcome::Fenced(LeadEpoch::new(2))
    );

    // 4. The confirmed epoch-2 Lead Daemon removes the retired member.
    promoted.on_member_removed("lead");

    // 5. A third epoch starts while partitioned and is confirmed by operator.
    let mut operator_lead = Leadership::starting(
        "worker-b",
        LeadEpoch::new(3),
        ["worker-a".to_owned(), "worker-b".to_owned()],
    );
    operator_lead.confirm_by_operator().unwrap();

    assert_eq!(
        String::from_utf8(captured.written()).unwrap(),
        THREE_MEMBERS
    );
}
