//! The CDP command log the Run trace keeps (spec #577) while the Run's
//! `browser.cdp_command_log` is on: one JSON line per command the engine
//! sends a browser, its method only, never its params, in
//! `trace/cdp-commands.jsonl` of the Run's folder, written by the Run
//! trace's one writer (the engine's `run_trace`, ADR 0140's E5). Each
//! browser's connection records its commands there, the one place every
//! command crosses, its last `Browser.close` included.

use std::io;
use std::pin::Pin;

use crate::browser_run::BrowserRun;
use goodfetch_engine::run_trace::RunTrace;
use goodfetch_engine::settings::{RunSettings, get_or_default};
use goodfetch_protocol::page::BROWSER_CDP_COMMAND_LOG;

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
pub(super) async fn command_log(run: &BrowserRun) -> io::Result<Option<goodfetch_cdp::CommandLog>> {
    if !get_or_default::<bool>(
        run.identity.settings().map(RunSettings::resolved),
        &BROWSER_CDP_COMMAND_LOG,
    ) {
        return Ok(None);
    }
    let trace = &run.trace;
    trace.start().await?;
    Ok(Some(goodfetch_cdp::CommandLog::new(CommandLog(
        trace.clone(),
    ))))
}
