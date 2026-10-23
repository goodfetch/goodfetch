//! The CDP command log the Run trace keeps (spec #577): one JSON line per
//! command the engine sends a browser, its method only, never its params,
//! in `trace/cdp-commands.jsonl` of the Run's folder. A blocking actor of
//! the Run's owner appends the lines, a group at a time.
#![expect(
    clippy::disallowed_methods,
    reason = "the log's actor writes its file on the tasks module's dedicated thread, never on an async worker"
)]

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_engine::tasks::TaskOwner;
use tokio::sync::mpsc;

/// The log's file in the Run's folder.
pub(crate) const FILE: &str = "trace/cdp-commands.jsonl";

/// How many methods wait for the writer before a sender waits.
const QUEUE: usize = 1024;

/// A handle that records a command's method.
#[derive(Clone, Debug)]
pub(crate) struct CommandLog {
    methods: mpsc::Sender<&'static str>,
}

impl CommandLog {
    /// The log of the Run whose folder is `run_folder`, written by an actor
    /// of `owner`, which waits for it once every handle is gone.
    pub(crate) fn start(owner: &TaskOwner, run_folder: &Path) -> io::Result<Self> {
        let (methods, mailbox) = mpsc::channel(QUEUE);
        let file: PathBuf = run_folder.join(FILE);
        owner.actor("cdp command log", mailbox, move |methods: Vec<&'static str>| {
            if let Err(error) = append(&file, &methods) {
                tracing::warn!(%error, file = %file.display(), "the CDP command log cannot be written");
            }
        })?;
        Ok(Self { methods })
    }

    /// Records that `method` is being sent.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may lose the line.
    pub(crate) async fn record(&self, method: &'static str) {
        let _ = self.methods.send(method).await;
    }
}

fn append(file: &Path, methods: &[&'static str]) -> io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut lines = Vec::new();
    for method in methods {
        serde_json::to_writer(&mut lines, &serde_json::json!({ "method": method }))?;
        lines.push(b'\n');
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?
        .write_all(&lines)
}
