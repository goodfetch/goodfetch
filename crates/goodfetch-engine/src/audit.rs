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

#![expect(
    clippy::disallowed_types,
    reason = "the set of unchained Runs is read or changed in one step, never across an `.await` or under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use goodfetch_protocol::audit::{
    AUDIT_LOG_ANCHOR, AUDIT_LOG_PRUNED, AUDIT_RETENTION, Actor, AuditEntry, AuditVerified,
    ExportedAuditEntry,
};
use goodfetch_protocol::catalog::{Product, SharedType, SharedTypeValue};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::formats::{AUDIT_LOG, Compatibility};
use goodfetch_protocol::run::{PointInTime, RunId};
use goodfetch_protocol::settings::SettingsLayer;
use jiff::{SignedDuration, Timestamp};
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::durable;
use crate::home::Home;
use crate::settings::{Layers, SettingsFiles, resolve};

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
    NEVER_DELETED,
];

/// The trigger that refuses a deletion, which a pruning alone drops and
/// creates again within its transaction.
const NEVER_DELETED: &str =
    "CREATE TRIGGER IF NOT EXISTS entries_never_deleted BEFORE DELETE ON entries
        BEGIN SELECT RAISE(ABORT, 'the Audit log is append-only'); END";

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
    /// The surface they did it through, if any.
    pub surface: Option<Surface>,
    /// The settings layer it came through, if one applies.
    pub layer: Option<SettingsLayer>,
    /// The Run it happened in, if any.
    pub run_id: Option<RunId>,
}

/// Where the engine also sends each entry it appends, its hash with it,
/// once the store holds it: the Audit log's OpenTelemetry events, which an
/// executable whose `audit.otlp` is on installs (ADR 0016). Sending never
/// blocks the append, and an entry it fails to send stays in the store.
#[derive(Clone)]
pub struct AuditEvents(Arc<dyn Fn(&ExportedAuditEntry) + Send + Sync>);

impl AuditEvents {
    /// Events that `send` sends, called with each new entry on the thread
    /// that appended it.
    pub fn new(send: impl Fn(&ExportedAuditEntry) + Send + Sync + 'static) -> Self {
        Self(Arc::new(send))
    }
}

impl std::fmt::Debug for AuditEvents {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AuditEvents")
    }
}

/// The head file's content: the last sequence and its hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Head {
    sequence: u64,
    hash: String,
}

/// The Audit log of one user: its store, its head file, the VFS the store
/// opens through, and the machine's settings files its `audit.retention`
/// is read from.
#[derive(Clone, Debug)]
pub struct AuditLog {
    path: PathBuf,
    head: PathBuf,
    vfs: Option<String>,
    retained_under: Option<(SettingsFiles, Product)>,
    events: Option<AuditEvents>,
    /// The Runs whose entries are written unchained, shared by its clones.
    unchained: Arc<Mutex<BTreeSet<RunId>>>,
}

impl AuditLog {
    /// The Audit log of `home`, opened through the SQLite VFS named `vfs`
    /// (none in production, ADR 0060).
    pub fn of(home: &Home, vfs: Option<String>) -> Self {
        Self {
            path: home.audit_log(),
            head: home.audit_log_head(),
            vfs,
            retained_under: None,
            events: None,
            unchained: Arc::default(),
        }
    }

    /// Writes every later entry of `run_id` unchained, as a Run whose
    /// `security.audit_chain` is off has them (ADR 0016), here and in every
    /// clone of this log, but those [`AuditLog::record_chained`] records;
    /// answers whether the Run was chained until now.
    pub fn unchain(&self, run_id: RunId) -> bool {
        self.unchained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(run_id)
    }

    /// Chains every later entry of `run_id` again; answers whether it was
    /// unchained.
    pub fn rechain(&self, run_id: &RunId) -> bool {
        self.unchained
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(run_id)
    }

    /// Whether the entries of `run_id` are written unchained.
    fn is_unchained(&self, run_id: Option<&RunId>) -> bool {
        run_id.is_some_and(|run_id| {
            self.unchained
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(run_id)
        })
    }

    /// The same, each entry it appends also sent to `events`.
    #[must_use]
    pub fn sending_to(mut self, events: Option<AuditEvents>) -> Self {
        self.events = events;
        self
    }

