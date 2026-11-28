//! Blocking twin of [`crate::RunView`] (ADRs 0081 and 0102).

use std::path::Path;

use goodfetch_protocol::run::{RecordingSaved, RunId, RunParams, RunProgress, RunReport};

use super::{Call, EventIter, KeyValueStoreView, RunTrace, SaveFile};
use crate::answer::Answer;

/// A Run named by its Run ID, opened as a view, whose calls block the
/// calling thread.
#[derive(Clone, Debug)]
#[must_use = "a view is the Run's ID; closing it frees only the view"]
pub struct RunView {
    inner: crate::RunView,
}

impl RunView {
    /// The blocking face of `inner`.
    pub(crate) fn new(inner: crate::RunView) -> Self {
        Self { inner }
    }

    /// The Run's ID, which names it for good. Read without a round trip.
    pub fn id(&self) -> &RunId {
        self.inner.id()
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
    pub fn progress(&self) -> EventIter<Answer<RunProgress>> {
        EventIter::new(self.inner.progress(), self.inner.connection())
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
        Call::new(self.inner.report(), self.inner.connection())
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
        Call::new(
            self.inner.trace().map(|trace| trace.map(RunTrace::new)),
            self.inner.connection(),
        )
    }

    /// Read-only view of the Run's Key-value store, whose calls block.
    #[must_use]
    pub fn key_value_store(&self) -> KeyValueStoreView {
        KeyValueStoreView::from(self.inner.key_value_store())
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
    pub fn save_recording(&self, path: impl AsRef<Path>) -> SaveFile<RecordingSaved> {
        SaveFile::new(self.inner.save_recording(path), self.inner.connection())
    }

    /// Closes the view. Frees only the view: the Run goes on.
    pub fn close(self) {
        self.inner.close();
    }
}
