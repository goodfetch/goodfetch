//! The SQL tab (ADRs 0031 and 0057; spec #593): `dataset.query`, a
//! read-only SQL query over the runs a person selects, each run's tables
//! registered as table functions that read its run file, with external
//! access off, time, memory and row limits, and secret fields never
//! columns; the tables a query can read, with their columns' types; and
//! its Error codes and settings. Foxhound's own; a catalog read every
//! surface offers.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Reach, Setting, Stability, ValueKind, schema_of,
};
use crate::run::RunId;
use crate::settings::SettingLimit;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The most runs one query selects.
pub const MAX_RUNS: usize = 100;

/// The type of a column of a run table, as a query reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnType {
    /// Text, `VARCHAR`.
    Text,
    /// A whole number, `BIGINT`.
    Integer,
    /// A JSON value as its text, `VARCHAR`, which DuckDB's JSON functions
    /// read.
    Json,
}

/// One column of a run table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunColumn {
    /// Its name.
    pub name: &'static str,
    /// Its type.
    pub column_type: ColumnType,
}

/// A run table a query reads, as the table function of its name, which
/// takes a selected run's Run ID: `records('my-run')`. Its columns are the
/// only fields of the run file it reads; no secret field is one (ADR 0057).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunTable {
    /// Its name, the table function's.
    pub name: &'static str,
    /// Its columns, in order; the first is always `run`, the Run ID.
    pub columns: &'static [RunColumn],
}

const fn column(name: &'static str, column_type: ColumnType) -> RunColumn {
    RunColumn { name, column_type }
}

/// Each Dataset's committed Records, as `run.records` reads them.
pub const RECORDS: RunTable = RunTable {
    name: "records",
    columns: &[
        column("run", ColumnType::Text),
        column("dataset", ColumnType::Text),
        column("number", ColumnType::Integer),
        column("value", ColumnType::Json),
        column("request_id", ColumnType::Text),
        column("url", ColumnType::Text),
        column("committed_at", ColumnType::Text),
        column("provenance", ColumnType::Json),
    ],
};

/// Each Dataset's rejected Records, as `run.rejected_records` reads them.
pub const REJECTED_RECORDS: RunTable = RunTable {
    name: "rejected_records",
    columns: &[
        column("run", ColumnType::Text),
        column("dataset", ColumnType::Text),
        column("number", ColumnType::Integer),
        column("data", ColumnType::Json),
        column("request_id", ColumnType::Text),
        column("rejection", ColumnType::Json),
    ],
};

/// The Datasets a crawl declares.
pub const DATASETS: RunTable = RunTable {
    name: "datasets",
    columns: &[
        column("run", ColumnType::Text),
        column("name", ColumnType::Text),
        column("position", ColumnType::Integer),
        column("schema", ColumnType::Json),
        column("key", ColumnType::Json),
    ],
};

/// Every run table a query reads.
pub const RUN_TABLES: [RunTable; 3] = [RECORDS, REJECTED_RECORDS, DATASETS];

/// What `dataset.query` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatasetQueryParams {
    /// One `SELECT` or `WITH` statement.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sql: String,
    /// The runs it reads, from 1 to 100: only these are registered.
    #[schemars(length(min = 1, max = 100))]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runs: Vec<RunId>,
    /// The most rows it returns, at most `dataset.query.max_rows`; absent,
    /// that setting's value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1))]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: Option<u64>,
}

/// One column of a query's result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueryColumn {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its SQL type, as DuckDB names it, such as `BIGINT` or `VARCHAR`.
    #[serde(rename = "type")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub column_type: String,
}

/// What `dataset.query` answers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetQueryResult {
    /// The result's columns, in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub columns: Vec<QueryColumn>,
    /// Its first rows, each a value per column as JSON, at most the
    /// call's `limit`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rows: Vec<Vec<Value>>,
    /// How many rows the query produced, those not returned counted.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub total: u64,
}

