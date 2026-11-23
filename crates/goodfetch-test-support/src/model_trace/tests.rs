use tracing_subscriber::layer::SubscriberExt as _;

use super::model_trace;
use crate::trace::CapturedLog;

/// What the layer wrote, one parsed line each.
fn traced(targets: &[&'static str], work: impl FnOnce()) -> Vec<serde_json::Value> {
    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber =
        tracing_subscriber::registry().with(model_trace(targets, move || writer.clone()));
    tracing::subscriber::with_default(subscriber, work);
    String::from_utf8(captured.written())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn each_event_of_a_models_target_is_one_line_of_its_fields() {
    let run_id = "crawl";
    let lines = traced(&["goodfetch::ledger"], || {
        tracing::trace!(target: "goodfetch::ledger", transition = "take_slice", slice = 3_i64, size = 2_u64, run_id = %run_id, held = true, "a run took a Slice");
        tracing::trace!(target: "goodfetch::ledger", transition = "give_back", slice = 3_i64, used = 1_u64, "a run gave back a Slice");
    });

    assert_eq!(
        lines,
        [
            serde_json::json!({ "transition": "take_slice", "slice": 3, "size": 2, "run_id": "crawl", "held": true }),
            serde_json::json!({ "transition": "give_back", "slice": 3, "used": 1 }),
        ]
    );
}

#[test]
fn events_of_other_targets_are_left_out() {
    let lines = traced(&["goodfetch::lease"], || {
        tracing::info!(target: "goodfetch::run", transition = "start", "a Run started");
        tracing::trace!(target: "goodfetch::ledger", transition = "charge", amount = 1_u64, "charged");
        tracing::trace!(target: "goodfetch::lease", transition = "grant", lease_epoch = 1_u64, "granted");
    });

    assert_eq!(
        lines,
        [serde_json::json!({ "transition": "grant", "lease_epoch": 1 })]
    );
}
