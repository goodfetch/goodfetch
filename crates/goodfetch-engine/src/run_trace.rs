//! The Run trace (ADRs 0047, 0085 and 0093): what a Run records beside its
//! outcomes, written by one writer per Run (ADR 0140's E5), [`RunTrace`],
//! into the Run's `trace/` folder. Its kinds of entry:
//!
//! - **The Provenance log** (ADR 0047), `trace/provenance.jsonl`: one entry
//!   per action a page or a fetch tier took for the Run, written only when
//!   the Run's `provenance.log` is on (ADR 0138).
//! - **The request log**, `trace/requests.jsonl`: one JSON line per request
//!   event the engine records for a Run, each the event's catalog name and
//!   its data, as the event stream would carry it. A request that failed
//!   while no call waited for it, such as one the egress policy refused, is
//!   recorded here as `page.request_failed`, so a refusal is never a
//!   timeout: a page's with its `PageRequest`, and one the engine's own HTTP
//!   client sent for the Run, which no page sent, with the URL, method and
//!   failure it knows (`RequestFailed`). A request a site's service worker
//!   sent itself, which belongs to no page, is recorded as
//!   `service_worker.request` whether it went on or was refused (ADR 0121).
//! - **The CDP command log** (spec #577), `trace/cdp-commands.jsonl`, while
//!   the Run's `browser.cdp_command_log` is on: one JSON line per command the
//!   engine sends a browser, its method only.
//!
//! Every Provenance and request-log line passes through the Run's `Mask` as
//! a kept output (spec #564): no value a Run resolved is written, unless the
//! Run's `security.redaction` is off.
//!
//! The writer is an actor of the Run's owner, started at the Run's first
//! entry, the Run's work, once, counted on `E5.run_spawns`. It holds one
//! buffered handle per file, its folder created and the file opened at that
//! file's first entry, the Run's work too, counted on `E5.run_file_opens`: an
//! entry creates no folder, opens no file and spawns no task, unless it
//! creates again a folder whose creation failed, opens again a file that
//! failed at its open or a write, or starts again a writer whose start
//! failed, each counted on `E5.file_opens` or `E5.spawns`. It writes what
//! its mailbox held at once, each file's lines in one `write` with no sync,
//! and its buffer past 64 KiB; an awaited entry returns once its line is
//! written, so a read straight after the Call that wrote it sees it and a
//! process killed then has written it. The Run's end stops the writer once
//! it wrote what its mailbox held.
#![expect(
    clippy::disallowed_types,
    reason = "`tried` is a flag set and read only inside `get_or_try_init`'s one start at a time, never across an `.await` or a lock, so there is no interleaving for loom to test (ADR 0060)"
)]

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::Event;
use goodfetch_protocol::home::{RUN_TRACE_CDP_COMMANDS, RUN_TRACE_PROVENANCE, RUN_TRACE_REQUESTS};
use goodfetch_protocol::page::events::PAGE_REQUEST_FAILED;
use goodfetch_protocol::page::{RequestFailed, SERVICE_WORKER_REQUEST, ServiceWorkerRequest};
use serde::Serialize;
use tokio::sync::{OnceCell, mpsc, oneshot};

use crate::mask::{Mask, Scope};
use crate::tasks::TaskOwner;

/// How many entries wait for the writer before a sender waits.
const QUEUE: usize = 1024;

/// What each file's buffer holds before the writer writes it, whatever its
/// mailbox still holds (ADR 0140's E5).
const BUFFER: usize = 64 * 1024;

/// A Run's Run trace: the handle of its one writer, cloned by whoever
/// records an entry of the Run.
#[derive(Clone, Debug)]
pub struct RunTrace(Arc<Trace>);

/// What every handle of a Run's Run trace shares.
#[derive(Debug)]
struct Trace {
    /// The Run's folder, which the trace's folder is in.
    folder: PathBuf,
    /// The Run's owner, which runs the writer and stops it at the Run's end.
    owner: Arc<TaskOwner>,
    /// The Run's, which every Provenance and request-log line passes.
    mask: Mask,
    /// Where the writer counts its work.
    work: CountedWork,
    /// Whether the Run writes its Provenance log (`provenance.log`).
    logs_provenance: bool,
    /// The writer's mailbox, once the Run's first entry started it.
    writer: OnceCell<mpsc::Sender<Entry>>,
    /// Whether an entry tried to start the writer: the first try is the
    /// Run's, and one after it failed is the entry's that tries again.
    tried: AtomicBool,
}

/// One line the writer appends to `log`, and who waits for it, if anyone.
#[derive(Debug)]
struct Entry {
    log: Log,
    line: Vec<u8>,
    written: Option<oneshot::Sender<io::Result<()>>>,
}

/// One of the Run trace's files.
#[derive(Clone, Copy, Debug)]
enum Log {
    Provenance,
    Requests,
    CdpCommands,
}

impl Log {
    const ALL: [Self; 3] = [Self::Provenance, Self::Requests, Self::CdpCommands];

    /// How many files the writer holds.
    const COUNT: usize = Self::ALL.len();

