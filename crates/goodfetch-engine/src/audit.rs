//! The Audit log (ADRs 0016, 0055, 0066 and 0098): one per-user SQLite
//! store in Goodfetch home, outside the per-major folders, to which every
//! engine of the user appends, whatever its major or Product, through
//! [`AuditLog::record`]: every area records its security events there, by
//! a kind `goodfetch-protocol` registers. It is append-only: nothing in the
//! engine updates or deletes an entry, and the store's own triggers refuse
//! both to any connection that has not dropped them.
//!
//! Each entry is chained to the one before it, its hash being
//! `SHA-256(previous hash ‖ RFC 8785 canonical JSON of the entry)`, the
//! first one's previous hash 32 zero bytes; the store keeps each entry as
//! that canonical text beside its hash, so an engine verifies entries of a
//! newer minor whose fields it does not know. After each append, the chain's
//! head (the last sequence and its hash) is written to a head file beside
//! the store, so a truncated tail fails [`AuditLog::verify`]. A head file
//! behind the store, as one engine leaves it when another appended between
//! its commit and its head write, still verifies.
//!
//! The store opens through a VFS named by a value, none in production, and
//! is of the Usage ledger's family: its major never changes and a newer
//! minor only adds. Its calls block: callers run them under their owner's
//! blocking policy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_protocol::audit::{Actor, AuditEntry, AuditVerified};
use goodfetch_protocol::catalog::{SharedType, SharedTypeValue};
use goodfetch_protocol::formats::{AUDIT_LOG, Compatibility};
use goodfetch_protocol::run::{PointInTime, RunId};
use goodfetch_protocol::settings::SettingsLayer;
use jiff::Timestamp;
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::home::Home;

/// How long an append waits for another engine's.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The previous hash of the first entry.
const NO_PREVIOUS: [u8; 32] = [0; 32];

/// The tables and triggers of this version. A newer minor only adds tables
/// and columns, each column with a default, so these statements keep
/// working on its files.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT",
    "CREATE TABLE IF NOT EXISTS entries (
        sequence INTEGER NOT NULL PRIMARY KEY,
        entry TEXT NOT NULL,
        hash TEXT NOT NULL
    ) STRICT",
    "CREATE TRIGGER IF NOT EXISTS entries_never_updated BEFORE UPDATE ON entries
        BEGIN SELECT RAISE(ABORT, 'the Audit log is append-only'); END",
    "CREATE TRIGGER IF NOT EXISTS entries_never_deleted BEFORE DELETE ON entries
        BEGIN SELECT RAISE(ABORT, 'the Audit log is append-only'); END",
];

/// Why the Audit log could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AuditError {
    /// SQLite failed; the append committed nothing.
    #[error("the Audit log at `{path}`: {message}")]
    Sqlite {
        /// The store's file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// The head file could not be written.
    #[error("the Audit log's head file at `{path}`: {message}")]
    Head {
        /// The head file.
        path: PathBuf,
        /// The OS's message.
        message: String,
    },
    /// The store is of another major of the format (ADR 0030).
    #[error("the Audit log is {found}, which this engine cannot read")]
    Format {
        /// The store's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
}

/// Where an entry's event came from: who did it, the settings layer it
/// came through and the Run it happened in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// Who did it.
    pub actor: Actor,
    /// The settings layer it came through, if one applies.
    pub layer: Option<SettingsLayer>,
    /// The Run it happened in, if any.
    pub run_id: Option<RunId>,
}

/// The head file's content: the last sequence and its hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Head {
    sequence: u64,
    hash: String,
}

/// The Audit log of one user: its store, its head file and the VFS the
/// store opens through.
#[derive(Clone, Debug)]
pub struct AuditLog {
    path: PathBuf,
    head: PathBuf,
    vfs: Option<String>,
}

impl AuditLog {
    /// The Audit log of `home`, opened through the SQLite VFS named `vfs`
    /// (none in production, ADR 0060).
    pub fn of(home: &Home, vfs: Option<String>) -> Self {
        Self {
            path: home.audit_log(),
            head: home.audit_log_head(),
            vfs,
        }
    }

    /// Its store's file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Its head file.
    pub fn head_path(&self) -> &Path {
        &self.head
    }

