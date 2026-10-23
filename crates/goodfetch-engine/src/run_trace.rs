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
//! (ADR 0121).
#![expect(
    clippy::disallowed_methods,
    reason = "the log appends to its file through the owner's blocking policy, never on an async worker"
)]

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_protocol::catalog::Event;
use goodfetch_protocol::page::events::PAGE_REQUEST_FAILED;
use goodfetch_protocol::page::{RequestFailed, SERVICE_WORKER_REQUEST, ServiceWorkerRequest};

use serde::Serialize;

use crate::tasks::TaskOwner;

/// The log's file in the Run's folder.
pub const FILE: &str = "trace/requests.jsonl";

/// A Run's request log.
#[derive(Debug)]
pub struct RequestLog {
    file: Arc<Path>,
    owner: TaskOwner,
}

impl RequestLog {
    /// The log of the Run whose folder is `run_folder`, written through
    /// `owner`'s blocking policy; its holder waits for `owner`.
    pub fn new(owner: TaskOwner, run_folder: &Path) -> Self {
        let file: PathBuf = run_folder.join(FILE);
        Self {
            file: file.into(),
            owner,
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
        let mut line = serde_json::to_vec(&serde_json::json!({
            "event": name,
            "data": data,
        }))?;
        line.push(b'\n');
        let file = Arc::clone(&self.file);
        self.owner.blocking(move || append(&file, &line)).await
    }
}

fn append(file: &Path, line: &[u8]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(line)
}
