//! A view of a Run another program started (ADR 0102), such as a detached
//! one. A Connection's `open_run` opens it by the Run's ID with no round
//! trip. It takes no writer role, reads `progress()`, `report()`, `trace()`
//! and the Key-value store's reads (`get`, `get_with`, `get_bytes`, `keys`),
//! and closing it frees only the view: the Run goes on. Python's
//! `run_view.py` and TypeScript's `run-view.ts` are its twins.

use std::path::Path;

use goodfetch_protocol::run::{
    RECORDING_SAVE_AS, RUN_PROGRESS_READ, RUN_REPORT, RecordingSaveAsParams, RecordingSaved, RunId,
    RunParams, RunProgress, RunReport,
};
use goodfetch_protocol::run_trace::{RUN_TRACE, RunTraceResult};

use crate::answer::Answer;
use crate::call::Call;
use crate::connection::ClientConnection;
use crate::error::Error;
use crate::event_stream::EventStream;
use crate::host::Host;
use crate::key_value_store::KeyValueStoreView;
use crate::markers::warn_operation;
use crate::run::progress;
use crate::run_copies::RunTrace;

/// A Run named by its Run ID, opened as a view: one another program
/// started, such as a detached Run. It takes no writer role.
#[derive(Debug, Clone)]
#[must_use = "a view is the Run's ID; closing it frees only the view"]
pub struct RunView {
    host: Host<RunId>,
}

impl RunView {
    /// The Connection its calls go through.
    pub(crate) fn connection(&self) -> &ClientConnection {
        self.host.connection()
    }

    /// The Run's ID, which names it for good. Read without a round trip.
    pub fn id(&self) -> &RunId {
        self.host.id()
    }

    /// Reads a Run's progress: what it does and why it waits. Without `after`,
    /// it answers at once; given `after`, the position of the value the reader
    /// last read, it waits for a change and answers the latest value, so values
    /// in between are coalesced. The value whose `last` is true ends the
    /// sequence: the Run ended, and its `lifecycle` and `stop_reason` say how,
    /// or no engine works it (`writer: none`) until a start with its Run ID
    /// resumes it. A Run another engine works is read through that engine, the
    /// owner the Run index names, and one no engine works from its committed
    /// state; a read never takes the Run's writer lock. A Run ID the Run index
    /// does not list fails with `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn progress(&self) -> EventStream<Answer<RunProgress>> {
        warn_operation(RUN_PROGRESS_READ.name);
        EventStream::from_items(progress::Following::new(self.host.clone()))
    }

    /// Reads a Run's report: at once for a Run that ended, and at its end for
    /// a live one, which the read never ends. A Run another engine works is
    /// read through that engine, and one no engine works from its committed
    /// state, never taking its writer lock. A Run that has no live writer and
    /// has not ended fails with `run_interrupted`, whose `fix` names a start
    /// with its Run ID, and a Run ID the Run index does not list with
    /// `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn report(&self) -> Call<Answer<RunReport>, RunParams> {
        warn_operation(RUN_REPORT.name);
        self.host.call(
            RUN_REPORT.name,
            RunParams {
                run_id: self.host.id().clone(),
            },
        )
    }

    /// Reads a Run's trace copy as recorded so far, or null when the Run keeps
    /// no trace (ADRs 0102 and 0116). Returns only once the copy is whole,
    /// with its size and SHA-256 digest; a write the engine cannot make fails
    /// with `file_write_failed`, side `engine`, and leaves no unfinished file.
    /// Takes no writer lock, so a view reads a Run no program holds without
    /// resuming it. A Run ID the Run index does not list fails with
    /// `run_not_found`.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn trace(&self) -> Call<Option<RunTrace>, RunParams> {
        warn_operation(RUN_TRACE.name);
        let connection = self.host.connection().clone();
        self.host
            .call(
                RUN_TRACE.name,
                RunParams {
                    run_id: self.host.id().clone(),
                },
            )
            .map(move |result: RunTraceResult| {
                result
                    .trace
                    .map(|descriptor| RunTrace::new(connection, descriptor))
            })
    }

    /// Read-only view of the Run's Key-value store (ADRs 0102 and 0103):
    /// `get`, `get_with`, `get_bytes` and `keys`, taking no writer lock.
    #[must_use]
    pub fn key_value_store(&self) -> KeyValueStoreView {
        KeyValueStoreView::new(self.host.clone())
    }

    /// Saves a Run's recorded Response recording as a WARC 1.1 file, whole or
    /// not at all (ADRs 0105 and 0116). An Attached engine given the program's
    /// path writes it through a hidden temporary file beside it, synced and
    /// renamed over it, creating missing folders and replacing a file there;
    /// any other engine writes nothing on its own machine and sends the file
    /// in `$/chunk` notifications of at most 64 KiB, one per credit granted
    /// with `$/credit`, for the SDK to write. A Run that recorded no responses
    /// raises `recording_invalid`. A path that cannot be written raises
    /// `path_not_writable`, and full or failing storage `file_write_failed`,
    /// leaving the path as it was.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn save_recording(
        &self,
        path: impl AsRef<Path>,
    ) -> crate::connection::SaveFile<RecordingSaved> {
        warn_operation(RECORDING_SAVE_AS.name);
        let path = path.as_ref();
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self.host.connection().save_file(
            RECORDING_SAVE_AS.name,
            &RecordingSaveAsParams {
                run_id: self.host.id().clone(),
                path: None,
                filename,
            },
            path,
            "save_recording",
        )
    }

    /// Closes the view. Frees only the view: the Run goes on.
    pub fn close(self) {}
}

impl ClientConnection {
    /// Opens a view of the Run `run_id` names, one another program started
    /// such as a detached Run, with no round trip (ADR 0102). The view takes
    /// no writer role, and closing it frees only the view.
    ///
    /// # Errors
    ///
    /// `closed` once the Connection is closed, and `connection_forked` in
    /// another process than the one that opened it.
    pub fn open_run(&self, run_id: &RunId) -> Result<RunView, Error> {
        self.ensure_opener()?;
        if self.is_closed() {
            return Err(Error::closed());
        }
        Ok(RunView {
            host: Host::view(self.clone(), run_id.clone()),
        })
    }
}

/// C-SEND-SYNC: a view moves between threads, is shared between them and
/// outlives the borrow that made it (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<RunView>();
};