    /// Appends an entry of `kind`, a value of the shared type `audit_kind`,
    /// holding `details`, from `origin`, recorded at `now`, chained
    /// to the last entry, then writes the head file; answers the entry.
    ///
    /// # Errors
    ///
    /// When the store or the head file cannot be written, or the store is
    /// of another major.
    pub fn record(
        &self,
        origin: &Origin,
        kind: &SharedTypeValue,
        details: Value,
        now: Timestamp,
    ) -> Result<AuditEntry, AuditError> {
        debug_assert_eq!(kind.shared_type, SharedType::AuditKind);
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed(&error))?;
        let last: Option<(i64, String)> = transaction
            .query_row(
                "SELECT sequence, hash FROM entries ORDER BY sequence DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|error| failed(&error))?;
        let (sequence, previous) = match last {
            Some((sequence, hash)) => (
                u64::try_from(sequence).unwrap_or(0) + 1,
                decode(&hash).ok_or_else(|| failed(&"the last entry's hash is no SHA-256"))?,
            ),
            None => (1, NO_PREVIOUS),
        };
        let entry = AuditEntry {
            sequence,
            time: PointInTime::from_rfc3339(now.to_string()),
            kind: kind.name.to_owned(),
            actor: origin.actor.clone(),
            layer: origin.layer,
            run_id: origin.run_id.clone(),
            details,
        };
        let canonical =
            serde_json_canonicalizer::to_string(&entry).map_err(|error| failed(&error))?;
        let hash = chained(&previous, &canonical);
        transaction
            .execute(
                "INSERT INTO entries (sequence, entry, hash) VALUES (?1, ?2, ?3)",
                (stored(sequence), &canonical, &hash),
            )
            .map_err(|error| failed(&error))?;
        transaction.commit().map_err(|error| failed(&error))?;
        self.write_head(&Head { sequence, hash })?;
        Ok(entry)
    }

    /// Verifies the chain from its first entry and the head file against
    /// its last: intact, or the first sequence that does not verify. A log
    /// never written verifies intact and empty, and is not created.
    ///
    /// # Errors
    ///
    /// When the store cannot be read, or is of another major.
    pub fn verify(&self) -> Result<AuditVerified, AuditError> {
        if !self.path.exists() {
            let first_broken = self.read_head().map(|_| 1);
            return Ok(AuditVerified {
                intact: first_broken.is_none(),
                entries: 0,
                first_broken,
            });
        }
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let connection = self.open()?;
        let mut statement = connection
            .prepare("SELECT sequence, entry, hash FROM entries ORDER BY sequence")
            .map_err(|error| failed(&error))?;
        let mut rows = statement.query([]).map_err(|error| failed(&error))?;
        let (mut entries, mut first_broken) = (0, None);
        let (mut expected, mut previous) = (1, NO_PREVIOUS);
        while let Some(row) = rows.next().map_err(|error| failed(&error))? {
            entries += 1;
            if first_broken.is_some() {
                continue;
            }
            let sequence: i64 = row.get(0).map_err(|error| failed(&error))?;
            let entry: String = row.get(1).map_err(|error| failed(&error))?;
            let hash: String = row.get(2).map_err(|error| failed(&error))?;
            match chains(stored(expected), &previous, sequence, &entry, &hash) {
                Some(next) => {
                    previous = next;
                    expected += 1;
                }
                None => first_broken = Some(expected),
            }
        }
        let last = expected - 1;
        let first_broken = first_broken.or_else(|| match self.read_head() {
            Some(head) if head.sequence <= last => {
                let hash: Option<String> = connection
                    .query_row(
                        "SELECT hash FROM entries WHERE sequence = ?1",
                        [stored(head.sequence)],
                        |row| row.get(0),
                    )
                    .optional()
                    .ok()
                    .flatten();
                (hash.as_deref() != Some(head.hash.as_str())).then_some(head.sequence.max(1))
            }
            Some(_) => Some(last + 1),
            None => (last > 0).then_some(last + 1),
        });
        Ok(AuditVerified {
            intact: first_broken.is_none(),
            entries,
            first_broken,
        })
    }

