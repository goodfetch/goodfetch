//! DuckDB in-binary Sink (ADRs 0008, 0026, 0103 and 0127; issue 1163):
//! delivers committed Records into a local DuckDB database file in every write
//! mode (`append`, `upsert`, `insert_only`, `replace`, `history`) with
//! transactional epoch fencing (`GuaranteeClass::Fenced`), schema evolution via
//! [`TableSchema`], and ADR 0127 error translation.

use std::collections::BTreeMap;
use std::path::PathBuf;

use duckdb::types::Value as DuckValue;
use duckdb::{Connection, params_from_iter};
use goodfetch_protocol::foxhound::sink::{SchemaContract, WriteMode};

use crate::contract::{
    Batch, Delivered, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};
use crate::schema::{ColumnType, ColumnValue, PreparedRow, SchemaChange, TableSchema};

/// Default DuckDB target table when `target` is omitted.
pub const DEFAULT_DUCKDB_TARGET: &str = "records";

/// Errors returned by [`DuckDbSink`] before translation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DuckDbError {
    /// Invalid configuration (empty path or table name).
    #[error("invalid DuckDB Sink configuration: {0}")]
    Config(String),
    /// Error from the `duckdb` crate.
    #[error("{0}")]
    Db(#[from] duckdb::Error),
    /// Record missing required key or refused by `freeze` contract.
    #[error("rejected record: {0}")]
    RejectedRecord(String),
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn duckdb_type(ty: ColumnType) -> &'static str {
    match ty {
        ColumnType::Bool => "BOOLEAN",
        ColumnType::Integer => "BIGINT",
        ColumnType::Float => "DOUBLE",
        ColumnType::Text | ColumnType::Json => "VARCHAR",
    }
}

fn from_duckdb_type(ty: &str) -> ColumnType {
    match ty.to_ascii_uppercase().as_str() {
        "BOOLEAN" | "BOOL" => ColumnType::Bool,
        "BIGINT" | "INTEGER" | "INT" | "SMALLINT" => ColumnType::Integer,
        "DOUBLE" | "FLOAT" | "REAL" => ColumnType::Float,
        _ => ColumnType::Text,
    }
}

/// Built-in DuckDB Sink implementing [`Primitives`].
pub struct DuckDbSink {
    conn: Connection,
    target: String,
    mode: WriteMode,
    contract: SchemaContract,
    flatten: bool,
    schema: Option<TableSchema>,
    schema_applied_for: BTreeMap<String, bool>,
}

impl DuckDbSink {
    /// Opens a DuckDB Sink at `path` (stripping an optional `duckdb://` or
    /// `file://` prefix) targeting `table`.
    ///
    /// # Errors
    ///
    /// [`DuckDbError::Config`] or [`DuckDbError::Db`].
    pub fn open(
        destination: &str,
        table: impl Into<String>,
        mode: WriteMode,
        contract: SchemaContract,
        flatten: bool,
    ) -> Result<Self, DuckDbError> {
        let table = table.into();
        if table.trim().is_empty() {
            return Err(DuckDbError::Config("target table must not be empty".into()));
        }
        let cleaned = destination
            .strip_prefix("duckdb://")
            .or_else(|| destination.strip_prefix("file://"))
            .unwrap_or(destination)
            .trim();
        if cleaned.is_empty() {
            return Err(DuckDbError::Config(
                "destination path must not be empty".into(),
            ));
        }
        let conn = if cleaned == ":memory:" {
            Connection::open_in_memory()?
        } else {
            Connection::open(PathBuf::from(cleaned))?
        };
        let mut sink = Self {
            conn,
            target: table,
            mode,
            contract,
            flatten,
            schema: None,
            schema_applied_for: BTreeMap::new(),
        };
        sink.ensure_tables()?;
        Ok(sink)
    }

    fn staging_table(&self) -> String {
        format!("{}__goodfetch_staging", self.target)
    }

    fn write_table(&self) -> String {
        if self.mode == WriteMode::Replace {
            self.staging_table()
        } else {
            self.target.clone()
        }
    }

    fn ensure_tables(&mut self) -> Result<(), DuckDbError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _goodfetch_deliveries (
                run_id VARCHAR PRIMARY KEY,
                run_epoch BIGINT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS _goodfetch_delivery_log (
                delivery_id VARCHAR PRIMARY KEY,
                run_id VARCHAR NOT NULL,
                run_epoch BIGINT NOT NULL,
                dataset VARCHAR NOT NULL,
                record_number BIGINT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS _goodfetch_swaps (
                run_id VARCHAR PRIMARY KEY,
                run_epoch BIGINT NOT NULL
            );",
        )?;
        create_duckdb_table(&self.conn, &self.target)?;
        if self.mode == WriteMode::Replace {
            create_duckdb_table(&self.conn, &self.staging_table())?;
        }
        Ok(())
    }

