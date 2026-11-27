//! PostgreSQL and Supabase in-binary Sinks (ADRs 0008, 0026, 0103 and 0127;
//! issue 1162): delivers committed Records to PostgreSQL or Supabase (direct
//! PostgreSQL connection) in every write mode (`append`, `upsert`,
//! `insert_only`, `replace`, `history`) with transactional epoch fencing
//! (`GuaranteeClass::Fenced`), native `ON CONFLICT` handling, schema evolution
//! via [`TableSchema`], and `SqlState` failure translation.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::foxhound::sink::{SchemaContract, WriteMode};
use serde_json::Value;
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, NoTls, Transaction};

use crate::contract::{
    Batch, Delivered, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};
use crate::schema::{ColumnType, ColumnValue, PreparedRow, RowOutcome, SchemaChange, TableSchema};

/// Default PostgreSQL target table when `target` is omitted.
pub const DEFAULT_POSTGRES_TARGET: &str = "records";

/// Errors returned by [`PostgresSink`] before translation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PostgresError {
    /// Invalid destination connection string or table identifier.
    #[error("invalid PostgreSQL Sink configuration: {0}")]
    Config(String),
    /// Error returned by `tokio-postgres`.
    #[error("{0}")]
    Db(#[from] tokio_postgres::Error),
    /// A keyed Record lacked its key or violated a `freeze` schema contract.
    #[error("rejected record: {0}")]
    RejectedRecord(String),
    /// No active Tokio runtime available when called synchronously.
    #[error("PostgreSQL Sink requires an active Tokio runtime")]
    NoRuntime,
}

/// Owned SQL parameter value implementing `ToSql` for dynamic queries.
enum PgParam {
    Bool(Option<bool>),
    I64(Option<i64>),
    F64(Option<f64>),
    Text(Option<String>),
    Json(Option<Value>),
}

impl PgParam {
    fn as_sql(&self) -> &(dyn ToSql + Sync) {
        match self {
            Self::Bool(v) => v,
            Self::I64(v) => v,
            Self::F64(v) => v,
            Self::Text(v) => v,
            Self::Json(v) => v,
        }
    }
}

/// Quotes a PostgreSQL identifier safely (`"..."` with doubled internal `"`).
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Built-in PostgreSQL / Supabase Sink implementing [`Primitives`].
pub struct PostgresSink {
    dsn: String,
    target: String,
    mode: WriteMode,
    contract: SchemaContract,
    flatten: bool,
    client: Option<Client>,
    tasks: TaskOwner,
    schema: Option<TableSchema>,
    schema_applied_for: BTreeMap<String, bool>,
}

