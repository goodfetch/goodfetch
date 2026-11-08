//! Each run table the SQL tab reads (ADR 0057), with its columns' types,
//! and where it is in the run file: the Datasets' own tables ([`crate::TABLES`]), described by this crate,
//! which creates them: each run table's table, its key and the column each
//! of its columns reads, and whether a run file holds them at all, which a
//! Run no crawl wrote does not. A change of those tables changes this
//! description with them, so a reader of the run file that goes through it
//! cannot drift from them.

use rusqlite::Connection;

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
pub(crate) const RECORDS: RunTable = RunTable {
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
pub(crate) const REJECTED_RECORDS: RunTable = RunTable {
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
pub(crate) const DATASETS: RunTable = RunTable {
    name: "datasets",
    columns: &[
        column("run", ColumnType::Text),
        column("name", ColumnType::Text),
        column("position", ColumnType::Integer),
        column("schema", ColumnType::Json),
        column("key", ColumnType::Json),
    ],
};

/// Where one run table's rows are in the run file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunFileTable {
    /// The run table it serves.
    pub run_table: RunTable,
    /// The run file's table.
    pub name: &'static str,
    /// Its key, in order, by which a reader pages through it.
    pub key: &'static [&'static str],
    /// The run file's column of each of the run table's columns, in their
    /// order; `None` for `run`, the Run ID, which no row holds.
    columns: &'static [Option<&'static str>],
}

/// Every run table a query reads, each with its place in the run file.
pub const RUN_FILE_TABLES: [RunFileTable; 3] = [RECORDS_TABLE, REJECTED_TABLE, DATASETS_TABLE];

/// The committed Records' place in the run file.
pub(crate) const RECORDS_TABLE: RunFileTable = RunFileTable {
    run_table: RECORDS,
    name: "records",
    key: &["dataset", "number"],
    columns: &[
        None,
        Some("dataset"),
        Some("number"),
        Some("data"),
        Some("work_item"),
        Some("url"),
        Some("committed_at"),
        Some("provenance"),
    ],
};

/// Each Dataset's rejection table's place in the run file.
pub(crate) const REJECTED_TABLE: RunFileTable = RunFileTable {
    run_table: REJECTED_RECORDS,
    name: "rejected_records",
    key: &["dataset", "number"],
    columns: &[
        None,
        Some("dataset"),
        Some("number"),
        Some("data"),
        Some("work_item"),
        Some("rejection"),
    ],
};

/// The declared Datasets' place in the run file.
pub(crate) const DATASETS_TABLE: RunFileTable = RunFileTable {
    run_table: DATASETS,
    name: "datasets",
    key: &["name"],
    columns: &[
        None,
        Some("name"),
        Some("position"),
        Some("schema"),
        Some("key"),
    ],
};

impl RunFileTable {
    /// The run file's column the run table's column at `index` reads;
    /// `None` for `run`, and past its columns.
    pub fn column(&self, index: usize) -> Option<&'static str> {
        self.columns.get(index).copied().flatten()
    }

    /// Whether the run file at `connection` holds this table: a Run no
    /// crawl wrote holds no Dataset's table.
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn held_by(&self, connection: &Connection) -> rusqlite::Result<bool> {
        connection
            .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1")?
            .exists([self.name])
    }
}

#[cfg(test)]
mod tests;
