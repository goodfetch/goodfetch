//! The CDP command log the Run trace keeps (spec #577): one JSON line per
//! command the engine sends a browser, its method only, never its params,
//! in `trace/cdp-commands.jsonl` of the Run's folder, written by the Run
//! trace's one writer (the engine's `run_trace`, ADR 0140's E5).
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

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::run_trace::RunTrace;
use goodfetch_engine::runs::Run;

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
    if let Err(error) = run.provenance(entry).await {
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

/// A handle that records a command's method.
#[derive(Clone, Debug)]
pub(crate) struct CommandLog {
    /// The Run trace whose writer writes its lines.
    trace: RunTrace,
    /// Where each send to its writer is counted (ADR 0140's C1).
    work: CountedWork,
}

impl CommandLog {
    /// The log `trace`'s writer writes, each send counted on `work`, both
    /// its Run's; nothing starts before its first line.
    pub(crate) fn new(trace: RunTrace, work: CountedWork) -> Self {
        Self { trace, work }
    }

    /// The log `trace`'s writer writes, as [`CommandLog::new`]'s, that
    /// writer started.
    ///
    /// # Errors
    ///
    /// When the writer cannot start.
    pub(crate) async fn start(trace: &RunTrace, work: &CountedWork) -> io::Result<Self> {
        trace.start().await?;
        Ok(Self::new(trace.clone(), work.clone()))
    }

    /// The Counted work of the Run whose commands it records.
    pub(crate) fn work(&self) -> &CountedWork {
        &self.work
    }

    /// Records that `method` is being sent.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may lose the line.
    pub(crate) async fn record(&self, method: &'static str) {
        self.work.count(Row::CdpLogSendsOff);
        self.trace.cdp_command(method).await;
    }
}