    /// The same, each append first pruning what the `audit.retention` of
    /// the machine's layers in `files` prunes, read then as an engine of
    /// `product` resolves it; without it, nothing is ever pruned but
    /// through [`AuditLog::prune`].
    #[must_use]
    pub fn retained_under(mut self, files: SettingsFiles, product: Product) -> Self {
        self.retained_under = Some((files, product));
        self
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
    /// holding `details`, from `origin`, recorded at `now`, chained to the
    /// last chained entry, or unchained when its Run is (see
    /// [`AuditLog::unchain`]), then writes the head file; answers the
    /// entry. Under an `audit.retention`, it first prunes what it prunes at
    /// `now`.
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
        let chained = !self.is_unchained(origin.run_id.as_ref());
        self.record_as(origin, kind, details, now, chained)
    }

    /// As [`AuditLog::record`], but chained whatever its Run: the record
    /// of the switch that unchains a Run is itself chained (ADR 0016).
    ///
    /// # Errors
    ///
    /// As [`AuditLog::record`].
    pub fn record_chained(
        &self,
        origin: &Origin,
        kind: &SharedTypeValue,
        details: Value,
        now: Timestamp,
    ) -> Result<AuditEntry, AuditError> {
        self.record_as(origin, kind, details, now, true)
    }

