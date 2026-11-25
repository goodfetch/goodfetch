//! Full-text search of a crawl's Records (ADR 0031; spec #593, stories 208
//! and 209), over the FTS5 index, `record_search`, that `goodfetch-dataset`
//! fills in each Record's commit while `dataset.search.index` is on. A
//! search's text is its words, each quoted, all of which a Record holds
//! (`expression`). `dataset.search` reads one Dataset of one run, a page
//! by number (`page`); `record.search` runs on DuckDB over the selected
//! runs' indexes, a table function per run reading its best matches from
//! its run file read-only, and keeps the best of all (`Sandbox::search`).
//! A run file
//! that predates the index, or has no Datasets, matches nothing.
#![expect(
    unsafe_code,
    reason = "DuckDB's table functions hand their extra info as a raw pointer and their BIGINT and DOUBLE columns as raw storage (STYLE.md, `unsafe`)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "DuckDB calls the table function on its own threads, outside the engine's tasks, so its state is guarded by std's Mutex, which no loom model reaches (ADR 0060)"
)]

use std::error::Error;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};

use duckdb::core::{DataChunkHandle, Inserter as _, LogicalTypeHandle, LogicalTypeId};
use duckdb::vtab::{BindInfo, InitInfo, TableFunctionInfo, VTab};
use goodfetch_dataset::{RECORD_COLUMNS, dataset_record};
use goodfetch_protocol::foxhound::dataset::DatasetRecord;
use goodfetch_protocol::foxhound::search::FoundRecord;
use goodfetch_protocol::run::RunId;
use rusqlite::Connection;
use serde_json::Value;

use crate::errors::Failure;
use crate::sandbox::Sandbox;
use crate::tables::{OpenRun, Registered};

/// The FTS5 expression that matches a Record holding every word of `text`,
/// each quoted so no word is read as FTS5's syntax; none when `text` holds
/// no word.
pub(crate) fn expression(text: &str) -> Option<String> {
    let mut expression = String::new();
    for word in text.split_whitespace() {
        if !expression.is_empty() {
            expression.push(' ');
        }
        expression.push('"');
        expression.push_str(&word.replace('"', "\"\""));
        expression.push('"');
    }
    (!expression.is_empty()).then_some(expression)
}

/// One page of `dataset.search`: the Records of `dataset` numbered after
/// `after` that `expression` matches, in number order, at most `limit`.
pub(crate) fn page(
    connection: &Connection,
    dataset: &str,
    expression: &str,
    after: u64,
    limit: u32,
) -> rusqlite::Result<Vec<DatasetRecord>> {
    if !indexed(connection)? {
        return Ok(Vec::new());
    }
    let after = i64::try_from(after).unwrap_or(i64::MAX);
    connection
        .prepare_cached(&format!(
            "SELECT {RECORD_COLUMNS} FROM records
             WHERE dataset = ?1 AND number > ?3 AND number IN (
                 SELECT number FROM record_search
                 WHERE record_search MATCH ?2 AND dataset = ?1 AND number > ?3
             )
             ORDER BY number LIMIT ?4"
        ))?
        .query_map((dataset, expression, after, limit), dataset_record)?
        .collect()
}

/// Whether the run file `connection` holds has the index.
fn indexed(connection: &Connection) -> rusqlite::Result<bool> {
    connection
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE name = 'record_search')")?
        .query_row([], |row| row.get(0))
}

/// What `record.search` looks for in each run: the Records holding every
/// word of its text, in the Datasets it names (every one when none), the
/// best `limit` of each run.
#[derive(Clone, Debug)]
pub struct Matching {
    expression: String,
    datasets: Option<Vec<String>>,
    limit: u32,
}

impl Matching {
    /// The Records holding every word of `text`, in `datasets` (every one
    /// when none), at most `limit`; none when `text` holds no word.
    pub fn of(text: &str, datasets: Option<Vec<String>>, limit: u32) -> Option<Self> {
        Some(Self {
            expression: expression(text)?,
            datasets,
            limit,
        })
    }
}

/// The name of the table function [`Sandbox::search`] reads each run
/// through.
const MATCHES: &str = "record_matches";

impl Sandbox {
    /// The Records `matching` finds in the runs this connection selected,
    /// the best matches first, then by run, Dataset and number, at most its
    /// `limit`: each run's best, read from its index by a table function,
    /// and the best of all kept by DuckDB, under the connection's limits.
    /// It takes the connection, which registers the function for this one
    /// search: DuckDB keeps a table function's first registration.
    pub fn search(self, matching: Matching) -> Result<Vec<FoundRecord>, Failure> {
        let limit = matching.limit;
        let registered = Arc::clone(self.registered());
        let mut runs = String::new();
        for run in registered.runs() {
            if !runs.is_empty() {
                runs.push_str("\nUNION ALL ");
            }
            // A Run ID is quoted as SQL text, its `'` doubled.
            let _ = write!(
                runs,
                "SELECT * FROM {MATCHES}('{}')",
                run.replace('\'', "''")
            );
        }
        self.connection()
            .register_table_function_with_extra_info::<MatchesFunction, _>(
                MATCHES,
                &Extra {
                    registered,
                    matching: Arc::new(matching),
                },
            )
            .map_err(|error| crate::errors::translate(&error))?;
        let answer = self.query(
            &format!(
                "SELECT run, dataset, number, value::JSON AS value FROM (\n{runs}\n)
                 ORDER BY score, run, dataset, number"
            ),
            Some(u64::from(limit)),
        )?;
        answer.rows.into_iter().map(found_record).collect()
    }
}

