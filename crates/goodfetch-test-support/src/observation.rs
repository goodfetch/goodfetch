//! The Observation log (ADR 0060): the ordered plain records of one
//! Simulation or fault test, which the invariant checkers judge. It holds
//! values only, keeps security values redacted, and is written as NDJSON in
//! the test's folder when the test fails, apart from `tracing` output and
//! TLA+ traces.
#![expect(
    clippy::disallowed_methods,
    reason = "test support writes a failed test's log outside any async worker (ADR 0062)"
)]

use std::fmt::{self, Write as _};
use std::io::{self, Write as _};
use std::path::PathBuf;

use serde::{Serialize, Serializer};
use sha2::{Digest as _, Sha256};

use crate::env::TestEnv;

/// The file a failed test's log is written to, in its folder.
pub const LOG_FILE: &str = "observation-log.ndjson";

/// What a redacted security value is written as.
pub const REDACTED: &str = "[redacted]";

/// A lease, run or lead epoch (ADR 0003): a later copy holds a greater one.
pub type Epoch = u64;

/// One test's Observation log: its records in the order they happened.
/// Made with [`ObservationLog::for_test`], it writes itself as NDJSON in the
/// test's folder when the test fails, and lists that file on stderr as
/// `goodfetch-observation-log=<path>`.
pub struct ObservationLog<R: Serialize> {
    records: Vec<Record<R>>,
    on_failure: Option<PathBuf>,
}

impl<R: Serialize> ObservationLog<R> {
    /// An empty log, written nowhere.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            on_failure: None,
        }
    }

    /// An empty log, written to [`LOG_FILE`] in the test's folder if the
    /// test fails. The folder is kept then, so the file stays.
    pub fn for_test(env: &TestEnv) -> Self {
        Self {
            records: Vec::new(),
            on_failure: Some(env.folder().join(LOG_FILE)),
        }
    }

    /// Adds a record after the others.
    pub fn push(&mut self, record: Record<R>) {
        self.records.push(record);
    }

    /// The records, in order.
    pub fn records(&self) -> &[Record<R>] {
        &self.records
    }

    /// Writes one JSON object per record, one per line.
    pub fn write_ndjson(&self, writer: &mut impl io::Write) -> io::Result<()> {
        for record in &self.records {
            serde_json::to_writer(&mut *writer, record)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    }
}

impl<R: Serialize> Default for ObservationLog<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Serialize> Drop for ObservationLog<R> {
    fn drop(&mut self) {
        let Some(path) = &self.on_failure else {
            return;
        };
        if !std::thread::panicking() {
            return;
        }
        let written = std::fs::File::create(path).and_then(|file| {
            let mut file = io::BufWriter::new(file);
            self.write_ndjson(&mut file)?;
            file.flush()
        });
        #[expect(
            clippy::print_stderr,
            reason = "a failed test lists its Observation log in its output (ADR 0060)"
        )]
        match written {
            Ok(()) => eprintln!("goodfetch-observation-log={}", path.display()),
            Err(error) => eprintln!(
                "the Observation log cannot be written to {}: {error}",
                path.display()
            ),
        }
    }
}

/// One plain record of an Observation log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Record<R> {
    /// When it happened, in simulated milliseconds. A real-process test
    /// records none, since real time would differ between runs of a seed.
    pub at: Option<u64>,
    /// The host it happened on: a simulated host's name, or the program of
    /// a real process.
    pub host: String,
    /// What happened.
    pub operation: Operation<R>,
}

impl<R> Record<R> {
    /// A record of this operation on this host, at no simulated time.
    pub fn new(host: impl Into<String>, operation: Operation<R>) -> Self {
        Self {
            at: None,
            host: host.into(),
            operation,
        }
    }

    /// The same record at this simulated millisecond.
    #[must_use]
    pub fn at(mut self, milliseconds: u64) -> Self {
        self.at = Some(milliseconds);
        self
    }
}

/// An operation, answer or fault the checkers read, or one of the test's
/// own (`Other`), which they skip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation<R> {
    /// `epoch` became current: a write under an earlier epoch is stale from
    /// here on.
    Fence {
        /// The new epoch.
        epoch: Epoch,
    },
    /// A copy holding `epoch` wrote a record to the destination.
    Write {
        /// The writer's epoch.
        epoch: Epoch,
        /// The record's key.
        key: String,
        /// Its value's digest.
        digest: ValueDigest,
    },
    /// The run acknowledged a record as committed.
    Commit {
        /// The record's key.
        key: String,
        /// Its value's digest.
        digest: ValueDigest,
    },
    /// The destination, read back, holds a record written under `epoch`:
    /// one per copy it holds.
    Visible {
        /// The epoch it was written under.
        epoch: Epoch,
        /// The record's key.
        key: String,
        /// Its value's digest.
        digest: ValueDigest,
    },
    /// A Limit of `usage` on a scope, replacing any earlier one.
    Limit {
        /// The scope, with its Usage kind (`install/requests`).
        scope: String,
        /// The most Usage it allows.
        usage: u64,
    },
    /// Usage reserved against a scope: a slice taken, or a reserved cost.
    Reserve {
        /// The scope.
        scope: String,
        /// The Usage reserved.
        usage: u64,
    },
    /// Reserved Usage settled: `reserved` returned, and `charged` charged in
    /// its place (a slice returned or reclaimed, or Usage charged alone).
    Settle {
        /// The scope.
        scope: String,
        /// The Usage no longer reserved.
        reserved: u64,
        /// The Usage charged.
        charged: u64,
    },
    /// A fault the test injected.
    Fault(Fault),
    /// One of the test's own operations, which no checker reads.
    Other(R),
}

/// A fault a test injected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fault {
    /// A real process was killed (`kill -9`, `TerminateProcess`) at the
    /// `occurrence`-th protocol message naming `event`.
    Kill {
        /// The protocol event: the message's JSON-RPC `method`.
        event: String,
        /// Which occurrence, from 1.
        occurrence: u32,
    },
    /// A process crash: unsynced writes kept, as the crash VFS's
    /// `CrashKind::Process` gives.
    Crash,
    /// A power cut: unsynced writes dropped, torn or reordered, as the crash
    /// VFS's `CrashKind::PowerCut` and LazyFS's `clear_cache` give.
    PowerCut,
}

/// The SHA-256 digest of a value, in lowercase hexadecimal, so a record
/// holds no value itself.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ValueDigest(String);

impl ValueDigest {
    /// The digest of these bytes.
    pub fn of(value: impl AsRef<[u8]>) -> Self {
        let digest = Sha256::digest(value.as_ref());
        let mut hexadecimal = String::with_capacity(2 * digest.len());
        for byte in digest {
            let _ = write!(hexadecimal, "{byte:02x}");
        }
        Self(hexadecimal)
    }

    /// The hexadecimal digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A security value (a key, token or certificate) in a test's own record.
/// It is written and printed as [`REDACTED`], and any two are equal, since
/// they come from the OS generator and differ between runs of one seed
/// (ADR 0060).
#[derive(Clone)]
pub struct Redacted<T>(T);

impl<T> Redacted<T> {
    /// Wraps a security value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The value, for the test's own use; never for a record.
    pub fn value(&self) -> &T {
        &self.0
    }
}

impl<T> PartialEq for Redacted<T> {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl<T> Eq for Redacted<T> {}

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

impl<T> Serialize for Redacted<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(REDACTED)
    }
}

#[cfg(test)]
mod tests;
