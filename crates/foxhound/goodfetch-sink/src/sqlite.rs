//! The in-binary SQLite Sink, in Records mode: each Record a row of the
//! target table with its `_goodfetch_*` system columns and normalized
//! `snake_case` typed field columns, and the delivery log in
//! `_goodfetch_deliveries`, whose run epoch is the fence (ADR 0008). A
//! write checks the fence, evolves the table per `schema.contract` and
//! `schema.flatten`, writes its rows and moves its position in one
//! immediate transaction, so it is Fenced (ADR 0026). Its translation
//! point turns SQLite's result codes and schema-freeze refusals into the
//! delivery failures (ADR 0127).
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

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_protocol::foxhound::sink::{SchemaContract, SinkSchema, WriteMode};
use rusqlite::types::Value as SqlValue;
use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Transaction, TransactionBehavior, params_from_iter,
};

use crate::contract::{
    Batch, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};
use crate::schema::{
    ColumnType, ColumnValue, PreparedRow, RowOutcome, SchemaChange, SchemaCounters, TableSchema,
};

#[cfg(test)]
mod tests;

/// The fixed system columns every write binds first, as `?1` to `?8`.
const SYSTEM_COLUMNS: [&str; 8] = [
    "_goodfetch_delivery_id",
    "_goodfetch_run_id",
    "_goodfetch_run_epoch",
    "_goodfetch_dataset",
    "_goodfetch_record_number",
    "_goodfetch_committed_at",
    "_goodfetch_meta",
    "_goodfetch_key",
];

/// A failure from the SQLite Sink before translation into [`Failure`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SqliteError {
    /// SQLite returned an error.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    /// The Sink's `freeze` schema contract refused a Record (ADR 0026).
    #[error("schema contract `freeze` refused Record: {0}")]
    SchemaFrozen(String),
}

/// A SQLite database a crawl delivers Records into.
pub struct SqliteSink {
    connection: Connection,
    mode: WriteMode,
    schema_settings: SinkSchema,
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
    /// The statement removing the rows an older copy of a Run wrote.
    clear: String,
    /// Current user-column schema of the written table.
    schema: TableSchema,
    /// Datasets whose declared JSON Schema has already been applied.
    declared_applied: BTreeSet<String>,
    /// Schema changes applied so far, in order.
    schema_changes: Vec<SchemaChange>,
    /// Rows and values dropped under `discard_row` / `discard_value`.
    schema_counters: SchemaCounters,
}

impl SqliteSink {
    /// The Sink delivering into the table `target` of the database
    /// `connection` opened, made at the first probe, under `append` and
    /// `evolve`.
    #[must_use]
    pub fn new(connection: Connection, target: &str) -> Self {
        Self::of(connection, target, WriteMode::Append, SinkSchema::default())
    }

    /// The same Sink under the write mode `mode`.
    #[must_use]
    pub fn with_mode(self, mode: WriteMode) -> Self {
        Self::of(
            self.connection,
            &self.target_name,
            mode,
            self.schema_settings,
        )
    }

    /// The same Sink with `schema` (`schema.contract` and `schema.flatten`,
    /// ADR 0026).
    #[must_use]
    pub fn with_schema(mut self, schema: SinkSchema) -> Self {
        self.schema_settings = schema;
        self
    }

    /// Every schema change this Sink has applied so far.
    #[must_use]
    pub fn schema_changes(&self) -> &[SchemaChange] {
        &self.schema_changes
    }

    /// Rows and values dropped by non-evolving schema contracts so far.
    #[must_use]
    pub const fn schema_counters(&self) -> SchemaCounters {
        self.schema_counters
    }

    fn contract(&self) -> SchemaContract {
        self.schema_settings.contract.unwrap_or_default()
    }

    fn flatten(&self) -> bool {
        self.schema_settings.flatten.unwrap_or(false)
    }