impl PostgresSink {
    /// Creates a new PostgreSQL (or Supabase direct) Sink targeting `table`.
    ///
    /// # Errors
    ///
    /// [`PostgresError::Config`] if `dsn` or `table` is empty.
    pub fn new(
        dsn: impl Into<String>,
        table: impl Into<String>,
        mode: WriteMode,
        contract: SchemaContract,
        flatten: bool,
    ) -> Result<Self, PostgresError> {
        let dsn = dsn.into();
        let table = table.into();
        if dsn.trim().is_empty() {
            return Err(PostgresError::Config(
                "destination DSN must not be empty".to_owned(),
            ));
        }
        if table.trim().is_empty() {
            return Err(PostgresError::Config(
                "target table name must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            dsn,
            target: table,
            mode,
            contract,
            flatten,
            client: None,
            tasks: TaskOwner::root(BlockingPolicy::Threads),
            schema: None,
            schema_applied_for: BTreeMap::new(),
        })
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

    async fn ensure_connected(&mut self) -> Result<(), PostgresError> {
        if self.client.is_some() {
            return Ok(());
        }
        let (client, connection) = tokio_postgres::connect(&self.dsn, NoTls).await?;
        self.tasks.spawn(async move {
            if let Err(err) = connection.await {
                tracing::warn!(error = %err, "PostgreSQL Sink connection task ended with error");
            }
        });
        self.client = Some(client);
        Ok(())
    }

    async fn ensure_tables(&mut self) -> Result<(), PostgresError> {
        self.ensure_connected().await?;
        let target_q = quote_ident(&self.target);
        let staging_q = quote_ident(&self.staging_table());
        let mode = self.mode;
        let client = self
            .client
            .as_ref()
            .expect("invariant: connected immediately above");

        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS _goodfetch_deliveries (
                    run_id TEXT PRIMARY KEY,
                    run_epoch BIGINT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS _goodfetch_delivery_log (
                    delivery_id TEXT PRIMARY KEY,
                    run_id TEXT NOT NULL,
                    run_epoch BIGINT NOT NULL,
                    dataset TEXT NOT NULL,
                    record_number BIGINT NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_gf_delivery_log_run
                    ON _goodfetch_delivery_log (run_id, run_epoch, dataset, record_number);
                CREATE TABLE IF NOT EXISTS _goodfetch_swaps (
                    run_id TEXT PRIMARY KEY,
                    run_epoch BIGINT NOT NULL
                );",
            )
            .await?;

        create_data_table(client, &self.target, &target_q, mode).await?;
        if mode == WriteMode::Replace {
            create_data_table(client, &self.staging_table(), &staging_q, WriteMode::Append).await?;
        }
        Ok(())
    }

    async fn load_schema(&mut self) -> Result<(), PostgresError> {
        if self.schema.is_some() {
            return Ok(());
        }
        let table = self.write_table();
        let client = self
            .client
            .as_ref()
            .expect("invariant: connected before load_schema");
        let rows = client
            .query(
                "SELECT column_name, data_type
                 FROM information_schema.columns
                 WHERE table_schema = current_schema() AND table_name = $1
                 ORDER BY ordinal_position",
                &[&table],
            )
            .await?;
        let existing: Vec<(String, ColumnType)> = rows
            .into_iter()
            .filter_map(|row| {
                let name: String = row.get(0);
                if name.starts_with("_goodfetch_") {
                    return None;
                }
                let data_type: String = row.get(1);
                Some((name, ColumnType::from_postgres_type(&data_type)))
            })
            .collect();
        self.schema = Some(TableSchema::from_existing(&existing));
        Ok(())
    }

    async fn probe_async(&mut self, run: &RunCopy) -> Result<Probe, PostgresError> {
        self.ensure_tables().await?;
        self.load_schema().await?;
        let client = self
            .client
            .as_mut()
            .expect("invariant: connected in ensure_tables");
        let tx = client.transaction().await?;
        let epoch_i64 = i64::try_from(run.run_epoch).unwrap_or(i64::MAX);
        let row = tx
            .query_opt(
                "SELECT run_epoch FROM _goodfetch_deliveries WHERE run_id = $1 FOR UPDATE",
                &[&run.run_id],
            )
            .await?;
        match row {
            None => {
                tx.execute(
                    "INSERT INTO _goodfetch_deliveries (run_id, run_epoch) VALUES ($1, $2)",
                    &[&run.run_id, &epoch_i64],
                )
                .await?;
            }
            Some(r) => {
                let current: i64 = r.get(0);
                if epoch_i64 > current {
                    tx.execute(
                        "UPDATE _goodfetch_deliveries SET run_epoch = $2 WHERE run_id = $1",
                        &[&run.run_id, &epoch_i64],
                    )
                    .await?;
                }
            }
        }
        let delivered_rows = tx
            .query(
                "SELECT dataset, MAX(record_number)
                 FROM _goodfetch_delivery_log
                 WHERE run_id = $1 AND run_epoch = $2
                 GROUP BY dataset",
                &[&run.run_id, &epoch_i64],
            )
            .await?;
        tx.commit().await?;

        let mut delivered = BTreeMap::new();
        for r in delivered_rows {
            let ds: String = r.get(0);
            let max_num: i64 = r.get(1);
            if let Ok(num) = u64::try_from(max_num) {
                delivered.insert(ds, num);
            }
        }
        Ok(Probe {
            class: GuaranteeClass::Fenced,
            delivered,
        })
    }

    async fn write_async(&mut self, batch: &Batch) -> Result<Written, PostgresError> {
        self.ensure_tables().await?;
        self.load_schema().await?;

        let mut pending_changes = Vec::new();
        if let Some(declared) = &batch.declared_schema
            && !self
                .schema_applied_for
                .get(&batch.dataset)
                .copied()
                .unwrap_or(false)
        {
            let schema = self.schema.as_mut().expect("invariant: schema loaded");
            pending_changes.extend(schema.apply_declared_schema(declared, self.flatten));
            self.schema_applied_for.insert(batch.dataset.clone(), true);
        }

        let mut prepared_records: Vec<(&Delivered, Option<PreparedRow>)> =
            Vec::with_capacity(batch.records.len());
        for record in &batch.records {
            if self.mode != WriteMode::Append && record.key.is_none() {
                return Err(PostgresError::RejectedRecord(
                    "record missing required key fields".to_owned(),
                ));
            }
            let schema = self.schema.as_mut().expect("invariant: schema loaded");
            let (outcome, changes, _counters) =
                schema.prepare_record(&record.value, self.contract, self.flatten);
            pending_changes.extend(changes);
            match outcome {
                RowOutcome::Write(row) => prepared_records.push((record, Some(row))),
                RowOutcome::DiscardRow => prepared_records.push((record, None)),
                RowOutcome::Refuse(reason) => {
                    return Err(PostgresError::RejectedRecord(reason));
                }
            }
        }

        let write_table = self.write_table();
        let target_table = self.target.clone();
        let mode = self.mode;
        let client = self
            .client
            .as_mut()
            .expect("invariant: connected in ensure_tables");
        let tx = client.transaction().await?;

        let epoch_i64 = i64::try_from(batch.run.run_epoch).unwrap_or(i64::MAX);
        if let Some(superseded) = check_and_bump_fence(&tx, &batch.run.run_id, epoch_i64).await? {
            tx.rollback().await?;
            return Ok(Written::Superseded {
                run_epoch: superseded,
            });
        }

        apply_schema_changes(&tx, &write_table, &pending_changes).await?;
        if mode == WriteMode::Replace {
            apply_schema_changes(&tx, &target_table, &pending_changes).await?;
        }

        for (record, row_opt) in prepared_records {
            let num_i64 = i64::try_from(record.number).unwrap_or(i64::MAX);
            let inserted = tx
                .execute(
                    "INSERT INTO _goodfetch_delivery_log (delivery_id, run_id, run_epoch, dataset, record_number)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (delivery_id) DO NOTHING",
                    &[
                        &record.delivery_id.as_str(),
                        &batch.run.run_id,
                        &epoch_i64,
                        &batch.dataset,
                        &num_i64,
                    ],
                )
                .await?;
            if inserted == 0 {
                continue;
            }
            if let Some(row) = row_opt {
                let schema = self.schema.as_ref().expect("invariant: schema loaded");
                insert_row(
                    &tx,
                    &write_table,
                    mode,
                    &batch.run,
                    &batch.dataset,
                    record,
                    &row,
                    schema,
                )
                .await?;
            }
        }

        tx.commit().await?;
        Ok(Written::Delivered)
    }

    async fn finish_async(&mut self, run: &RunCopy) -> Result<Written, PostgresError> {
        if self.mode != WriteMode::Replace {
            return Ok(Written::Delivered);
        }
        self.ensure_tables().await?;
        let target_q = quote_ident(&self.target);
        let staging_q = quote_ident(&self.staging_table());
        let client = self
            .client
            .as_mut()
            .expect("invariant: connected in ensure_tables");
        let tx = client.transaction().await?;
        let epoch_i64 = i64::try_from(run.run_epoch).unwrap_or(i64::MAX);

        if let Some(superseded) = check_and_bump_fence(&tx, &run.run_id, epoch_i64).await? {
            tx.rollback().await?;
            return Ok(Written::Superseded {
                run_epoch: superseded,
            });
        }
        let swapped = tx
            .query_opt(
                "SELECT run_epoch FROM _goodfetch_swaps WHERE run_id = $1 FOR UPDATE",
                &[&run.run_id],
            )
            .await?;
        if let Some(row) = swapped {
            let prev: i64 = row.get(0);
            if prev >= epoch_i64 {
                tx.commit().await?;
                return Ok(Written::Delivered);
            }
        }

        tx.execute(&format!("DELETE FROM {target_q}"), &[]).await?;
        tx.execute(
            &format!(
                "INSERT INTO {target_q} SELECT * FROM {staging_q} WHERE _goodfetch_run_id = $1"
            ),
            &[&run.run_id],
        )
        .await?;
        tx.execute(
            &format!("DELETE FROM {staging_q} WHERE _goodfetch_run_id = $1"),
            &[&run.run_id],
        )
        .await?;
        tx.execute(
            "INSERT INTO _goodfetch_swaps (run_id, run_epoch) VALUES ($1, $2)
             ON CONFLICT (run_id) DO UPDATE SET run_epoch = EXCLUDED.run_epoch",
            &[&run.run_id, &epoch_i64],
        )
        .await?;
        tx.commit().await?;
        Ok(Written::Delivered)
    }
}

impl Primitives for PostgresSink {
    type Error = PostgresError;

    fn probe(&mut self, run: &RunCopy) -> Result<Probe, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| PostgresError::NoRuntime)?;
        handle.block_on(self.probe_async(run))
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| PostgresError::NoRuntime)?;
        handle.block_on(self.write_async(batch))
    }

