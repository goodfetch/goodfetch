//! The Usage ledger (ADRs 0041, 0045, 0060 and 0098): one per-user SQLite
//! file in Goodfetch home, outside the per-major folders, which every engine
//! of the user opens, whatever its major or Product. It holds the Limits
//! across runs as generic rows, the Slices runs take against them and the
//! Usage charged. A run takes a Slice in one transaction, sized by
//! [`slice_size`]; it charges what it uses against its Slices and takes
//! another when they run out; it gives back what is left of them when it
//! ends. An engine that opens the ledger reclaims each Slice whose run's
//! writer lock is free or held at a later run epoch, charging the Usage the
//! run journaled under the Slice's epoch, or the whole Slice, marked for
//! review, when the run's state cannot be read.
//!
//! The file opens through a VFS named by a value, none in production, the
//! crash VFS in tests, and is of the Usage ledger's family: its major never
//! changes, a newer minor only adds rows, tables and columns, and a row this
//! engine cannot evaluate is kept untouched. Each transition the slices and
//! reclaim model names (`test-support/models/slices/`) is a `tracing` event
//! under this module's target.
//!
//! Its calls block: callers run them under their owner's blocking policy.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::formats::{Compatibility, USAGE_LEDGER};
use goodfetch_protocol::run::{LimitScope, Period, PeriodLength, PointInTime, ReachedLimit};
use goodfetch_protocol::usage::{Limit, LimitSetter, LimitUsage};
use jiff::Timestamp;
use rusqlite::{Connection, OpenFlags, Row, TransactionBehavior};

pub use self::charging::{Charged, Charging, Refusal};
pub use self::period::{PeriodBounds, bounds};
pub use self::sizing::{SliceSizing, slice_size};
pub use self::standing::{Placement, standing};

/// The ledger's file in Goodfetch home.

/// How long a transaction waits for another engine's.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The tables of this version. A newer minor only adds tables and columns,
/// each column with a default, so these statements keep working on its
/// files.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT",
    "CREATE TABLE IF NOT EXISTS limits (
        limit_id INTEGER PRIMARY KEY,
        scope_kind TEXT NOT NULL,
        scope_id TEXT NOT NULL,
        usage_kind TEXT NOT NULL,
        currency TEXT,
        period TEXT,
        time_zone TEXT,
        amount INTEGER NOT NULL,
        must_understand INTEGER NOT NULL,
        set_by TEXT NOT NULL,
        set_at TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS slices (
        slice_id INTEGER PRIMARY KEY,
        limit_id INTEGER NOT NULL,
        period_start TEXT NOT NULL,
        run_id TEXT NOT NULL,
        run_epoch INTEGER NOT NULL,
        run_folder TEXT NOT NULL,
        size INTEGER NOT NULL,
        charged INTEGER,
        state TEXT NOT NULL,
        taken_at TEXT NOT NULL
    ) STRICT",
    "CREATE INDEX IF NOT EXISTS slices_by_limit ON slices (limit_id, period_start, state)",
    "CREATE INDEX IF NOT EXISTS slices_by_state ON slices (state)",
];

/// A Limit's row in the ledger.
pub type LimitId = i64;

/// A Slice's row in the ledger.
pub type SliceId = i64;

/// Why the ledger could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum LedgerError {
    /// SQLite failed; the transaction committed nothing.
    #[error("the Usage ledger at `{path}`: {message}")]
    Sqlite {
        /// The ledger's file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// The file is of another major of the format (ADR 0030).
    #[error("the Usage ledger is {found}, which this engine cannot read")]
    Format {
        /// The file's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
}

/// The run that holds a Slice: its Run ID, the run epoch it took it under
/// and the folder of its run state, which a reclaimer reads.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Holder {
    /// Its Run ID.
    pub run_id: String,
    /// Its run epoch when it took the Slice.
    pub epoch: u64,
    /// The folder of its run state.
    pub folder: PathBuf,
}

/// A Slice a run holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slice {
    /// Its row.
    pub id: SliceId,
    /// How much of its Limit it holds.
    pub size: u64,
}

