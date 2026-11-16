//! What the delivery loop does with what the end of a batch moved of the
//! crawl's pending requests (#2169): the rules the Frontier learned there
//! merged some and gave others back.

use std::collections::BTreeSet;

use goodfetch_engine::admission::Admission;
use goodfetch_engine::runs::Run;

use super::{fill, journal, show_counted};
use crate::progress::SharedInFlight;
use crate::run_state::RunState;
use crate::run_state::teacher::{Move, Moved};

/// Takes what the end of a batch `moved` of the crawl's pending requests
/// (#2169): counts what it changed of the crawl's requests by state and
/// shows it, takes out of their queues the requests a rule it taught
/// merged, keeps pending those a rule it stopped made pending again, and
/// fills those queues again, which also puts the latter back, then
/// journals what the queues changed.
pub(super) async fn merged(
    run: &Run,
    (admission, state): (&mut Admission, &mut RunState),
    in_flight: &SharedInFlight,
    (moved, changed): Moved,
) {
    let mut keys = BTreeSet::new();
    // Each change is in a batch a read may not see yet (ADR 0142).
    for (moving, commits) in moved {
        match moving {
            Move::Merged(id, key) => {
                admission.withdraw(&key, &id);
                state.gone([(id, commits)]);
                keys.insert(key);
            }
            Move::Given(leased) => {
                for waiting in state.added(vec![*leased], commits) {
                    if !waiting.key.is_empty() {
                        keys.insert(waiting.key);
                    }
                }
            }
        }
    }
    state.changed(changed);
    show_counted(run, state, in_flight, []).await;
    for key in keys {
        if let Err(error) = fill(state, admission, Some(key)).await {
            tracing::warn!(%error, run_id = %run.id(), "a queue a learned rule thinned was not filled again");
        }
    }
    journal(run, state.stored(), admission).await;
}