    /// Its place among the writer's handles.
    const fn at(self) -> usize {
        match self {
            Self::Provenance => 0,
            Self::Requests => 1,
            Self::CdpCommands => 2,
        }
    }

    /// Its file in the Run's folder.
    fn file(self) -> &'static str {
        match self {
            Self::Provenance => RUN_TRACE_PROVENANCE,
            Self::Requests => RUN_TRACE_REQUESTS,
            Self::CdpCommands => RUN_TRACE_CDP_COMMANDS,
        }
    }
}

// Each log's place is its index in `Log::ALL`, which every log is in.
const _: () = {
    let mut at = 0;
    while at < Log::COUNT {
        assert!(Log::ALL[at].at() == at);
        at += 1;
    }
};

impl RunTrace {
    /// The Run trace of the Run whose folder is `run_folder`, written by an
    /// actor of `owner`, the Run's, masked by `mask`, the Run's, counting on
    /// `work`; it writes Provenance entries when `logs_provenance`, the Run's
    /// `provenance.log`. Nothing starts or opens until its first entry.
    pub fn new(
        owner: Arc<TaskOwner>,
        run_folder: &Path,
        mask: Mask,
        work: CountedWork,
        logs_provenance: bool,
    ) -> Self {
        Self(Arc::new(Trace {
            folder: run_folder.to_owned(),
            owner,
            mask,
            work,
            logs_provenance,
            writer: OnceCell::new(),
            tried: AtomicBool::new(false),
        }))
    }

    /// Starts the writer, unless an entry already did: the Run's work, once,
    /// counted on `E5.run_spawns`, and an entry's on `E5.spawns` when it
    /// tries again after a start failed.
    ///
    /// # Errors
    ///
    /// When the OS refuses the writer's thread; the next entry tries again.
    pub async fn start(&self) -> io::Result<()> {
        self.writer().await.map(drop)
    }

    async fn writer(&self) -> io::Result<&mpsc::Sender<Entry>> {
        self.0
            .writer
            .get_or_try_init(|| async {
                let (entries, mailbox) = mpsc::channel(QUEUE);
                let mut files = Files::new(&self.0.folder, self.0.work.clone());
                // `get_or_try_init` runs one start at a time.
                let again = self.0.tried.swap(true, Ordering::Relaxed);
                self.0.work.count(if again {
                    Row::RunTraceSpawns
                } else {
                    Row::RunTraceRunSpawns
                });
                self.0
                    .owner
                    .actor("run trace", mailbox, move |group| files.write(group))?;
                Ok(entries)
            })
            .await
    }

    /// Appends `entry`, one Provenance entry, as a line of the log, masked
    /// by the Run's `Mask`, and returns once it is written. With the Run's
    /// `provenance.log` off, the entry is neither masked, encoded nor written.
    ///
    /// # Errors
    ///
    /// When the writer cannot start, or the file cannot be opened or
    /// written; the next entry opens it again.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops a line being written once it was
    /// sent to the writer.
    pub async fn provenance(&self, mut entry: serde_json::Value) -> io::Result<()> {
        if !self.0.logs_provenance {
            return Ok(());
        }
        self.0
            .mask
            .masks()
            .apply_value(&mut entry, Scope::KeptOutput);
        let mut line = serde_json::to_vec(&entry)?;
        line.push(b'\n');
        self.written(Log::Provenance, line).await
    }

    /// Records that a command whose method is `method` is being sent to one
    /// of the Run's browsers: its line is written with the next group the
    /// writer writes, and no one waits for it.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may lose the line.
    pub async fn cdp_command(&self, method: &'static str) {
        /// A line of the CDP command log, encoded as it is.
        #[derive(Serialize)]
        struct Command {
            method: &'static str,
        }
        let mut line = Vec::with_capacity(method.len() + 14);
        if let Err(error) = serde_json::to_writer(&mut line, &Command { method }) {
            tracing::warn!(%error, "a CDP command line of the Run trace cannot be encoded");
            return;
        }
        line.push(b'\n');
        match self.writer().await {
            Ok(writer) => {
                let _ = writer
                    .send(Entry {
                        log: Log::CdpCommands,
                        line,
                        written: None,
                    })
                    .await;
            }
            Err(error) => {
                tracing::warn!(%error, "the Run trace's writer cannot start");
            }
        }
    }

    /// Sends `line` to the writer for `log` and waits until it is written.
    async fn written(&self, log: Log, line: Vec<u8>) -> io::Result<()> {
        let writer = self.writer().await?;
        let (written, wrote) = oneshot::channel();
        writer
            .send(Entry {
                log,
                line,
                written: Some(written),
            })
            .await
            .map_err(|_| ended())?;
        wrote.await.unwrap_or_else(|_| Err(ended()))
    }
}

/// The error of an entry the writer took no more, once the Run ended.
fn ended() -> io::Error {
    io::Error::other("the Run trace's writer has ended with its Run")
}

/// The Run trace's request log, through the Run's writer.
#[derive(Debug)]
pub struct RequestLog {
    trace: RunTrace,
}