    fn record_as(
        &self,
        origin: &Origin,
        kind: &SharedTypeValue,
        details: Value,
        now: Timestamp,
        chained: bool,
    ) -> Result<AuditEntry, AuditError> {
        debug_assert_eq!(kind.shared_type, SharedType::AuditKind);
        if let Some((retention, layer)) = self.retention() {
            self.prune(retention, Some(layer), now)?;
        }
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed(&error))?;
        let (sequence, previous) = self.after_last(&transaction)?;
        let entry = AuditEntry {
            sequence,
            time: PointInTime::from_rfc3339(now.to_string()),
            kind: kind.name.to_owned(),
            actor: origin.actor.clone(),
            surface: origin.surface,
            layer: origin.layer,
            run_id: origin.run_id.clone(),
            details,
            chained,
        };
        let hash = self.append(&transaction, &previous, &entry)?;
        transaction.commit().map_err(|error| failed(&error))?;
        self.appended(entry, hash)
    }

    /// Prunes the entries older than `retention` at `now`, which `layer`
    /// set, from the log's beginning only: they give way to an anchor at
    /// the last pruned sequence, carrying its hash, and a pruning record
    /// naming it is chained at the tail. Answers the pruning record; none
    /// when no entry, the anchor aside, is that old.
    ///
    /// # Errors
    ///
    /// When the store or the head file cannot be written, or the store is
    /// of another major.
    pub fn prune(
        &self,
        retention: Duration,
        layer: Option<SettingsLayer>,
        now: Timestamp,
    ) -> Result<Option<AuditEntry>, AuditError> {
        let cutoff = SignedDuration::try_from(retention)
            .ok()
            .and_then(|retention| now.checked_sub(retention).ok());
        let Some(cutoff) = cutoff.filter(|_| self.path.exists()) else {
            return Ok(None);
        };
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed(&error))?;
        let Some((through, last_pruned_hash)) = self.older_than(&transaction, cutoff)? else {
            return Ok(None);
        };
        let (sequence, previous) = self.after_last(&transaction)?;
        let time = PointInTime::from_rfc3339(now.to_string());
        let anchor = anchor_entry(through, &last_pruned_hash, &time, layer);
        let anchor =
            serde_json_canonicalizer::to_string(&anchor).map_err(|error| failed(&error))?;
        transaction
            .execute_batch("DROP TRIGGER entries_never_deleted")
            .map_err(|error| failed(&error))?;
        transaction
            .execute(
                "DELETE FROM entries WHERE sequence <= ?1",
                [stored(through)],
            )
            .map_err(|error| failed(&error))?;
        transaction
            .execute(NEVER_DELETED, [])
            .map_err(|error| failed(&error))?;
        transaction
            .execute(
                "INSERT INTO entries (sequence, entry, hash) VALUES (?1, ?2, ?3)",
                (stored(through), &anchor, &last_pruned_hash),
            )
            .map_err(|error| failed(&error))?;
        let pruned = AuditEntry {
            sequence,
            time,
            kind: AUDIT_LOG_PRUNED.name.to_owned(),
            actor: Actor::Engine,
            surface: None,
            layer,
            run_id: None,
            details: json!({
                "through_sequence": through,
                "last_pruned_hash": last_pruned_hash,
                "retention": u64::try_from(retention.as_millis()).unwrap_or(u64::MAX),
            }),
            chained: true,
        };
        let hash = self.append(&transaction, &previous, &pruned)?;
        transaction.commit().map_err(|error| failed(&error))?;
        self.appended(pruned, hash).map(Some)
    }

    /// The `audit.retention` the machine's layers set now, with the layer
    /// that set it; none when unset, or when they cannot be read, which
    /// prunes nothing.
    fn retention(&self) -> Option<(Duration, SettingsLayer)> {
        let (files, product) = self.retained_under.as_ref()?;
        let machine = files
            .load(*product)
            .inspect_err(|error| {
                tracing::warn!(%error, "`audit.retention` cannot be read, so nothing is pruned");
            })
            .ok()?;
        let layers = Layers {
            operator_policy: machine.operator_policy,
            user_configuration: machine.user_configuration,
            ..Layers::default()
        };
        let resolved = resolve(&layers, *product, *product).ok()?;
        let retention = resolved.get(AUDIT_RETENTION.name)?;
        Some((
            Duration::from_millis(retention.value.as_u64()?),
            retention.layer,
        ))
    }

    /// Writes the head file after `entry`, committed with `hash`, sends it
    /// to the events, and answers it.
    fn appended(&self, entry: AuditEntry, hash: String) -> Result<AuditEntry, AuditError> {
        self.write_head(&Head {
            sequence: entry.sequence,
            hash: hash.clone(),
        })?;
        match &self.events {
            Some(events) => {
                let exported = ExportedAuditEntry { entry, hash };
                (events.0)(&exported);
                Ok(exported.entry)
            }
            None => Ok(entry),
        }
    }

    /// The sequence the next entry takes and the hash a chained one chains
    /// to: the last chained entry's, past the unchained ones after it.
    fn after_last(&self, transaction: &Transaction<'_>) -> Result<(u64, [u8; 32]), AuditError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut statement = transaction
            .prepare("SELECT sequence, entry, hash FROM entries ORDER BY sequence DESC")
            .map_err(|error| failed(&error))?;
        let mut rows = statement.query([]).map_err(|error| failed(&error))?;
        let mut next = None;
        while let Some(row) = rows.next().map_err(|error| failed(&error))? {
            let sequence: i64 = row.get(0).map_err(|error| failed(&error))?;
            let entry: String = row.get(1).map_err(|error| failed(&error))?;
            let hash: String = row.get(2).map_err(|error| failed(&error))?;
            let next = *next.get_or_insert(u64::try_from(sequence).unwrap_or(0) + 1);
            if !is_unchained(&entry) {
                let previous = decode(&hash)
                    .ok_or_else(|| failed(&"the last chained entry's hash is no SHA-256"))?;
                return Ok((next, previous));
            }
        }
        Ok((next.unwrap_or(1), NO_PREVIOUS))
    }

    /// Writes `entry`, chained to `previous`, and answers its hash.
    fn append(
        &self,
        transaction: &Transaction<'_>,
        previous: &[u8; 32],
        entry: &AuditEntry,
    ) -> Result<String, AuditError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let canonical =
            serde_json_canonicalizer::to_string(entry).map_err(|error| failed(&error))?;
        let hash = chained(
            if entry.chained {
                previous
            } else {
                &NO_PREVIOUS
            },
            &canonical,
        );
        transaction
            .execute(
                "INSERT INTO entries (sequence, entry, hash) VALUES (?1, ?2, ?3)",
                (stored(entry.sequence), &canonical, &hash),
            )
            .map_err(|error| failed(&error))?;
        Ok(hash)
    }

    /// The last sequence of the log's beginning whose entries, the anchor
    /// aside, were all recorded before `cutoff`, with its hash; none when
    /// the first is not that old.
    fn older_than(
        &self,
        transaction: &Transaction<'_>,
        cutoff: Timestamp,
    ) -> Result<Option<(u64, String)>, AuditError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut statement = transaction
            .prepare("SELECT sequence, entry, hash FROM entries ORDER BY sequence")
            .map_err(|error| failed(&error))?;
        let mut rows = statement.query([]).map_err(|error| failed(&error))?;
        let (mut through, mut first) = (None, true);
        // The hash the next chained entry chains to, which an anchor at the
        // last pruned sequence carries: an unchained entry leaves it.
        let mut chain = data_encoding::HEXLOWER.encode(&NO_PREVIOUS);
        while let Some(row) = rows.next().map_err(|error| failed(&error))? {
            let sequence: i64 = row.get(0).map_err(|error| failed(&error))?;
            let entry: String = row.get(1).map_err(|error| failed(&error))?;
            let hash: String = row.get(2).map_err(|error| failed(&error))?;
            let value: Value = serde_json::from_str(&entry).unwrap_or(Value::Null);
            if value["chained"] != Value::Bool(false) {
                chain = hash;
            }
            let is_anchor = value["kind"].as_str() == Some(AUDIT_LOG_ANCHOR.name);
            if std::mem::take(&mut first) && is_anchor {
                continue;
            }
            let time = value["time"]
                .as_str()
                .and_then(|time| time.parse::<Timestamp>().ok());
            match (time, u64::try_from(sequence)) {
                (Some(time), Ok(sequence)) if time < cutoff => {
                    through = Some((sequence, chain.clone()));
                }
                _ => break,
            }
        }
        Ok(through)
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
        let mut walk = Walk::new();
        while let Some(row) = rows.next().map_err(|error| failed(&error))? {
            let sequence: i64 = row.get(0).map_err(|error| failed(&error))?;
            let entry: String = row.get(1).map_err(|error| failed(&error))?;
            let hash: String = row.get(2).map_err(|error| failed(&error))?;
            walk.step(sequence, &entry, &hash);
        }
        let first = walk.first();
        let (entries, last, first_broken) = walk.end();
        let first_broken = first_broken.or_else(|| match self.read_head() {
            Some(head) if head.sequence < first => None,
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
            None => (entries > 0).then_some(last + 1),
        });
        Ok(AuditVerified {
            intact: first_broken.is_none(),
            entries,
            first_broken,
        })
    }

    /// Every entry in sequence with its hash.
    ///
    /// # Errors
    ///
    /// When the store cannot be read, or is of another major.
    pub fn export(&self) -> Result<Vec<ExportedAuditEntry>, AuditError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let connection = self.open()?;
        let mut statement = connection
            .prepare("SELECT sequence, entry, hash FROM entries ORDER BY sequence")
            .map_err(|error| failed(&error))?;
        let mut rows = statement.query([]).map_err(|error| failed(&error))?;
        let mut exported = Vec::new();
        while let Some(row) = rows.next().map_err(|error| failed(&error))? {
            let sequence: i64 = row.get(0).map_err(|error| failed(&error))?;
            let entry: String = row.get(1).map_err(|error| failed(&error))?;
            let hash: String = row.get(2).map_err(|error| failed(&error))?;
            let entry = serde_json::from_str(&entry).map_err(|error| {
                failed(&format!(
                    "the entry at sequence {sequence} is unreadable: {error}"
                ))
            })?;
            exported.push(ExportedAuditEntry { entry, hash });
        }
        Ok(exported)
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
        crate::sqlite_wal::enter(&connection, BUSY_TIMEOUT).map_err(|error| failed(&error))?;
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
    /// so a reader never sees half of one, and an entry recorded still
    /// verifies after a power cut (ADR 0002).
    fn write_head(&self, head: &Head) -> Result<(), AuditError> {
        let failed = |error: &dyn std::fmt::Display| AuditError::Head {
            path: self.head.clone(),
            message: error.to_string(),
        };
        let text = serde_json::to_vec(head).map_err(|error| failed(&error))?;
        durable::replace(&self.head, &text).map_err(|error| failed(&error))
    }

    fn failed(&self, error: &dyn std::fmt::Display) -> AuditError {
        AuditError::Sqlite {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }
}

