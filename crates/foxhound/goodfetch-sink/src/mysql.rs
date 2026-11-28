//! MySQL and MariaDB in-binary Sink (ADRs 0008, 0026, 0103 and 0127; issue
//! 1163): delivers committed Records into MySQL or MariaDB over `mysql_async`
//! in every write mode (`append`, `upsert`, `insert_only`, `replace`,
//! `history`) with transactional `SELECT ... FOR UPDATE` epoch fencing
//! (`GuaranteeClass::Fenced`), schema evolution via [`TableSchema`], and
//! ADR 0127 error translation.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

use goodfetch_protocol::foxhound::sink::{SchemaContract, WriteMode};
use mysql_async::prelude::Queryable as _;
use mysql_async::{Opts, Pool, TxOpts, Value as MySqlValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::contract::{
    Batch, Delivered, Failure, FailureKind, GuaranteeClass, Primitives, Probe, RunCopy, Written,
};
use crate::schema::{ColumnType, ColumnValue, PreparedRow, SchemaChange, TableSchema};

/// Default MySQL/MariaDB target table when `target` is omitted.
pub const DEFAULT_MYSQL_TARGET: &str = "records";

/// Errors returned by [`MySqlSink`] before translation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MySqlError {
    /// Invalid configuration (bad URL or empty table name).
    #[error("invalid MySQL Sink configuration: {0}")]
    Config(String),
    /// Error from `mysql_async`.
    #[error("{0}")]
    Db(#[from] mysql_async::Error),
    /// Record missing required key or refused by `freeze` contract.
    #[error("rejected record: {0}")]
    RejectedRecord(String),
    /// No active Tokio runtime available when called synchronously.
    #[error("MySQL Sink requires an active Tokio runtime")]
    NoRuntime,
}

fn quote_ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

fn mysql_type(ty: ColumnType) -> &'static str {
    match ty {
        ColumnType::Bool => "TINYINT(1)",
        ColumnType::Integer => "BIGINT",
        ColumnType::Float => "DOUBLE",
        ColumnType::Text | ColumnType::Json => "LONGTEXT",
    }
}

fn from_mysql_type(ty: &str) -> ColumnType {
    match ty.to_ascii_lowercase().as_str() {
        "tinyint" | "bit" => ColumnType::Bool,
        "bigint" | "int" | "integer" | "smallint" | "mediumint" => ColumnType::Integer,
        "double" | "float" | "decimal" => ColumnType::Float,
        "json" => ColumnType::Json,
        _ => ColumnType::Text,
    }
}

/// Built-in MySQL / MariaDB Sink implementing [`Primitives`].
pub struct MySqlSink {
    dsn: SecretString,
    target: String,
    mode: WriteMode,
    contract: SchemaContract,
    flatten: bool,
    pool: Option<Pool>,
    schema: Option<TableSchema>,
    schema_applied_for: BTreeMap<String, bool>,
}

impl fmt::Debug for MySqlSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MySqlSink")
            .field("dsn", &"<redacted>")
            .field("target", &self.target)
            .field("mode", &self.mode)
            .field("contract", &self.contract)
            .field("flatten", &self.flatten)
            .finish_non_exhaustive()
    }
}

impl MySqlSink {
    /// Creates a MySQL/MariaDB Sink from a `mysql://...` DSN targeting `table`.
    ///
    /// # Errors
    ///
    /// [`MySqlError::Config`] if `dsn` or `table` is empty or unparsable.
    pub fn new(
        dsn: impl Into<String>,
        table: impl Into<String>,
        mode: WriteMode,
        contract: SchemaContract,
        flatten: bool,
    ) -> Result<Self, MySqlError> {
        Self::from_secret(
            SecretString::from(dsn.into()),
            table,
            mode,
            contract,
            flatten,
        )
    }