/// The Record one row of [`Sandbox::search`]'s query holds.
fn found_record(row: Vec<Value>) -> Result<FoundRecord, Failure> {
    let Ok([Value::String(run), Value::String(dataset), number, value]) =
        <[Value; 4]>::try_from(row)
    else {
        return Err(Failure::Unrecognized);
    };
    let (Ok(run_id), Some(number)) = (RunId::parse(&run), number.as_u64()) else {
        return Err(Failure::Unrecognized);
    };
    Ok(FoundRecord {
        run_id,
        dataset,
        number,
        value,
    })
}

#[derive(Clone)]
struct Extra {
    registered: Arc<Registered>,
    matching: Arc<Matching>,
}

/// `record_matches(run)`: the best matches of one selected run, with their
/// score, lower the better.
struct MatchesFunction;

struct Bound {
    run: String,
    open: OpenRun,
    matching: Arc<Matching>,
}

struct Scan {
    done: Mutex<bool>,
}

/// One run's match: its Dataset, its number, the Record's JSON text and
/// its score.
struct Match {
    dataset: String,
    number: i64,
    data: String,
    score: f64,
}

impl VTab for MatchesFunction {
    type InitData = Scan;
    type BindData = Bound;

    fn bind(bind: &BindInfo) -> Result<Self::BindData, Box<dyn Error>> {
        // SAFETY: `Sandbox::search` gave this function an `Extra` as its extra info,
        // which DuckDB's catalog keeps while the function is registered, so
        // for every bind of it.
        let extra = unsafe { &*bind.get_extra_info::<Extra>() };
        let run = bind.get_parameter(0).to_string();
        let Some(open) = extra.registered.run(&run) else {
            return Err(format!("the run {run} is not one this search selected").into());
        };
        for (name, id) in [
            ("run", LogicalTypeId::Varchar),
            ("dataset", LogicalTypeId::Varchar),
            ("number", LogicalTypeId::Bigint),
            ("value", LogicalTypeId::Varchar),
            ("score", LogicalTypeId::Double),
        ] {
            bind.add_result_column(name, LogicalTypeHandle::from(id));
        }
        Ok(Bound {
            run,
            open: Arc::clone(open),
            matching: Arc::clone(&extra.matching),
        })
    }

    fn init(init: &InitInfo) -> Result<Self::InitData, Box<dyn Error>> {
        init.set_max_threads(1);
        Ok(Scan {
            done: Mutex::new(false),
        })
    }

    fn func(
        function: &TableFunctionInfo<Self>,
        output: &mut DataChunkHandle,
    ) -> Result<(), Box<dyn Error>> {
        let bound = function.get_bind_data();
        let mut done = function
            .get_init_data()
            .done
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if *done {
            output.set_len(0);
            return Ok(());
        }
        *done = true;
        let matches = match (bound.open)()? {
            Some(connection) if indexed(&connection)? => best(&connection, &bound.matching)?,
            _ => Vec::new(),
        };
        write(output, &bound.run, &matches);
        Ok(())
    }

    fn parameters() -> Option<Vec<LogicalTypeHandle>> {
        Some(vec![LogicalTypeHandle::from(LogicalTypeId::Varchar)])
    }
}

/// The best matches of `matching` in the run file `connection` holds, at
/// most its `limit`, in one read.
fn best(connection: &Connection, matching: &Matching) -> rusqlite::Result<Vec<Match>> {
    let datasets = matching.datasets.as_ref().map(|datasets| {
        serde_json::to_string(datasets).expect("invariant: Dataset names encode as JSON")
    });
    connection
        .prepare(
            "SELECT record_search.dataset, record_search.number, records.data, bm25(record_search)
             FROM record_search JOIN records
                 ON records.dataset = record_search.dataset AND records.number = record_search.number
             WHERE record_search MATCH ?1
                 AND (?3 IS NULL OR record_search.dataset IN (SELECT value FROM json_each(?3)))
             ORDER BY 4, 1, 2 LIMIT ?2",
        )?
        .query_map((&matching.expression, matching.limit, datasets), |row| {
            Ok(Match {
                dataset: row.get(0)?,
                number: row.get(1)?,
                data: row.get(2)?,
                score: row.get(3)?,
            })
        })?
        .collect()
}

/// Writes `matches`, the run `run`'s, into `output`: at most a search's
/// most, 1000, under a DuckDB vector's size.
fn write(output: &mut DataChunkHandle, run: &str, matches: &[Match]) {
    let (runs, datasets, values) = (
        output.flat_vector(0),
        output.flat_vector(1),
        output.flat_vector(3),
    );
    for (index, found) in matches.iter().enumerate() {
        runs.insert(index, run);
        datasets.insert(index, found.dataset.as_str());
        values.insert(index, found.data.as_str());
    }
    let mut numbers = output.flat_vector(2);
    // SAFETY: the column was bound as BIGINT, whose vector stores one `i64`
    // per row for its capacity, a DuckDB vector's size, which a search's
    // most never exceeds; nothing else borrows it while this fills it.
    let slots = unsafe { numbers.as_mut_slice::<i64>() };
    for (index, found) in matches.iter().enumerate() {
        slots[index] = found.number;
    }
    let mut scores = output.flat_vector(4);
    // SAFETY: as above, the column bound as DOUBLE, one `f64` per row.
    let slots = unsafe { scores.as_mut_slice::<f64>() };
    for (index, found) in matches.iter().enumerate() {
        slots[index] = found.score;
    }
    output.set_len(matches.len());
}