impl RequestLog {
    /// The request log of the Run whose Run trace is `trace`.
    pub fn new(trace: &RunTrace) -> Self {
        Self {
            trace: trace.clone(),
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
    /// Dropping the future never stops a line being written once it was
    /// sent to the writer.
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
    /// Dropping the future never stops a line being written once it was
    /// sent to the writer.
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
    /// Dropping the future never stops a line being written once it was
    /// sent to the writer.
    pub async fn service_worker_request(&self, request: &ServiceWorkerRequest) -> io::Result<()> {
        self.record_line(SERVICE_WORKER_REQUEST, request).await
    }

    /// Encodes the line once, from the event's own data, and masks it as
    /// text, string by string only where a masked value broke it, as a
    /// call's answer is (#1882's E5-1).
    async fn record_line(&self, name: &str, data: &impl Serialize) -> io::Result<()> {
        /// A line of the request log, its members in the order of their
        /// names.
        #[derive(Serialize)]
        struct Line<'l, D> {
            data: &'l D,
            event: &'l str,
        }
        let trace = &self.trace.0;
        let mut line = serde_json::to_vec(&Line { data, event: name })?;
        line.push(b'\n');
        let line = trace.mask.masks().apply_json_line(line, Scope::KeptOutput);
        trace.work.count(Row::InterceptedEntries);
        self.trace.written(Log::Requests, line).await
    }
}

/// The writer's state, on its actor: each file's buffered handle, opened at
/// its first entry.
struct Files {
    folder: PathBuf,
    work: CountedWork,
    /// Whether the trace's folder was created.
    made: bool,
    /// Whether its creation was tried.
    folder_tried: bool,
    /// Whether each file's open was tried.
    file_tried: [bool; Log::COUNT],
    held: [Option<BufWriter<File>>; Log::COUNT],
}

impl Files {
    fn new(run_folder: &Path, work: CountedWork) -> Self {
        Self {
            folder: run_folder.to_owned(),
            work,
            made: false,
            folder_tried: false,
            file_tried: [false; Log::COUNT],
            held: [const { None }; Log::COUNT],
        }
    }

    /// Writes `group`, what the mailbox held, in order: each line into its
    /// file's buffer, then each file it touched in one `write`, then tells
    /// each waiting sender how its line went. A file that failed is closed,
    /// and the next entry opens it again.
    fn write(&mut self, group: Vec<Entry>) {
        let mut failed: [Option<io::ErrorKind>; Log::COUNT] = [None; Log::COUNT];
        let mut touched = [false; Log::COUNT];
        let mut waiting = Vec::new();
        for Entry { log, line, written } in group {
            let at = log.at();
            touched[at] = true;
            if let Err(error) = self.append(log, &line) {
                tracing::warn!(%error, file = log.file(), "the Run trace cannot be written");
                failed[at].get_or_insert(error.kind());
            }
            if let Some(written) = written {
                waiting.push((at, written));
            }
        }
        for log in Log::ALL {
            let at = log.at();
            if !touched[at] {
                continue;
            }
            if let Some(file) = &mut self.held[at]
                && let Err(error) = file.flush()
            {
                tracing::warn!(%error, file = log.file(), "the Run trace cannot be written");
                failed[at].get_or_insert(error.kind());
            }
        }
        for (at, kind) in failed.iter().enumerate() {
            if kind.is_some()
                && let Some(file) = self.held[at].take()
            {
                // Closed with what its buffer holds unwritten, so a line
                // reported failed is never written later and the next
                // entry opens the file again.
                drop(file.into_parts());
            }
        }
        for (at, written) in waiting {
            let _ = written.send(failed[at].map_or(Ok(()), |kind| {
                Err(io::Error::new(kind, "the Run trace cannot be written"))
            }));
        }
    }

    /// Appends `line` to `log`'s buffer, opening its file and creating the
    /// trace's folder first when no entry has: the Run's work, once each,
    /// counted on `E5.run_file_opens`, and the entry's on `E5.file_opens`
    /// when it tries again what failed.
    fn append(&mut self, log: Log, line: &[u8]) -> io::Result<()> {
        let at = log.at();
        if let Some(file) = &mut self.held[at] {
            return file.write_all(line);
        }
        let path = self.folder.join(log.file());
        if !self.made {
            if let Some(folder) = path.parent() {
                self.work
                    .create_dir_all(opening(&mut self.folder_tried), folder)?;
            }
            self.made = true;
        }
        let file = self.work.open_file(
            opening(&mut self.file_tried[at]),
            OpenOptions::new().create(true).append(true),
            &path,
        )?;
        self.held[at]
            .insert(BufWriter::with_capacity(BUFFER, file))
            .write_all(line)
    }
}

/// The row an open or a folder's creation counts on: its first try is the
/// Run's, on `E5.run_file_opens`, and one after the file or folder failed,
/// at its open or a write, is the entry's that tries again, on
/// `E5.file_opens`.
fn opening(tried: &mut bool) -> Row {
    if std::mem::replace(tried, true) {
        Row::RunTraceFileOpens
    } else {
        Row::RunTraceRunFileOpens
    }
}

#[cfg(test)]
mod tests;
