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
//! minor only adds. Its writes go to one writer, an actor of the owner it is
//! given, which opens the store at its first write and keeps it open, its
//! statements prepared, and the chain's tail its last commit left, which it
//! reads back only once another engine committed (#1886). Its reads,
//! [`AuditLog::verify`] and [`AuditLog::export`], block: callers run them
//! under their owner's blocking policy.
//!
//! Size: one concern, the one store's hash chain, written by the append and
//! the prune and read by the verify and the export, so the chain's rules
//! read side by side.

#![expect(
    clippy::disallowed_types,
    reason = "the set of unchained Runs is read or changed in one step, never across an `.await` or under another lock, so there is no lock order to test under loom (ADR 0060)"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::audit::{
    AUDIT_LOG_ANCHOR, AUDIT_LOG_PRUNED, AUDIT_RETENTION, Actor, AuditEntry, AuditVerified,
    ExportedAuditEntry,
};
use goodfetch_protocol::catalog::{SharedType, SharedTypeValue};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::formats::{AUDIT_LOG, Compatibility};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use jiff::{SignedDuration, Timestamp};
use rusqlite::{Connection, OptionalExtension as _};

use crate::sqlite_store::run_cached;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::clock::Clock;
use crate::durable;
use crate::home::Home;
use crate::settings::Settings;
use crate::sqlite_store::{
    Access, Counting, Durability, File, Folder, OpenError, Opening, Schema, Synchronous, Wait,
};
use crate::tasks::{StateActor, TaskOwner};

/// How often the log prunes, after its pass as the engine starts.
const PRUNE_EVERY: Duration = Duration::from_hours(24);

/// How long an append waits for another engine's.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// How its commits reach the disk: as a synced per-user store's, but for its
/// tables' commit, unsynced (#2117).
const DURABILITY: Durability = Durability {
    opening: Synchronous::Off,
    ..Durability::SYNCED
};

/// The previous hash of the first entry.
const NO_PREVIOUS: [u8; 32] = [0; 32];