/// `dataset.query`.
pub const DATASET_QUERY: Operation = Operation {
    name: "dataset.query",
    description: "Runs one read-only SQL statement, `SELECT` or `WITH`, over the runs `runs` selects, at most 100, and answers its columns, its first rows up to `limit` (at most `dataset.query.max_rows`) and how many rows it produced. Each run's tables are table functions that take its Run ID: `records(run)` (each Dataset's committed Records), `rejected_records(run)` and `datasets(run)`, which read its run file read-only; secret fields are never columns. Any other statement, a write, `ATTACH`, `INSTALL`, `LOAD`, `COPY`, a read of a file or URL, or a run not selected fails with `query_invalid`; past `timeout.dataset_query` it fails with `deadline_exceeded`, and past `dataset.query.memory_limit` with `query_memory_exceeded`. A run that does not exist fails with `run_not_found`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<DatasetQueryParams>,
    result: schema_of::<DatasetQueryResult>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A statement the SQL tab does not run (spec #593).
pub const QUERY_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "query_invalid",
    description: "The SQL tab did not run the statement: it does not parse, names a table, column or function that does not exist or a run the query did not select, is not one `SELECT` or `WITH` statement (a write, `ATTACH`, `INSTALL`, `LOAD` or `COPY`), or reads a file or URL, which the SQL tab never reaches. The details say which. Read only the selected runs' tables, and select the run first.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A query past `dataset.query.memory_limit` (spec #593).
pub const QUERY_MEMORY_EXCEEDED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "query_memory_exceeded",
    description: "The query needed more memory than `dataset.query.memory_limit`, so it was stopped. The details name the setting, its value and its layer. Aggregate or filter sooner, select fewer runs, or raise `dataset.query.memory_limit`.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A failure of DuckDB its translation point does not recognise (ADR
/// 0127).
pub const QUERY_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "query_failed",
    description: "The query failed in a way the engine does not recognise, such as a run file it could not read; the engine's log holds the cause. Try again; if it fails the same way, report it with the log.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Why `query_invalid` refused a statement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QueryRefusal {
    /// It does not parse.
    Syntax,
    /// It names a table, column, function or type that does not exist.
    UnknownName,
    /// It is not one `SELECT` or `WITH` statement.
    NotARead,
    /// It names a run the query did not select.
    RunNotSelected,
    /// It reads a file or URL.
    OutsideRead,
}

/// What `query_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueryInvalidDetails {
    /// Why the statement was refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: QueryRefusal,
}

/// What `query_memory_exceeded` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct QueryMemoryExceededDetails {
    /// `dataset.query.memory_limit`, its value in bytes and its layer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: SettingLimit,
}

fn thirty_seconds() -> Value {
    json!(30_000)
}

fn five_hundred_twelve_mebibytes() -> Value {
    json!(512 * 1024 * 1024)
}

fn ten_thousand() -> Value {
    json!(10_000)
}

/// `timeout.dataset_query` (ADR 0080; spec #593).
pub const TIMEOUT_DATASET_QUERY: Setting = Setting {
    name: "timeout.dataset_query",
    description: "How long one `dataset.query` may run before it is stopped and fails with `deadline_exceeded`: 30 s.",
    kind: ValueKind::Duration,
    default: thirty_seconds,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `dataset.query.memory_limit` (spec #593).
pub const DATASET_QUERY_MEMORY_LIMIT: Setting = Setting {
    name: "dataset.query.memory_limit",
    description: "The most memory, in bytes, one `dataset.query` may use; past it, the query fails with `query_memory_exceeded`. 512 MiB (536870912) by default.",
    kind: ValueKind::Count,
    default: five_hundred_twelve_mebibytes,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `dataset.query.max_rows` (spec #593).
pub const DATASET_QUERY_MAX_ROWS: Setting = Setting {
    name: "dataset.query.max_rows",
    description: "The most rows one `dataset.query` returns; the rest are counted in its `total`, not returned. 10,000 by default.",
    kind: ValueKind::Count,
    default: ten_thousand,
    clearable: false,
    authority: Authority::Every,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// The module's entries.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[DATASET_QUERY],
    events: &[],
    handler_requests: &[],
    error_codes: &[QUERY_INVALID, QUERY_MEMORY_EXCEEDED, QUERY_FAILED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        TIMEOUT_DATASET_QUERY,
        DATASET_QUERY_MEMORY_LIMIT,
        DATASET_QUERY_MAX_ROWS,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