    /// Creates a MySQL/MariaDB Sink from a [`SecretString`] DSN targeting `table`.
    ///
    /// # Errors
    ///
    /// [`MySqlError::Config`] if `dsn` or `table` is empty or unparsable.
    pub fn from_secret(
        dsn: SecretString,
        table: impl Into<String>,
        mode: WriteMode,
        contract: SchemaContract,
        flatten: bool,
    ) -> Result<Self, MySqlError> {
        let table = table.into();
        if dsn.expose_secret().trim().is_empty() {
            return Err(MySqlError::Config(
                "destination DSN must not be empty".into(),
            ));
        }
        if table.trim().is_empty() {
            return Err(MySqlError::Config("target table must not be empty".into()));
        }
        let _ = Opts::from_url(dsn.expose_secret())
            .map_err(|_| MySqlError::Config("invalid MySQL destination DSN".into()))?;
        Ok(Self {
            dsn,
            target: table,
            mode,
            contract,
            flatten,
            pool: None,
            schema: None,
            schema_applied_for: BTreeMap::new(),
        })
    }

    /// Returns the highest recorded `run_epoch` for `(self.target, run_id)` if
    /// present.
    ///
    /// # Errors
    ///
    /// Connection or query errors from MySQL/MariaDB.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future cancels the in-flight query without modifying state.
    pub async fn current_epoch(&mut self, run_id: &str) -> Result<Option<u64>, MySqlError> {
        self.ensure_tables().await?;
        let pool = self.pool()?;
        let mut conn = pool.get_conn().await?;
        let row: Option<i64> = conn
            .exec_first(
                "SELECT `run_epoch` FROM `_goodfetch_deliveries` WHERE `target` = ? AND `run_id` = ?",
                (&self.target, run_id),
            )
            .await?;
        Ok(row.and_then(|v| u64::try_from(v).ok()))
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

    fn pool(&mut self) -> Result<Pool, MySqlError> {
        if let Some(pool) = &self.pool {
            return Ok(pool.clone());
        }
        let opts = Opts::from_url(self.dsn.expose_secret())
            .map_err(|_| MySqlError::Config("invalid MySQL destination DSN".into()))?;
        let pool = Pool::new(opts);
        self.pool = Some(pool.clone());
        Ok(pool)
    }

    async fn ensure_tables(&mut self) -> Result<(), MySqlError> {
        let pool = self.pool()?;
        let mut conn = pool.get_conn().await?;
        conn.query_drop(
            "CREATE TABLE IF NOT EXISTS `_goodfetch_deliveries` (
                `target` VARCHAR(191) NOT NULL,
                `run_id` VARCHAR(191) NOT NULL,
                `run_epoch` BIGINT NOT NULL,
                PRIMARY KEY (`target`, `run_id`)
            ) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin",
        )
        .await?;
        conn.query_drop(
            "CREATE TABLE IF NOT EXISTS `_goodfetch_delivery_log` (
                `target` VARCHAR(191) NOT NULL,
                `delivery_id` VARCHAR(512) NOT NULL,
                `run_id` VARCHAR(191) NOT NULL,
                `run_epoch` BIGINT NOT NULL,
                `dataset` VARCHAR(191) NOT NULL,
                `record_number` BIGINT NOT NULL,
                PRIMARY KEY (`target`, `delivery_id`),
                INDEX `idx_gf_log_run` (`target`, `run_id`, `run_epoch`, `dataset`, `record_number`)
            ) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin",
        )
        .await?;
        conn.query_drop(
            "CREATE TABLE IF NOT EXISTS `_goodfetch_swaps` (
                `target` VARCHAR(191) NOT NULL,
                `run_id` VARCHAR(191) NOT NULL,
                `run_epoch` BIGINT NOT NULL,
                PRIMARY KEY (`target`, `run_id`)
            ) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin",
        )
        .await?;
        create_mysql_table(&mut conn, &self.target, self.mode).await?;
        if self.mode == WriteMode::Replace {
            create_mysql_table(&mut conn, &self.staging_table(), WriteMode::Upsert).await?;
        }
        Ok(())
    }

    async fn load_schema(&mut self) -> Result<(), MySqlError> {
        if self.schema.is_some() {
            return Ok(());
        }
        let table = self.write_table();
        let pool = self.pool()?;
        let mut conn = pool.get_conn().await?;
        let rows: Vec<(String, String)> = conn
            .exec(
                "SELECT COLUMN_NAME, DATA_TYPE
                 FROM INFORMATION_SCHEMA.COLUMNS
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?
                 ORDER BY ORDINAL_POSITION",
                (&table,),
            )
            .await?;
        let existing: Vec<(String, ColumnType)> = rows
            .into_iter()
            .filter(|(name, _)| !name.starts_with("_goodfetch_"))
            .map(|(name, ty)| (name, from_mysql_type(&ty)))
            .collect();
        self.schema = Some(TableSchema::from_existing(&existing));
        Ok(())
    }

    async fn probe_async(&mut self, run: &RunCopy) -> Result<Probe, MySqlError> {
        self.ensure_tables().await?;
        self.load_schema().await?;
        let target = self.target.clone();
        let write_table = self.write_table();
        let mode = self.mode;
        let pool = self.pool()?;
        let mut conn = pool.get_conn().await?;
        let mut tx = conn.start_transaction(TxOpts::default()).await?;
        let epoch_i64 = i64::try_from(run.run_epoch).unwrap_or(i64::MAX);
        let _ = check_and_bump_mysql_fence(
            &mut tx,
            &target,
            &write_table,
            mode,
            &run.run_id,
            epoch_i64,
        )
        .await?;
        let delivered_rows: Vec<(String, i64)> = tx
            .exec(
                "SELECT `dataset`, MAX(`record_number`)
                 FROM `_goodfetch_delivery_log`
                 WHERE `target` = ? AND `run_id` = ? AND `run_epoch` = ?
                 GROUP BY `dataset`",
                (&target, &run.run_id, epoch_i64),
            )
            .await?;
        tx.commit().await?;

        let mut delivered = BTreeMap::new();
        for (ds, max_num) in delivered_rows {
            if let Ok(num) = u64::try_from(max_num) {
                delivered.insert(ds, num);
            }
        }
        Ok(Probe {
            class: GuaranteeClass::Fenced,
            delivered,
        })
    }

    async fn write_async(&mut self, batch: &Batch) -> Result<Written, MySqlError> {
        self.ensure_tables().await?;
        self.load_schema().await?;

        let mut working_schema = self
            .schema
            .as_ref()
            .expect("invariant: schema loaded")
            .clone();
        let mut working_applied = self.schema_applied_for.clone();
        let (prepared, pending_changes) = working_schema
            .prepare_batch(
                &mut working_applied,
                batch,
                self.mode,
                self.contract,
                self.flatten,
            )
            .map_err(MySqlError::RejectedRecord)?;

        let write_table = self.write_table();
        let target_table = self.target.clone();
        let mode = self.mode;
        let pool = self.pool()?;
        let mut conn = pool.get_conn().await?;
        let epoch_i64 = i64::try_from(batch.run.run_epoch).unwrap_or(i64::MAX);

        // Pre-check fence before any implicit-commit DDL so a superseded zombie
        // never alters the live table.
        let current: Option<i64> = conn
            .exec_first(
                "SELECT `run_epoch` FROM `_goodfetch_deliveries` WHERE `target` = ? AND `run_id` = ?",
                (&target_table, &batch.run.run_id),
            )
            .await?;
        if let Some(cur) = current
            && cur > epoch_i64
        {
            return Ok(Written::Superseded {
                run_epoch: u64::try_from(cur).unwrap_or(u64::MAX),
            });
        }

        // DDL in MySQL implicitly commits, so apply schema evolution before starting the batch transaction.
        apply_mysql_changes(&mut conn, &write_table, &pending_changes).await?;
        if mode == WriteMode::Replace {
            apply_mysql_changes(&mut conn, &target_table, &pending_changes).await?;
        }

        let mut tx = conn.start_transaction(TxOpts::default()).await?;
        if let Some(newer) = check_and_bump_mysql_fence(
            &mut tx,
            &target_table,
            &write_table,
            mode,
            &batch.run.run_id,
            epoch_i64,
        )
        .await?
        {
            tx.rollback().await?;
            return Ok(Written::Superseded { run_epoch: newer });
        }

        for (record, row_opt) in prepared {
            let num_i64 = i64::try_from(record.number).unwrap_or(i64::MAX);
            tx.exec_drop(
                "INSERT IGNORE INTO `_goodfetch_delivery_log` (`target`, `delivery_id`, `run_id`, `run_epoch`, `dataset`, `record_number`)
                 VALUES (?, ?, ?, ?, ?, ?)",
                (
                    &target_table,
                    record.delivery_id.as_str(),
                    &batch.run.run_id,
                    epoch_i64,
                    &batch.dataset,
                    num_i64,
                ),
            )
            .await?;
            if tx.affected_rows() == 0 {
                continue;
            }
            if let Some(row) = row_opt {
                insert_mysql_row(
                    &mut tx,
                    &write_table,
                    mode,
                    &batch.run,
                    &batch.dataset,
                    record,
                    &row,
                )
                .await?;
            }
        }
        tx.commit().await?;
        self.schema = Some(working_schema);
        self.schema_applied_for = working_applied;
        Ok(Written::Delivered)
    }

    async fn finish_async(&mut self, run: &RunCopy) -> Result<Written, MySqlError> {
        if self.mode != WriteMode::Replace {
            return Ok(Written::Delivered);
        }
        self.ensure_tables().await?;
        let target = self.target.clone();
        let staging = self.staging_table();
        let target_q = quote_ident(&target);
        let staging_q = quote_ident(&staging);
        let pool = self.pool()?;
        let mut conn = pool.get_conn().await?;
        let epoch_i64 = i64::try_from(run.run_epoch).unwrap_or(i64::MAX);

        let staging_cols: Vec<(String, String)> = conn
            .exec(
                "SELECT COLUMN_NAME, DATA_TYPE
                 FROM INFORMATION_SCHEMA.COLUMNS
                 WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?
                 ORDER BY ORDINAL_POSITION",
                (&staging,),
            )
            .await?;
        let mut col_names = Vec::with_capacity(staging_cols.len());
        for (name, ty) in &staging_cols {
            if !name.starts_with("_goodfetch_") {
                add_mysql_column_if_missing(
                    &mut conn,
                    &target_q,
                    name,
                    mysql_type(from_mysql_type(ty)),
                )
                .await?;
            }
            col_names.push(quote_ident(name));
        }
        let col_list = col_names.join(", ");

        let mut tx = conn.start_transaction(TxOpts::default()).await?;
        if let Some(newer) = check_and_bump_mysql_fence(
            &mut tx,
            &target,
            &staging,
            WriteMode::Replace,
            &run.run_id,
            epoch_i64,
        )
        .await?
        {
            tx.rollback().await?;
            return Ok(Written::Superseded { run_epoch: newer });
        }
        let prev: Option<i64> = tx
            .exec_first(
                "SELECT `run_epoch` FROM `_goodfetch_swaps` WHERE `target` = ? AND `run_id` = ? FOR UPDATE",
                (&target, &run.run_id),
            )
            .await?;
        if prev.is_some_and(|p| p >= epoch_i64) {
            tx.commit().await?;
            return Ok(Written::Delivered);
        }
        tx.query_drop(format!("DELETE FROM {target_q}")).await?;
        tx.exec_drop(
            format!(
                "INSERT INTO {target_q} ({col_list}) SELECT {col_list} FROM {staging_q} WHERE `_goodfetch_run_id` = ? AND `_goodfetch_run_epoch` = ?"
            ),
            (&run.run_id, epoch_i64),
        )
        .await?;
        tx.exec_drop(
            format!("DELETE FROM {staging_q} WHERE `_goodfetch_run_id` = ?"),
            (&run.run_id,),
        )
        .await?;
        tx.exec_drop(
            "INSERT INTO `_goodfetch_swaps` (`target`, `run_id`, `run_epoch`) VALUES (?, ?, ?)
             ON DUPLICATE KEY UPDATE `run_epoch` = VALUES(`run_epoch`)",
            (&target, &run.run_id, epoch_i64),
        )
        .await?;
        tx.commit().await?;
        Ok(Written::Delivered)
    }
}

