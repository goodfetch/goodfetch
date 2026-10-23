//! A Run's outcome records (ADR 0072): one `tracing` record per Error code
//! a Run's driver records in its run state and no reply carries, such as a
//! work item's `handler_lost`. The engine's JSON log and the test `tracing`
//! subscriber carry them into a test's exercise record, where the exercise
//! audit reads them beside `rpc`'s dispatch records.

use super::Run;

/// The `tracing` target of the outcome records.
pub const OUTCOME_TARGET: &str = "goodfetch::run::outcome";

impl Run {
    /// Records, under [`OUTCOME_TARGET`], that its driver recorded
    /// `error_code` in its run state, which no reply carries.
    pub fn record_outcome(&self, error_code: &str) {
        tracing::trace!(target: OUTCOME_TARGET, run_id = %self.id, error_code, "the run recorded an error code");
    }
}
