//! The Provenance log of the Run (ADR 0047), which the engine's
//! `run_trace` writes masked by the Run's `Mask`: one JSON
//! line per entry, in `trace/provenance.jsonl`, each naming its `action`
//! and its page, such as the live hiding a page ran under, a rule the guard
//! lifted, or each `page.formats` call with the Clean-up rules' version and
//! what extraction-side removal took from its document, and what
//! a fetch tier did, such as a hedge it sent or a response two requests
//! shared (ADR 0040). The Run trace's CDP command log, beside it, is the
//! CDP module's.

use goodfetch_counted_work::Row;
use goodfetch_engine::run_trace::RunTrace;

use crate::browser_run::BrowserRun;

/// Records `entry`, one Provenance entry of a Run, such as of one of its
/// pages or of a request a fetch tier sent for it, as a line of `trace`,
/// its Run trace's Provenance log, appended through the Run's blocking
/// policy to the file the Run holds; a line that cannot be written is
/// logged and lost.
///
/// # Cancel safety
///
/// Dropping the future never stops a line being written once it began.
pub async fn provenance(trace: &RunTrace, entry: serde_json::Value) {
    if let Err(error) = trace.provenance(entry).await {
        tracing::warn!(%error, "a Provenance entry of the Run trace cannot be written");
    }
}

/// Records `entry` in `run`'s Run trace as [`provenance`] does, counted on
/// `row`: an entry a Shared path's unit writes, such as a navigation's
/// judgement (ADR 0140).
///
/// # Cancel safety
///
/// As [`provenance`]'s.
pub(crate) async fn counted_provenance(run: &BrowserRun, row: Row, entry: serde_json::Value) {
    run.identity.metrics().counted_work().count(row);
    provenance(&run.trace, entry).await;
}
