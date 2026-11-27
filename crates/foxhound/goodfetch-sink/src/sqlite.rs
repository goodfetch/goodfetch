//! The in-binary SQLite Sink, in Records mode: each Record a row of the
//! target table with its system columns, and the delivery log in
//! `_goodfetch_deliveries`, whose run epoch is the fence (ADR 0008). A
//! write checks the fence, writes its rows and moves its position in one
//! immediate transaction, so it is Fenced (ADR 0026). Its translation
//! point turns SQLite's result codes into the delivery failures (ADR
//! 0127).
//!
//! Each write mode of ADR 0026 is a statement on the key the driver gives
//! each Record, held in `_goodfetch_key` as the JSON array of its fields'
//! values, beside a unique index on the Dataset and the key: `append` adds
//! a row; `upsert` replaces the row of its key; `insert_only` leaves it;
//! `history` closes the current version (`_goodfetch_valid_to`) and adds
//! one valid from its commit; `replace` writes into a staging table, which
//! the finished Run's [`Primitives::finish`] swaps in for the target's
//! rows in one transaction, noted in `_goodfetch_swaps` so a second finish
//! does nothing.

use std::collections::BTreeMap;

use goodfetch_protocol::foxhound::sink::WriteMode;
use rusqlite::{Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior};

use crate::contract::{
    Batch, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};

#[cfg(test)]
mod tests;

/// The columns each write binds, as `?1` to `?8` in this order.
const COLUMNS: &str = "_goodfetch_delivery_id, _goodfetch_run_id, _goodfetch_run_epoch, \
     _goodfetch_dataset, _goodfetch_record_number, _goodfetch_committed_at, record, _goodfetch_key";

/// A SQLite database a crawl delivers Records into.
pub struct SqliteSink {
    connection: Connection,
    mode: WriteMode,
    /// The target table's name, as the delivery log keys it.
    target_name: String,
    /// The target table's name, quoted as an SQL identifier.
    target: String,
    /// The name of the table each write goes to: the target's, or its
    /// staging table's under `replace`.
    written_name: String,
    /// That table's name, quoted.
    written: String,
    /// Under `history`, the statement closing the current version of a
    /// Record's key before it is written.
    close: Option<String>,
    /// The statement writing one Record.
    insert: String,
    /// The statement removing the rows an older copy of a Run wrote.
    clear: String,
}

impl SqliteSink {
    /// The Sink delivering into the table `target` of the database
    /// `connection` opened, made at the first probe, under `append`.
    #[must_use]
    pub fn new(connection: Connection, target: &str) -> Self {
        Self::of(connection, target, WriteMode::Append)
    }

    /// The same Sink under the write mode `mode`.
    #[must_use]
    pub fn with_mode(self, mode: WriteMode) -> Self {
        Self::of(self.connection, &self.target_name, mode)
    }

    fn of(connection: Connection, target: &str, mode: WriteMode) -> Self {
        let written_name = match mode {
            WriteMode::Replace => format!("{target}__goodfetch_staging"),
            WriteMode::Append | WriteMode::Upsert | WriteMode::InsertOnly | WriteMode::History => {
                target.to_owned()
            }
        };
        let written = quoted(&written_name);
        let identity = identity(mode);
        let values = "?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8";
        let insert = match mode {
            WriteMode::Append => format!(
                "INSERT INTO {written} AS held ({COLUMNS}) VALUES ({values})
                 ON CONFLICT (_goodfetch_delivery_id) DO UPDATE SET
                     _goodfetch_run_epoch = excluded._goodfetch_run_epoch,
                     _goodfetch_committed_at = excluded._goodfetch_committed_at,
                     record = excluded.record
                 WHERE excluded._goodfetch_run_epoch > held._goodfetch_run_epoch"
            ),
            // The last Record of a key wins (ADR 0026); one this table
            // already holds, by its delivery ID, is not written twice.
            WriteMode::Upsert | WriteMode::Replace => format!(
                "INSERT INTO {written} ({COLUMNS})
                 SELECT {values}
                 WHERE NOT EXISTS (SELECT 1 FROM {written} WHERE _goodfetch_delivery_id = ?1)
                 ON CONFLICT ({identity}) WHERE _goodfetch_key IS NOT NULL
                 DO UPDATE SET
                     _goodfetch_delivery_id = excluded._goodfetch_delivery_id,
                     _goodfetch_run_id = excluded._goodfetch_run_id,
                     _goodfetch_run_epoch = excluded._goodfetch_run_epoch,
                     _goodfetch_record_number = excluded._goodfetch_record_number,
                     _goodfetch_committed_at = excluded._goodfetch_committed_at,
                     record = excluded.record"
            ),
            WriteMode::InsertOnly => format!(
                "INSERT INTO {written} ({COLUMNS}) VALUES ({values}) ON CONFLICT DO NOTHING"
            ),
            WriteMode::History => format!(
                "INSERT INTO {written} ({COLUMNS}, _goodfetch_valid_from) VALUES ({values}, ?6)
                 ON CONFLICT (_goodfetch_delivery_id) DO NOTHING"
            ),
        };
        let close = (mode == WriteMode::History).then(|| {
            format!(
                "UPDATE {written} SET _goodfetch_valid_to = ?6
                 WHERE _goodfetch_dataset = ?4 AND _goodfetch_key = ?8
                     AND _goodfetch_valid_to IS NULL
                     AND NOT EXISTS (SELECT 1 FROM {written} WHERE _goodfetch_delivery_id = ?1)"
            )
        });
        Self {
            connection,
            mode,
            target_name: target.to_owned(),
            target: quoted(target),
            clear: format!(
                "DELETE FROM {written} WHERE _goodfetch_run_id = ?1 AND _goodfetch_run_epoch < ?2"
            ),
            close,
            insert,
            written,
            written_name,
        }
    }