impl Primitives for MySqlSink {
    type Error = MySqlError;

    fn probe(&mut self, run: &RunCopy) -> Result<Probe, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| MySqlError::NoRuntime)?;
        handle.block_on(self.probe_async(run))
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| MySqlError::NoRuntime)?;
        handle.block_on(self.write_async(batch))
    }

    fn finish(&mut self, run: &RunCopy) -> Result<Written, Self::Error> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| MySqlError::NoRuntime)?;
        handle.block_on(self.finish_async(run))
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        map_mysql_error(error)
    }
}

async fn create_mysql_table(
    conn: &mut mysql_async::Conn,
    table: &str,
    mode: WriteMode,
) -> Result<(), MySqlError> {
    let table_q = quote_ident(table);
    let unique_clause = if matches!(mode, WriteMode::Upsert | WriteMode::InsertOnly) {
        ", UNIQUE KEY `uq_gf_key` (`_goodfetch_dataset`(128), `_goodfetch_key`(255))"
    } else {
        ""
    };
    conn.query_drop(format!(
        "CREATE TABLE IF NOT EXISTS {table_q} (
            `_goodfetch_delivery_id` VARCHAR(512) PRIMARY KEY,
            `_goodfetch_dataset` VARCHAR(255) NOT NULL,
            `_goodfetch_record_number` BIGINT NOT NULL,
            `_goodfetch_run_id` VARCHAR(255) NOT NULL,
            `_goodfetch_run_epoch` BIGINT NOT NULL,
            `_goodfetch_committed_at` VARCHAR(64) NOT NULL,
            `_goodfetch_meta` LONGTEXT,
            `_goodfetch_key` VARCHAR(512),
            `_goodfetch_deleted` TINYINT(1) NOT NULL DEFAULT 0,
            `_goodfetch_valid_from` VARCHAR(64),
            `_goodfetch_valid_to` VARCHAR(64){unique_clause}
        ) CHARACTER SET utf8mb4 COLLATE utf8mb4_bin"
    ))
    .await?;
    Ok(())
}

