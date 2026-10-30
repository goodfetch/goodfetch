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
use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::run::{LimitScope, Period, PeriodLength, PointInTime, ReachedLimit};
use goodfetch_protocol::usage::{Limit, LimitUsage};
use jiff::Timestamp;
use rusqlite::{Connection, OpenFlags, Row, TransactionBehavior};

pub use self::charging::{Charged, Charging, Refusal, Reservation};
pub use self::period::{PeriodBounds, bounds};
pub use self::sizing::{SliceSizing, slice_size};
pub use self::standing::{Placement, standing};
pub use self::temporary::{temporary_filesystem, temporary_in};

/// The target of the `tracing` events that name the slices and reclaim
/// model's transitions (`test-support/models/slices/`), each with its
/// `transition` and the model's variables as fields.
pub const TARGET: &str = "goodfetch::ledger";

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
        set_at TEXT NOT NULL,
        managed_major TEXT
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
    /// The disk was full or failed an I/O at a transaction (ADR 0122): it
    /// was rolled back, and nothing of it is acknowledged.
    #[error("the Usage ledger at `{path}` could not be written to its storage: {message}")]
    Storage {
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
    /// When it was set last, as written: added, raised or rewritten by its
    /// managed Operator policy.
    pub set_at: String,
    /// The major of the managed Operator policy whose row it is.
    pub managed_major: Option<String>,
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
        if let Some(folder) = self.path.parent() {
            #[expect(
                clippy::disallowed_methods,
                reason = "the ledger's calls run under their caller's blocking policy"
            )]
            std::fs::create_dir_all(folder).map_err(|error| self.failed(&error))?;
        }
        let connection = self
            .connect(OpenFlags::SQLITE_OPEN_CREATE)
            .map_err(|error| self.failed(&error))?;
        self.prepared(connection)
    }

    /// Opens the file as [`Ledger::open`] does when it exists; none when no
    /// engine created it yet, so it holds no Slice.
    fn open_existing(&self) -> Result<Option<Connection>, LedgerError> {
        match self.connect(OpenFlags::empty()) {
            Ok(connection) => self.prepared(connection).map(Some),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.code == rusqlite::ErrorCode::CannotOpen && self.absent() =>
            {
                Ok(None)
            }
            Err(error) => Err(self.failed(&error)),
        }
    }

    /// Whether the file is missing, as far as it can tell: through a VFS of
    /// a test's, SQLite's refusal to open it without creating it says so.
    fn absent(&self) -> bool {
        self.vfs.is_some() || matches!(self.path.try_exists(), Ok(false))
    }

    /// Connects to the file read-write, with `extra` flags.
    fn connect(&self, extra: OpenFlags) -> rusqlite::Result<Connection> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX | extra;
        match &self.vfs {
            Some(vfs) => Connection::open_with_flags_and_vfs(&self.path, flags, vfs.as_str()),
            None => Connection::open_with_flags(&self.path, flags),
        }
    }

    /// Sets up `connection`, creating the tables it lacks, and refuses a
    /// ledger of another major.
    fn prepared(&self, mut connection: Connection) -> Result<Connection, LedgerError> {
        let failed = |error: &(dyn std::error::Error + 'static)| self.failed(error);
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

    /// The failure of a transaction of the ledger on `error`: a full disk
    /// or an I/O error as such, after which SQLite may have rolled the whole
    /// transaction back, and the transaction's drop rolls back the rest (ADR
    /// 0122).
    fn failed(&self, error: &(dyn std::error::Error + 'static)) -> LedgerError {
        let storage = error
            .downcast_ref::<rusqlite::Error>()
            .and_then(rusqlite::Error::sqlite_error_code)
            .is_some_and(|code| {
                matches!(
                    code,
                    rusqlite::ErrorCode::DiskFull | rusqlite::ErrorCode::SystemIoFailure
                )
            });
        if storage {
            LedgerError::Storage {
                path: self.path.clone(),
                message: error.to_string(),
            }
        } else {
            LedgerError::Sqlite {
                path: self.path.clone(),
                message: error.to_string(),
            }
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
        let id =
            insert_limit(&connection, limit, None, now).map_err(|error| self.failed(&error))?;
        tracing::trace!(target: TARGET, transition = "set_limit", limit = id, amount = limit.amount, "a Limit was added");
        Ok(id)
    }

    /// Sets `limit`, a person's, in place of every Limit row of its scope,
    /// Usage kind, currency and period that a managed Operator policy did
    /// not set, keeping the Usage counted against each, in one
    /// transaction; adds it when there is none (ADR 0041). Answers its row,
    /// the first when there are several.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be written.
    pub fn raise(&self, limit: &Limit, now: Timestamp) -> Result<LimitId, LedgerError> {
        let mut connection = self.open()?;
        let period = limit.period.as_ref().map(|period| period.every.as_str());
        let time_zone = limit
            .period
            .as_ref()
            .map(|period| period.time_zone.as_str());
        let id = (|| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            transaction.execute(
                "UPDATE limits SET amount = ?1, must_understand = ?2, set_by = ?3, set_at = ?4
                 WHERE scope_kind = ?5 AND scope_id = ?6 AND usage_kind = ?7 AND currency IS ?8
                   AND period IS ?9 AND time_zone IS ?10 AND set_by <> 'managed_policy'",
                (
                    amount(limit.amount),
                    limit.must_understand,
                    limit.set_by.as_str(),
                    now.to_string(),
                    &limit.scope.kind,
                    &limit.scope.id,
                    &limit.usage_kind,
                    &limit.currency,
                    period,
                    time_zone,
                ),
            )?;
            let found: Option<LimitId> = transaction.query_row(
                "SELECT MIN(limit_id) FROM limits
                 WHERE scope_kind = ?1 AND scope_id = ?2 AND usage_kind = ?3 AND currency IS ?4
                   AND period IS ?5 AND time_zone IS ?6 AND set_by <> 'managed_policy'",
                (
                    &limit.scope.kind,
                    &limit.scope.id,
                    &limit.usage_kind,
                    &limit.currency,
                    period,
                    time_zone,
                ),
                |row| row.get(0),
            )?;
            let id = match found {
                Some(id) => id,
                None => insert_limit(&transaction, limit, None, now)?,
            };
            transaction.commit()?;
            Ok(id)
        })()
        .map_err(|error: rusqlite::Error| self.failed(&error))?;
        tracing::trace!(target: TARGET, transition = "set_limit", limit = id, amount = limit.amount, "a person set a Limit");
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

    /// Every Limit, as [`Ledger::limits`] reads them, on `kept`, a
    /// connection its caller keeps between reads, opened now when it holds
    /// none and let go when a read fails, so the next one opens afresh
    /// (the deep review's 599-12: a run's every charge reopened the file).
    ///
    /// # Errors
    ///
    /// When the ledger cannot be read.
    fn limits_on(&self, kept: &mut Option<Connection>) -> Result<Vec<StoredLimit>, LedgerError> {
        let read = match kept {
            Some(connection) => stored_limits(connection),
            None => stored_limits(kept.insert(self.open()?)),
        };
        read.map_err(|error| {
            *kept = None;
            self.failed(&error)
        })
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
            .iter()
            .map(|stored| {
                limit_usage(&connection, stored, product, now).map_err(|error| self.failed(&error))
            })
            .collect()
    }

    /// The Limit of row `id` as [`Ledger::usage`] gives it; none when no
    /// row has it.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be read.
    pub fn usage_of(
        &self,
        id: LimitId,
        product: Product,
        now: Timestamp,
    ) -> Result<Option<LimitUsage>, LedgerError> {
        let connection = self.open()?;
        let limits = stored_limits(&connection).map_err(|error| self.failed(&error))?;
        limits
            .iter()
            .find(|stored| stored.id == id)
            .map(|stored| {
                limit_usage(&connection, stored, product, now).map_err(|error| self.failed(&error))
            })
            .transpose()
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

    /// Writes `limits`, the install Limits the managed Operator policy of
    /// `major` names, as that major's managed rows, in one transaction
    /// (ADR 0041): a row of the same Usage kind, currency and period takes
    /// the policy's amount and keeps the Usage counted against it, a Limit
    /// it lacks is added, and a row of that major the policy no longer
    /// names is dropped with its Slices. Rows of other majors, and rows
    /// anyone else set, are left as they are. Each engine does this when it
    /// starts; with no Limit to write, a ledger no engine created holds no
    /// row to drop, and stays uncreated.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be written.
    pub fn write_managed(
        &self,
        major: &str,
        limits: &[Limit],
        now: Timestamp,
    ) -> Result<(), LedgerError> {
        let opened = if limits.is_empty() {
            self.open_existing()?
        } else {
            Some(self.open()?)
        };
        let Some(mut connection) = opened else {
            return Ok(());
        };
        (|| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let held: Vec<StoredLimit> = stored_limits(&transaction)?
                .into_iter()
                .filter(|stored| stored.managed_major.as_deref() == Some(major))
                .collect();
            let mut kept = Vec::new();
            for limit in limits {
                let period = limit.period.as_ref().map(|period| period.every.as_str());
                let time_zone = limit
                    .period
                    .as_ref()
                    .map(|period| period.time_zone.as_str());
                let same = held.iter().find(|stored| {
                    stored.scope == limit.scope
                        && stored.usage_kind == limit.usage_kind
                        && stored.currency == limit.currency
                        && stored.period.as_deref() == period
                        && stored.time_zone.as_deref() == time_zone
                        && !kept.contains(&stored.id)
                });
                match same {
                    Some(stored) if stored.amount == limit.amount => kept.push(stored.id),
                    Some(stored) => {
                        transaction.execute(
                            "UPDATE limits SET amount = ?1, set_at = ?2 WHERE limit_id = ?3",
                            (amount(limit.amount), now.to_string(), stored.id),
                        )?;
                        kept.push(stored.id);
                    }
                    None => kept.push(insert_limit(&transaction, limit, Some(major), now)?),
                }
            }
            for stored in held.iter().filter(|stored| !kept.contains(&stored.id)) {
                drop_limit(&transaction, stored.id)?;
            }
            transaction.commit()
        })()
        .map_err(|error: rusqlite::Error| self.failed(&error))?;
        tracing::trace!(target: TARGET, transition = "set_limit", major, limits = limits.len(), "a managed Operator policy's Limits were written");
        Ok(())
    }

    /// What `upgrade-files` does to the managed rows when it gives `to` its
    /// copy of `from`'s files (ADR 0041): `from`'s rows become `to`'s,
    /// keeping their Usage, unless `to` has rows of its own already, and
    /// the rows of every other major `older` than `to` names, whether
    /// still supported or not, are dropped, so that an uninstalled major's
    /// cap does not linger. An older major still in use writes its rows
    /// back when it starts. Answers how many rows `to` took over and how
    /// many were dropped.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be written.
    pub fn upgrade_managed(
        &self,
        from: &str,
        to: &str,
        older: impl Fn(&str) -> bool,
    ) -> Result<(usize, usize), LedgerError> {
        let mut connection = self.open()?;
        let counts = (|| {
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let managed: Vec<(LimitId, String)> = stored_limits(&transaction)?
                .into_iter()
                .filter_map(|stored| stored.managed_major.map(|major| (stored.id, major)))
                .collect();
            let has_own = managed.iter().any(|(_, major)| major == to);
            let (mut taken, mut dropped) = (0, 0);
            for (id, major) in &managed {
                if major == to || !older(major) {
                    continue;
                }
                if major == from && !has_own {
                    transaction.execute(
                        "UPDATE limits SET managed_major = ?1 WHERE limit_id = ?2",
                        (to, id),
                    )?;
                    taken += 1;
                } else {
                    drop_limit(&transaction, *id)?;
                    dropped += 1;
                }
            }
            transaction.commit()?;
            Ok((taken, dropped))
        })()
        .map_err(|error: rusqlite::Error| self.failed(&error))?;
        tracing::trace!(target: TARGET, transition = "set_limit", from, to, taken = counts.0, dropped = counts.1, "upgrade-files replaced older majors' managed Limits");
        Ok(counts)
    }

    /// Takes a Slice of the Limit `stored` for `holder`, in the period that
    /// holds `now`, in one transaction: sized by `sizing` from what is left
    /// and the live runs holding Slices of it, and at least `needing`, as a
    /// reserved worst case needs; none when less than that is left, or
    /// nothing.
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
        needing: u64,
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
            let size = slice_size(sizing, stored.amount, left, others + 1).max(needing);
            if size == 0 || size > left {
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
                tracing::trace!(target: TARGET, transition = "take_slice", limit = stored.id, slice = slice.id, size = slice.size, run_id = %holder.run_id, epoch = holder.epoch, "a run took a Slice");
            }
            None => {
                tracing::trace!(target: TARGET, transition = "refuse_slice", limit = stored.id, run_id = %holder.run_id, epoch = holder.epoch, "a Limit had nothing left for a run");
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
            tracing::trace!(target: TARGET, transition = "give_back", slice = slice, used = used, "a run gave back a Slice");
        }
        Ok(())
    }

    /// Reclaims every held Slice whose run `probe` finds gone: charges what
    /// the run journaled under the Slice's epoch, at most the Slice, or the
    /// whole Slice, marked for review, when its run state cannot be read.
    /// A ledger no engine created yet holds no Slice, and stays uncreated.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be read or written.
    pub fn reclaim(&self, probe: &dyn Fn(&Holder) -> Probed) -> Result<Reclaimed, LedgerError> {
        let Some(mut connection) = self.open_existing()? else {
            return Ok(Reclaimed::default());
        };
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
                    tracing::trace!(target: TARGET, transition = "mark_for_review", slice = slice, charged = charged, run_id = %holder.run_id, epoch = holder.epoch, "a dead run's Slice was charged in full for review");
                    reclaimed.for_review.push(slice);
                } else {
                    tracing::trace!(target: TARGET, transition = "reclaim", slice = slice, charged = charged, run_id = %holder.run_id, epoch = holder.epoch, "a dead run's Slice was reclaimed");
                    reclaimed.charged.push(slice);
                }
            }
        }
        Ok(reclaimed)
    }
}