    /// Makes `table`, named `name`, with the columns and the index the
    /// write mode needs; `keyed` gives it the unique index on the key.
    fn ready(&self, name: &str, table: &str, keyed: bool) -> rusqlite::Result<()> {
        self.connection.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                 _goodfetch_delivery_id TEXT NOT NULL PRIMARY KEY,
                 _goodfetch_run_id TEXT NOT NULL,
                 _goodfetch_run_epoch INTEGER NOT NULL,
                 _goodfetch_dataset TEXT NOT NULL,
                 _goodfetch_record_number INTEGER NOT NULL,
                 _goodfetch_committed_at TEXT NOT NULL,
                 record TEXT NOT NULL,
                 _goodfetch_key TEXT
             ) STRICT;"
        ))?;
        // A table an earlier mode made lacks the columns of a later one:
        // columns are only added (ADR 0026).
        let held = self
            .connection
            .prepare_cached("SELECT name FROM pragma_table_info(?1)")?
            .query_map([name], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let history = keyed && self.mode == WriteMode::History;
        let needed: &[&str] = if history {
            &[
                "_goodfetch_key",
                "_goodfetch_valid_from",
                "_goodfetch_valid_to",
            ]
        } else {
            &["_goodfetch_key"]
        };
        for column in needed {
            if !held.iter().any(|held| held == column) {
                self.connection
                    .execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT"))?;
            }
        }
        if !keyed {
            return Ok(());
        }
        let key_index = quoted(&format!("{name}__goodfetch_key"));
        if history {
            // One current version of each key; the closed ones beside it.
            self.connection.execute_batch(&format!(
                "DROP INDEX IF EXISTS {key_index};
                 CREATE UNIQUE INDEX IF NOT EXISTS {current} ON {table}
                     (_goodfetch_dataset, _goodfetch_key)
                     WHERE _goodfetch_key IS NOT NULL AND _goodfetch_valid_to IS NULL;",
                current = quoted(&format!("{name}__goodfetch_current")),
            ))
        } else {
            self.connection.execute_batch(&format!(
                "CREATE UNIQUE INDEX IF NOT EXISTS {key_index} ON {table}
                     ({identity}) WHERE _goodfetch_key IS NOT NULL;",
                identity = identity(self.mode),
            ))
        }
    }
}

/// The columns that identify a Record among the written table's rows
/// under `mode`: its Dataset and key, and under `replace` its Run too,
/// since the staging table holds each Run's Records apart until its finish.
fn identity(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::Replace => "_goodfetch_run_id, _goodfetch_dataset, _goodfetch_key",
        WriteMode::Append | WriteMode::Upsert | WriteMode::InsertOnly | WriteMode::History => {
            "_goodfetch_dataset, _goodfetch_key"
        }
    }
}