async fn check_and_bump_mysql_fence(
    tx: &mut mysql_async::Transaction<'_>,
    target: &str,
    write_table: &str,
    mode: WriteMode,
    run_id: &str,
    epoch_i64: i64,
) -> Result<Option<u64>, MySqlError> {
    let current: Option<i64> = tx
        .exec_first(
            "SELECT `run_epoch` FROM `_goodfetch_deliveries` WHERE `target` = ? AND `run_id` = ? FOR UPDATE",
            (target, run_id),
        )
        .await?;
    match current {
        None => {
            tx.exec_drop(
                "INSERT INTO `_goodfetch_deliveries` (`target`, `run_id`, `run_epoch`) VALUES (?, ?, ?)",
                (target, run_id, epoch_i64),
            )
            .await?;
            Ok(None)
        }
        Some(cur) if cur > epoch_i64 => Ok(Some(u64::try_from(cur).unwrap_or(u64::MAX))),
        Some(cur) => {
            if epoch_i64 > cur {
                tx.exec_drop(
                    "UPDATE `_goodfetch_deliveries` SET `run_epoch` = ? WHERE `target` = ? AND `run_id` = ?",
                    (epoch_i64, target, run_id),
                )
                .await?;
                let write_q = quote_ident(write_table);
                tx.exec_drop(
                    format!(
                        "DELETE FROM {write_q} WHERE `_goodfetch_run_id` = ? AND `_goodfetch_run_epoch` < ?"
                    ),
                    (run_id, epoch_i64),
                )
                .await?;
                tx.exec_drop(
                    "DELETE FROM `_goodfetch_delivery_log` WHERE `target` = ? AND `run_id` = ? AND `run_epoch` < ?",
                    (target, run_id, epoch_i64),
                )
                .await?;
                if mode == WriteMode::History {
                    tx.exec_drop(
                        format!(
                            "UPDATE {write_q} SET `_goodfetch_valid_to` = NULL
                             WHERE `_goodfetch_run_id` = ? AND `_goodfetch_valid_to` IS NOT NULL"
                        ),
                        (run_id,),
                    )
                    .await?;
                }
            }
            Ok(None)
        }
    }
}

