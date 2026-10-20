//! The `run.*` operations every Product carries (ADRs 0029, 0036 and 0102):
//! `run.start` begins a Run its program drives, or returns the one its Run
//! ID names; `run.finish` answers a Run's report once it ended; `run.abort`
//! stops it. Also the one translation of a start's or a Run's failure into
//! its Error code, which a Product's own start, such as Foxhound's
//! `crawl.start`, answers with too.

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::StoppedBy;
use goodfetch_protocol::run::{
    RUN_ABORT, RUN_CONFLICT, RUN_FINISH, RUN_ID_INVALID, RUN_ID_PATTERN, RUN_IN_USE, RUN_NOT_FOUND,
    RUN_START, RUN_STATE_UNAVAILABLE, RunConflictDetails, RunDetails, RunId, RunIdInvalidDetails,
    RunKind, RunParams, RunReport, RunStartParams, RunStarted, StartedAs,
};

use crate::composition::{CallContext, CallError, Handler};
use crate::runs::{EndedBy, RunError, Start, StartError, Started};

/// The `run.*` handlers.
pub(crate) fn handlers() -> [Handler; 3] {
    [
        Handler::new(&RUN_START, start),
        Handler::new(&RUN_FINISH, finish),
        Handler::new(&RUN_ABORT, abort),
    ]
}

/// Reads a start's Run ID, refusing one that is not with `run_id_invalid`.
///
/// # Errors
///
/// `run_id_invalid` when `run_id` is no Run ID.
pub fn run_id(run_id: Option<&str>) -> Result<Option<RunId>, CallError> {
    run_id
        .map(|text| {
            RunId::parse(text).map_err(|reason| {
                CallError::new(
                    &RUN_ID_INVALID,
                    format!("`{text}` is not a Run ID: {reason}."),
                    &RunIdInvalidDetails {
                        pattern: RUN_ID_PATTERN.to_owned(),
                    },
                )
            })
        })
        .transpose()
}

/// The Error code a start's failure answers with.
pub fn start_error(error: StartError) -> CallError {
    match error {
        StartError::Conflict {
            run_id,
            differences,
        } => CallError::new(
            &RUN_CONFLICT,
            format!(
                "The Run ID `{run_id}` names a Run started with other inputs; start under another Run ID."
            ),
            &RunConflictDetails {
                run_id,
                differences,
            },
        ),
        StartError::InUse { run_id, folder } => CallError::new(
            &RUN_IN_USE,
            format!("Another engine writes the Run `{run_id}`."),
            &RunDetails {
                run_id,
                path: Some(folder.display().to_string()),
            },
        ),
        StartError::Unavailable {
            run_id,
            folder,
            error,
        } => CallError::new(
            &RUN_STATE_UNAVAILABLE,
            format!("The state of the Run `{run_id}` is unavailable: {error}"),
            &RunDetails {
                run_id,
                path: Some(folder.display().to_string()),
            },
        ),
        StartError::Stopped => stopped(),
    }
}

fn stopped() -> CallError {
    CallError::protocol(
        goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
        "the engine's Runs have stopped",
    )
}

fn run_error(error: RunError) -> CallError {
    match error {
        RunError::NotFound(run_id) => CallError::new(
            &RUN_NOT_FOUND,
            format!("No Run has the Run ID `{run_id}`."),
            &RunDetails { run_id, path: None },
        ),
        RunError::Stopped => stopped(),
    }
}

async fn start(params: RunStartParams, context: CallContext) -> Result<RunStarted, CallError> {
    let run_id = run_id(params.run_id.as_deref())?;
    let started = context
        .runs()
        .start(Start {
            run_id,
            tags: params.tags,
            kind: RunKind::Collie,
            needs: Product::Collie,
            inputs: BTreeMap::new(),
            ended_by: EndedBy::Finish,
            initialize: None,
        })
        .await
        .map_err(start_error)?;
    Ok(match started {
        Started::New(run) => RunStarted {
            run_id: run.id().clone(),
            started_as: StartedAs::New,
            lifecycle: goodfetch_protocol::run::Lifecycle::Running,
            needs: Product::Collie,
        },
        Started::Existing(existing) => existing,
    })
}

async fn finish(params: RunParams, context: CallContext) -> Result<RunReport, CallError> {
    context
        .runs()
        .finish(params.run_id)
        .await
        .map_err(run_error)
}

async fn abort(params: RunParams, context: CallContext) -> Result<RunReport, CallError> {
    context
        .runs()
        .abort(params.run_id, StoppedBy::Client)
        .await
        .map_err(run_error)
}
