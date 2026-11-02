//! One table function per run table (ADR 0057): each takes a selected
//! run's Run ID and reads that run's file read-only through `rusqlite`,
//! in one read transaction, a page of rows by key at a time, only the
//! columns the query reads (projection pushdown), on as many of DuckDB's
//! threads as it gives the scan. The columns are those
//! `goodfetch-protocol` declares for the table, so no other field of the
//! run file, and no secret one, is ever a column. A Run ID the query did
//! not select is refused at bind, and the refusal kept for the caller.
#![expect(
    unsafe_code,
    reason = "DuckDB's table functions hand their extra info as a raw pointer and their BIGINT columns as raw storage (STYLE.md, `unsafe`)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "DuckDB calls the table functions on its own threads, outside the engine's tasks, so their state is guarded by std's Mutex, which no loom model reaches (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{Arc, Mutex, PoisonError};

use duckdb::Connection as Duckdb;
use duckdb::core::{DataChunkHandle, Inserter as _, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab};
use goodfetch_protocol::foxhound::sql_tab::{ColumnType, RUN_TABLES, RunTable};
use rusqlite::Connection;
use rusqlite::types::ValueRef;

/// What opens one run's file read-only; `None` when it has none.
pub type OpenRun = Arc<dyn Fn() -> rusqlite::Result<Option<Connection>> + Send + Sync>;

/// The rows one read of a run file takes: one DuckDB vector.
const PAGE: usize = 2048;

/// Why a table function refused a query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// It named a run the query did not select.
    RunNotSelected,
}

/// The runs a query selected, by Run ID, and what a table function refused.
pub(crate) struct Registered {
    runs: BTreeMap<String, OpenRun>,
    refusal: Mutex<Option<Refusal>>,
}

impl Registered {
    /// The runs `runs` selects, by Run ID.
    pub(crate) fn new(runs: impl IntoIterator<Item = (String, OpenRun)>) -> Arc<Self> {
        Arc::new(Self {
            runs: runs.into_iter().collect(),
            refusal: Mutex::new(None),
        })
    }

    /// What a table function refused in the last query, taken.
    pub(crate) fn refusal(&self) -> Option<Refusal> {
        self.refusal
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    fn refuse(&self, refusal: Refusal) {
        *self.refusal.lock().unwrap_or_else(PoisonError::into_inner) = Some(refusal);
    }
}

/// Registers every run table's function on `duckdb`, each reading the
/// runs of `registered`.
pub(crate) fn register(duckdb: &Duckdb, registered: &Arc<Registered>) -> duckdb::Result<()> {
    for table in RUN_TABLES {
        duckdb.register_table_function_with_extra_info::<RunTableFunction, _>(
            table.name,
            &Extra {
                table,
                registered: Arc::clone(registered),
            },
        )?;
    }
    Ok(())
}

/// Where a run table's columns come from in the run file.
struct Source {
    /// The run file's table.
    table: &'static str,
    /// Its key, in order, by which pages are read.
    key: &'static [&'static str],
}

/// The run file's table and column of the run table `table`'s column
/// `column`; `None` for `run`, the Run ID, which no row holds.
fn source(table: &RunTable) -> Source {
    match table.name {
        "datasets" => Source {
            table: "datasets",
            key: &["name"],
        },
        "rejected_records" => Source {
            table: "rejected_records",
            key: &["dataset", "number"],
        },
        _ => Source {
            table: "records",
            key: &["dataset", "number"],
        },
    }
}

/// The run file's column a run table's column reads; `None` for `run`.
fn source_column(column: &str) -> Option<&str> {
    match column {
        "run" => None,
        "request_id" => Some("work_item"),
        "value" => Some("data"),
        other => Some(other),
    }
}

#[derive(Clone)]
struct Extra {
    table: RunTable,
    registered: Arc<Registered>,
}

/// A run table's function: one per table, told apart by its extra info.
struct RunTableFunction;

struct Bound {
    table: RunTable,
    run: String,
    open: OpenRun,
}

struct Scan {
    /// The run table's columns the query reads, in the output's order.
    columns: Vec<Option<usize>>,
    /// The connection and the key of the last row read, once opened.
    cursor: Mutex<Cursor>,
}

enum Cursor {
    Unopened,
    Reading {
        // Boxed: `rusqlite`'s `hooks` feature, which the ledger's commit hook
        // turns on, makes a connection most of the enum.
        connection: Box<Connection>,
        after: Option<Vec<rusqlite::types::Value>>,
    },
    Done,
}

impl VTab for RunTableFunction {
    type InitData = Scan;
    type BindData = Bound;

    fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn Error>> {
        // SAFETY: `register` gave every one of these functions an `Extra`
        // as its extra info, which DuckDB's catalog keeps while the
        // function is registered, so for every bind of it.
        let extra = unsafe { &*bind.get_extra_info::<Extra>() };
        let run = bind.get_parameter(0).to_string();
        let Some(open) = extra.registered.runs.get(&run) else {
            extra.registered.refuse(Refusal::RunNotSelected);
            return Err(format!("the run {run} is not one this query selected").into());
        };
        for column in extra.table.columns {
            let id = match column.column_type {
                ColumnType::Integer => LogicalTypeId::Bigint,
                ColumnType::Text | ColumnType::Json => LogicalTypeId::Varchar,
            };
            bind.add_result_column(column.name, LogicalTypeHandle::from(id));
        }
        Ok(Bound {
            table: extra.table,
            run,
            open: Arc::clone(open),
        })
    }

