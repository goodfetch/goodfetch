//! The Run trace's request log (ADRs 0085 and 0093): one JSON line per
//! request event the engine records for a Run, in `trace/requests.jsonl`
//! of the Run's folder, each the event's catalog name and its data, as the
//! event stream would carry it. A request that failed while no call waited
//! for it, such as one the egress policy refused, is recorded here as
//! `page.request_failed`, so a refusal is never a timeout: a page's with its
//! `PageRequest`, and one the engine's own HTTP client sent for the Run, which
//! no page sent, with the URL, method and failure it knows (`RequestFailed`).
//! A request a site's service worker sent itself, which belongs to no page,
//! is recorded as `service_worker.request` whether it went on or was refused
//! (ADR 0121). Every line passes through the Run's `Mask` as a kept output
//! (spec #564): no value a Run resolved is written, unless the Run's
//! `security.redaction` is off.
//!
//! Each line is one entry an intercepted request sends the Run trace's
//! writer, and the folder and file the log opens to append it are that
//! request's, counted on the interception's rows (`C5.entries`,
//! `C5.file_opens`; ADR 0140). The Provenance log's file is the Run's: its
//! Run holds it, opened once, at its first entry, so no page's entry opens
//! it (`F1.file_opens`).

use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::Event;
use goodfetch_protocol::home::{RUN_TRACE_PROVENANCE, RUN_TRACE_REQUESTS};
use goodfetch_protocol::page::events::PAGE_REQUEST_FAILED;
use goodfetch_protocol::page::{RequestFailed, SERVICE_WORKER_REQUEST, ServiceWorkerRequest};

use serde::Serialize;

use crate::mask::{Mask, Scope};
use crate::tasks::TaskOwner;

/// A Run's request log.
#[derive(Debug)]
pub struct RequestLog {
    file: Arc<Path>,
    owner: TaskOwner,
    mask: Mask,
    work: CountedWork,
}

impl RequestLog {
    /// The log of the Run whose folder is `run_folder`, written through
    /// `owner`'s blocking policy, masked by `mask`, the Run's, counting on
    /// `work`; its holder waits for `owner`.
    pub fn new(owner: TaskOwner, run_folder: &Path, mask: Mask, work: CountedWork) -> Self {
        let file: PathBuf = run_folder.join(RUN_TRACE_REQUESTS);
        Self {
            file: file.into(),
            owner,
            mask,
            work,
        }
    }

    /// Records that a request no call waited for failed.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops a line being written once it began.
    pub async fn request_failed(&self, failed: &RequestFailed) -> io::Result<()> {
        self.record(&PAGE_REQUEST_FAILED, failed).await
    }

    /// Records `event` with its `data`, as the event stream carries it, such
    /// as a page's `page.request_failed` with its `PageRequest`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops a line being written once it began.
    pub async fn record(&self, event: &Event, data: &impl Serialize) -> io::Result<()> {
        self.record_line(event.name, data).await
    }

    /// Records a request a site's service worker sent itself, which no
    /// page's event reports (ADR 0121), as `service_worker.request`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops a line being written once it began.
    pub async fn service_worker_request(&self, request: &ServiceWorkerRequest) -> io::Result<()> {
        self.record_line(SERVICE_WORKER_REQUEST, request).await
    }

    async fn record_line(&self, name: &str, data: &impl Serialize) -> io::Result<()> {
        let mut entry = serde_json::json!({
            "event": name,
            "data": data,
        });
        self.mask.masks().apply_value(&mut entry, Scope::KeptOutput);
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        self.work.count(Row::InterceptedEntries);
        write(
            &self.owner,
            Arc::clone(&self.file),
            line,
            &self.work,
            Row::InterceptedFileOpens,
        )
        .await
    }
}

/// A Run's Provenance log, which holds its file from its first entry until
/// the Run ends (ADR 0140's E5), so an entry creates no folder and opens no
/// file. Each entry is written to the file at once, in one `write` with no
/// sync and nothing held back: a read straight after the Call that wrote it
/// sees it, and a killed process loses no entry it wrote.
///
/// With `provenance.log` off (ADR 0138), it holds no file and writes no
/// entry: an entry is neither masked, encoded nor written.
#[derive(Debug)]
pub struct Provenance {
    held: Arc<Held>,
    /// Whether the Run writes its Provenance log.
    logs: bool,
}

