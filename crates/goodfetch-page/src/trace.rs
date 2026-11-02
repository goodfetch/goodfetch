//! The CDP command log the Run trace keeps (spec #577): one JSON line per
//! command the engine sends a browser, its method only, never its params,
//! in `trace/cdp-commands.jsonl` of the Run's folder. A blocking actor of
//! the Run's owner appends the lines, a group at a time.
//!
//! Beside it, the Provenance log of the Run (ADR 0047), which the engine's
//! `run_trace` writes masked by the Run's `Mask`: one JSON
//! line per entry, in `trace/provenance.jsonl`, each naming its `action`
//! and its page, such as the live hiding a page ran under, a rule the guard
//! lifted, or each `page.formats` call with the Clean-up rules' version and
//! what extraction-side removal took from its document, and what
//! a fetch tier did, such as a hedge it sent or a response two requests
//! shared (ADR 0040).
#![expect(
    clippy::disallowed_methods,
    reason = "the log's actor writes its file on the tasks module's dedicated thread, never on an async worker"
)]

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::runs::Run;
use goodfetch_engine::tasks::TaskOwner;
use tokio::sync::mpsc;

/// The log's file in the Run's folder.
pub(crate) const FILE: &str = "trace/cdp-commands.jsonl";

/// Records `entry`, one Provenance entry of `run`, such as of one of its
/// pages or of a request a fetch tier sent for it, as a line of
/// the Run trace's Provenance log, appended through the Run's blocking
/// policy; a line that cannot be written is logged and lost.
///
/// # Cancel safety
///
/// Dropping the future never stops a line being written once it began.
pub async fn provenance(run: &Run, entry: &serde_json::Value) {
    let written = goodfetch_engine::run_trace::provenance(
        run.owner(),
        run.folder(),
        run.secrets().mask(),
        entry,
    )
    .await;
    if let Err(error) = written {
        tracing::warn!(%error, "a Provenance entry of the Run trace cannot be written");
    }
}

/// How many methods wait for the writer before a sender waits.
const QUEUE: usize = 1024;

/// A handle that records a command's method.
#[derive(Clone, Debug)]
pub(crate) struct CommandLog {
    methods: mpsc::Sender<&'static str>,
    /// Where each send to its writer is counted (ADR 0140's C1).
    work: CountedWork,
}

impl CommandLog {
    /// The log of the Run whose folder is `run_folder`, written by an actor
    /// of `owner`, which waits for it once every handle is gone, counting
    /// what it does on `work`.
    pub(crate) fn start(
        owner: &TaskOwner,
        run_folder: &Path,
        work: CountedWork,
    ) -> io::Result<Self> {
        let (methods, mailbox) = mpsc::channel(QUEUE);
        let file: PathBuf = run_folder.join(FILE);
        let writer_work = work.clone();
        owner.actor("cdp command log", mailbox, move |methods: Vec<&'static str>| {
            if let Err(error) = append(&file, &methods, &writer_work) {
                tracing::warn!(%error, file = %file.display(), "the CDP command log cannot be written");
            }
        })?;
        Ok(Self { methods, work })
    }

    /// A log that writes nothing, for a test's browser that no Run holds.
    #[cfg(test)]
    pub(crate) fn unwritten() -> Self {
        let (methods, _) = mpsc::channel(1);
        Self {
            methods,
            work: CountedWork::default(),
        }
    }

    /// Records that `method` is being sent.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may lose the line.
    pub(crate) async fn record(&self, method: &'static str) {
        self.work.count(Row::CdpLogSendsOff);
        let _ = self.methods.send(method).await;
    }
}

fn append(file: &Path, methods: &[&'static str], work: &CountedWork) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        work.count(Row::CdpFileOpens);
        std::fs::create_dir_all(folder)?;
    }
    let mut lines = Vec::new();
    for method in methods {
        serde_json::to_writer(&mut lines, &serde_json::json!({ "method": method }))?;
        lines.push(b'\n');
    }
    work.count(Row::CdpFileOpens);
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(&lines)
}