async fn add_mysql_column_if_missing(
    conn: &mut mysql_async::Conn,
    table_q: &str,
    name: &str,
    ty: &str,
) -> Result<(), MySqlError> {
    let col_q = quote_ident(name);
    match conn
        .query_drop(format!("ALTER TABLE {table_q} ADD COLUMN {col_q} {ty}"))
        .await
    {
        Ok(()) => Ok(()),
        // 1060 = ER_DUP_FIELDNAME (portable across Oracle MySQL 8.0/8.4 and MariaDB).
        Err(mysql_async::Error::Server(ref s)) if s.code == 1060 => Ok(()),
        Err(err) => Err(MySqlError::Db(err)),
    }
}

async fn apply_mysql_changes(
    conn: &mut mysql_async::Conn,
    table: &str,
    changes: &[SchemaChange],
) -> Result<(), MySqlError> {
    let table_q = quote_ident(table);
    for change in changes {
        match change {
            SchemaChange::AddedColumn { name, column_type }
            | SchemaChange::AddedVariantColumn {
                name, column_type, ..
            } => {
                add_mysql_column_if_missing(conn, &table_q, name, mysql_type(*column_type)).await?;
            }
            SchemaChange::WidenedColumn { name, to, .. } => {
                let col_q = quote_ident(name);
                let ty = mysql_type(*to);
                conn.query_drop(format!("ALTER TABLE {table_q} MODIFY COLUMN {col_q} {ty}"))
                    .await?;
            }
        }
    }
    Ok(())
}

