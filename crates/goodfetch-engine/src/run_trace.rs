//! The Run trace's request log (ADRs 0085 and 0093): one JSON line per
//! request event the engine records for a Run, in `trace/requests.jsonl`
//! of the Run's folder, each the event's catalog name and its data, as the
//! event stream would carry it. A request that failed while no call waited
//! for it, such as one the egress policy refused, is recorded here as
//! `page.request_failed`, so a refusal is never a timeout.
#![expect(
    clippy::disallowed_methods,
    reason = "the log appends to its file through the owner's blocking policy, never on an async worker"
)]

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_protocol::page::{PAGE_REQUEST_FAILED, RequestFailed};

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
        let mut line = serde_json::to_vec(&serde_json::json!({
            "event": PAGE_REQUEST_FAILED.name,
            "data": failed,
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