/// `name` quoted as an SQL identifier.
fn quoted(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// A count or number as SQLite's integer.
fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The fence of the Run `run_id` at the target `target`: the highest run
/// epoch its delivery log holds, none before its first write.
fn fence(
    transaction: &Transaction<'_>,
    target: &str,
    run_id: &str,
) -> rusqlite::Result<Option<i64>> {
    Ok(transaction
        .prepare_cached(
            "SELECT max(run_epoch) FROM _goodfetch_deliveries WHERE target = ?1 AND run_id = ?2",
        )?
        .query_row((target, run_id), |row| row.get(0))
        .optional()?
        .flatten())
}

/// The later run epoch `fence` holds than `run_epoch`, as a write's answer.
fn superseded(fence: Option<i64>, run_epoch: i64) -> Option<Written> {
    fence
        .filter(|&fence| fence > run_epoch)
        .map(|later| Written::Superseded {
            run_epoch: u64::try_from(later).unwrap_or_default(),
        })
}

impl Primitives for SqliteSink {
    type Error = rusqlite::Error;

    fn probe(&mut self, run: &RunCopy) -> rusqlite::Result<Probe> {
        let keyed = self.mode != WriteMode::Append;
        if self.mode == WriteMode::Replace {
            self.ready(&self.target_name, &self.target, false)?;
            self.connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS _goodfetch_swaps (
                     target TEXT NOT NULL,
                     run_id TEXT NOT NULL,
                     run_epoch INTEGER NOT NULL,
                     PRIMARY KEY (target, run_id)
                 ) STRICT, WITHOUT ROWID;",
            )?;
        }
        self.ready(&self.written_name, &self.written, keyed)?;
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS _goodfetch_deliveries (
                 target TEXT NOT NULL,
                 run_id TEXT NOT NULL,
                 dataset TEXT NOT NULL,
                 run_epoch INTEGER NOT NULL,
                 last_number INTEGER NOT NULL,
                 PRIMARY KEY (target, run_id, dataset)
             ) STRICT, WITHOUT ROWID;",
        )?;
        let mut positions = self.connection.prepare_cached(
            "SELECT dataset, last_number FROM _goodfetch_deliveries
             WHERE target = ?1 AND run_id = ?2 AND run_epoch = ?3",
        )?;
        let delivered = positions
            .query_map(
                (&self.target_name, &run.run_id, integer(run.run_epoch)),
                |row| {
                    let last: i64 = row.get(1)?;
                    Ok((row.get(0)?, u64::try_from(last).unwrap_or_default()))
                },
            )?
            .collect::<rusqlite::Result<BTreeMap<String, u64>>>()?;
        Ok(Probe {
            class: GuaranteeClass::Fenced,
            delivered,
        })
    }

    fn write(&mut self, batch: &Batch) -> rusqlite::Result<Written> {
        let Some(last) = batch.records.last().map(|record| record.number) else {
            return Ok(Written::Delivered);
        };
        let run_epoch = integer(batch.run.run_epoch);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fence = fence(&transaction, &self.target_name, &batch.run.run_id)?;
        if let Some(superseded) = superseded(fence, run_epoch) {
            return Ok(superseded);
        }
        {
            if fence.is_some_and(|fence| fence < run_epoch) {
                // This copy takes the Run over: the older copy's rows go,
                // those past this copy's reach included, so the
                // destination ends with this copy's Records alone.
                transaction
                    .prepare_cached(&self.clear)?
                    .execute((&batch.run.run_id, run_epoch))?;
                if self.mode == WriteMode::History {
                    // A version the older copy closed is current again
                    // when the version that closed it went with it.
                    transaction
                        .prepare_cached(&format!(
                            "UPDATE {written} SET _goodfetch_valid_to = NULL WHERE rowid IN (
                                 SELECT max(rowid) FROM {written} WHERE _goodfetch_key IS NOT NULL
                                 GROUP BY _goodfetch_dataset, _goodfetch_key
                                 HAVING count(_goodfetch_valid_to) = count(*))",
                            written = self.written,
                        ))?
                        .execute([])?;
                }
                transaction
                    .prepare_cached(
                        "DELETE FROM _goodfetch_deliveries
                         WHERE target = ?1 AND run_id = ?2 AND run_epoch < ?3",
                    )?
                    .execute((&self.target_name, &batch.run.run_id, run_epoch))?;
            }
            let mut close = match &self.close {
                Some(close) => Some(transaction.prepare_cached(close)?),
                None => None,
            };
            let mut insert = transaction.prepare_cached(&self.insert)?;
            for record in &batch.records {
                let value = record.value.to_string();
                let key = record.key.as_ref().map(ToString::to_string);
                let row = (
                    record.delivery_id.as_str(),
                    batch.run.run_id.as_str(),
                    run_epoch,
                    batch.dataset.as_str(),
                    integer(record.number),
                    record.committed_at.as_str(),
                    value.as_str(),
                    key.as_deref(),
                );
                if let Some(close) = close.as_mut() {
                    close.execute(row)?;
                }
                insert.execute(row)?;
            }
            transaction
                .prepare_cached(
                    "INSERT INTO _goodfetch_deliveries (target, run_id, dataset, run_epoch, last_number)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT (target, run_id, dataset) DO UPDATE SET
                         last_number = CASE WHEN run_epoch = excluded.run_epoch
                             THEN max(last_number, excluded.last_number)
                             ELSE excluded.last_number END,
                         run_epoch = excluded.run_epoch",
                )?
                .execute((
                    &self.target_name,
                    &batch.run.run_id,
                    &batch.dataset,
                    run_epoch,
                    integer(last),
                ))?;
        }
        transaction.commit()?;
        Ok(Written::Delivered)
    }

    fn finish(&mut self, run: &RunCopy) -> rusqlite::Result<Written> {
        if self.mode != WriteMode::Replace {
            return Ok(Written::Delivered);
        }
        let run_epoch = integer(run.run_epoch);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let swapped: Option<i64> = transaction
            .prepare_cached(
                "SELECT run_epoch FROM _goodfetch_swaps WHERE target = ?1 AND run_id = ?2",
            )?
            .query_row((&self.target_name, &run.run_id), |row| row.get(0))
            .optional()?;
        let fence = fence(&transaction, &self.target_name, &run.run_id)?.max(swapped);
        if let Some(superseded) = superseded(fence, run_epoch) {
            return Ok(superseded);
        }
        if swapped != Some(run_epoch) {
            transaction.execute_batch(&format!("DELETE FROM {target}", target = self.target))?;
            transaction
                .prepare_cached(&format!(
                    "INSERT INTO {target} ({COLUMNS}) SELECT {COLUMNS} FROM {written}
                     WHERE _goodfetch_run_id = ?1 AND _goodfetch_run_epoch = ?2
                     ORDER BY _goodfetch_dataset, _goodfetch_record_number",
                    target = self.target,
                    written = self.written,
                ))?
                .execute((&run.run_id, run_epoch))?;
            transaction
                .prepare_cached(&format!(
                    "DELETE FROM {written} WHERE _goodfetch_run_id = ?1",
                    written = self.written,
                ))?
                .execute([&run.run_id])?;
            transaction
                .prepare_cached(
                    "INSERT INTO _goodfetch_swaps (target, run_id, run_epoch) VALUES (?1, ?2, ?3)
                     ON CONFLICT (target, run_id) DO UPDATE SET run_epoch = excluded.run_epoch",
                )?
                .execute((&self.target_name, &run.run_id, run_epoch))?;
        }
        transaction.commit()?;
        Ok(Written::Delivered)
    }

    fn map_failure(&self, error: rusqlite::Error) -> Failure {
        let kind = match &error {
            rusqlite::Error::SqliteFailure(failure, _) => kind(failure.code),
            _ => FailureKind::Unrecognized,
        };
        Failure::new(kind, error.to_string())
    }
}

/// What SQLite's result `code` leads to.
fn kind(code: ErrorCode) -> FailureKind {
    match code {
        ErrorCode::DatabaseBusy
        | ErrorCode::DatabaseLocked
        | ErrorCode::DiskFull
        | ErrorCode::SystemIoFailure
        | ErrorCode::OutOfMemory => FailureKind::Transient,
        ErrorCode::ReadOnly
        | ErrorCode::PermissionDenied
        | ErrorCode::AuthorizationForStatementDenied => FailureKind::Auth,
        ErrorCode::CannotOpen | ErrorCode::NotADatabase => FailureKind::Gone,
        ErrorCode::ConstraintViolation | ErrorCode::TooBig | ErrorCode::TypeMismatch => {
            FailureKind::RejectedRecords
        }
        _ => FailureKind::Unrecognized,
    }
}
