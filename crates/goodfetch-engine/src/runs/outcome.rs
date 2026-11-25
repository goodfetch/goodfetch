//! A Run's outcome records (ADR 0072): one `tracing` record per Error code
//! a Run's driver records in its run state and no reply carries, such as a
//! work item's `handler_lost`. The engine's JSON log and the test `tracing`
//! subscriber carry them into a test's exercise record, where the exercise
//! audit reads them beside `rpc`'s dispatch records. A record needs only the
//! Run's identity (ADR 0145).

use goodfetch_protocol::connection::{ENGINE_PANICKED, PanicEnded};

use super::Run;
use crate::run_identity::RunIdentity;

/// The `tracing` target of the outcome records.
pub const OUTCOME_TARGET: &str = "goodfetch::run::outcome";

/// Records, under [`OUTCOME_TARGET`], that the driver of the Run `run`
/// names recorded `error_code` in its run state, which no reply carries.
pub fn record_outcome(run: &RunIdentity, error_code: &str) {
    tracing::trace!(target: OUTCOME_TARGET, run_id = %run.id(), error_code, "the run recorded an error code");
}

/// Records, under [`OUTCOME_TARGET`], that a panic in the Run `run` ended
/// what `ended` names and the Run went on: its trace names
/// `engine_panicked` with `ended` (ADR 0148), which no reply carries.
pub fn record_panicked(run: &RunIdentity, ended: PanicEnded) {
    tracing::trace!(
        target: OUTCOME_TARGET,
        run_id = %run.id(),
        error_code = ENGINE_PANICKED.name,
        ended = ended.as_str(),
        "the run recorded an error code"
    );
}

impl Run {
    /// Records, under [`OUTCOME_TARGET`], that its driver recorded
    /// `error_code` in its run state, which no reply carries.
    pub fn record_outcome(&self, error_code: &str) {
        record_outcome(&self.identity, error_code);
    }
}