    fn of(
        connection: Connection,
        target: &str,
        mode: WriteMode,
        schema_settings: SinkSchema,
    ) -> Self {
        let written_name = match mode {
            WriteMode::Replace => format!("{target}__goodfetch_staging"),
            WriteMode::Append | WriteMode::Upsert | WriteMode::InsertOnly | WriteMode::History => {
                target.to_owned()
            }
        };
        let written = quoted(&written_name);
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
            schema_settings,
            target_name: target.to_owned(),
            target: quoted(target),
            clear: format!(
                "DELETE FROM {written} WHERE _goodfetch_run_id = ?1 AND _goodfetch_run_epoch < ?2"
            ),
            close,
            written,
            written_name,
            schema: TableSchema::default(),
            declared_applied: BTreeSet::new(),
            schema_changes: Vec::new(),
            schema_counters: SchemaCounters::default(),
        }
    }

    /// The highest `run_epoch` already recorded in `_goodfetch_deliveries` for
    /// `run_id` against this Sink's target, if any. Used by post-Run delivery
    /// operations (`sink.deliver`) so redelivering or replaying does not raise
    /// the epoch above already-delivered rows and wipe them.
    #[must_use]
    pub fn current_epoch(&self, run_id: &str) -> Option<u64> {
        let has_deliveries: bool = self
            .connection
            .prepare_cached(
                "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_goodfetch_deliveries')",
            )
            .ok()?
            .query_row([], |row| row.get(0))
            .ok()?;
        if !has_deliveries {
            return None;
        }
        let epoch: Option<i64> = self
            .connection
            .prepare_cached(
                "SELECT max(run_epoch) FROM _goodfetch_deliveries WHERE target = ?1 AND run_id = ?2",
            )
            .ok()?
            .query_row((&self.target_name, run_id), |row| row.get(0))
            .optional()
            .ok()?
            .flatten();
        epoch.and_then(|value| u64::try_from(value).ok())
    }

    /// Makes `table`, named `name`, with the system columns and the index
    /// the write mode needs; `keyed` gives it the unique index on the key.
    fn ready(&self, name: &str, table: &str, keyed: bool) -> rusqlite::Result<()> {
        self.connection.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {table} (
                 _goodfetch_delivery_id TEXT NOT NULL PRIMARY KEY,
                 _goodfetch_run_id TEXT NOT NULL,
                 _goodfetch_run_epoch INTEGER NOT NULL,
                 _goodfetch_dataset TEXT NOT NULL,
                 _goodfetch_record_number INTEGER NOT NULL,
                 _goodfetch_committed_at TEXT NOT NULL,
                 _goodfetch_meta TEXT,
                 _goodfetch_key TEXT
             ) STRICT;"
        ))?;
        let held = read_table_columns(&self.connection, name)?;
        let history = keyed && self.mode == WriteMode::History;
        let needed: &[&str] = if history {
            &[
                "_goodfetch_meta",
                "_goodfetch_key",
                "_goodfetch_valid_from",
                "_goodfetch_valid_to",
            ]
        } else {
            &["_goodfetch_meta", "_goodfetch_key"]
        };
        for column in needed {
            if !held.iter().any(|(name, _)| name == column) {
                self.connection
                    .execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT"))?;
            }
        }
        ensure_indexes(&self.connection, name, table, self.mode, keyed)
    }

    /// Builds the `INSERT` statement for `user_columns` under `self.mode`.
    fn insert_sql(&self, user_columns: &[(&str, ColumnType)]) -> String {
        let mut names: Vec<String> = SYSTEM_COLUMNS.iter().map(|c| (*c).to_owned()).collect();
        for (name, _) in user_columns {
            names.push(quoted(name));
        }
        let history = self.mode == WriteMode::History;
        if history {
            names.push("_goodfetch_valid_from".to_owned());
        }
        let col_list = names.join(", ");
        let total = SYSTEM_COLUMNS.len() + user_columns.len();
        let mut placeholders: Vec<String> = (1..=total).map(|i| format!("?{i}")).collect();
        if history {
            placeholders.push("?6".to_owned());
        }
        let val_list = placeholders.join(", ");
        let written = &self.written;
        let identity = identity(self.mode);
        match self.mode {
            WriteMode::Append => {
                let mut sets = vec![
                    "_goodfetch_run_epoch = excluded._goodfetch_run_epoch".to_owned(),
                    "_goodfetch_committed_at = excluded._goodfetch_committed_at".to_owned(),
                    "_goodfetch_meta = excluded._goodfetch_meta".to_owned(),
                ];
                for (name, _) in user_columns {
                    let q = quoted(name);
                    sets.push(format!("{q} = excluded.{q}"));
                }
                format!(
                    "INSERT INTO {written} AS held ({col_list}) VALUES ({val_list})
                     ON CONFLICT (_goodfetch_delivery_id) DO UPDATE SET {}
                     WHERE excluded._goodfetch_run_epoch > held._goodfetch_run_epoch",
                    sets.join(", ")
                )
            }
            WriteMode::Upsert | WriteMode::Replace => {
                let mut sets = vec![
                    "_goodfetch_delivery_id = excluded._goodfetch_delivery_id".to_owned(),
                    "_goodfetch_run_id = excluded._goodfetch_run_id".to_owned(),
                    "_goodfetch_run_epoch = excluded._goodfetch_run_epoch".to_owned(),
                    "_goodfetch_record_number = excluded._goodfetch_record_number".to_owned(),
                    "_goodfetch_committed_at = excluded._goodfetch_committed_at".to_owned(),
                    "_goodfetch_meta = excluded._goodfetch_meta".to_owned(),
                ];
                for (name, _) in user_columns {
                    let q = quoted(name);
                    sets.push(format!("{q} = excluded.{q}"));
                }
                format!(
                    "INSERT INTO {written} ({col_list})
                     SELECT {val_list}
                     WHERE NOT EXISTS (SELECT 1 FROM {written} WHERE _goodfetch_delivery_id = ?1)
                     ON CONFLICT ({identity}) WHERE _goodfetch_key IS NOT NULL
                     DO UPDATE SET {}",
                    sets.join(", ")
                )
            }
            WriteMode::InsertOnly => format!(
                "INSERT INTO {written} ({col_list}) VALUES ({val_list}) ON CONFLICT DO NOTHING"
            ),
            WriteMode::History => format!(
                "INSERT INTO {written} ({col_list}) VALUES ({val_list})
                 ON CONFLICT (_goodfetch_delivery_id) DO NOTHING"
            ),
        }
    }
}

