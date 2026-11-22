//! A live Run's admission as it opens (ADRs 0138 and 0145): a function of
//! its run file's writer, its clock, its entropy and the Politeness it
//! shares, which takes no `Run`.

use std::sync::Arc;

use super::{Admission, Gates};
use crate::clock::Clock;
use crate::entropy::RunEntropy;
use crate::politeness::{PolitenessError, RunPoliteness};
use crate::run_state::{RunState, RunStateError};
use crate::settings::Resolved;

/// Why a Run's admission did not open.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AdmissionError {
    /// Its run file could not be read.
    #[error(transparent)]
    RunState(#[from] RunStateError),
    /// The Politeness it shares could not be opened.
    #[error(transparent)]
    Politeness(#[from] PolitenessError),
}

/// The admission under `settings` of the Run whose run file `state`
/// writes: the gate `gates` opens, from what its run file journaled, read
/// on `clock`, drawing from an `Entropy` forked from `entropy`, its rate
/// cap, and, when the gate names a rule that paces its keys, the
/// Politeness `politeness.sharing` says it shares, from `politeness`: the
/// per-user Politeness file for `shared` and `per_daemon`, a file of its
/// own in memory for `run`. A rule that paces nothing opens none (ADR
/// 0138). `None` when the composition passes in no gate.
///
/// # Errors
///
/// When its run file cannot be read, or the Politeness file opened.
///
/// # Cancel safety
///
/// Dropping the future loses only the answer.
pub async fn of_run(
    gates: Option<&Arc<dyn Gates>>,
    (state, clock): (&RunState, Clock),
    entropy: &RunEntropy,
    politeness: &RunPoliteness,
    settings: &Resolved,
) -> Result<Option<Admission>, AdmissionError> {
    let Some(gates) = gates.cloned() else {
        return Ok(None);
    };
    let opening = settings.clone();
    let (wall, now) = (clock.wall(), clock.now());
    let entropy = entropy.fork().await;
    let gate = state
        .read(move |connection| gates.open(&opening, connection, wall, now, entropy))
        .await?;
    let admission = Admission::new(gate, settings);
    if !admission.shares(settings) {
        // A Politeness of its own that its other code opened, for its
        // robots.txt copy, is served by an actor instead.
        if let Some(own) = politeness.held_by_admission().await {
            politeness.serve(own)?;
        }
        return Ok(Some(admission));
    }
    // The one Politeness the Run shares, its proxies' limits included.
    let (shared, holder) = politeness.open(settings).await?;
    let own = politeness.held_by_admission().await;
    Ok(Some(admission.sharing(
        (shared, own),
        holder,
        clock,
        settings,
    )))
}
