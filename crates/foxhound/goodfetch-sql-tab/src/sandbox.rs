//! The query connection (ADRs 0031 and 0057): an in-memory DuckDB whose
//! external access, extension autoloading and installing are off and whose
//! configuration is locked as it opens, so a query reaches only the run
//! tables registered for the runs it selected; its memory limit, with
//! nothing spilled to disk; and the one statement it runs, checked to be a
//! single `SELECT` or `WITH` before it runs, whose first rows it returns
//! and the rest it counts.
#![expect(
    clippy::disallowed_types,
    reason = "a query is stopped from another thread while DuckDB runs it on its own, outside the engine's tasks, through std's atomic, which no loom model reaches (ADR 0060)"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use duckdb::{Config, Connection, InterruptHandle};
use goodfetch_protocol::foxhound::sql_tab::{QueryColumn, QueryRefusal};
use serde_json::Value;

use crate::errors::{self, Failure};
use crate::tables::{self, OpenRun, Refusal, Registered};

/// The limits a query runs under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most memory it uses, in bytes.
    pub memory: u64,
    /// The most rows it returns.
    pub rows: u64,
}

/// A query's answer: its columns, its first rows, and how many it produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    /// Its columns, in order.
    pub columns: Vec<QueryColumn>,
    /// Its first rows, each a JSON value per column.
    pub rows: Vec<Vec<Value>>,
    /// How many rows it produced.
    pub total: u64,
}

/// The query connection over the runs a query selected.
pub struct Sandbox {
    connection: Connection,
    registered: Arc<Registered>,
    limits: Limits,
    stopped: Arc<AtomicBool>,
}

/// What stops the query a [`Sandbox`] runs, from another thread: the step
/// running now is interrupted, and no later step starts.
#[derive(Clone)]
pub struct Stop {
    stopped: Arc<AtomicBool>,
    interrupt: Arc<InterruptHandle>,
}

impl Stop {
    /// Stops the query.
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.interrupt.interrupt();
    }
}

impl Sandbox {
    /// A locked connection over `runs`, each a Run ID and what opens its
    /// run file, under `limits`.
    pub fn open(
        runs: impl IntoIterator<Item = (String, OpenRun)>,
        limits: Limits,
    ) -> Result<Self, Failure> {
        let connection = locked(limits).map_err(|error| errors::translate(&error))?;
        let registered = Registered::new(runs);
        tables::register(&connection, &registered).map_err(|error| errors::translate(&error))?;
        Ok(Self {
            connection,
            registered,
            limits,
            stopped: Arc::new(AtomicBool::new(false)),
        })
    }

    /// What stops the query running on it, from another thread.
    pub fn stopper(&self) -> Stop {
        Stop {
            stopped: Arc::clone(&self.stopped),
            interrupt: self.connection.interrupt_handle(),
        }
    }

    /// `Interrupted` once the query was stopped, so no step starts after.
    fn going(&self) -> Result<(), Failure> {
        if self.stopped.load(Ordering::SeqCst) {
            return Err(Failure::Interrupted);
        }
        Ok(())
    }

    /// Runs `sql`, one `SELECT` or `WITH` statement, and answers its
    /// columns, its first `limit` rows (at most the row limit) and how
    /// many it produced.
    pub fn query(&self, sql: &str, limit: Option<u64>) -> Result<Answer, Failure> {
        self.going()?;
        let serialized: String = self
            .connection
            .query_row("SELECT json_serialize_sql(?)", [sql], |row| row.get(0))
            .map_err(|error| self.failed(&error))?;
        errors::one_read(&serialized)?;
        let statement = sql
            .trim_end()
            .trim_end_matches(|character: char| character == ';' || character.is_whitespace());
        self.going()?;
        let columns = self.columns(statement)?;
        self.going()?;
        let most = limit.map_or(self.limits.rows, |limit| limit.min(self.limits.rows));
        let mut prepared = self
            .connection
            .prepare(&format!(
                "SELECT json_array(*COLUMNS(*))::VARCHAR FROM (\n{statement}\n)"
            ))
            .map_err(|error| self.failed(&error))?;
        let mut produced = prepared.query([]).map_err(|error| self.failed(&error))?;
        let mut rows = Vec::new();
        let mut total = 0_u64;
        while let Some(row) = produced.next().map_err(|error| self.failed(&error))? {
            self.going()?;
            total += 1;
            if total <= most {
                let text: String = row.get(0).map_err(|error| self.failed(&error))?;
                let Ok(Value::Array(values)) = serde_json::from_str(&text) else {
                    return Err(Failure::Unrecognized);
                };
                rows.push(values);
            }
        }
        Ok(Answer {
            columns,
            rows,
            total,
        })
    }

    /// The columns of `statement`, by name and DuckDB's type.
    fn columns(&self, statement: &str) -> Result<Vec<QueryColumn>, Failure> {
        let mut described = self
            .connection
            .prepare(&format!(
                "SELECT column_name, column_type FROM (DESCRIBE (\n{statement}\n))"
            ))
            .map_err(|error| self.failed(&error))?;
        described
            .query_map([], |row| {
                Ok(QueryColumn {
                    name: row.get(0)?,
                    column_type: row.get(1)?,
                })
            })
            .and_then(Iterator::collect)
            .map_err(|error| self.failed(&error))
    }

    /// What `error` means, a run table's refusal first.
    fn failed(&self, error: &duckdb::Error) -> Failure {
        match self.registered.refusal() {
            Some(Refusal::RunNotSelected) => Failure::Invalid {
                reason: QueryRefusal::RunNotSelected,
                explanation: error.to_string(),
            },
            None => errors::translate(error),
        }
    }
}

/// An in-memory DuckDB locked as it opens: no external access, no
/// extension autoloaded or installed, no community or unsigned extension,
/// nothing spilled to disk, `limits`' memory, its configuration locked.
fn locked(limits: Limits) -> duckdb::Result<Connection> {
    let config = Config::default()
        .enable_external_access(false)?
        .enable_autoload_extension(false)?
        .with("autoinstall_known_extensions", "false")?
        .with("allow_community_extensions", "false")?
        .with("allow_persistent_secrets", "false")?
        .with("max_temp_directory_size", "0B")?
        .max_memory(&format!("{}B", limits.memory))?
        .with("lock_configuration", "true")?;
    Connection::open_in_memory_with_flags(config)
}