/// Verifies an exported log, the JSONL `audit export` writes, which may
/// come from anywhere: each line an entry with its hash, chained to the
/// line before it from the first, or from the anchor the first line is.
/// Answers intact, or the first sequence that does not verify: a line that
/// is no entry breaks the chain at its place. An export has no head file,
/// so its tail is the last line it holds. Blank lines are skipped.
pub fn verify_exported(jsonl: &[u8]) -> AuditVerified {
    let mut walk = Walk::new();
    for line in jsonl.split(|byte| *byte == b'\n') {
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        match exported_line(line) {
            Some((sequence, entry, hash)) => walk.step(sequence, &entry, &hash),
            None => walk.step(-1, "", ""),
        }
    }
    let (entries, _, first_broken) = walk.end();
    AuditVerified {
        intact: first_broken.is_none(),
        entries,
        first_broken,
    }
}

/// The fuzz target `audit_jsonl`'s body (ADR 0067): any bytes read as an
/// exported Audit log, which may come from anywhere. It counts each line
/// that is not blank, answers intact exactly when no sequence broke, and
/// names a broken sequence from 1; it panics only on a bug.
///
/// # Panics
///
/// When the reader breaks one of those rules.
pub fn fuzz_audit_jsonl(data: &[u8]) {
    let verified = verify_exported(data);
    let lines = data
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.trim_ascii().is_empty())
        .count();
    assert_eq!(
        usize::try_from(verified.entries).ok(),
        Some(lines),
        "an export's entries are its lines that are not blank"
    );
    assert_eq!(
        verified.intact,
        verified.first_broken.is_none(),
        "an export is intact exactly when no sequence broke"
    );
    assert!(
        verified.first_broken.is_none_or(|sequence| sequence >= 1),
        "a broken sequence counts from 1"
    );
}