    fn init(init: &InitInfo) -> Result<Self::InitData, Box<dyn Error>> {
        let columns = init
            .get_column_indices()
            .into_iter()
            .map(|index| usize::try_from(index).ok())
            .collect();
        let threads = std::thread::available_parallelism().map_or(1, usize::from);
        init.set_max_threads(u64::try_from(threads).unwrap_or(1));
        Ok(Scan {
            columns,
            cursor: Mutex::new(Cursor::Unopened),
        })
    }

    fn func(
        function: &TableFunctionInfo<Self>,
        output: &mut DataChunkHandle,
    ) -> Result<(), Box<dyn Error>> {
        let bound = function.get_bind_data();
        let scan = function.get_init_data();
        let mut cursor = scan.cursor.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(*cursor, Cursor::Unopened) {
            *cursor = match (bound.open)()? {
                Some(connection) if holds(&connection, &bound.table)? => {
                    connection.execute_batch("BEGIN")?;
                    Cursor::Reading {
                        connection: Box::new(connection),
                        after: None,
                    }
                }
                _ => Cursor::Done,
            };
        }
        let Cursor::Reading { connection, after } = &mut *cursor else {
            output.set_len(0);
            return Ok(());
        };
        let rows = page(connection, bound, &scan.columns, after.as_deref())?;
        if rows.len() < PAGE {
            write(output, bound, &scan.columns, &rows);
            *cursor = Cursor::Done;
            return Ok(());
        }
        *after = rows.last().map(|row| row.key.clone());
        write(output, bound, &scan.columns, &rows);
        Ok(())
    }

    fn supports_pushdown() -> bool {
        true
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }
}

/// Whether the run file holds the run table's source: a Run no crawl
/// wrote holds no Dataset's table, and reads as none.
fn holds(connection: &Connection, table: &RunTable) -> rusqlite::Result<bool> {
    connection
        .prepare("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1")?
        .exists([source(table).table])
}

/// One row read: its key, and its values by the run table's column.
struct Row {
    key: Vec<rusqlite::types::Value>,
    values: Vec<rusqlite::types::Value>,
}

/// The next page of rows after the key `after`, in key order.
fn page(
    connection: &Connection,
    bound: &Bound,
    columns: &[Option<usize>],
    after: Option<&[rusqlite::types::Value]>,
) -> rusqlite::Result<Vec<Row>> {
    let source = source(&bound.table);
    let selected: Vec<&str> = columns
        .iter()
        .map(|column| {
            column
                .and_then(|index| bound.table.columns.get(index))
                .and_then(|column| source_column(column.name))
                .unwrap_or("NULL")
        })
        .collect();
    let key = source.key.join(", ");
    let placeholders = vec!["?"; source.key.len()].join(", ");
    let filter = if after.is_some() {
        format!("WHERE ({key}) > ({placeholders})")
    } else {
        String::new()
    };
    let sql = format!(
        "SELECT {key}, {} FROM {} {filter} ORDER BY {key} LIMIT {PAGE}",
        if selected.is_empty() {
            "NULL".to_owned()
        } else {
            selected.join(", ")
        },
        source.table,
    );
    let mut statement = connection.prepare(&sql)?;
    let width = source.key.len();
    let read = |row: &rusqlite::Row<'_>| -> rusqlite::Result<Row> {
        let key = (0..width)
            .map(|index| row.get::<_, rusqlite::types::Value>(index))
            .collect::<rusqlite::Result<_>>()?;
        let values = (0..selected.len())
            .map(|index| row.get::<_, rusqlite::types::Value>(width + index))
            .collect::<rusqlite::Result<_>>()?;
        Ok(Row { key, values })
    };
    match after {
        Some(after) => statement
            .query_map(rusqlite::params_from_iter(after), read)?
            .collect(),
        None => statement.query_map([], read)?.collect(),
    }
}

/// Writes `rows` into `output`, the projected columns in order.
fn write(output: &mut DataChunkHandle, bound: &Bound, columns: &[Option<usize>], rows: &[Row]) {
    for (place, column) in columns.iter().enumerate() {
        let declared = column.and_then(|index| bound.table.columns.get(index));
        let mut vector = output.flat_vector(place);
        match declared {
            Some(declared) if declared.name == "run" => {
                for row in 0..rows.len() {
                    vector.insert(row, bound.run.as_str());
                }
            }
            Some(declared) if declared.column_type == ColumnType::Integer => {
                // SAFETY: the column was bound as BIGINT, whose vector
                // stores one `i64` per row for its capacity, a DuckDB
                // vector's size, which a page never exceeds; nothing else
                // borrows it while this function fills it.
                let slots = unsafe { vector.as_mut_slice::<i64>() };
                let mut nulls = Vec::new();
                for (index, row) in rows.iter().enumerate() {
                    match ValueRef::from(&row.values[place]) {
                        ValueRef::Integer(value) => slots[index] = value,
                        _ => nulls.push(index),
                    }
                }
                for index in nulls {
                    vector.set_null(index);
                }
            }
            Some(_) => {
                for (index, row) in rows.iter().enumerate() {
                    match ValueRef::from(&row.values[place]) {
                        ValueRef::Text(text) => vector.insert(index, text),
                        ValueRef::Integer(value) => {
                            vector.insert(index, value.to_string().as_str());
                        }
                        _ => vector.set_null(index),
                    }
                }
            }
            None => {
                for index in 0..rows.len() {
                    vector.set_null(index);
                }
            }
        }
    }
    output.set_len(rows.len());
}
