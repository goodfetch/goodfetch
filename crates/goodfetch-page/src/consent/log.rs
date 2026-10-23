//! The Run trace's Consent prompt log (ADR 0048): one JSON line per prompt
//! the runner refused, hid, left or failed to refuse, in
//! `trace/consent.jsonl` of the Run's folder, with the page, the document's
//! URL, the rule that matched and whether the outcome was verified, as
//! Provenance will record it per page.
#![expect(
    clippy::disallowed_methods,
    reason = "the log appends to its file through the owner's blocking policy, never on an async worker"
)]

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_engine::tasks::TaskOwner;
use goodfetch_protocol::page::PageId;

use super::messages::Outcome;

/// The log's file in the Run's folder.
pub(super) const FILE: &str = "trace/consent.jsonl";

/// One page's lines of the log.
pub(super) struct ConsentLog {
    file: Arc<Path>,
    owner: TaskOwner,
    page: PageId,
}

impl ConsentLog {
    /// The log of the page `page` of the Run whose folder is `run_folder`,
    /// written through `owner`'s blocking policy.
    pub(super) fn new(owner: TaskOwner, run_folder: &Path, page: PageId) -> Self {
        let file: PathBuf = run_folder.join(FILE);
        Self {
            file: file.into(),
            owner,
            page,
        }
    }

    /// Records `outcome`.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future never stops a line being written once it began.
    pub(super) async fn record(&self, outcome: &Outcome) -> io::Result<()> {
        let mut line = serde_json::to_vec(&serde_json::json!({
            "page_id": self.page,
            "url": outcome.url,
            "action": outcome.action,
            "rule": outcome.rule,
            "verified": outcome.verified,
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
