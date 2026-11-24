//! The CDP command log the Run trace keeps (spec #577) while the Run's
//! `browser.cdp_command_log` is on: one JSON line per command the engine
//! sends a browser, its method only, never its params, in
//! `trace/cdp-commands.jsonl` of the Run's folder, written by the Run
//! trace's one writer (the engine's `run_trace`, ADR 0140's E5). Each
//! browser's connection records its commands there, the one place every
//! command crosses, its last `Browser.close` included.
//!
//! Beside it, the Provenance log of the Run (ADR 0047), which the engine's
//! `run_trace` writes masked by the Run's `Mask`: one JSON
//! line per entry, in `trace/provenance.jsonl`, each naming its `action`
//! and its page, such as the live hiding a page ran under, a rule the guard
//! lifted, or each `page.formats` call with the Clean-up rules' version and
//! what extraction-side removal took from its document, and what
//! a fetch tier did, such as a hedge it sent or a response two requests
//! shared (ADR 0040).

use std::io;
use std::pin::Pin;

use goodfetch_counted_work::Row;
use goodfetch_engine::run_trace::RunTrace;
use goodfetch_engine::runs::{Run, run_trace};
use goodfetch_engine::settings::{RunSettings, get_or_default};
use goodfetch_protocol::page::BROWSER_CDP_COMMAND_LOG;

/// Records `entry`, one Provenance entry of `run`, such as of one of its
/// pages or of a request a fetch tier sent for it, as a line of
/// the Run trace's Provenance log, appended through the Run's blocking
/// policy to the file the Run holds; a line that cannot be written is
/// logged and lost.
///
/// # Cancel safety
///
/// Dropping the future never stops a line being written once it began.
pub async fn provenance(run: &Run, entry: serde_json::Value) {
    provenance_in(run_trace(run), entry).await;
}

/// Records `entry` as [`provenance`] does, in the Run trace `trace`, for a
/// holder of the Run's trace alone (ADR 0145).
///
/// # Cancel safety
///
/// As [`provenance`]'s.
pub(crate) async fn provenance_in(trace: &RunTrace, entry: serde_json::Value) {
    if let Err(error) = trace.provenance(entry).await {
        tracing::warn!(%error, "a Provenance entry of the Run trace cannot be written");
    }
}

/// Records `entry` as [`provenance`] does, counted on `row`: an entry a
/// Shared path's unit writes, such as a navigation's judgement (ADR 0140).
///
/// # Cancel safety
///
/// As [`provenance`]'s.
pub(crate) async fn counted_provenance(run: &Run, row: Row, entry: serde_json::Value) {
    run.metrics().counted_work().count(row);
    provenance(run, entry).await;
}

/// The Run trace's CDP command log, as a browser's connection records each
/// command's method in it.
#[derive(Debug)]
struct CommandLog(RunTrace);

impl goodfetch_cdp::CommandLogImpl for CommandLog {
    fn record(&self, method: &'static str) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.0.cdp_command(method))
    }
}

/// The CDP command log every browser connection of `run` records its
/// commands in, the Run trace's writer started, when the Run's
/// `browser.cdp_command_log` is on; none while it is off, so no command's
/// method goes to the writer (ADR 0140's C1).
///
/// # Errors
///
/// When the writer cannot start.
pub(crate) async fn command_log(run: &Run) -> io::Result<Option<goodfetch_cdp::CommandLog>> {
    if !get_or_default::<bool>(
        run.settings().map(RunSettings::resolved),
        &BROWSER_CDP_COMMAND_LOG,
    ) {
        return Ok(None);
    }
    let trace = run_trace(run);
    trace.start().await?;
    Ok(Some(goodfetch_cdp::CommandLog::new(CommandLog(
        trace.clone(),
    ))))
}