    fn load_schema(&mut self) -> Result<(), DuckDbError> {
        if self.schema.is_some() {
            return Ok(());
        }
        let table = self.write_table();
        let mut stmt = self.conn.prepare(
            "SELECT column_name, data_type
             FROM information_schema.columns
             WHERE table_name = ?
             ORDER BY ordinal_position",
        )?;
        let rows = stmt.query_map([&table], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut existing = Vec::new();
        for row in rows {
            let (name, ty) = row?;
            if !name.starts_with("_goodfetch_") {
                existing.push((name, from_duckdb_type(&ty)));
            }
        }
        self.schema = Some(TableSchema::from_existing(&existing));
        Ok(())
    }

    fn check_and_bump_fence(&self, run: &RunCopy) -> Result<Option<u64>, DuckDbError> {
        let epoch_i64 = i64::try_from(run.run_epoch).unwrap_or(i64::MAX);
        let mut stmt = self
            .conn
            .prepare("SELECT run_epoch FROM _goodfetch_deliveries WHERE run_id = ?")?;
        let existing: Option<i64> = stmt
            .query_map([&run.run_id], |r| r.get(0))?
            .next()
            .transpose()?;
        match existing {
            None => {
                self.conn.execute(
                    "INSERT INTO _goodfetch_deliveries (run_id, run_epoch) VALUES (?, ?)",
                    duckdb::params![&run.run_id, epoch_i64],
                )?;
                Ok(None)
            }
            Some(current) if current > epoch_i64 => {
                Ok(Some(u64::try_from(current).unwrap_or(u64::MAX)))
            }
            Some(current) => {
                if epoch_i64 > current {
                    self.conn.execute(
                        "UPDATE _goodfetch_deliveries SET run_epoch = ? WHERE run_id = ?",
                        duckdb::params![epoch_i64, &run.run_id],
                    )?;
                }
                Ok(None)
            }
        }
    }

    fn apply_changes(&self, table: &str, changes: &[SchemaChange]) -> Result<(), DuckDbError> {
        let table_q = quote_ident(table);
        for change in changes {
            match change {
                SchemaChange::AddedColumn { name, column_type }
                | SchemaChange::AddedVariantColumn {
                    name, column_type, ..
                } => {
                    let col_q = quote_ident(name);
                    let ty = duckdb_type(*column_type);
                    self.conn.execute(
                        &format!("ALTER TABLE {table_q} ADD COLUMN IF NOT EXISTS {col_q} {ty}"),
                        [],
                    )?;
                }
                SchemaChange::WidenedColumn { name, to, .. } => {
                    let col_q = quote_ident(name);
                    let ty = duckdb_type(*to);
                    self.conn.execute(
                        &format!("ALTER TABLE {table_q} ALTER COLUMN {col_q} SET DATA TYPE {ty}"),
                        [],
                    )?;
                }
            }
        }
        Ok(())
    }
}

fn create_duckdb_table(conn: &Connection, table: &str) -> Result<(), DuckDbError> {
    let table_q = quote_ident(table);
    conn.execute_batch(&format!(
        "CREATE TABLE IF NOT EXISTS {table_q} (
            _goodfetch_delivery_id VARCHAR PRIMARY KEY,
            _goodfetch_dataset VARCHAR NOT NULL,
            _goodfetch_record_number BIGINT NOT NULL,
            _goodfetch_run_id VARCHAR NOT NULL,
            _goodfetch_run_epoch BIGINT NOT NULL,
            _goodfetch_committed_at VARCHAR NOT NULL,
            _goodfetch_meta VARCHAR,
            _goodfetch_key VARCHAR,
            _goodfetch_deleted BOOLEAN NOT NULL DEFAULT FALSE,
            _goodfetch_valid_from VARCHAR,
            _goodfetch_valid_to VARCHAR
        );"
    ))?;
    Ok(())
}

impl Primitives for DuckDbSink {
    type Error = DuckDbError;