/// An exported line's sequence, its entry's text without its hash, and its
/// hash; none when the line is no JSON object holding both.
fn exported_line(line: &[u8]) -> Option<(i64, String, String)> {
    let Value::Object(mut entry) = serde_json::from_slice(line).ok()? else {
        return None;
    };
    let Value::String(hash) = entry.remove("hash")? else {
        return None;
    };
    let sequence = entry.get("sequence")?.as_i64()?;
    Some((sequence, Value::Object(entry).to_string(), hash))
}

/// A walk along a chain's entries in the order they are held, from its
/// first, which [`AuditLog::verify`] and [`verify_exported`] share.
struct Walk {
    /// How many entries it took.
    entries: u64,
    /// The sequence the next entry must hold.
    expected: u64,
    /// The hash the next entry chains to.
    previous: [u8; 32],
    /// The first sequence that did not verify.
    first_broken: Option<u64>,
    /// The anchor the chain starts from, when its beginning was pruned.
    anchor: Option<(u64, Value)>,
    /// The last pruning record of the chain.
    pruned: Option<Value>,
}

impl Walk {
    fn new() -> Self {
        Self {
            entries: 0,
            expected: 1,
            previous: NO_PREVIOUS,
            first_broken: None,
            anchor: None,
            pruned: None,
        }
    }

    /// Takes the next entry, held at `sequence`, whose text is `entry` and
    /// whose hash is `hash`.
    fn step(&mut self, sequence: i64, entry: &str, hash: &str) {
        self.entries += 1;
        if self.first_broken.is_some() {
            return;
        }
        if self.entries == 1
            && let Some((at, value, previous)) = anchor_of(sequence, entry, hash)
        {
            self.anchor = Some((at, value));
            self.expected = at + 1;
            self.previous = previous;
            return;
        }
        if is_unchained(entry) {
            // An unchained entry holds its place and its own hash, and the
            // chain goes on past it from the entry before it.
            if !stands_alone(stored(self.expected), sequence, entry, hash) {
                self.first_broken = Some(self.expected);
            }
            self.expected += 1;
            return;
        }
        match chains(stored(self.expected), &self.previous, sequence, entry, hash) {
            Some((next, value)) => {
                if value["kind"].as_str() == Some(AUDIT_LOG_PRUNED.name) {
                    self.pruned = Some(value);
                }
                self.previous = next;
                self.expected += 1;
            }
            None => self.first_broken = Some(self.expected),
        }
    }

    /// The chain's first sequence: its anchor's, or 1.
    fn first(&self) -> u64 {
        self.anchor.as_ref().map_or(1, |(at, _)| *at)
    }