/// The Provenance log's file, opened at its first entry.
#[derive(Debug)]
struct Held {
    path: PathBuf,
    #[expect(
        clippy::disallowed_types,
        reason = "the log's file, taken only inside its blocking write with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
    )]
    file: std::sync::Mutex<Option<File>>,
}

impl Provenance {
    /// The log of the Run whose folder is `run_folder`, opening nothing yet.
    pub fn new(run_folder: &Path) -> Self {
        Self::logging(run_folder, true)
    }

    /// The log of the Run whose folder is `run_folder`, which writes its
    /// entries when `logs`, its `provenance.log`, and else none.
    pub fn logging(run_folder: &Path, logs: bool) -> Self {
        Self {
            logs,
            held: Arc::new(Held {
                path: run_folder.join(RUN_TRACE_PROVENANCE),
                #[expect(
                    clippy::disallowed_types,
                    reason = "the log's file, the Mutex its field declares (ADR 0060)"
                )]
                file: std::sync::Mutex::new(None),
            }),
        }
    }

    /// Appends `entry`, one Provenance entry, as a line of the log, masked by
    /// `mask`, the Run's, and written through `owner`'s blocking policy,
    /// counting on `work`.
    ///
    /// # Errors
    ///
    /// When the file cannot be opened or written; the next entry opens it
    /// again.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops a line being written once it began.
    pub async fn record(
        &self,
        owner: &TaskOwner,
        mask: &Mask,
        work: &CountedWork,
        mut entry: serde_json::Value,
    ) -> io::Result<()> {
        if !self.logs {
            return Ok(());
        }
        mask.masks().apply_value(&mut entry, Scope::KeptOutput);
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        work.count(Row::RunTraceSpawns);
        let (held, work) = (Arc::clone(&self.held), work.clone());
        owner.blocking(move || held.append(&line, &work)).await
    }
}

impl Held {
    /// Appends `line` to the file, opening it and creating its folder first
    /// when no entry has: the Run's work, once, counted on `E5.file_opens`
    /// alone, never on a page's `F1.file_opens`.
    fn append(&self, line: &[u8], work: &CountedWork) -> io::Result<()> {
        let mut held = self
            .file
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(file) = &mut *held {
            return file.write_all(line);
        }
        if let Some(folder) = self.path.parent() {
            work.create_dir_all(Row::RunTraceFileOpens, folder)?;
        }
        held.insert(work.open_file(
            Row::RunTraceFileOpens,
            OpenOptions::new().create(true).append(true),
            &self.path,
        )?)
        .write_all(line)
    }
}

/// Appends `line`, one entry, to `file` on a blocking task of `owner`'s,
/// counted on `E5.spawns` whatever `owner`'s policy, as a Run's runs one in
/// production (ADR 0140, Exception X2), its folder and file opens also on
/// `opens`.
async fn write(
    owner: &TaskOwner,
    file: Arc<Path>,
    line: Vec<u8>,
    work: &CountedWork,
    opens: Row,
) -> io::Result<()> {
    work.count(Row::RunTraceSpawns);
    let work = work.clone();
    owner
        .blocking(move || append(&file, &line, &work, opens))
        .await
}

/// Appends `line` to `file`, creating its folder: each counted on
/// `E5.file_opens`, the entry's work, and on `opens`, the work of what the
/// entry records (Exception X2): `C5.file_opens` for a request log's line, an
/// intercepted or refused request's.
fn append(file: &Path, line: &[u8], work: &CountedWork, opens: Row) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        work.count(opens);
        work.create_dir_all(Row::RunTraceFileOpens, folder)?;
    }
    work.count(opens);
    work.open_file(
        Row::RunTraceFileOpens,
        OpenOptions::new().create(true).append(true),
        file,
    )?
    .write_all(line)
}

#[cfg(test)]
mod tests;
