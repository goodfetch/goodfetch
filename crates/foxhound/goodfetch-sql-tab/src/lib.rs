//! Foxhound's SQL tab (ADRs 0031 and 0057; spec #593): `dataset.query`,
//! one read-only SQL statement over the runs a person selects, at most
//! 100. Each run's tables are DuckDB table functions reading its run file
//! read-only through `rusqlite` (`tables`); the connection has external
//! access, extension autoloading and installing off and its configuration
//! locked as it opens, and runs only one `SELECT` or `WITH` under the
//! memory and row limits (`sandbox`); the query runs under
//! `timeout.dataset_query`, past which it is interrupted; and DuckDB's
//! failures are translated in one place (`errors`, ADR 0127). Its own
//! crate under `crates/foxhound/`: Collie must not link DuckDB, which is
//! heavy and rarely changes (STYLE.md, reasons 2 and 3).

mod errors;
mod sandbox;
mod tables;

use std::sync::Arc;

use goodfetch_dataset::open_run;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::run::settings_error;
use goodfetch_engine::settings::{Opening, Resolved, catalog_value, value_as};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::foxhound::sql_tab::{
    DATASET_QUERY, DATASET_QUERY_MAX_ROWS, DATASET_QUERY_MEMORY_LIMIT, DatasetQueryParams,
    DatasetQueryResult, MAX_RUNS, QUERY_ANSWER, QUERY_FAILED, QUERY_INVALID, QUERY_MEMORY_EXCEEDED,
    QueryInvalidDetails, QueryMemoryExceededDetails, TIMEOUT_DATASET_QUERY,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use serde_json::Map;

pub use errors::Failure;
pub use sandbox::{Answer, Limits, Sandbox, Stop};
pub use tables::OpenRun;

/// What the SQL tab adds to Foxhound's engine: `dataset.query`.
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-sql-tab").with_handler(Handler::new(&DATASET_QUERY, query))
}

/// Interrupts the query when the call stops waiting for it, at its
/// deadline or because it was cancelled, so DuckDB stops too.
struct StopOnDrop(Stop);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.stop();
    }
}

async fn query(
    params: DatasetQueryParams,
    context: CallContext,
) -> Result<DatasetQueryResult, CallError> {
    let mut runs = params.runs;
    runs.sort();
    runs.dedup();
    if runs.is_empty() || runs.len() > MAX_RUNS {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!(
                "`runs` selects {} runs: select from 1 to {MAX_RUNS}",
                runs.len()
            ),
        ));
    }
    let (settings, _) = context
        .engine()
        .settings()
        .open(Opening::default())
        .await
        .map_err(|error| settings_error(&error, &context))?;
    let resolved = settings.resolved();
    let mut selected = Vec::with_capacity(runs.len());
    let mut first = None;
    for run_id in runs {
        let reader = open_run(&context, run_id.clone()).await?;
        let opener = reader.opener();
        let open: OpenRun = Arc::new(move || opener.open());
        selected.push((run_id.as_str().to_owned(), open));
        first.get_or_insert(reader);
    }
    let reader = first.expect("invariant: a query selects at least one run");
    let memory = limit_of(resolved, &DATASET_QUERY_MEMORY_LIMIT);
    let limits = Limits {
        memory: value_as(&DATASET_QUERY_MEMORY_LIMIT, &memory.value),
        rows: resolved.get(&DATASET_QUERY_MAX_ROWS),
    };
    let sandbox = Sandbox::open(selected, limits).map_err(|failure| answer(failure, &memory))?;
    let stop = StopOnDrop(sandbox.stopper());
    let deadline = Deadline::of(resolved, &TIMEOUT_DATASET_QUERY)
        .unwrap_or_else(|| Deadline::declared(&TIMEOUT_DATASET_QUERY));
    let sql = params.sql;
    let limit = params.limit;
    let answered = deadline
        .within(
            &context.engine().clock(),
            QUERY_ANSWER,
            reader.blocking(move || sandbox.query(&sql, limit)),
        )
        .await?;
    drop(stop);
    let Answer {
        columns,
        rows,
        total,
    } = answered.map_err(|failure| answer(failure, &memory))?;
    Ok(DatasetQueryResult {
        columns,
        rows,
        total,
    })
}

/// The setting `setting`, its value and its layer, as `resolved` holds it.
fn limit_of(resolved: &Resolved, setting: &goodfetch_protocol::catalog::Setting) -> SettingLimit {
    match resolved.effective(setting) {
        Some(effective) => SettingLimit {
            key: setting.name.to_owned(),
            value: effective.value.clone(),
            layer: effective.layer,
        },
        None => SettingLimit {
            key: setting.name.to_owned(),
            value: catalog_value(setting).clone(),
            layer: SettingsLayer::Default,
        },
    }
}

/// The Error code a query's failure answers with.
fn answer(failure: Failure, memory: &SettingLimit) -> CallError {
    match failure {
        Failure::Invalid {
            reason,
            explanation,
        } => CallError::new(
            &QUERY_INVALID,
            format!("The SQL tab did not run the statement: {explanation}"),
            &QueryInvalidDetails { reason },
        ),
        Failure::MemoryExceeded => CallError::new(
            &QUERY_MEMORY_EXCEEDED,
            format!(
                "The query needed more than `{}` ({} bytes); aggregate or filter sooner, select fewer runs, or raise it.",
                memory.key, memory.value
            ),
            &QueryMemoryExceededDetails {
                limit: memory.clone(),
            },
        ),
        Failure::Interrupted | Failure::Unrecognized => CallError::new(
            &QUERY_FAILED,
            "The query failed in a way the engine does not recognise; the engine's log holds the cause.",
            &Map::new(),
        ),
    }
}