    fn probe(&mut self, run: &RunCopy) -> Result<Probe, Self::Error> {
        self.load_schema()?;
        self.conn.execute_batch("BEGIN TRANSACTION")?;
        let res = (|| {
            let _ = self.check_and_bump_fence(run)?;
            let epoch_i64 = i64::try_from(run.run_epoch).unwrap_or(i64::MAX);
            let mut stmt = self.conn.prepare(
                "SELECT dataset, MAX(record_number)
                 FROM _goodfetch_delivery_log
                 WHERE run_id = ? AND run_epoch = ?
                 GROUP BY dataset",
            )?;
            let mut delivered = BTreeMap::new();
            let rows = stmt.query_map(duckdb::params![&run.run_id, epoch_i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (ds, max_num) = row?;
                if let Ok(num) = u64::try_from(max_num) {
                    delivered.insert(ds, num);
                }
            }
            Ok::<_, DuckDbError>(delivered)
        })();
        match res {
            Ok(delivered) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(Probe {
                    class: GuaranteeClass::Fenced,
                    delivered,
                })
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Self::Error> {
        self.load_schema()?;
        let schema = self.schema.as_mut().expect("invariant: schema loaded");
        let (prepared, pending_changes) = schema
            .prepare_batch(
                &mut self.schema_applied_for,
                batch,
                self.mode,
                self.contract,
                self.flatten,
            )
            .map_err(DuckDbError::RejectedRecord)?;

        let write_table = self.write_table();
        let target_table = self.target.clone();
        let mode = self.mode;

        self.conn.execute_batch("BEGIN TRANSACTION")?;
        let outcome = (|| {
            if let Some(newer) = self.check_and_bump_fence(&batch.run)? {
                return Ok(Written::Superseded { run_epoch: newer });
            }
            self.apply_changes(&write_table, &pending_changes)?;
            if mode == WriteMode::Replace {
                self.apply_changes(&target_table, &pending_changes)?;
            }
            let epoch_i64 = i64::try_from(batch.run.run_epoch).unwrap_or(i64::MAX);
            for (record, row_opt) in prepared {
                let already: i64 = self.conn.query_row(
                    "SELECT COUNT(*) FROM _goodfetch_delivery_log WHERE delivery_id = ?",
                    [record.delivery_id.as_str()],
                    |r| r.get(0),
                )?;
                if already > 0 {
                    continue;
                }
                let num_i64 = i64::try_from(record.number).unwrap_or(i64::MAX);
                self.conn.execute(
                    "INSERT INTO _goodfetch_delivery_log (delivery_id, run_id, run_epoch, dataset, record_number)
                     VALUES (?, ?, ?, ?, ?)",
                    duckdb::params![
                        record.delivery_id.as_str(),
                        &batch.run.run_id,
                        epoch_i64,
                        &batch.dataset,
                        num_i64
                    ],
                )?;
                if let Some(row) = row_opt {
                    insert_duckdb_row(
                        &self.conn,
                        &write_table,
                        mode,
                        &batch.run,
                        &batch.dataset,
                        record,
                        &row,
                    )?;
                }
            }
            Ok::<_, DuckDbError>(Written::Delivered)
        })();

        match outcome {
            Ok(written) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(written)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    fn finish(&mut self, run: &RunCopy) -> Result<Written, Self::Error> {
        if self.mode != WriteMode::Replace {
            return Ok(Written::Delivered);
        }
        let target_q = quote_ident(&self.target);
        let staging_q = quote_ident(&self.staging_table());
        let epoch_i64 = i64::try_from(run.run_epoch).unwrap_or(i64::MAX);
        self.conn.execute_batch("BEGIN TRANSACTION")?;
        let outcome = (|| {
            if let Some(newer) = self.check_and_bump_fence(run)? {
                return Ok(Written::Superseded { run_epoch: newer });
            }
            let prev: Option<i64> = self
                .conn
                .prepare("SELECT run_epoch FROM _goodfetch_swaps WHERE run_id = ?")?
                .query_map([&run.run_id], |r| r.get(0))?
                .next()
                .transpose()?;
            if prev.is_some_and(|p| p >= epoch_i64) {
                return Ok(Written::Delivered);
            }
            self.conn.execute(&format!("DELETE FROM {target_q}"), [])?;
            self.conn.execute(
                &format!(
                    "INSERT INTO {target_q} SELECT * FROM {staging_q} WHERE _goodfetch_run_id = ?"
                ),
                [&run.run_id],
            )?;
            self.conn.execute(
                &format!("DELETE FROM {staging_q} WHERE _goodfetch_run_id = ?"),
                [&run.run_id],
            )?;
            self.conn.execute(
                "INSERT INTO _goodfetch_swaps (run_id, run_epoch) VALUES (?, ?)
                 ON CONFLICT (run_id) DO UPDATE SET run_epoch = excluded.run_epoch",
                duckdb::params![&run.run_id, epoch_i64],
            )?;
            Ok::<_, DuckDbError>(Written::Delivered)
        })();
        match outcome {
            Ok(w) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(w)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        map_duckdb_error(error)
    }
}

fn insert_duckdb_row(
    conn: &Connection,
    table: &str,
    mode: WriteMode,
    run: &RunCopy,
    dataset: &str,
    record: &Delivered,
    row: &PreparedRow,
) -> Result<(), DuckDbError> {
    let table_q = quote_ident(table);
    let key_text = record
        .key
        .as_ref()
        .map(|k| serde_json::to_string(k).unwrap_or_default());

    if let Some(ref k) = key_text {
        match mode {
            WriteMode::Upsert => {
                conn.execute(
                    &format!(
                        "DELETE FROM {table_q} WHERE _goodfetch_dataset = ? AND _goodfetch_key = ?"
                    ),
                    duckdb::params![dataset, k],
                )?;
            }
            WriteMode::InsertOnly => {
                let exists: i64 = conn.query_row(
                    &format!("SELECT COUNT(*) FROM {table_q} WHERE _goodfetch_dataset = ? AND _goodfetch_key = ?"),
                    duckdb::params![dataset, k],
                    |r| r.get(0),
                )?;
                if exists > 0 {
                    return Ok(());
                }
            }
            WriteMode::History => {
                conn.execute(
                    &format!(
                        "UPDATE {table_q} SET _goodfetch_valid_to = ?
                         WHERE _goodfetch_dataset = ? AND _goodfetch_key = ? AND _goodfetch_valid_to IS NULL"
                    ),
                    duckdb::params![&record.committed_at, dataset, k],
                )?;
            }
            WriteMode::Append | WriteMode::Replace => {}
        }
    }

    let mut cols: Vec<String> = vec![
        "_goodfetch_delivery_id".into(),
        "_goodfetch_dataset".into(),
        "_goodfetch_record_number".into(),
        "_goodfetch_run_id".into(),
        "_goodfetch_run_epoch".into(),
        "_goodfetch_committed_at".into(),
        "_goodfetch_meta".into(),
        "_goodfetch_key".into(),
        "_goodfetch_valid_from".into(),
    ];
    let mut vals: Vec<DuckValue> = vec![
        DuckValue::Text(record.delivery_id.as_str().to_owned()),
        DuckValue::Text(dataset.to_owned()),
        DuckValue::BigInt(i64::try_from(record.number).unwrap_or(i64::MAX)),
        DuckValue::Text(run.run_id.clone()),
        DuckValue::BigInt(i64::try_from(run.run_epoch).unwrap_or(i64::MAX)),
        DuckValue::Text(record.committed_at.clone()),
        row.meta
            .as_ref()
            .map_or(DuckValue::Null, |m| DuckValue::Text(m.to_string())),
        key_text.map_or(DuckValue::Null, DuckValue::Text),
        if mode == WriteMode::History {
            DuckValue::Text(record.committed_at.clone())
        } else {
            DuckValue::Null
        },
    ];
    for (col, val) in &row.columns {
        cols.push(col.clone());
        vals.push(match val {
            ColumnValue::Null => DuckValue::Null,
            ColumnValue::Bool(b) => DuckValue::Boolean(*b),
            ColumnValue::Integer(i) => DuckValue::BigInt(*i),
            ColumnValue::Float(f) => DuckValue::Double(*f),
            ColumnValue::Text(s) | ColumnValue::Json(s) => DuckValue::Text(s.clone()),
        });
    }
    let col_list: Vec<String> = cols.iter().map(|c| quote_ident(c)).collect();
    let placeholders = vec!["?"; vals.len()].join(", ");
    let sql = format!(
        "INSERT INTO {table_q} ({}) VALUES ({placeholders})",
        col_list.join(", ")
    );
    conn.execute(&sql, params_from_iter(vals))?;
    Ok(())
}

/// Translates [`DuckDbError`] into the driver's [`Failure`] (ADR 0127).
pub fn map_duckdb_error(error: DuckDbError) -> Failure {
    match error {
        DuckDbError::Config(msg) => Failure::new(FailureKind::Permanent, msg),
        DuckDbError::RejectedRecord(msg) => Failure::new(FailureKind::RejectedRecords, msg),
        DuckDbError::Db(err) => {
            let text = err.to_string();
            if text.contains("Constraint Error") || text.contains("Conversion Error") {
                Failure::new(FailureKind::RejectedRecords, text)
            } else if text.contains("Permission denied") {
                Failure::new(FailureKind::Auth, text)
            } else if text.contains("lock") || text.contains("busy") {
                Failure::new(FailureKind::Transient, text)
            } else {
                tracing::warn!(error = %text, "unrecognized DuckDB failure; treating as transient");
                Failure::new(FailureKind::Unrecognized, text)
            }
        }
    }
}