/// What a reclaimer finds of a Slice's run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Probed {
    /// Its writer lock is held at the Slice's run epoch: the run is live.
    Live,
    /// Its writer lock is free, or held at a later run epoch: the engine
    /// that took the Slice is gone. `used` is the Usage the run journaled
    /// under the Slice's epoch, by Slice.
    Gone {
        /// What it used of each Slice.
        used: BTreeMap<SliceId, u64>,
    },
    /// Its run state cannot be read.
    Unreadable,
}

/// What a reclaim did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reclaimed {
    /// The Slices charged from what their runs journaled.
    pub charged: Vec<SliceId>,
    /// The Slices charged in full and marked for review.
    pub for_review: Vec<SliceId>,
}

/// One Limit row as the ledger holds it, read whatever version wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredLimit {
    /// Its row.
    pub id: LimitId,
    /// Its scope.
    pub scope: LimitScope,
    /// The Usage kind it caps.
    pub usage_kind: String,
    /// The currency of a price Limit.
    pub currency: Option<String>,
    /// Its period's length as written, if any.
    pub period: Option<String>,
    /// Its period's time zone as written, if any.
    pub time_zone: Option<String>,
    /// How much it allows per period.
    pub amount: u64,
    /// Whether an engine that cannot evaluate it must refuse runs it
    /// cannot place outside it.
    pub must_understand: bool,
    /// Who set it, as written.
    pub set_by: String,
}

impl StoredLimit {
    /// Its period, when this engine knows its length.
    pub fn known_period(&self) -> Option<Period> {
        let every = match self.period.as_deref()? {
            "day" => PeriodLength::Day,
            "week" => PeriodLength::Week,
            "month" => PeriodLength::Month,
            _ => return None,
        };
        Some(Period {
            every,
            time_zone: self.time_zone.clone().unwrap_or_default(),
        })
    }

    /// The Limit as the protocol describes it; a period or a setter this
    /// engine does not know is left out or read as a Client's.
    pub fn limit(&self) -> Limit {
        Limit {
            scope: self.scope.clone(),
            usage_kind: self.usage_kind.clone(),
            currency: self.currency.clone(),
            period: self.known_period(),
            amount: self.amount,
            must_understand: self.must_understand,
            set_by: match self.set_by.as_str() {
                "person" => LimitSetter::Person,
                "managed_policy" => LimitSetter::ManagedPolicy,
                _ => LimitSetter::Client,
            },
        }
    }

    /// The Limit as `limit_reached` names it, its period's end `resets_at`.
    pub fn reached(&self, resets_at: Option<Timestamp>) -> ReachedLimit {
        ReachedLimit {
            scope: self.scope.clone(),
            usage_kind: self.usage_kind.clone(),
            currency: self.currency.clone(),
            period: self.known_period(),
            resets_at: resets_at.map(point_in_time),
        }
    }

    /// The key of the period that holds `now`: its start, or empty for a
    /// Limit without a period or one this engine cannot evaluate.
    fn period_key(&self, now: Timestamp) -> (String, Option<PeriodBounds>) {
        match self
            .known_period()
            .and_then(|period| bounds(&period, now).ok())
        {
            Some(found) => (found.start.to_string(), Some(found)),
            None => (String::new(), None),
        }
    }
}

/// A point in time of the protocol's, from a timestamp.
pub fn point_in_time(at: Timestamp) -> PointInTime {
    PointInTime::from_rfc3339(at.to_string())
}

/// The Usage ledger of one user: its file, and the VFS it opens through.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ledger {
    path: PathBuf,
    vfs: Option<String>,
}

impl Ledger {
    /// The ledger at `path`, opened through the SQLite VFS named `vfs`:
    /// none in production, the crash VFS in tests (ADR 0060).
    pub fn at(path: PathBuf, vfs: Option<String>) -> Self {
        Self { path, vfs }
    }

