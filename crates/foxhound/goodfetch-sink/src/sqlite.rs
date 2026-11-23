//! The in-binary SQLite Sink, in Records mode with `append`: each Record a
//! row of the target table with its system columns, and the delivery log
//! in `_goodfetch_deliveries`, whose run epoch is the fence (ADR 0008). A
//! write checks the fence, writes its rows and moves its position in one
//! immediate transaction, so it is Fenced (ADR 0026). Its translation
//! point turns SQLite's result codes into the delivery failures (ADR
//! 0127).

use std::collections::BTreeMap;

use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior};

use crate::contract::{
    Batch, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};

#[cfg(test)]
mod tests;

/// A SQLite database a crawl delivers Records into.
pub struct SqliteSink {
    connection: Connection,
    /// The target table's name, quoted as an SQL identifier.
    target: String,
    /// The target table's name, as the delivery log keys it.
    target_name: String,
    /// The statement writing one Record into the target table.
    insert: String,
    /// The statement removing the rows an older copy of a Run wrote.
    clear: String,
}

impl SqliteSink {
    /// The Sink delivering into the table `target` of the database
    /// `connection` opened, made at the first probe.
    #[must_use]
    pub fn new(connection: Connection, target: &str) -> Self {
        let quoted = format!("\"{}\"", target.replace('"', "\"\""));
        Self {
            connection,
            insert: format!(
                "INSERT INTO {quoted} AS held (
                     _goodfetch_delivery_id, _goodfetch_run_id, _goodfetch_run_epoch,
                     _goodfetch_dataset, _goodfetch_record_number, _goodfetch_committed_at, record
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT (_goodfetch_delivery_id) DO UPDATE SET
                     _goodfetch_run_epoch = excluded._goodfetch_run_epoch,
                     _goodfetch_committed_at = excluded._goodfetch_committed_at,
                     record = excluded.record
                 WHERE excluded._goodfetch_run_epoch > held._goodfetch_run_epoch"
            ),
            clear: format!(
                "DELETE FROM {quoted} WHERE _goodfetch_run_id = ?1 AND _goodfetch_run_epoch < ?2"
            ),
            target: quoted,
            target_name: target.to_owned(),
        }
    }
}

/// A count or number as SQLite's integer.
fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

impl Primitives for SqliteSink {
    type Error = rusqlite::Error;

    fn probe(&mut self, run: &RunCopy) -> rusqlite::Result<Probe> {
        self.connection.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {target} (
                 _goodfetch_delivery_id TEXT NOT NULL PRIMARY KEY,
                 _goodfetch_run_id TEXT NOT NULL,
                 _goodfetch_run_epoch INTEGER NOT NULL,
                 _goodfetch_dataset TEXT NOT NULL,
                 _goodfetch_record_number INTEGER NOT NULL,
                 _goodfetch_committed_at TEXT NOT NULL,
                 record TEXT NOT NULL
             ) STRICT;
             CREATE TABLE IF NOT EXISTS _goodfetch_deliveries (
                 target TEXT NOT NULL,
                 run_id TEXT NOT NULL,
                 dataset TEXT NOT NULL,
                 run_epoch INTEGER NOT NULL,
                 last_number INTEGER NOT NULL,
                 PRIMARY KEY (target, run_id, dataset)
             ) STRICT, WITHOUT ROWID;",
            target = self.target,
        ))?;
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
        let fence: Option<i64> = transaction
            .prepare_cached(
                "SELECT max(run_epoch) FROM _goodfetch_deliveries WHERE target = ?1 AND run_id = ?2",
            )?
            .query_row((&self.target_name, &batch.run.run_id), |row| row.get(0))
            .optional()?
            .flatten();
        if let Some(later) = fence.filter(|&fence| fence > run_epoch) {
            return Ok(Written::Superseded {
                run_epoch: u64::try_from(later).unwrap_or_default(),
            });
        }
        {
            if fence.is_some_and(|fence| fence < run_epoch) {
                // This copy takes the Run over: the older copy's rows go,
                // those past this copy's reach included, so the
                // destination ends with this copy's Records alone.
                transaction
                    .prepare_cached(&self.clear)?
                    .execute((&batch.run.run_id, run_epoch))?;
                transaction
                    .prepare_cached(
                        "DELETE FROM _goodfetch_deliveries
                         WHERE target = ?1 AND run_id = ?2 AND run_epoch < ?3",
                    )?
                    .execute((&self.target_name, &batch.run.run_id, run_epoch))?;
            }
            let mut insert = transaction.prepare_cached(&self.insert)?;
            for record in &batch.records {
                insert.execute((
                    record.delivery_id.as_str(),
                    &batch.run.run_id,
                    run_epoch,
                    &batch.dataset,
                    integer(record.number),
                    &record.committed_at,
                    record.value.to_string(),
                ))?;
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