    /// How many entries it took, the last sequence that verified, and the
    /// first that did not: an anchor that the last pruning record does not
    /// name, as one rewritten or moved, is broken at its own sequence.
    fn end(self) -> (u64, u64, Option<u64>) {
        let first_broken = self.first_broken.or_else(|| {
            let (at, anchor) = self.anchor.as_ref()?;
            let named = self.pruned.as_ref().and_then(anchor_named_by);
            let canonical = |value: &Value| serde_json_canonicalizer::to_string(value).ok();
            (named.as_ref().and_then(canonical) != canonical(anchor)).then_some(*at)
        });
        (self.entries, self.expected - 1, first_broken)
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
/// text's canonical form all agree. Answers its hash's bytes and its text
/// read when it is.
fn chains(
    expected: i64,
    previous: &[u8; 32],
    sequence: i64,
    entry: &str,
    hash: &str,
) -> Option<([u8; 32], Value)> {
    if sequence != expected {
        return None;
    }
    let value: Value = serde_json::from_str(entry).ok()?;
    if value.get("sequence").and_then(Value::as_i64) != Some(expected) {
        return None;
    }
    let canonical = serde_json_canonicalizer::to_string(&value).ok()?;
    if chained(previous, &canonical) != hash {
        return None;
    }
    Some((decode(hash)?, value))
}

/// Whether the text of a row, `entry`, is an unchained entry's: one
/// marked `chained: false`.
fn is_unchained(entry: &str) -> bool {
    serde_json::from_str::<Value>(entry).is_ok_and(|value| value["chained"] == Value::Bool(false))
}

/// Whether an unchained row, stored at `sequence` with the text `entry`
/// and the hash `hash`, is the entry expected at `expected`: its place and
/// the sequence its text holds agree, and its hash is over its canonical
/// form alone.
fn stands_alone(expected: i64, sequence: i64, entry: &str, hash: &str) -> bool {
    chains(expected, &NO_PREVIOUS, sequence, entry, hash).is_some()
}

/// The anchor a log's first row is, stored at `sequence` with the text
/// `entry` and the hash `hash`: an entry of the anchor's kind at its own
/// sequence, carrying that hash as the last pruned one. Answers its
/// sequence, its text read and its hash's bytes, from which the rest
/// chains; none for any other row.
fn anchor_of(sequence: i64, entry: &str, hash: &str) -> Option<(u64, Value, [u8; 32])> {
    let value: Value = serde_json::from_str(entry).ok()?;
    let at = u64::try_from(sequence).ok().filter(|at| *at >= 1)?;
    let holds = value["kind"].as_str() == Some(AUDIT_LOG_ANCHOR.name)
        && value["sequence"].as_i64() == Some(sequence)
        && value["details"]["last_pruned_hash"].as_str() == Some(hash);
    if !holds {
        return None;
    }
    Some((at, value, decode(hash)?))
}

/// The anchor a pruning leaves at `through`, the last pruned sequence,
/// whose hash is `last_pruned_hash`, at the pruning's `time` and `layer`.
fn anchor_entry(
    through: u64,
    last_pruned_hash: &str,
    time: &PointInTime,
    layer: Option<SettingsLayer>,
) -> AuditEntry {
    AuditEntry {
        sequence: through,
        time: time.clone(),
        kind: AUDIT_LOG_ANCHOR.name.to_owned(),
        actor: Actor::Engine,
        surface: None,
        layer,
        run_id: None,
        details: json!({ "last_pruned_hash": last_pruned_hash }),
        chained: true,
    }
}

/// The anchor the pruning record `pruned` names, as JSON.
fn anchor_named_by(pruned: &Value) -> Option<Value> {
    let pruned: AuditEntry = serde_json::from_value(pruned.clone()).ok()?;
    let through = pruned.details["through_sequence"].as_u64()?;
    let hash = pruned.details["last_pruned_hash"].as_str()?;
    serde_json::to_value(anchor_entry(through, hash, &pruned.time, pruned.layer)).ok()
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

#[cfg(test)]
mod audit_jsonl {
    goodfetch_test_support::fuzz_replay!(crate::audit::fuzz_audit_jsonl, "fuzz/audit_jsonl");
}