    /// Its file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Opens the file, creating it and its tables when it does not exist,
    /// and refuses one of another major.
    fn open(&self) -> Result<Connection, LedgerError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        if let Some(folder) = self.path.parent() {
            #[expect(
                clippy::disallowed_methods,
                reason = "the ledger's calls run under their caller's blocking policy"
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
                (USAGE_LEDGER.name, USAGE_LEDGER.version.to_string()),
            )
            .map_err(|error| failed(&error))?;
        let found: String = transaction
            .query_row(
                "SELECT version FROM format WHERE name = ?1",
                [USAGE_LEDGER.name],
                |row| row.get(0),
            )
            .map_err(|error| failed(&error))?;
        transaction.commit().map_err(|error| failed(&error))?;
        check_format(&found)?;
        Ok(connection)
    }

    fn failed(&self, error: &dyn std::fmt::Display) -> LedgerError {
        LedgerError::Sqlite {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }

    /// Adds `limit`, set at `now`, beside the Limits already there: the
    /// tightest one that covers a run holds it.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be written.
    pub fn add_limit(&self, limit: &Limit, now: Timestamp) -> Result<LimitId, LedgerError> {
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO limits (scope_kind, scope_id, usage_kind, currency, period, time_zone, amount, must_understand, set_by, set_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                (
                    &limit.scope.kind,
                    &limit.scope.id,
                    &limit.usage_kind,
                    &limit.currency,
                    limit.period.as_ref().map(|period| period.every.as_str()),
                    limit.period.as_ref().map(|period| period.time_zone.as_str()),
                    amount(limit.amount),
                    limit.must_understand,
                    limit.set_by.as_str(),
                    now.to_string(),
                ),
            )
            .map_err(|error| self.failed(&error))?;
        let id = connection.last_insert_rowid();
        tracing::info!(target: "goodfetch_engine::ledger", transition = "set_limit", limit = id, amount = limit.amount, "a Limit was added");
        Ok(id)
    }

    /// Every Limit row, in the order they were added.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be read.
    pub fn limits(&self) -> Result<Vec<StoredLimit>, LedgerError> {
        let connection = self.open()?;
        stored_limits(&connection).map_err(|error| self.failed(&error))
    }

    /// Every Limit, with what `product`'s engine makes of it and the Usage
    /// counted against it in the period that holds `now`, by scope.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be read.
    pub fn usage(&self, product: Product, now: Timestamp) -> Result<Vec<LimitUsage>, LedgerError> {
        let connection = self.open()?;
        let mut limits = stored_limits(&connection).map_err(|error| self.failed(&error))?;
        limits.sort_by(|left, right| left.scope.cmp(&right.scope).then(left.id.cmp(&right.id)));
        limits
            .into_iter()
            .map(|stored| {
                let (key, found) = stored.period_key(now);
                let (charged, held) =
                    counted(&connection, stored.id, &key).map_err(|error| self.failed(&error))?;
                Ok(LimitUsage {
                    limit: stored.limit(),
                    standing: standing(&stored, product),
                    charged,
                    held,
                    left: stored.amount.saturating_sub(charged.saturating_add(held)),
                    period_started_at: found.map(|found| point_in_time(found.start)),
                    resets_at: found.map(|found| point_in_time(found.end)),
                })
            })
            .collect()
    }

    /// What is left of the Limit `stored` in the period that holds `now`.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be read.
    pub fn left(&self, stored: &StoredLimit, now: Timestamp) -> Result<u64, LedgerError> {
        let connection = self.open()?;
        let (key, _) = stored.period_key(now);
        let (charged, held) =
            counted(&connection, stored.id, &key).map_err(|error| self.failed(&error))?;
        Ok(stored.amount.saturating_sub(charged.saturating_add(held)))
    }

    /// Takes a Slice of the Limit `stored` for `holder`, in the period that
    /// holds `now`, in one transaction: sized by `sizing` from what is left
    /// and the live runs holding Slices of it; none when nothing is left.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be written.
    pub fn take_slice(
        &self,
        stored: &StoredLimit,
        holder: &Holder,
        now: Timestamp,
        sizing: SliceSizing,
    ) -> Result<Option<Slice>, LedgerError> {
        let mut connection = self.open()?;
        let (key, _) = stored.period_key(now);
        let taken = (|| {
            let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let (charged, held) = counted(&transaction, stored.id, &key)?;
            let left = stored.amount.saturating_sub(charged.saturating_add(held));
            let others: u64 = transaction.query_row(
                "SELECT COUNT(DISTINCT run_id) FROM slices
                 WHERE limit_id = ?1 AND period_start = ?2 AND state = 'held' AND run_id <> ?3",
                (stored.id, &key, &holder.run_id),
                |row| row.get::<_, i64>(0),
            )
            .map(|count| u64::try_from(count).unwrap_or_default())?;
            let size = slice_size(sizing, stored.amount, left, others + 1);
            if size == 0 {
                return Ok(None);
            }
            transaction.execute(
                "INSERT INTO slices (limit_id, period_start, run_id, run_epoch, run_folder, size, charged, state, taken_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, 'held', ?7)",
                (
                    stored.id,
                    &key,
                    &holder.run_id,
                    amount(holder.epoch),
                    holder.folder.to_string_lossy(),
                    amount(size),
                    now.to_string(),
                ),
            )?;
            let id = transaction.last_insert_rowid();
            transaction.commit()?;
            Ok(Some(Slice { id, size }))
        })()
        .map_err(|error: rusqlite::Error| self.failed(&error))?;
        match taken {
            Some(slice) => {
                tracing::info!(target: "goodfetch_engine::ledger", transition = "take_slice", limit = stored.id, slice = slice.id, size = slice.size, run_id = %holder.run_id, epoch = holder.epoch, "a run took a Slice")
            }
            None => {
                tracing::info!(target: "goodfetch_engine::ledger", transition = "refuse_slice", limit = stored.id, run_id = %holder.run_id, epoch = holder.epoch, "a Limit had nothing left for a run")
            }
        }
        Ok(taken)
    }

    /// Gives back what is left of each Slice of `used`, charging what its
    /// run used of it; a Slice no longer held, such as one a reclaimer
    /// charged, stays as it is.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be written.
    pub fn give_back(&self, used: &[(SliceId, u64)]) -> Result<(), LedgerError> {
        if used.is_empty() {
            return Ok(());
        }
        let mut connection = self.open()?;
        (|| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            for (slice, used) in used {
                transaction.execute(
                    "UPDATE slices SET charged = MIN(size, ?2), state = 'returned'
                     WHERE slice_id = ?1 AND state = 'held'",
                    (slice, amount(*used)),
                )?;
            }
            transaction.commit()
        })()
        .map_err(|error: rusqlite::Error| self.failed(&error))?;
        for (slice, used) in used {
            tracing::info!(target: "goodfetch_engine::ledger", transition = "give_back", slice = slice, used = used, "a run gave back a Slice");
        }
        Ok(())
    }

    /// Reclaims every held Slice whose run `probe` finds gone: charges what
    /// the run journaled under the Slice's epoch, at most the Slice, or the
    /// whole Slice, marked for review, when its run state cannot be read.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be read or written.
    pub fn reclaim(&self, probe: &dyn Fn(&Holder) -> Probed) -> Result<Reclaimed, LedgerError> {
        let mut connection = self.open()?;
        let held = held_slices(&connection).map_err(|error| self.failed(&error))?;
        let mut reclaimed = Reclaimed::default();
        for (holder, slices) in held {
            let probed = probe(&holder);
            let charges: Vec<(SliceId, u64, &'static str)> = match &probed {
                Probed::Live => continue,
                Probed::Gone { used } => slices
                    .iter()
                    .map(|slice| {
                        let used = used.get(&slice.id).copied().unwrap_or(0);
                        (slice.id, used.min(slice.size), "reclaimed")
                    })
                    .collect(),
                Probed::Unreadable => slices
                    .iter()
                    .map(|slice| (slice.id, slice.size, "review"))
                    .collect(),
            };
            let done = (|| {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let mut done = Vec::new();
                for (slice, charged, state) in &charges {
                    let changed = transaction.execute(
                        "UPDATE slices SET charged = ?2, state = ?3 WHERE slice_id = ?1 AND state = 'held'",
                        (slice, amount(*charged), state),
                    )?;
                    if changed == 1 {
                        done.push((*slice, *charged, *state));
                    }
                }
                transaction.commit()?;
                Ok(done)
            })()
            .map_err(|error: rusqlite::Error| self.failed(&error))?;
            for (slice, charged, state) in done {
                if state == "review" {
                    tracing::info!(target: "goodfetch_engine::ledger", transition = "mark_for_review", slice = slice, charged = charged, run_id = %holder.run_id, epoch = holder.epoch, "a dead run's Slice was charged in full for review");
                    reclaimed.for_review.push(slice);
                } else {
                    tracing::info!(target: "goodfetch_engine::ledger", transition = "reclaim", slice = slice, charged = charged, run_id = %holder.run_id, epoch = holder.epoch, "a dead run's Slice was reclaimed");
                    reclaimed.charged.push(slice);
                }
            }
        }
        Ok(reclaimed)
    }
}