async fn insert_mysql_row(
    tx: &mut mysql_async::Transaction<'_>,
    table: &str,
    mode: WriteMode,
    run: &RunCopy,
    dataset: &str,
    record: &Delivered,
    row: &PreparedRow,
) -> Result<(), MySqlError> {
    let table_q = quote_ident(table);
    let key_text = record
        .key
        .as_ref()
        .map(|k| serde_json::to_string(k).unwrap_or_default());

    if mode == WriteMode::History
        && let Some(ref k) = key_text
    {
        tx.exec_drop(
            format!(
                "UPDATE {table_q} SET `_goodfetch_valid_to` = ?
                 WHERE `_goodfetch_dataset` = ? AND `_goodfetch_key` = ? AND `_goodfetch_valid_to` IS NULL"
            ),
            (&record.committed_at, dataset, k),
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
    let mut vals: Vec<MySqlValue> = vec![
        MySqlValue::from(record.delivery_id.as_str()),
        MySqlValue::from(dataset),
        MySqlValue::from(i64::try_from(record.number).unwrap_or(i64::MAX)),
        MySqlValue::from(run.run_id.as_str()),
        MySqlValue::from(i64::try_from(run.run_epoch).unwrap_or(i64::MAX)),
        MySqlValue::from(record.committed_at.as_str()),
        row.meta
            .as_ref()
            .map_or(MySqlValue::NULL, |m| MySqlValue::from(m.to_string())),
        key_text
            .as_deref()
            .map_or(MySqlValue::NULL, MySqlValue::from),
        if mode == WriteMode::History {
            MySqlValue::from(record.committed_at.as_str())
        } else {
            MySqlValue::NULL
        },
    ];
    let mut user_cols = Vec::new();
    for (col, val) in &row.columns {
        cols.push(col.clone());
        user_cols.push(col.clone());
        vals.push(match val {
            ColumnValue::Null => MySqlValue::NULL,
            ColumnValue::Bool(b) => MySqlValue::from(i8::from(*b)),
            ColumnValue::Integer(i) => MySqlValue::from(*i),
            ColumnValue::Float(f) => MySqlValue::from(*f),
            ColumnValue::Text(s) | ColumnValue::Json(s) => MySqlValue::from(s.as_str()),
        });
    }
    let col_list: Vec<String> = cols.iter().map(|c| quote_ident(c)).collect();
    let placeholders = vec!["?"; vals.len()].join(", ");
    let mut sql = match mode {
        WriteMode::InsertOnly => format!(
            "INSERT IGNORE INTO {table_q} ({}) VALUES ({placeholders})",
            col_list.join(", ")
        ),
        _ => format!(
            "INSERT INTO {table_q} ({}) VALUES ({placeholders})",
            col_list.join(", ")
        ),
    };
    if matches!(mode, WriteMode::Upsert | WriteMode::Replace) {
        let mut updates = vec![
            "`_goodfetch_delivery_id` = VALUES(`_goodfetch_delivery_id`)".to_owned(),
            "`_goodfetch_record_number` = VALUES(`_goodfetch_record_number`)".to_owned(),
            "`_goodfetch_run_id` = VALUES(`_goodfetch_run_id`)".to_owned(),
            "`_goodfetch_run_epoch` = VALUES(`_goodfetch_run_epoch`)".to_owned(),
            "`_goodfetch_committed_at` = VALUES(`_goodfetch_committed_at`)".to_owned(),
            "`_goodfetch_meta` = VALUES(`_goodfetch_meta`)".to_owned(),
        ];
        for u in &user_cols {
            let q = quote_ident(u);
            updates.push(format!("{q} = VALUES({q})"));
        }
        let _ = write!(sql, " ON DUPLICATE KEY UPDATE {}", updates.join(", "));
    }
    tx.exec_drop(sql, mysql_async::Params::Positional(vals))
        .await?;
    Ok(())
}

/// Translates [`MySqlError`] into the driver's [`Failure`] (ADR 0127).
pub fn map_mysql_error(error: MySqlError) -> Failure {
    match error {
        MySqlError::Config(msg) => Failure::new(FailureKind::Permanent, msg),
        MySqlError::NoRuntime => Failure::new(
            FailureKind::Permanent,
            "MySQL Sink requires an active Tokio runtime",
        ),
        MySqlError::RejectedRecord(msg) => Failure::new(FailureKind::RejectedRecords, msg),
        MySqlError::Db(err) => {
            let text = err.to_string();
            match err {
                mysql_async::Error::Server(server) => match server.code {
                    1044 | 1045 => Failure::new(FailureKind::Auth, text),
                    1062 | 1048 | 1366 | 1406 | 3819 => {
                        Failure::new(FailureKind::RejectedRecords, text)
                    }
                    1205 | 1213 | 1040 => Failure::new(FailureKind::Transient, text),
                    1064 | 1146 | 1054 => Failure::new(FailureKind::Permanent, text),
                    _ => {
                        tracing::warn!(error = %text, "unrecognized MySQL server error; treating as transient");
                        Failure::new(FailureKind::Unrecognized, text)
                    }
                },
                mysql_async::Error::Io(_) => Failure::new(FailureKind::Transient, text),
                _ => {
                    tracing::warn!(error = %text, "unrecognized MySQL error; treating as transient");
                    Failure::new(FailureKind::Unrecognized, text)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn server_err(code: u16) -> MySqlError {
        MySqlError::Db(mysql_async::Error::Server(mysql_async::ServerError {
            code,
            message: format!("code {code}"),
            state: "HY000".into(),
        }))
    }

    #[rstest]
    #[case::config(MySqlError::Config("bad dsn".into()), FailureKind::Permanent)]
    #[case::no_runtime(MySqlError::NoRuntime, FailureKind::Permanent)]
    #[case::rejected(MySqlError::RejectedRecord("missing key".into()), FailureKind::RejectedRecords)]
    #[case::auth_1045(server_err(1045), FailureKind::Auth)]
    #[case::rejected_1062(server_err(1062), FailureKind::RejectedRecords)]
    #[case::transient_1213(server_err(1213), FailureKind::Transient)]
    #[case::permanent_1146(server_err(1146), FailureKind::Permanent)]
    #[case::unrecognized_server(server_err(9999), FailureKind::Unrecognized)]
    fn map_mysql_error_classifies_known_and_unrecognized(
        #[case] err: MySqlError,
        #[case] expected: FailureKind,
    ) {
        assert_eq!(map_mysql_error(err).kind(), expected);
    }

    #[test]
    fn debug_redacts_mysql_dsn() {
        let sink = MySqlSink::new(
            "mysql://alice:super-secret-mysql-pass@localhost:3306/db",
            "records",
            WriteMode::Append,
            SchemaContract::Evolve,
            false,
        )
        .unwrap();
        let rendered = format!("{sink:?}");
        assert!(!rendered.contains("super-secret-mysql-pass"));
        assert!(rendered.contains("<redacted>"));
    }
}