/// Adds the row of `limit`, set at `now`, a managed Operator policy's of
/// `managed_major` when it names one.
fn insert_limit(
    connection: &Connection,
    limit: &Limit,
    managed_major: Option<&str>,
    now: Timestamp,
) -> rusqlite::Result<LimitId> {
    connection.execute(
        "INSERT INTO limits (scope_kind, scope_id, usage_kind, currency, period, time_zone, amount, must_understand, set_by, set_at, managed_major)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            managed_major,
        ),
    )?;
    Ok(connection.last_insert_rowid())
}

/// Drops the Limit row `id` and its Slices.
fn drop_limit(connection: &Connection, id: LimitId) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM slices WHERE limit_id = ?1", [id])?;
    connection.execute("DELETE FROM limits WHERE limit_id = ?1", [id])?;
    Ok(())
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

/// The Limit `stored` with what `product`'s engine makes of it and its
/// Usage in the period that holds `now`.
fn limit_usage(
    connection: &Connection,
    stored: &StoredLimit,
    product: Product,
    now: Timestamp,
) -> rusqlite::Result<LimitUsage> {
    let (key, found) = stored.period_key(now);
    let (charged, held) = counted(connection, stored.id, &key)?;
    Ok(LimitUsage {
        limit: stored.limit(),
        standing: standing(stored, product),
        charged,
        held,
        left: stored.amount.saturating_sub(charged.saturating_add(held)),
        period_started_at: found.map(|found| point_in_time(found.start)),
        resets_at: found.map(|found| point_in_time(found.end)),
        managed_major: stored.managed_major.clone(),
    })
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
        "SELECT limit_id, scope_kind, scope_id, usage_kind, currency, period, time_zone, amount, must_understand, set_by, managed_major, set_at
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
            managed_major: row.get(10)?,
            set_at: row.get(11)?,
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
mod temporary;

#[cfg(test)]
mod tests;