/// A Slice held under one run's holder.
struct HeldSlice {
    id: SliceId,
    size: u64,
}

fn held_slices(connection: &Connection) -> rusqlite::Result<BTreeMap<Holder, Vec<HeldSlice>>> {
    let mut statement = connection.prepare(
        "SELECT slice_id, run_id, run_epoch, run_folder, size FROM slices WHERE state = 'held' ORDER BY slice_id",
    )?;
    let mut held: BTreeMap<Holder, Vec<HeldSlice>> = BTreeMap::new();
    let rows = statement.query_map([], |row| {
        Ok((
            Holder {
                run_id: row.get(1)?,
                epoch: counted_value(row, 2)?,
                folder: PathBuf::from(row.get::<_, String>(3)?),
            },
            HeldSlice {
                id: row.get(0)?,
                size: counted_value(row, 4)?,
            },
        ))
    })?;
    for row in rows {
        let (holder, slice) = row?;
        held.entry(holder).or_default().push(slice);
    }
    Ok(held)
}

/// The Usage charged and held against the Limit `limit` in the period
/// `key`.
fn counted(connection: &Connection, limit: LimitId, key: &str) -> rusqlite::Result<(u64, u64)> {
    connection.query_row(
        "SELECT COALESCE(SUM(CASE WHEN state = 'held' THEN 0 ELSE charged END), 0),
                COALESCE(SUM(CASE WHEN state = 'held' THEN size ELSE 0 END), 0)
         FROM slices WHERE limit_id = ?1 AND period_start = ?2",
        (limit, key),
        |row| Ok((counted_value(row, 0)?, counted_value(row, 1)?)),
    )
}

