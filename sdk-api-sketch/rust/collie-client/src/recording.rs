//! Response recordings (ADR 0105, decided in #482).
//!
//! A Run's start may name a Response recording: a WARC 1.1 file on the program's machine, which
//! the SDK reads and writes there, as it does a Plan file (ADR 0091), so a Daemon or Goodfetch
//! Cloud works the same.
//!
//! - Replay: every request the Run would send to an address a page can reach (ADR 0017's first
//!   class: crawl requests, navigations, browser subresources, `page.request`, robots.txt) is
//!   answered from the file and never sent. One the file lacks is refused: a call the program made
//!   fails with `response_not_recorded`, a crawl request ends `failed` with it, and a subresource
//!   fails in its page as a network error. Every miss is listed in the Run report. A host the
//!   `resolve` setting maps to loopback (ADR 0059), such as a test's own server, is reached as
//!   usual; any other address is answered from the file. Requests match on method, URL, the
//!   canonical body's hash taken after Secrets are masked, and the `Vary` headers the file holds.
//! - Record: the Run fetches as usual, and the file is replaced, atomically, by every response it
//!   received when the Run ends. No credential is kept.
//!
//! The testing kit (`goodfetch_collie::testing`, `goodfetch_foxhound::testing`) replays by default. Model calls are never recorded: a
//! Plan replayed with `plan.frozen` or the kit's Scripted model stands in for the model.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A Response recording a Run's start names: replayed, or recorded into.
#[derive(Debug, Clone)]
pub struct Recording {
    _private: (),
}

impl Recording {
    /// Answers the Run's requests from this WARC file, relative to the working directory, and
    /// sends none.
    pub fn replay(path: impl AsRef<Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }

    /// Writes every response the Run received to this file when the Run ends, replacing it.
    pub fn record(path: impl AsRef<Path>) -> Self {
        let _ = path.as_ref();
        unimplemented!()
    }
}

/// Whether a Run replayed or recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecordingMode {
    /// Answered from the file.
    Replay,
    /// Written to the file.
    Record,
    /// A mode this SDK does not know, as received.
    Unknown(String),
}

/// A request a replaying Run did not find in its recording, and so never sent.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RecordingMiss {
    /// Its method.
    pub method: String,
    /// Its URL.
    pub url: String,
}

/// What a Run did with its Response recording, in its report.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RecordingReport {
    /// Replay or record.
    pub mode: RecordingMode,
    /// The file.
    pub path: PathBuf,
    /// Responses answered from the file, or written to it.
    pub responses: u64,
    /// When the responses used were first fetched: the recording's age. `None` when none was used.
    pub recorded_from: Option<SystemTime>,
    /// The latest of them.
    pub recorded_to: Option<SystemTime>,
    /// The engine that wrote the file.
    pub recorded_by: Option<String>,
    /// Every request not found, in order. Empty when recording.
    pub misses: Vec<RecordingMiss>,
}

/// Details of `response_not_recorded`, a refusal: the request was never sent. Not retryable.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResponseNotRecorded {
    /// Its method.
    pub method: String,
    /// Its URL.
    pub url: String,
    /// The recording.
    pub recording: PathBuf,
    /// Recorded URLs closest to it (same host and path), for a query string that changed.
    pub nearest: Vec<String>,
}

/// Details of `recording_invalid`: the file cannot be read, is not WARC, or names a WARC version
/// this engine does not read (ADR 0030).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RecordingInvalid {
    /// The recording.
    pub recording: PathBuf,
    /// What is wrong with it.
    pub reason: String,
}
