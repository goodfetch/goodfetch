//! Where each run table the SQL tab reads (ADR 0057) is in the run file:
//! the Datasets' own tables ([`crate::TABLES`]), described by this crate,
//! which creates them: each run table's table, its key and the column each
//! of its columns reads, and whether a run file holds them at all, which a
//! Run no crawl wrote does not. A change of those tables changes this
//! description with them, so a reader of the run file that goes through it
//! cannot drift from them.

use goodfetch_protocol::foxhound::sql_tab::{DATASETS, RECORDS, REJECTED_RECORDS, RunTable};
use rusqlite::Connection;

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

/// Each run table's place in the run file, in the order of
/// `goodfetch_protocol::foxhound::sql_tab::RUN_TABLES`.
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
