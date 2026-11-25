//! Foxhound's SQL tab (ADRs 0031 and 0057; spec #593): `dataset.query`,
//! one read-only SQL statement over the runs a person selects, at most
//! 100. Each run's tables are DuckDB table functions reading its run file
//! read-only through `rusqlite` (`tables`); the connection has external
//! access, extension autoloading and installing off and its configuration
//! locked as it opens, and runs only one `SELECT` or `WITH` under the
//! memory and row limits (`sandbox`); the query runs under
//! `timeout.dataset_query`, past which it is interrupted; and DuckDB's
//! failures are translated in one place (`errors`, ADR 0127). The full-text
//! search of a run's Records, `dataset.search`, and of the Records of the
//! runs a person selects, `record.search`, read the FTS5 index each Record's
//! commit fills; the latter runs on DuckDB under the same limits
//! (`search`). Its own
//! crate under `crates/foxhound/`: Collie must not link DuckDB, which is
//! heavy and rarely changes (STYLE.md, reasons 2 and 3).

mod errors;
mod sandbox;
mod search;
mod tables;

use std::sync::Arc;

use goodfetch_dataset::{open_dataset, open_run};
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::run::{run_error, settings_error, through_owner};
use goodfetch_engine::runs::RunReader;
use goodfetch_engine::settings::{Opening, Resolved, catalog_value, value_as};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::foxhound::search::{
    DATASET_SEARCH, DatasetSearchPage, DatasetSearchParams, RECORD_SEARCH, RecordSearchParams,
    RecordSearchResult, SEARCH_DEFAULT, SEARCH_MOST,
};
use goodfetch_protocol::foxhound::sql_tab::{
    DATASET_QUERY, DATASET_QUERY_MAX_ROWS, DATASET_QUERY_MEMORY_LIMIT, DatasetQueryParams,
    DatasetQueryResult, MAX_RUNS, QUERY_ANSWER, QUERY_FAILED, QUERY_INVALID, QUERY_MEMORY_EXCEEDED,
    QueryInvalidDetails, QueryMemoryExceededDetails, TIMEOUT_DATASET_QUERY,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use serde_json::Map;

pub use errors::Failure;
pub use sandbox::{Answer, Limits, Sandbox, Stop};
pub use search::Matching;
pub use tables::OpenRun;

/// What the SQL tab adds to Foxhound's engine: `dataset.query`,
/// `dataset.search` and `record.search`.
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-sql-tab")
        .with_handler(Handler::new(&DATASET_QUERY, query))
        .with_handler(Handler::new(&DATASET_SEARCH, dataset_search))
        .with_handler(Handler::new(&RECORD_SEARCH, record_search))
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
    let Selected {
        sandbox,
        reader,
        memory,
        deadline,
    } = selected(params.runs, &context).await?;
    let stop = StopOnDrop(sandbox.stopper());
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

async fn dataset_search(
    params: DatasetSearchParams,
    context: CallContext,
) -> Result<DatasetSearchPage, CallError> {
    if let Some(answer) = through_owner(&context, &DATASET_SEARCH, &params.run_id, &params).await? {
        return Ok(answer);
    }
    let expression = search::expression(&params.text).ok_or_else(no_word)?;
    let limit = search_limit(params.limit)?;
    let mut reader = open_dataset(&context, params.run_id.clone(), &params.dataset).await?;
    let (dataset, after) = (params.dataset, params.after.unwrap_or(0));
    let records = reader
        .read(move |connection| search::page(connection, &dataset, &expression, after, limit))
        .await
        .map_err(|error| run_error(error, &context))?;
    let full = records.len() == usize::try_from(limit).unwrap_or(usize::MAX);
    let next = records.last().filter(|_| full).map(|record| record.number);
    Ok(DatasetSearchPage { records, next })
}

async fn record_search(
    params: RecordSearchParams,
    context: CallContext,
) -> Result<RecordSearchResult, CallError> {
    let limit = search_limit(params.limit)?;
    let matching = Matching::of(&params.text, params.datasets, limit).ok_or_else(no_word)?;
    let Selected {
        sandbox,
        reader,
        memory,
        deadline,
    } = selected(params.runs, &context).await?;
    let stop = StopOnDrop(sandbox.stopper());
    let found = deadline
        .within(
            &context.engine().clock(),
            QUERY_ANSWER,
            reader.blocking(move || sandbox.search(matching)),
        )
        .await?;
    drop(stop);
    let records = found.map_err(|failure| answer(failure, &memory))?;
    Ok(RecordSearchResult { records })
}

/// `invalid_params` for a search whose `text` holds no word.
fn no_word() -> CallError {
    CallError::protocol(
        ProtocolFailure::InvalidParams,
        "`text` holds no word: search for at least one".to_owned(),
    )
}

/// The most Records a search answers, `limit` or its default, else
/// `invalid_params`.
fn search_limit(limit: Option<u32>) -> Result<u32, CallError> {
    let limit = limit.unwrap_or(SEARCH_DEFAULT);
    if !(1..=SEARCH_MOST).contains(&limit) {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`limit` is {limit}: a search answers from 1 to {SEARCH_MOST} Records"),
        ));
    }
    Ok(limit)
}

/// The runs a call selected, in a locked connection, with a reader to run
/// its query on and what the query runs under.
struct Selected {
    sandbox: Sandbox,
    reader: RunReader,
    memory: SettingLimit,
    deadline: Deadline,
}

/// Opens the runs `runs` selects, from 1 to 100, each once, in a locked
/// connection under the engine's `dataset.query` limits.
async fn selected(mut runs: Vec<RunId>, context: &CallContext) -> Result<Selected, CallError> {
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
        .map_err(|error| settings_error(&error, context))?;
    let resolved = settings.resolved();
    let mut selected = Vec::with_capacity(runs.len());
    let mut first = None;
    for run_id in runs {
        let reader = open_run(context, run_id.clone()).await?;
        let opener = reader.opener();
        let open: OpenRun = Arc::new(move || opener.open());
        selected.push((run_id.as_str().to_owned(), open));
        first.get_or_insert(reader);
    }
    let reader = first.expect("invariant: a call selects at least one run");
    let memory = limit_of(resolved, &DATASET_QUERY_MEMORY_LIMIT);
    let limits = Limits {
        memory: value_as(&DATASET_QUERY_MEMORY_LIMIT, &memory.value),
        rows: resolved.get(&DATASET_QUERY_MAX_ROWS),
    };
    let sandbox = Sandbox::open(selected, limits).map_err(|failure| answer(failure, &memory))?;
    let deadline = Deadline::of(resolved, &TIMEOUT_DATASET_QUERY)
        .unwrap_or_else(|| Deadline::declared(&TIMEOUT_DATASET_QUERY));
    Ok(Selected {
        sandbox,
        reader,
        memory,
        deadline,
    })
}

/// The setting `setting`, its value and its layer, as `resolved` holds it.
fn limit_of(resolved: &Resolved, setting: &goodfetch_protocol::catalog::Setting) -> SettingLimit {
    match resolved.effective(setting) {
        Some(effective) => SettingLimit {
            key: setting.name.to_owned(),
            value: effective.value.clone(),
            layer: effective.layer,
            origin: effective.origin,
        },
        None => SettingLimit {
            key: setting.name.to_owned(),
            value: catalog_value(setting).clone(),
            layer: SettingsLayer::Default,
            origin: None,
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