fn stored_limits(connection: &Connection) -> rusqlite::Result<Vec<StoredLimit>> {
    let mut statement = connection.prepare(
        "SELECT limit_id, scope_kind, scope_id, usage_kind, currency, period, time_zone, amount, must_understand, set_by
         FROM limits ORDER BY limit_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(StoredLimit {
            id: row.get(0)?,
            scope: LimitScope {
                kind: row.get(1)?,
                id: row.get(2)?,
            },
            usage_kind: row.get(3)?,
            currency: row.get(4)?,
            period: row.get(5)?,
            time_zone: row.get(6)?,
            amount: counted_value(row, 7)?,
            must_understand: row.get(8)?,
            set_by: row.get(9)?,
        })
    })?;
    rows.collect()
}

/// A count the ledger stores as an integer, never negative.
fn counted_value(row: &Row<'_>, index: usize) -> rusqlite::Result<u64> {
    Ok(u64::try_from(row.get::<_, i64>(index)?).unwrap_or_default())
}

/// An amount as SQLite stores it.
fn amount(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Refuses a ledger of another major (ADR 0030): every minor of this one
/// opens.
fn check_format(found: &str) -> Result<(), LedgerError> {
    let refused = |compatibility| LedgerError::Format {
        found: found.to_owned(),
        compatibility,
    };
    let version = found
        .parse()
        .map_err(|_| refused(Compatibility::NewerMajor))?;
    match USAGE_LEDGER
        .rule
        .compatibility(USAGE_LEDGER.version, version)
    {
        Compatibility::Compatible => Ok(()),
        other => Err(refused(other)),
    }
}

mod charging;
mod period;
mod sizing;
mod standing;

#[cfg(test)]
mod tests;
