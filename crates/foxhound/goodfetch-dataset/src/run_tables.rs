//! Each run table the SQL tab reads (ADR 0057), with its columns' types,
//! and where it is in the run file: the Datasets' own tables
//! ([`crate::TABLES`]), described by this crate, which creates them: each
//! run table's table, its key and the column each of its columns reads, and
//! whether a run file holds them at all, which a Run no crawl wrote does
//! not. A change of those tables changes this description with them, so a
//! reader of the run file that goes through it cannot drift from them.

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
    /// The run file's column it reads; `None` for `run`, the Run ID, which
    /// no row holds.
    source: Option<&'static str>,
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

/// The column `name`, reading the run file's `source`.
const fn column(name: &'static str, column_type: ColumnType, source: &'static str) -> RunColumn {
    RunColumn {
        name,
        column_type,
        source: Some(source),
    }
}

/// The column every run table opens with: `run`, the Run ID.
const RUN: RunColumn = RunColumn {
    name: "run",
    column_type: ColumnType::Text,
    source: None,
};

/// Each Dataset's committed Records, as `run.records` reads them.
pub(crate) const RECORDS: RunTable = RunTable {
    name: "records",
    columns: &[
        RUN,
        column("dataset", ColumnType::Text, "dataset"),
        column("number", ColumnType::Integer, "number"),
        column("value", ColumnType::Json, "data"),
        column("request_id", ColumnType::Text, "work_item"),
        column("url", ColumnType::Text, "url"),
        column("committed_at", ColumnType::Text, "committed_at"),
        column("provenance", ColumnType::Json, "provenance"),
    ],
};

/// Each Dataset's rejected Records, as `run.rejected_records` reads them.
pub(crate) const REJECTED_RECORDS: RunTable = RunTable {
    name: "rejected_records",
    columns: &[
        RUN,
        column("dataset", ColumnType::Text, "dataset"),
        column("number", ColumnType::Integer, "number"),
        column("data", ColumnType::Json, "data"),
        column("request_id", ColumnType::Text, "work_item"),
        column("rejection", ColumnType::Json, "rejection"),
    ],
};

/// The Datasets a crawl declares.
pub(crate) const DATASETS: RunTable = RunTable {
    name: "datasets",
    columns: &[
        RUN,
        column("name", ColumnType::Text, "name"),
        column("position", ColumnType::Integer, "position"),
        column("schema", ColumnType::Json, "schema"),
        column("key", ColumnType::Json, "key"),
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
}

/// Every run table a query reads, each with its place in the run file.
pub const RUN_FILE_TABLES: [RunFileTable; 3] = [RECORDS_TABLE, REJECTED_TABLE, DATASETS_TABLE];

/// The committed Records' place in the run file.
pub(crate) const RECORDS_TABLE: RunFileTable = RunFileTable {
    run_table: RECORDS,
    name: "records",
    key: &["dataset", "number"],
};

/// Each Dataset's rejection table's place in the run file.
pub(crate) const REJECTED_TABLE: RunFileTable = RunFileTable {
    run_table: REJECTED_RECORDS,
    name: "rejected_records",
    key: &["dataset", "number"],
};

/// The declared Datasets' place in the run file.
pub(crate) const DATASETS_TABLE: RunFileTable = RunFileTable {
    run_table: DATASETS,
    name: "datasets",
    key: &["name"],
};

impl RunFileTable {
    /// The run file's column the run table's column at `index` reads;
    /// `None` for `run`, and past its columns.
    pub fn column(&self, index: usize) -> Option<&'static str> {
        self.run_table
            .columns
            .get(index)
            .and_then(|column| column.source)
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