    fn finish(&mut self, run: &RunCopy) -> Result<Written, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| PostgresError::NoRuntime)?;
        handle.block_on(self.finish_async(run))
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        map_postgres_error(error)
    }
}

async fn create_data_table(
    client: &Client,
    table_name: &str,
    table_q: &str,
    mode: WriteMode,
) -> Result<(), PostgresError> {
    let mut ddl = format!(
        "CREATE TABLE IF NOT EXISTS {table_q} (
            _goodfetch_delivery_id TEXT PRIMARY KEY,
            _goodfetch_dataset TEXT NOT NULL,
            _goodfetch_record_number BIGINT NOT NULL,
            _goodfetch_run_id TEXT NOT NULL,
            _goodfetch_run_epoch BIGINT NOT NULL,
            _goodfetch_committed_at TEXT NOT NULL,
            _goodfetch_meta JSONB,
            _goodfetch_key TEXT,
            _goodfetch_deleted BOOLEAN NOT NULL DEFAULT FALSE,
            _goodfetch_valid_from TEXT,
            _goodfetch_valid_to TEXT
        );"
    );
    if matches!(mode, WriteMode::Upsert | WriteMode::InsertOnly) {
        let idx = quote_ident(&format!("idx_{table_name}_gf_key"));
        let _ = write!(
            ddl,
            "CREATE UNIQUE INDEX IF NOT EXISTS {idx}
             ON {table_q} (_goodfetch_dataset, _goodfetch_key)
             WHERE _goodfetch_key IS NOT NULL;"
        );
    } else if mode == WriteMode::History {
        let idx = quote_ident(&format!("idx_{table_name}_gf_history_open"));
        let _ = write!(
            ddl,
            "CREATE INDEX IF NOT EXISTS {idx}
             ON {table_q} (_goodfetch_dataset, _goodfetch_key)
             WHERE _goodfetch_valid_to IS NULL;"
        );
    }
    client.batch_execute(&ddl).await?;
    Ok(())
}

