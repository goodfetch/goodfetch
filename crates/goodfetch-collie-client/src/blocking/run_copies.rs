//! Blocking twins of [`crate::Checkpoint`] and [`crate::RunTrace`] (ADRs 0081,
//! 0102 and 0116).

use std::path::Path;
use std::time::SystemTime;

use goodfetch_protocol::checkpoint::CheckpointSaved;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::run_trace::TraceSaved;

use super::{EventIter, SaveFile};

/// A completed Checkpoint of a Run, as [`crate::Checkpoint`], whose calls
/// block the calling thread.
#[derive(Clone, Debug)]
pub struct Checkpoint {
    inner: crate::Checkpoint,
}

impl Checkpoint {
    /// The blocking face of `inner`.
    pub(crate) fn new(inner: crate::Checkpoint) -> Self {
        Self { inner }
    }

    /// The Run's ID.
    #[must_use]
    pub fn run_id(&self) -> &RunId {
        self.inner.run_id()
    }

    /// Where it is: the file's path on the engine's machine.
    #[must_use]
    pub fn location(&self) -> &str {
        self.inner.location()
    }

    /// Its size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.inner.size()
    }

    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[must_use]
    pub fn digest(&self) -> &str {
        self.inner.digest()
    }

    /// The run epoch of the writer that took it (ADR 0003).
    #[must_use]
    pub fn run_epoch(&self) -> u64 {
        self.inner.run_epoch()
    }

    /// The position of the run's journal it holds.
    #[must_use]
    pub fn journal_position(&self) -> u64 {
        self.inner.journal_position()
    }

    /// When it was taken.
    #[must_use]
    pub fn time(&self) -> SystemTime {
        self.inner.time()
    }

    /// Saves a Checkpoint's file, which the engine keeps on its machine, whole
    /// or not at all (ADRs 0102 and 0116). An Attached engine given the
    /// program's path writes it through a hidden temporary file beside it,
    /// synced and renamed over it, creating missing folders and replacing a
    /// file there; any other engine writes nothing on its own machine and
    /// sends the file in `$/chunk` notifications of at most 64 KiB, one per
    /// credit granted with `$/credit`, for the SDK to write. Answers the
    /// file's size, its SHA-256 digest and its run epoch. A path that cannot
    /// be written raises `path_not_writable`, and full or failing storage
    /// `file_write_failed`, leaving the path as it was.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn save_as(&self, path: impl AsRef<Path>) -> SaveFile<CheckpointSaved> {
        SaveFile::new(self.inner.save_as(path), self.inner.connection())
    }

    /// Sends a Checkpoint's bytes in order, in `$/chunk` notifications of at
    /// most 64 KiB, one per credit granted with `$/credit`, and answers its
    /// size, its SHA-256 digest and its run epoch once every byte is sent
    /// (ADRs 0102 and 0116). Nothing is written on either machine: a program
    /// that writes the bytes itself meets its own language's file errors.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn stream(&self) -> EventIter<Vec<u8>> {
        EventIter::new(self.inner.stream(), self.inner.connection())
    }
}

/// A Run's trace copy, as [`crate::RunTrace`], whose calls block the calling
/// thread.
#[derive(Clone, Debug)]
pub struct RunTrace {
    inner: crate::RunTrace,
}

impl RunTrace {
    /// The blocking face of `inner`.
    pub(crate) fn new(inner: crate::RunTrace) -> Self {
        Self { inner }
    }

    /// The Run's ID.
    #[must_use]
    pub fn run_id(&self) -> &RunId {
        self.inner.run_id()
    }

    /// Where it is: the trace file's path on the engine's machine.
    #[must_use]
    pub fn location(&self) -> &str {
        self.inner.location()
    }

    /// Its size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.inner.size()
    }

    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[must_use]
    pub fn digest(&self) -> &str {
        self.inner.digest()
    }

    /// Saves a Run trace's file, which the engine keeps on its machine, whole
    /// or not at all (ADRs 0102 and 0116). An Attached engine given the
    /// program's path writes it through a hidden temporary file beside it,
    /// synced and renamed over it, creating missing folders and replacing a
    /// file there; any other engine writes nothing on its own machine and
    /// sends the file in `$/chunk` notifications of at most 64 KiB, one per
    /// credit granted with `$/credit`, for the SDK to write. Answers the
    /// file's size and its SHA-256 digest. A path that cannot be written
    /// raises `path_not_writable`, and full or failing storage
    /// `file_write_failed`, leaving the path as it was.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn save_as(&self, path: impl AsRef<Path>) -> SaveFile<TraceSaved> {
        SaveFile::new(self.inner.save_as(path), self.inner.connection())
    }

    /// Sends a Run trace's bytes in order, in `$/chunk` notifications of at
    /// most 64 KiB, one per credit granted with `$/credit`, and answers its
    /// size and its SHA-256 digest once every byte is sent (ADRs 0102 and
    /// 0116). Nothing is written on either machine: a program that writes the
    /// bytes itself meets its own language's file errors.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn stream(&self) -> EventIter<Vec<u8>> {
        EventIter::new(self.inner.stream(), self.inner.connection())
    }
}