/// The tables and triggers of this version. A newer minor only adds tables
/// and columns, each column with a default, so these statements keep
/// working on its files.
const SCHEMA: &[&str] = &[
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
    /// The log's writer is not running, as its owner stopped or the OS
    /// refused its thread; a write it was given may be in the log.
    #[error("the Audit log's writer at `{path}` is not running: {message}")]
    Writer {
        /// The store's file.
        path: PathBuf,
        /// Why.
        message: String,
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

/// An entry to append with others in one commit
/// ([`AuditLog::record_together`]): its event's origin, its kind, a value of
/// the shared type `audit_kind`, and its details.
#[derive(Clone)]
pub struct NewEntry {
    /// Where its event came from.
    pub origin: Origin,
    /// Its kind.
    pub kind: &'static SharedTypeValue,
    /// What it holds.
    pub details: Value,
    /// Whether it is chained whatever its Run, as the record of a switch
    /// that unchains a Run is (ADR 0016); otherwise it is unchained when its
    /// Run is (see [`AuditLog::unchain`]).
    pub always_chained: bool,
}

/// Where the engine also sends each entry it appends, its hash with it,
/// once the store holds it: the Audit log's OpenTelemetry events, which an
/// executable whose `audit.otlp` is on installs (ADR 0016). Sending never
/// blocks the append, and an entry it fails to send stays in the store.
#[derive(Clone)]
pub struct AuditEvents(Arc<SendEntry>);

/// What sends an entry: called with the entry and its line of an export.
type SendEntry = dyn Fn(&ExportedAuditEntry, &str) + Send + Sync;

impl AuditEvents {
    /// Events that `send` sends, called with each new entry and its line of
    /// an export, a JSON object of the same fields as the line `audit
    /// export` writes, made from the text the append stored rather than
    /// encoded again (#1886), on the writer's thread.
    pub fn new(send: impl Fn(&ExportedAuditEntry, &str) + Send + Sync + 'static) -> Self {
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

/// The Audit log of one user: its store, with its writer, and the Runs
/// whose entries it writes unchained. Clones share both.
#[derive(Clone, Debug)]
pub struct AuditLog {
    store: Arc<Store>,
    writer: Arc<StateActor<Held>>,
    /// The Runs whose entries are written unchained, shared by its clones.
    unchained: Arc<Mutex<BTreeSet<RunId>>>,
}

/// The store of an Audit log: its file, its head file, the VFS it opens
/// through, the settings from whose snapshot of the machine's settings its
/// `audit.retention` is read, where its entries are sent, and where its
/// work is counted.
#[derive(Clone, Debug)]
struct Store {
    path: PathBuf,
    head: PathBuf,
    vfs: Option<String>,
    retained_under: Option<Settings>,
    events: Option<AuditEvents>,
    work: CountedWork,
}

/// The name of the log's writer: an actor of its owner, started by the
/// first write, which every clone of the log shares.
const WRITER: &str = "audit-log-writer";

/// What the writer holds between its writes: its connection, opened by its
/// first, the tail its last commit left, and whether it opened the store
/// before, so an open after a failure counts as an entry's work, not the
/// log's (ADR 0140's E2).
#[derive(Debug, Default)]
struct Held {
    connection: Option<Connection>,
    tail: Option<Tail>,
    opened: bool,
}

/// Where the chain ends: the sequence the next entry takes, the hash a
/// chained one chains to, and the store's `data_version` when it was read,
/// which only another connection's commit changes, so a tail of the same
/// version is still the store's.
#[derive(Clone, Copy, Debug)]
struct Tail {
    next: u64,
    previous: [u8; 32],
    version: i64,
}

impl AuditLog {
    /// The Audit log of `home`, opened through the SQLite VFS named `vfs`
    /// (none in production, ADR 0060), written by an actor of `owner`.
    pub fn of(home: &Home, vfs: Option<String>, owner: Arc<TaskOwner>) -> Self {
        Self {
            store: Arc::new(Store {
                path: home.audit_log(),
                head: home.audit_log_head(),
                vfs,
                retained_under: None,
                events: None,
                work: CountedWork::default(),
            }),
            writer: Arc::new(StateActor::of(WRITER, owner)),
            unchained: Arc::default(),
        }
    }

    /// Its store, to configure: the log then writes through a writer of its
    /// own, whose connection counts and sends as the store now says.
    fn configured(&mut self) -> &mut Store {
        self.writer = Arc::new(StateActor::of(WRITER, Arc::clone(self.writer.owner())));
        Arc::make_mut(&mut self.store)
    }

    /// The same, its Counted work counted on `work` (ADR 0140): each
    /// configuration read, store open, prune pass, transaction and head-file
    /// write of its entries.
    #[must_use]
    pub fn counting_on(mut self, work: CountedWork) -> Self {
        self.configured().work = work;
        self
    }

    /// Writes every later entry of `run_id` unchained, as a Run whose
    /// `security.audit_chain` is off has them (ADR 0016), here and in every
    /// clone of this log, but those [`AuditLog::record_together`] records
    /// as `always_chained`;
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
        self.configured().events = events;
        self
    }

    /// The same, its passes started by [`AuditLog::start_pruning`] pruning
    /// what the `audit.retention` of the snapshot `settings` keeps of the
    /// machine's settings prunes, as the snapshot is at each pass: no pass
    /// reads a settings file (ADR 0140); without it, nothing is ever pruned
    /// but through [`AuditLog::prune`].
    #[must_use]
    pub fn retained_under(mut self, settings: Settings) -> Self {
        self.configured().retained_under = Some(settings);
        self
    }

    /// Its store's file.
    pub fn path(&self) -> &Path {
        &self.store.path
    }

    /// Its head file.
    pub fn head_path(&self) -> &Path {
        &self.store.head
    }

    /// Appends an entry of `kind`, a value of the shared type `audit_kind`,
    /// holding `details`, from `origin`, recorded at `now`, chained to the
    /// last chained entry, or unchained when its Run is (see
    /// [`AuditLog::unchain`]), then writes the head file; answers the
    /// entry. It prunes nothing: [`AuditLog::start_pruning`] does (ADR 0140).
    ///
    /// # Errors
    ///
    /// When the store or the head file cannot be written, the store is of
    /// another major, or its writer is not running.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the entry appended.
    pub async fn record(
        &self,
        origin: &Origin,
        kind: &'static SharedTypeValue,
        details: Value,
        now: Timestamp,
    ) -> Result<AuditEntry, AuditError> {
        let chained = !self.is_unchained(origin.run_id.as_ref());
        let mut recorded = self
            .commit_entries(vec![(origin.clone(), kind, details, chained)], now)
            .await?;
        recorded
            .pop()
            .ok_or_else(|| self.store.failed(&"the append answered no entry"))
    }

    /// Appends `entries` in their order, in one transaction with one sync,
    /// each chained as [`AuditLog::record`] chains it unless it is
    /// `always_chained`, then writes the head file once after the last;
    /// answers them. A Run's switched-off controls are recorded so
    /// as it starts (#2110): together, rather than a commit, a sync and a
    /// connection's close each.
    ///
    /// # Errors
    ///
    /// As [`AuditLog::record`]: none of them is appended then.
    ///
    /// # Cancel safety
    ///
    /// As [`AuditLog::record`].
    pub async fn record_together(
        &self,
        entries: Vec<NewEntry>,
        now: Timestamp,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        let entries = entries
            .into_iter()
            .map(|entry| {
                let chained =
                    entry.always_chained || !self.is_unchained(entry.origin.run_id.as_ref());
                (entry.origin, entry.kind, entry.details, chained)
            })
            .collect();
        self.commit_entries(entries, now).await
    }

    /// Appends `entries` on the writer, as [`Store::commit_entries`] does.
    async fn commit_entries(
        &self,
        entries: Vec<Appended>,
        now: Timestamp,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        if entries.is_empty() {
            return Ok(Vec::new());
        }
        self.write(move |store, held| store.commit_entries(held, entries, now))
            .await
    }

    /// Starts its pruning, a task of `owner` until `owner` stops: a pass
    /// now, as the engine starts, then one a day on `clock`, each pruning
    /// what the `audit.retention` it is retained under sets then, at the
    /// time `clock` reads (ADR 0140). No append prunes, so an entry past the
    /// retention may stay up to a day longer. A pass that fails is logged,
    /// and the next day's tries again.
    pub fn start_pruning(&self, clock: Clock, owner: &TaskOwner) {
        let log = self.clone();
        let token = owner.token().clone();
        owner.spawn(async move {
            tokio::select! {
                () = token.cancelled() => {}
                () = log.prune_daily(clock) => {}
            }
        });
    }

    /// Prunes as `audit.retention` sets it now, then again each day, for
    /// ever.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the log pruned.
    async fn prune_daily(&self, clock: Clock) {
        loop {
            if let Some((retention, layer)) = self.store.retention().await
                && let Err(error) = self.prune(retention, Some(layer), clock.timestamp()).await
            {
                tracing::warn!(%error, "the Audit log could not be pruned; it tries again in a day");
            }
            clock.sleep(PRUNE_EVERY).await;
        }
    }

    /// Prunes the entries older than `retention` at `now`, which `layer`
    /// set, from the log's beginning only: they give way to an anchor at
    /// the last pruned sequence, carrying its hash, and a pruning record
    /// naming it is chained at the tail. Answers the pruning record; none
    /// when no entry, the anchor aside, is that old.
    ///
    /// # Errors
    ///
    /// When the store or the head file cannot be written, the store is of
    /// another major, or its writer is not running.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave the log pruned.
    pub async fn prune(
        &self,
        retention: Duration,
        layer: Option<SettingsLayer>,
        now: Timestamp,
    ) -> Result<Option<AuditEntry>, AuditError> {
        self.write(move |store, held| store.prune(held, retention, layer, now))
            .await
    }

    /// Runs `work` on the writer, starting it at the first write, and
    /// answers what it returned. A write that fails leaves the writer
    /// nothing: the next opens the store again and reads its tail back.
    async fn write<T, F>(&self, work: F) -> Result<T, AuditError>
    where
        T: Send + 'static,
        F: FnOnce(&Store, &mut Held) -> Result<T, AuditError> + Send + 'static,
    {
        let store = Arc::clone(&self.store);
        self.writer
            .run(move |held| {
                let written = work(&store, held);
                if written.is_err() {
                    *held = Held {
                        opened: held.opened,
                        ..Held::default()
                    };
                }
                written
            })
            .await
            .map_err(|stopped| self.store.stopped(&stopped))?
    }

    /// Verifies the chain from its first entry and the head file against
    /// its last: intact, or the first sequence that does not verify. A log
    /// never written verifies intact and empty, and is not created.
    ///
    /// # Errors
    ///
    /// When the store cannot be read, or is of another major.
    pub fn verify(&self) -> Result<AuditVerified, AuditError> {
        self.store.verify()
    }

    /// Every entry in sequence with its hash.
    ///
    /// # Errors
    ///
    /// When the store cannot be read, or is of another major.
    pub fn export(&self) -> Result<Vec<ExportedAuditEntry>, AuditError> {
        self.store.export()
    }
}

/// An entry to append: its origin, its kind, its details and whether it is
/// chained.
type Appended = (Origin, &'static SharedTypeValue, Value, bool);

impl Store {
    /// Appends each of `entries` in one transaction, on the writer's
    /// connection, chained after the tail it holds while the store's
    /// `data_version` says no other connection committed since, then writes
    /// the head file after the last and sends each to the events.
    fn commit_entries(
        &self,
        held: &mut Held,
        entries: Vec<Appended>,
        now: Timestamp,
    ) -> Result<Vec<AuditEntry>, AuditError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let cached = held.tail;
        let connection = self.connection(&mut held.connection, &mut held.opened)?;
        let committed = self.immediate(connection, |transaction| {
            let mut tail = self.after_last(transaction, cached)?;
            let time = SystemTime::from(now);
            let mut appended = Vec::with_capacity(entries.len());
            for (origin, kind, details, chained) in entries {
                debug_assert_eq!(kind.shared_type, SharedType::AuditKind);
                let entry = AuditEntry {
                    sequence: tail.next,
                    time: time.clone(),
                    kind: kind.name.to_owned(),
                    actor: origin.actor,
                    surface: origin.surface,
                    layer: origin.layer,
                    run_id: origin.run_id,
                    details,
                    chained,
                };
                let (canonical, hash) = self.append(transaction, &tail.previous, &entry)?;
                if chained {
                    tail.previous =
                        decode(&hash).ok_or_else(|| failed(&"an entry's hash is no SHA-256"))?;
                }
                tail.next += 1;
                appended.push(Stored {
                    entry,
                    canonical,
                    hash,
                });
            }
            Ok(Some((appended, tail)))
        })?;
        // Nothing was appended when nothing was committed.
        let Some((appended, tail)) = committed else {
            return Ok(Vec::new());
        };
        held.tail = Some(tail);
        self.appended(appended)
    }

    /// Prunes as [`AuditLog::prune`] says, on the writer's connection,
    /// leaving it the tail its pruning record makes.
    fn prune(
        &self,
        held: &mut Held,
        retention: Duration,
        layer: Option<SettingsLayer>,
        now: Timestamp,
    ) -> Result<Option<AuditEntry>, AuditError> {
        self.work.count(Row::Prunes);
        self.work.count(Row::AuditPrunes);
        let cutoff = SignedDuration::try_from(retention)
            .ok()
            .and_then(|retention| now.checked_sub(retention).ok());
        let Some(cutoff) = cutoff.filter(|_| held.connection.is_some() || self.path.exists())
        else {
            return Ok(None);
        };
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let cached = held.tail;
        let connection = self.connection(&mut held.connection, &mut held.opened)?;
        let committed = self.immediate(connection, |transaction| {
            let Some((through, last_pruned_hash)) = self.older_than(transaction, cutoff)? else {
                return Ok(None);
            };
            let tail = self.after_last(transaction, cached)?;
            let time = SystemTime::from(now);
            let anchor = anchor_entry(through, &last_pruned_hash, &time, layer);
            let anchor =
                serde_json_canonicalizer::to_string(&anchor).map_err(|error| failed(&error))?;
            let execute = |statement: &str, parameters: &[&dyn rusqlite::ToSql]| {
                transaction
                    .prepare_cached(statement)
                    .and_then(|mut prepared| prepared.execute(parameters))
                    .map_err(|error| failed(&error))
            };
            execute("DROP TRIGGER entries_never_deleted", &[])?;
            execute(
                "DELETE FROM entries WHERE sequence <= ?1",
                &[&stored(through)],
            )?;
            execute(NEVER_DELETED, &[])?;
            execute(INSERT, &[&stored(through), &anchor, &last_pruned_hash])?;
            let pruned = AuditEntry {
                sequence: tail.next,
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
            let (canonical, hash) = self.append(transaction, &tail.previous, &pruned)?;
            Ok(Some((pruned, canonical, hash, tail)))
        })?;
        // Nothing older than the cutoff: its reads were rolled back.
        let Some((pruned, canonical, hash, mut tail)) = committed else {
            return Ok(None);
        };
        tail.previous = decode(&hash).ok_or_else(|| failed(&"an entry's hash is no SHA-256"))?;
        tail.next += 1;
        held.tail = Some(tail);
        Ok(self
            .appended(vec![Stored {
                entry: pruned,
                canonical,
                hash,
            }])?
            .pop())
    }

    /// Runs `work` in one IMMEDIATE transaction of `connection`: commits it
    /// when `work` answers something, rolls it back when `work` answers
    /// nothing or fails. Its `BEGIN`, `COMMIT` and `ROLLBACK` go through the
    /// connection's statement cache, as the run file's writer's do
    /// ([`run_cached`]), so a transaction prepares nothing once its
    /// connection prepared them (#1886). A rollback that fails after
    /// nothing was answered fails too, so the writer, which drops its
    /// connection on any failure, never keeps one mid-transaction; after a
    /// failure, that drop rolls back what a failed `ROLLBACK` left.
    fn immediate<T>(
        &self,
        connection: &Connection,
        work: impl FnOnce(&Connection) -> Result<Option<T>, AuditError>,
    ) -> Result<Option<T>, AuditError> {
        run_cached(connection, "BEGIN IMMEDIATE").map_err(|error| self.failed(&error))?;
        let rolled_back = || run_cached(connection, "ROLLBACK");
        match work(connection) {
            Ok(Some(answer)) => match run_cached(connection, "COMMIT") {
                Ok(()) => Ok(Some(answer)),
                Err(error) => {
                    let _ = rolled_back();
                    Err(self.failed(&error))
                }
            },
            Ok(None) => rolled_back()
                .map(|()| None)
                .map_err(|error| self.failed(&error)),
            Err(error) => {
                let _ = rolled_back();
                Err(error)
            }
        }
    }

    /// The writer's connection, the store opened now if it is not yet:
    /// once a log, or again after a failure, as `opened` says.
    fn connection<'held>(
        &self,
        held: &'held mut Option<Connection>,
        opened: &mut bool,
    ) -> Result<&'held mut Connection, AuditError> {
        let connection = match held.take() {
            Some(connection) => connection,
            None => self.open(!std::mem::replace(opened, true))?,
        };
        Ok(held.insert(connection))
    }

    /// The `audit.retention` the engine's snapshot of the machine's
    /// settings sets now, with the layer that set it; none when unset, when
    /// the log is retained under no settings, or when they cannot be read,
    /// which says nothing of the log's entries, so nothing is pruned on
    /// this pass.
    async fn retention(&self) -> Option<(Duration, SettingsLayer)> {
        let machine = self.retained_under.as_ref()?.snapshot().await;
        let resolved = machine
            .resolved()
            .inspect_err(|error| {
                tracing::warn!(%error, "`audit.retention` cannot be read, so nothing is pruned");
            })
            .ok()?;
        resolved
            .get::<Option<Duration>>(&AUDIT_RETENTION)
            .zip(resolved.effective(&AUDIT_RETENTION).map(|set| set.layer))
    }

    /// Writes the head file after the last of `appended`, sends each to the
    /// events with its line of an export, and answers them.
    fn appended(&self, appended: Vec<Stored>) -> Result<Vec<AuditEntry>, AuditError> {
        if let Some(last) = appended.last() {
            self.write_head(&Head {
                sequence: last.entry.sequence,
                hash: last.hash.clone(),
            })?;
        }
        Ok(appended
            .into_iter()
            .map(|stored| match &self.events {
                Some(events) => {
                    let line = line_of(&stored.canonical, &stored.hash);
                    let exported = ExportedAuditEntry {
                        entry: stored.entry,
                        hash: stored.hash,
                    };
                    (events.0)(&exported, &line);
                    exported.entry
                }
                None => stored.entry,
            })
            .collect())
    }

    /// Where the chain ends: `cached`, the writer's, while the store's
    /// `data_version` is still the one it was read at, or else read back
    /// from the last chained entry, past the unchained ones after it.
    fn after_last(
        &self,
        transaction: &Connection,
        cached: Option<Tail>,
    ) -> Result<Tail, AuditError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let version: i64 = transaction
            .prepare_cached("PRAGMA data_version")
            .and_then(|mut statement| statement.query_row([], |row| row.get(0)))
            .map_err(|error| failed(&error))?;
        if let Some(tail) = cached.filter(|tail| tail.version == version) {
            return Ok(tail);
        }
        let mut statement = transaction
            .prepare_cached("SELECT sequence, entry, hash FROM entries ORDER BY sequence DESC")
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
                return Ok(Tail {
                    next,
                    previous,
                    version,
                });
            }
        }
        Ok(Tail {
            next: next.unwrap_or(1),
            previous: NO_PREVIOUS,
            version,
        })
    }

    /// Writes `entry`, chained to `previous`, and answers its canonical
    /// text and its hash.
    fn append(
        &self,
        transaction: &Connection,
        previous: &[u8; 32],
        entry: &AuditEntry,
    ) -> Result<(String, String), AuditError> {
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
            .prepare_cached(INSERT)
            .and_then(|mut statement| {
                statement.execute((stored(entry.sequence), &canonical, &hash))
            })
            .map_err(|error| failed(&error))?;
        Ok((canonical, hash))
    }

    /// The last sequence of the log's beginning whose entries, the anchor
    /// aside, were all recorded before `cutoff`, with its hash; none when
    /// the first is not that old.
    fn older_than(
        &self,
        transaction: &Connection,
        cutoff: Timestamp,
    ) -> Result<Option<(u64, String)>, AuditError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut statement = transaction
            .prepare_cached("SELECT sequence, entry, hash FROM entries ORDER BY sequence")
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

    /// As [`AuditLog::verify`], on a connection of its own.
    fn verify(&self) -> Result<AuditVerified, AuditError> {
        if !self.path.exists() {
            let first_broken = self.read_head().map(|_| 1);
            return Ok(AuditVerified {
                intact: first_broken.is_none(),
                entries: 0,
                first_broken,
            });
        }
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let connection = self.open(false)?;
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
                    .prepare_cached("SELECT hash FROM entries WHERE sequence = ?1")
                    .and_then(|mut statement| {
                        statement.query_row([stored(head.sequence)], |row| row.get(0))
                    })
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

    /// As [`AuditLog::export`], on a connection of its own.
    fn export(&self) -> Result<Vec<ExportedAuditEntry>, AuditError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let connection = self.open(false)?;
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
            reason = "the Audit log's reads run under their caller's blocking policy"
        )]
        let bytes = std::fs::read(&self.head).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Opens the store, creating it and its tables when it does not exist,
    /// and refuses one of another major. A new log's tables, and its WAL's
    /// header, commit unsynced: no entry is in them yet, and the commit of
    /// its first entry syncs them too (#2117). An existing log's commit
    /// there writes nothing. The writer's `first` open and its set-up's
    /// commits count once a log; any other, the writer's again after a
    /// failure or a reader's, counts on the entry's rows (ADR 0140's E2).
    fn open(&self, first: bool) -> Result<Connection, AuditError> {
        let (opens, set_up) = if first {
            (Row::AuditLogOpens, Row::AuditLogTransactions)
        } else {
            (Row::AuditOpens, Row::AuditTransactions)
        };
        self.work.count(opens);
        let connection = File::new(&self.path, self.vfs.as_deref())
            .open(&Opening {
                folder: Folder::Make,
                access: Access::Create,
                wait: Wait::Busy(BUSY_TIMEOUT),
                durability: DURABILITY,
                // Beneath every caller, so the set-up's commits count too
                // (ADR 0140), on the log's row or the entry's.
                counting: Some(Counting {
                    work: &self.work,
                    opens: None,
                    transactions: set_up,
                    counts_switch: true,
                }),
                schema: Some(Schema {
                    format: &AUDIT_LOG,
                    statements: &[SCHEMA],
                }),
            })
            .map_err(|error| match error {
                OpenError::Format(refusal) => AuditError::Format {
                    found: refusal.found,
                    compatibility: refusal.compatibility,
                },
                other => self.failed(&other),
            })?;
        // Every commit after the set-up is an entry's or a prune's.
        Counting {
            work: &self.work,
            opens: None,
            transactions: Row::AuditTransactions,
            counts_switch: true,
        }
        .count_commits(&connection)
        .map_err(|error| self.failed(&error))?;
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
        self.work.count(Row::AuditSyncs);
        durable::replace(&self.head, &text).map_err(|error| failed(&error))
    }

    fn failed(&self, error: &dyn std::fmt::Display) -> AuditError {
        AuditError::Sqlite {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }

    fn stopped(&self, error: &dyn std::fmt::Display) -> AuditError {
        AuditError::Writer {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }
}

/// An entry the store now holds: the entry, its canonical text and its
/// hash.
struct Stored {
    entry: AuditEntry,
    canonical: String,
    hash: String,
}

/// The statement that writes an entry.
const INSERT: &str = "INSERT INTO entries (sequence, entry, hash) VALUES (?1, ?2, ?3)";

/// The line of an export of the entry whose canonical text is `canonical`
/// and whose hash is `hash`: that text, the hash added as its last member.
fn line_of(canonical: &str, hash: &str) -> String {
    let members = canonical
        .strip_suffix('}')
        .expect("invariant: an entry's canonical JSON is an object");
    format!("{members},\"hash\":\"{hash}\"}}")
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
    time: &SystemTime,
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod audit_jsonl {
    goodfetch_test_support::fuzz_replay!(crate::audit::fuzz_audit_jsonl, "fuzz/audit_jsonl");
}