/// Creates the mode's unique index(es) on `table`.
fn ensure_indexes(
    connection: &Connection,
    name: &str,
    table: &str,
    mode: WriteMode,
    keyed: bool,
) -> rusqlite::Result<()> {
    if !keyed {
        return Ok(());
    }
    let key_index = quoted(&format!("{name}__goodfetch_key"));
    if mode == WriteMode::History {
        connection.execute_batch(&format!(
            "DROP INDEX IF EXISTS {key_index};
             CREATE UNIQUE INDEX IF NOT EXISTS {current} ON {table}
                 (_goodfetch_dataset, _goodfetch_key)
                 WHERE _goodfetch_key IS NOT NULL AND _goodfetch_valid_to IS NULL;",
            current = quoted(&format!("{name}__goodfetch_current")),
        ))
    } else {
        connection.execute_batch(&format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS {key_index} ON {table}
                 ({identity}) WHERE _goodfetch_key IS NOT NULL;",
            identity = identity(mode),
        ))
    }
}

/// Reads `(name, declared_type)` for every column of `table_name`.
fn read_table_columns(
    connection: &Connection,
    table_name: &str,
) -> rusqlite::Result<Vec<(String, String)>> {
    connection
        .prepare_cached("SELECT name, type FROM pragma_table_info(?1) ORDER BY cid")?
        .query_map([table_name], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

/// Applies `changes` to `table_name` inside `transaction`, adding columns
/// or atomically rebuilding the `STRICT` table on a column widening.
fn apply_changes(
    transaction: &Transaction<'_>,
    table_name: &str,
    table_quoted: &str,
    mode: WriteMode,
    keyed: bool,
    changes: &[SchemaChange],
) -> rusqlite::Result<()> {
    for change in changes {
        match change {
            SchemaChange::AddedColumn { name, column_type }
            | SchemaChange::AddedVariantColumn {
                name, column_type, ..
            } => {
                let col = quoted(name);
                let sql_type = column_type.sqlite_type();
                transaction.execute_batch(&format!(
                    "ALTER TABLE {table_quoted} ADD COLUMN {col} {sql_type};"
                ))?;
                tracing::info!(
                    target: "goodfetch::sink::schema",
                    table = table_name,
                    column = name,
                    column_type = column_type.variant_suffix(),
                    "added column to Sink table"
                );
            }
            SchemaChange::WidenedColumn { name, from, to } => {
                widen_sqlite_column(
                    transaction,
                    table_name,
                    table_quoted,
                    mode,
                    keyed,
                    name,
                    *to,
                )?;
                tracing::info!(
                    target: "goodfetch::sink::schema",
                    table = table_name,
                    column = name,
                    from = from.variant_suffix(),
                    to = to.variant_suffix(),
                    "widened column in Sink table"
                );
            }
        }
    }
    Ok(())
}

/// Widens `column_name` in `STRICT` SQLite table `table_name` to `new_type`
/// atomically inside `transaction`.
fn widen_sqlite_column(
    transaction: &Transaction<'_>,
    table_name: &str,
    table_quoted: &str,
    mode: WriteMode,
    keyed: bool,
    column_name: &str,
    new_type: ColumnType,
) -> rusqlite::Result<()> {
    let existing = read_table_columns(transaction, table_name)?;
    let temp_name = format!("{table_name}__goodfetch_widen");
    let temp_quoted = quoted(&temp_name);
    let defs: Vec<String> = existing
        .iter()
        .map(|(name, sql_type)| {
            let ty = if name == column_name {
                new_type.sqlite_type()
            } else {
                sql_type.as_str()
            };
            let q = quoted(name);
            if name == "_goodfetch_delivery_id" {
                format!("{q} {ty} NOT NULL PRIMARY KEY")
            } else if matches!(
                name.as_str(),
                "_goodfetch_run_id"
                    | "_goodfetch_run_epoch"
                    | "_goodfetch_dataset"
                    | "_goodfetch_record_number"
                    | "_goodfetch_committed_at"
            ) {
                format!("{q} {ty} NOT NULL")
            } else {
                format!("{q} {ty}")
            }
        })
        .collect();
    let names: Vec<String> = existing.iter().map(|(name, _)| quoted(name)).collect();
    let col_list = names.join(", ");
    transaction.execute_batch(&format!(
        "DROP TABLE IF EXISTS {temp_quoted};
         CREATE TABLE {temp_quoted} ({defs}) STRICT;
         INSERT INTO {temp_quoted} ({col_list}) SELECT {col_list} FROM {table_quoted};
         DROP TABLE {table_quoted};
         ALTER TABLE {temp_quoted} RENAME TO {table_quoted};",
        defs = defs.join(", "),
    ))?;
    ensure_indexes(transaction, table_name, table_quoted, mode, keyed)
}

/// Ensures `target_name` holds every column of `source_name` with matching
/// types before a `replace` swap copies staging rows into the target.
fn sync_target_columns(
    transaction: &Transaction<'_>,
    source_name: &str,
    target_name: &str,
    target_quoted: &str,
) -> rusqlite::Result<Vec<String>> {
    let source_cols = read_table_columns(transaction, source_name)?;
    let target_cols = read_table_columns(transaction, target_name)?;
    let target_map: BTreeMap<String, String> = target_cols.into_iter().collect();
    for (name, sql_type) in &source_cols {
        match target_map.get(name) {
            None => {
                let q = quoted(name);
                transaction.execute_batch(&format!(
                    "ALTER TABLE {target_quoted} ADD COLUMN {q} {sql_type};"
                ))?;
            }
            Some(existing_type) if existing_type != sql_type && sql_type == "REAL" => {
                widen_sqlite_column(
                    transaction,
                    target_name,
                    target_quoted,
                    WriteMode::Replace,
                    false,
                    name,
                    ColumnType::Float,
                )?;
            }
            Some(_) => {}
        }
    }
    Ok(source_cols.into_iter().map(|(name, _)| name).collect())
}

/// Converts a [`ColumnValue`] into SQLite's owned dynamic parameter value.
fn to_sql_value(value: &ColumnValue) -> SqlValue {
    match value {
        ColumnValue::Null => SqlValue::Null,
        ColumnValue::Bool(b) => SqlValue::Integer(i64::from(*b)),
        ColumnValue::Integer(i) => SqlValue::Integer(*i),
        ColumnValue::Float(f) => SqlValue::Real(*f),
        ColumnValue::Text(s) | ColumnValue::Json(s) => SqlValue::Text(s.clone()),
    }
}

/// Builds the parameter vector for a single Record insert: the 8 system
/// columns followed by each user column in `user_columns`'s order.
fn row_params(
    batch: &Batch,
    record: &crate::contract::Delivered,
    run_epoch: i64,
    prepared: &PreparedRow,
    user_columns: &[(&str, ColumnType)],
) -> Vec<SqlValue> {
    let mut params = Vec::with_capacity(SYSTEM_COLUMNS.len() + user_columns.len());
    params.push(SqlValue::Text(record.delivery_id.as_str().to_owned()));
    params.push(SqlValue::Text(batch.run.run_id.clone()));
    params.push(SqlValue::Integer(run_epoch));
    params.push(SqlValue::Text(batch.dataset.clone()));
    params.push(SqlValue::Integer(integer(record.number)));
    params.push(SqlValue::Text(record.committed_at.clone()));
    params.push(
        prepared
            .meta
            .as_ref()
            .map_or(SqlValue::Null, |meta| SqlValue::Text(meta.to_string())),
    );
    params.push(
        record
            .key
            .as_ref()
            .map_or(SqlValue::Null, |key| SqlValue::Text(key.to_string())),
    );
    for (col_name, _) in user_columns {
        let sql_val = prepared
            .columns
            .get(*col_name)
            .map_or(SqlValue::Null, to_sql_value);
        params.push(sql_val);
    }
    params
}

struct PreparedBatch {
    working_schema: TableSchema,
    changes: Vec<SchemaChange>,
    counters: SchemaCounters,
    applied_declared: bool,
    rows: Vec<Option<PreparedRow>>,
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

impl SqliteSink {
    fn prepare_batch(&self, batch: &Batch) -> Result<PreparedBatch, SqliteError> {
        let contract = self.contract();
        let flatten = self.flatten();
        let mut working_schema = self.schema.clone();
        let mut changes = Vec::new();
        let mut counters = SchemaCounters::default();
        let mut applied_declared = false;
        if let Some(declared) = &batch.declared_schema
            && !self.declared_applied.contains(&batch.dataset)
        {
            changes.extend(working_schema.apply_declared_schema(declared, flatten));
            applied_declared = true;
        }
        let mut rows = Vec::with_capacity(batch.records.len());
        for record in &batch.records {
            let (outcome, record_changes, record_counters) =
                working_schema.prepare_record(&record.value, contract, flatten);
            changes.extend(record_changes);
            counters.add(record_counters);
            match outcome {
                RowOutcome::Write(row) => rows.push(Some(row)),
                RowOutcome::DiscardRow => rows.push(None),
                RowOutcome::Refuse(reason) => return Err(SqliteError::SchemaFrozen(reason)),
            }
        }
        Ok(PreparedBatch {
            working_schema,
            changes,
            counters,
            applied_declared,
            rows,
        })
    }
}

impl Primitives for SqliteSink {
    type Error = SqliteError;

    fn probe(&mut self, run: &RunCopy) -> Result<Probe, SqliteError> {
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
        let existing_user: Vec<(String, ColumnType)> =
            read_table_columns(&self.connection, &self.written_name)?
                .into_iter()
                .filter(|(name, _)| !name.starts_with("_goodfetch_"))
                .map(|(name, sql_type)| (name, ColumnType::from_sqlite_type(&sql_type)))
                .collect();
        self.schema = TableSchema::from_existing(&existing_user);
        self.declared_applied.clear();
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

    fn write(&mut self, batch: &Batch) -> Result<Written, SqliteError> {
        let Some(last) = batch.records.last().map(|record| record.number) else {
            return Ok(Written::Delivered);
        };
        let run_epoch = integer(batch.run.run_epoch);
        let keyed = self.mode != WriteMode::Append;
        let prepared = self.prepare_batch(batch)?;

        let user_columns = prepared.working_schema.columns();
        let insert_sql = self.insert_sql(&user_columns);
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let fence = fence(&transaction, &self.target_name, &batch.run.run_id)?;
        if let Some(superseded) = superseded(fence, run_epoch) {
            return Ok(superseded);
        }
        {
            if fence.is_some_and(|fence| fence < run_epoch) {
                transaction
                    .prepare_cached(&self.clear)?
                    .execute((&batch.run.run_id, run_epoch))?;
                if self.mode == WriteMode::History {
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
            apply_changes(
                &transaction,
                &self.written_name,
                &self.written,
                self.mode,
                keyed,
                &prepared.changes,
            )?;
            let mut close = match &self.close {
                Some(close) => Some(transaction.prepare_cached(close)?),
                None => None,
            };
            let mut insert = transaction.prepare_cached(&insert_sql)?;
            for (record, row) in batch.records.iter().zip(&prepared.rows) {
                let Some(row) = row else {
                    continue;
                };
                let params = row_params(batch, record, run_epoch, row, &user_columns);
                if let Some(close) = close.as_mut() {
                    close.execute(params_from_iter(&params[..SYSTEM_COLUMNS.len()]))?;
                }
                insert.execute(params_from_iter(&params))?;
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
        self.schema = prepared.working_schema;
        if prepared.applied_declared {
            self.declared_applied.insert(batch.dataset.clone());
        }
        self.schema_changes.extend(prepared.changes);
        self.schema_counters.add(prepared.counters);
        Ok(Written::Delivered)
    }

    fn finish(&mut self, run: &RunCopy) -> Result<Written, SqliteError> {
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
            let columns = sync_target_columns(
                &transaction,
                &self.written_name,
                &self.target_name,
                &self.target,
            )?;
            let col_list: String = columns
                .iter()
                .map(|c| quoted(c))
                .collect::<Vec<_>>()
                .join(", ");
            transaction.execute_batch(&format!("DELETE FROM {target}", target = self.target))?;
            transaction
                .prepare_cached(&format!(
                    "INSERT INTO {target} ({col_list}) SELECT {col_list} FROM {written}
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

    fn map_failure(&self, error: SqliteError) -> Failure {
        match error {
            SqliteError::SchemaFrozen(reason) => Failure::new(FailureKind::RejectedRecords, reason),
            SqliteError::Sqlite(error) => {
                let kind = match &error {
                    rusqlite::Error::SqliteFailure(failure, _) => kind(failure.code),
                    _ => FailureKind::Unrecognized,
                };
                Failure::new(kind, error.to_string())
            }
        }
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
