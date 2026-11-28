//! A Run's Checkpoint and `RunTrace` copies (ADRs 0102 and 0116):
//! [`Run::checkpoint`](crate::Run::checkpoint) answers a [`Checkpoint`], and
//! [`Run::trace`](crate::Run::trace) / [`RunView::trace`](crate::RunView::trace)
//! answers an `Option<RunTrace>`, only once the engine's file is whole.
//! `save_as(path)` writes the file on the program's machine whole or not at all
//! through a hidden temporary file, and `stream()` yields its bytes in order.

use std::path::Path;
use std::time::SystemTime;

use goodfetch_protocol::checkpoint::{
    CHECKPOINT_SAVE_AS, CHECKPOINT_STREAM, CheckpointSaveAsParams, CheckpointSaved,
    CheckpointStreamParams,
};
use goodfetch_protocol::run::{Checkpoint as WireCheckpoint, RunId};
use goodfetch_protocol::run_trace::{
    RunTrace as WireRunTrace, TRACE_SAVE_AS, TRACE_STREAM, TraceSaveAsParams, TraceSaved,
    TraceStreamParams,
};

use crate::connection::{BlockingRuntime, Core, SaveFile};
use crate::event_stream::EventStream;
use crate::markers::warn_operation;

/// A completed Checkpoint of a Run (ADRs 0002, 0102 and 0116): its descriptor
/// with [`Checkpoint::save_as`] to write the file on the program's machine
/// whole or not at all and [`Checkpoint::stream`] to read its bytes in order.
#[derive(Clone, Debug)]
pub struct Checkpoint {
    connection: Core,
    descriptor: WireCheckpoint,
}

impl Checkpoint {
    /// Wraps a completed wire descriptor with the connection core that serves its
    /// `checkpoint.save_as` and `checkpoint.stream` calls.
    pub(crate) fn new(connection: Core, descriptor: WireCheckpoint) -> Self {
        Self {
            connection,
            descriptor,
        }
    }

    /// Streams this Checkpoint's bytes and hands the stream and runtime to `wrap`
    /// (for the blocking face without exposing the core field).
    pub(crate) fn stream_with<R>(
        &self,
        wrap: impl FnOnce(EventStream<Vec<u8>>, BlockingRuntime) -> R,
    ) -> R {
        wrap(self.stream(), self.connection.runtime())
    }

    /// The Run's ID.
    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.descriptor.run_id
    }

    /// Where it is: the file's path on the engine's machine.
    #[must_use]
    pub fn location(&self) -> &str {
        &self.descriptor.location
    }

    /// Its size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.descriptor.size
    }

    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.descriptor.digest
    }

    /// The run epoch of the writer that took it (ADR 0003).
    #[must_use]
    pub fn run_epoch(&self) -> u64 {
        self.descriptor.run_epoch
    }

    /// The position of the run's journal it holds.
    #[must_use]
    pub fn journal_position(&self) -> u64 {
        self.descriptor.journal_position
    }

    /// When it was taken.
    #[must_use]
    pub fn time(&self) -> SystemTime {
        self.descriptor.time
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
        warn_operation(CHECKPOINT_SAVE_AS.name);
        self.connection.save_file(
            CHECKPOINT_SAVE_AS.name,
            &CheckpointSaveAsParams {
                run_id: self.descriptor.run_id.clone(),
                location: self.descriptor.location.clone(),
                path: None,
            },
            path.as_ref(),
            "save_as",
        )
    }

    /// Sends a Checkpoint's bytes in order, in `$/chunk` notifications of at
    /// most 64 KiB, one per credit granted with `$/credit`, and answers its
    /// size, its SHA-256 digest and its run epoch once every byte is sent
    /// (ADRs 0102 and 0116). Nothing is written on either machine: a program
    /// that writes the bytes itself meets its own language's file errors.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn stream(&self) -> EventStream<Vec<u8>> {
        warn_operation(CHECKPOINT_STREAM.name);
        self.connection.stream_chunks(
            CHECKPOINT_STREAM.name,
            &CheckpointStreamParams {
                run_id: self.descriptor.run_id.clone(),
                location: self.descriptor.location.clone(),
            },
        )
    }
}

/// A Run's trace copy (ADRs 0102 and 0116): its descriptor with
/// [`RunTrace::save_as`] to write the file on the program's machine whole or
/// not at all and [`RunTrace::stream`] to read its bytes in order.
#[derive(Clone, Debug)]
pub struct RunTrace {
    connection: Core,
    descriptor: WireRunTrace,
}

impl RunTrace {
    /// Wraps a completed wire descriptor with the connection core that serves its
    /// `trace.save_as` and `trace.stream` calls.
    pub(crate) fn new(connection: Core, descriptor: WireRunTrace) -> Self {
        Self {
            connection,
            descriptor,
        }
    }

    /// Streams this `RunTrace`'s bytes and hands the stream and runtime to `wrap`
    /// (for the blocking face without exposing the core field).
    pub(crate) fn stream_with<R>(
        &self,
        wrap: impl FnOnce(EventStream<Vec<u8>>, BlockingRuntime) -> R,
    ) -> R {
        wrap(self.stream(), self.connection.runtime())
    }

    /// The Run's ID.
    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.descriptor.run_id
    }

    /// Where it is: the trace file's path on the engine's machine.
    #[must_use]
    pub fn location(&self) -> &str {
        &self.descriptor.location
    }

    /// Its size in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.descriptor.size
    }

    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.descriptor.digest
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
        warn_operation(TRACE_SAVE_AS.name);
        self.connection.save_file(
            TRACE_SAVE_AS.name,
            &TraceSaveAsParams {
                run_id: self.descriptor.run_id.clone(),
                location: self.descriptor.location.clone(),
                path: None,
            },
            path.as_ref(),
            "save_as",
        )
    }

    /// Sends a Run trace's bytes in order, in `$/chunk` notifications of at
    /// most 64 KiB, one per credit granted with `$/credit`, and answers its
    /// size and its SHA-256 digest once every byte is sent (ADRs 0102 and
    /// 0116). Nothing is written on either machine: a program that writes the
    /// bytes itself meets its own language's file errors.
    ///
    /// Experimental: outside semver until it is promoted (ADR 0030).
    pub fn stream(&self) -> EventStream<Vec<u8>> {
        warn_operation(TRACE_STREAM.name);
        self.connection.stream_chunks(
            TRACE_STREAM.name,
            &TraceStreamParams {
                run_id: self.descriptor.run_id.clone(),
                location: self.descriptor.location.clone(),
            },
        )
    }
}

/// C-SEND-SYNC: copies move between threads and are shared between them (ADR 0081).
const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<Checkpoint>();
    send_sync_static::<RunTrace>();
};