async fn check_and_bump_fence(
    tx: &Transaction<'_>,
    run_id: &str,
    epoch_i64: i64,
) -> Result<Option<u64>, PostgresError> {
    let row = tx
        .query_opt(
            "SELECT run_epoch FROM _goodfetch_deliveries WHERE run_id = $1 FOR UPDATE",
            &[&run_id],
        )
        .await?;
    match row {
        None => {
            tx.execute(
                "INSERT INTO _goodfetch_deliveries (run_id, run_epoch) VALUES ($1, $2)",
                &[&run_id, &epoch_i64],
            )
            .await?;
            Ok(None)
        }
        Some(r) => {
            let current: i64 = r.get(0);
            if current > epoch_i64 {
                Ok(Some(u64::try_from(current).unwrap_or(u64::MAX)))
            } else {
                if epoch_i64 > current {
                    tx.execute(
                        "UPDATE _goodfetch_deliveries SET run_epoch = $2 WHERE run_id = $1",
                        &[&run_id, &epoch_i64],
                    )
                    .await?;
                }
                Ok(None)
            }
        }
    }
}

async fn apply_schema_changes(
    tx: &Transaction<'_>,
    table: &str,
    changes: &[SchemaChange],
) -> Result<(), PostgresError> {
    let table_q = quote_ident(table);
    for change in changes {
        match change {
            SchemaChange::AddedColumn { name, column_type }
            | SchemaChange::AddedVariantColumn {
                name, column_type, ..
            } => {
                let col_q = quote_ident(name);
                let sql_ty = column_type.postgres_type();
                tx.execute(
                    &format!("ALTER TABLE {table_q} ADD COLUMN IF NOT EXISTS {col_q} {sql_ty}"),
                    &[],
                )
                .await?;
            }
            SchemaChange::WidenedColumn { name, to, .. } => {
                let col_q = quote_ident(name);
                let sql_ty = to.postgres_type();
                tx.execute(
                    &format!("ALTER TABLE {table_q} ALTER COLUMN {col_q} TYPE {sql_ty}"),
                    &[],
                )
                .await?;
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "row insertion carries transaction, table, mode, run identity, dataset, record, prepared row and schema"
)]
async fn insert_row(
    tx: &Transaction<'_>,
    table: &str,
    mode: WriteMode,
    run: &RunCopy,
    dataset: &str,
    record: &Delivered,
    row: &PreparedRow,
    schema: &TableSchema,
) -> Result<(), PostgresError> {
    let table_q = quote_ident(table);
    let key_text = record
        .key
        .as_ref()
        .map(|k| serde_json::to_string(k).unwrap_or_default());

    if mode == WriteMode::History
        && let Some(ref k) = key_text
    {
        tx.execute(
            &format!(
                "UPDATE {table_q}
                 SET _goodfetch_valid_to = $1
                 WHERE _goodfetch_dataset = $2
                   AND _goodfetch_key = $3
                   AND _goodfetch_valid_to IS NULL"
            ),
            &[&record.committed_at, &dataset, k],
        )
        .await?;
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
    let mut params: Vec<PgParam> = vec![
        PgParam::Text(Some(record.delivery_id.as_str().to_owned())),
        PgParam::Text(Some(dataset.to_owned())),
        PgParam::I64(Some(i64::try_from(record.number).unwrap_or(i64::MAX))),
        PgParam::Text(Some(run.run_id.clone())),
        PgParam::I64(Some(i64::try_from(run.run_epoch).unwrap_or(i64::MAX))),
        PgParam::Text(Some(record.committed_at.clone())),
        PgParam::Json(row.meta.clone()),
        PgParam::Text(key_text),
        PgParam::Text((mode == WriteMode::History).then(|| record.committed_at.clone())),
    ];

    let mut user_col_names = Vec::new();
    for (col, val) in &row.columns {
        cols.push(col.clone());
        user_col_names.push(col.clone());
        let col_ty = schema.column_type(col).unwrap_or(ColumnType::Text);
        params.push(match val {
            ColumnValue::Null => match col_ty {
                ColumnType::Bool => PgParam::Bool(None),
                ColumnType::Integer => PgParam::I64(None),
                ColumnType::Float => PgParam::F64(None),
                ColumnType::Text => PgParam::Text(None),
                ColumnType::Json => PgParam::Json(None),
            },
            ColumnValue::Bool(b) => PgParam::Bool(Some(*b)),
            ColumnValue::Integer(i) => {
                if col_ty == ColumnType::Float {
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "widened DOUBLE PRECISION column receives integer value as f64"
                    )]
                    PgParam::F64(Some(*i as f64))
                } else {
                    PgParam::I64(Some(*i))
                }
            }
            ColumnValue::Float(f) => PgParam::F64(Some(*f)),
            ColumnValue::Text(s) => PgParam::Text(Some(s.clone())),
            ColumnValue::Json(j) => PgParam::Json(Some(
                serde_json::from_str(j).unwrap_or(Value::String(j.clone())),
            )),
        });
    }

    let sql = build_insert_sql(&table_q, mode, &cols, &user_col_names, params.len());
    let param_refs: Vec<&(dyn ToSql + Sync)> = params.iter().map(PgParam::as_sql).collect();
    tx.execute(&sql, &param_refs).await?;
    Ok(())
}

fn build_insert_sql(
    table_q: &str,
    mode: WriteMode,
    cols: &[String],
    user_col_names: &[String],
    param_count: usize,
) -> String {
    let col_list: Vec<String> = cols.iter().map(|c| quote_ident(c)).collect();
    let placeholders: Vec<String> = (1..=param_count).map(|i| format!("${i}")).collect();
    let mut sql = format!(
        "INSERT INTO {table_q} ({}) VALUES ({})",
        col_list.join(", "),
        placeholders.join(", ")
    );
    match mode {
        WriteMode::Append | WriteMode::Replace | WriteMode::History => {
            sql.push_str(" ON CONFLICT (_goodfetch_delivery_id) DO NOTHING");
        }
        WriteMode::InsertOnly => {
            sql.push_str(
                " ON CONFLICT (_goodfetch_dataset, _goodfetch_key) WHERE _goodfetch_key IS NOT NULL DO NOTHING",
            );
        }
        WriteMode::Upsert => {
            let mut updates = vec![
                "_goodfetch_delivery_id = EXCLUDED._goodfetch_delivery_id".to_owned(),
                "_goodfetch_record_number = EXCLUDED._goodfetch_record_number".to_owned(),
                "_goodfetch_run_id = EXCLUDED._goodfetch_run_id".to_owned(),
                "_goodfetch_run_epoch = EXCLUDED._goodfetch_run_epoch".to_owned(),
                "_goodfetch_committed_at = EXCLUDED._goodfetch_committed_at".to_owned(),
                "_goodfetch_meta = EXCLUDED._goodfetch_meta".to_owned(),
            ];
            for u in user_col_names {
                let q = quote_ident(u);
                updates.push(format!("{q} = EXCLUDED.{q}"));
            }
            let _ = write!(
                sql,
                " ON CONFLICT (_goodfetch_dataset, _goodfetch_key) WHERE _goodfetch_key IS NOT NULL DO UPDATE SET {}",
                updates.join(", ")
            );
        }
    }
    sql
}

/// Translates [`PostgresError`] into the driver's [`Failure`] (ADR 0127).
pub fn map_postgres_error(error: PostgresError) -> Failure {
    match error {
        PostgresError::Config(msg) => Failure::new(FailureKind::Permanent, msg),
        PostgresError::NoRuntime => Failure::new(
            FailureKind::Permanent,
            "PostgreSQL Sink requires an active Tokio runtime",
        ),
        PostgresError::RejectedRecord(msg) => Failure::new(FailureKind::RejectedRecords, msg),
        PostgresError::Db(err) => {
            let text = err.to_string();
            if let Some(code) = err.code() {
                if *code == SqlState::INVALID_PASSWORD
                    || *code == SqlState::INVALID_AUTHORIZATION_SPECIFICATION
                    || *code == SqlState::INSUFFICIENT_PRIVILEGE
                {
                    return Failure::new(FailureKind::Auth, text);
                }
                if *code == SqlState::CHECK_VIOLATION
                    || *code == SqlState::NOT_NULL_VIOLATION
                    || *code == SqlState::UNIQUE_VIOLATION
                    || *code == SqlState::INVALID_TEXT_REPRESENTATION
                    || *code == SqlState::STRING_DATA_RIGHT_TRUNCATION
                {
                    return Failure::new(FailureKind::RejectedRecords, text);
                }
                if *code == SqlState::T_R_SERIALIZATION_FAILURE
                    || *code == SqlState::T_R_DEADLOCK_DETECTED
                    || *code == SqlState::TOO_MANY_CONNECTIONS
                    || *code == SqlState::ADMIN_SHUTDOWN
                    || *code == SqlState::CRASH_SHUTDOWN
                    || *code == SqlState::CANNOT_CONNECT_NOW
                    || code.code().starts_with("08")
                {
                    return Failure::new(FailureKind::Transient, text);
                }
                if *code == SqlState::UNDEFINED_TABLE
                    || *code == SqlState::SYNTAX_ERROR
                    || *code == SqlState::UNDEFINED_COLUMN
                {
                    return Failure::new(FailureKind::Permanent, text);
                }
            }
            if err.is_closed() {
                return Failure::new(FailureKind::Transient, text);
            }
            tracing::warn!(error = %text, "unrecognized PostgreSQL failure; treating as transient");
            Failure::new(FailureKind::Unrecognized, text)
        }
    }
}