    /// The head file's content; none when it is missing or unreadable.
    fn read_head(&self) -> Option<Head> {
        #[expect(
            clippy::disallowed_methods,
            reason = "the Audit log's calls run under their caller's blocking policy"
        )]
        let bytes = std::fs::read(&self.head).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Opens the store, creating it and its tables when it does not exist,
    /// and refuses one of another major.
    fn open(&self) -> Result<Connection, AuditError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        if let Some(folder) = self.path.parent() {
            #[expect(
                clippy::disallowed_methods,
                reason = "the Audit log's calls run under their caller's blocking policy"
            )]
            std::fs::create_dir_all(folder).map_err(|error| failed(&error))?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connection = match &self.vfs {
            Some(vfs) => Connection::open_with_flags_and_vfs(&self.path, flags, vfs.as_str()),
            None => Connection::open_with_flags(&self.path, flags),
        }
        .map_err(|error| failed(&error))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| failed(&error))?;
        connection
            .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
            .map_err(|error| failed(&error))?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|error| failed(&error))?;
        if cfg!(target_os = "macos") {
            connection
                .pragma_update(None, "fullfsync", "ON")
                .map_err(|error| failed(&error))?;
        }
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed(&error))?;
        for statement in SCHEMA {
            transaction
                .execute(statement, [])
                .map_err(|error| failed(&error))?;
        }
        transaction
            .execute(
                "INSERT INTO format (name, version) VALUES (?1, ?2) ON CONFLICT (name) DO NOTHING",
                (AUDIT_LOG.name, AUDIT_LOG.version.to_string()),
            )
            .map_err(|error| failed(&error))?;
        let found: String = transaction
            .query_row(
                "SELECT version FROM format WHERE name = ?1",
                [AUDIT_LOG.name],
                |row| row.get(0),
            )
            .map_err(|error| failed(&error))?;
        transaction.commit().map_err(|error| failed(&error))?;
        check_format(&found)?;
        Ok(connection)
    }

    /// Replaces the head file with `head`, through a file renamed over it,
    /// so a reader never sees half of one.
    fn write_head(&self, head: &Head) -> Result<(), AuditError> {
        let failed = |error: &dyn std::fmt::Display| AuditError::Head {
            path: self.head.clone(),
            message: error.to_string(),
        };
        let folder = self.head.parent().unwrap_or_else(|| Path::new("."));
        let mut file = tempfile::NamedTempFile::new_in(folder).map_err(|error| failed(&error))?;
        serde_json::to_writer(&mut file, head).map_err(|error| failed(&error))?;
        file.as_file().sync_all().map_err(|error| failed(&error))?;
        file.persist(&self.head)
            .map_err(|error| failed(&error.error))?;
        Ok(())
    }

    fn failed(&self, error: &dyn std::fmt::Display) -> AuditError {
        AuditError::Sqlite {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }
}

/// The hash that chains an entry, whose canonical text is `canonical`, to
/// the entry before it, whose hash is `previous`, in lowercase hex.
fn chained(previous: &[u8; 32], canonical: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(previous);
    hasher.update(canonical.as_bytes());
    data_encoding::HEXLOWER.encode(&hasher.finalize())
}

/// Whether a row, stored at `sequence` with the text `entry` and the hash
/// `hash`, is the entry expected at `expected` after the one whose hash is
/// `previous`: its place, the sequence its text holds and its hash over the
/// text's canonical form all agree. Answers its hash's bytes when it is.
fn chains(
    expected: i64,
    previous: &[u8; 32],
    sequence: i64,
    entry: &str,
    hash: &str,
) -> Option<[u8; 32]> {
    if sequence != expected {
        return None;
    }
    let value: Value = serde_json::from_str(entry).ok()?;
    if value.get("sequence").and_then(Value::as_i64) != Some(expected) {
        return None;
    }
    let canonical = serde_json_canonicalizer::to_string(&value).ok()?;
    (chained(previous, &canonical) == hash)
        .then(|| decode(hash))
        .flatten()
}

/// A sequence as SQLite stores it.
fn stored(sequence: u64) -> i64 {
    i64::try_from(sequence).unwrap_or(i64::MAX)
}

/// A hash's 32 bytes from its lowercase hex; none for any other text.
fn decode(hash: &str) -> Option<[u8; 32]> {
    data_encoding::HEXLOWER
        .decode(hash.as_bytes())
        .ok()?
        .try_into()
        .ok()
}

/// Refuses a store of another major (ADR 0030): every minor of this one
/// opens.
fn check_format(found: &str) -> Result<(), AuditError> {
    let refused = |compatibility| AuditError::Format {
        found: found.to_owned(),
        compatibility,
    };
    let version = found
        .parse()
        .map_err(|_| refused(Compatibility::NewerMajor))?;
    match AUDIT_LOG.rule.compatibility(AUDIT_LOG.version, version) {
        Compatibility::Compatible => Ok(()),
        other => Err(refused(other)),
    }
}

#[cfg(test)]
mod tests;
