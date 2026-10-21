//! The `run.*` operations every Product carries (ADRs 0029, 0036 and 0102):
//! `run.start` begins a Run its program drives, or returns the one its Run
//! ID names; `run.finish` answers a Run's report once it ended; `run.abort`
//! stops it. Also the one translation of a start's, a Run's or a setting's
//! failure into its Error code, which a Product's own start, such as
//! Foxhound's `crawl.start`, answers with too.

use std::collections::BTreeMap;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::StoppedBy;
use goodfetch_protocol::run::{
    RUN_ABORT, RUN_BUSY, RUN_CONFLICT, RUN_FINISH, RUN_ID_INVALID, RUN_ID_PATTERN, RUN_IN_USE,
    RUN_NOT_FOUND, RUN_NOT_OWNED, RUN_START, RUN_STATE_UNAVAILABLE, RunAbortParams,
    RunConflictDetails, RunDetails, RunId, RunIdInvalidDetails, RunKind, RunParams, RunReport,
    RunStartParams, RunStarted, StartedAs,
};

use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::settings::{
    RunOrigin, SETTING_INVALID, SETTING_LOCKED, SETTING_NOT_APPLICABLE, SETTING_UNKNOWN,
    SettingInvalidDetails, SettingLockedDetails, SettingNotApplicableDetails,
    SettingUnknownDetails, TIMEOUT_RUN,
};

use crate::composition::{CallContext, CallError, Handler};
use crate::product::{Identity, Requested};
use crate::runs::{EndedBy, Forwarded, RunError, Start, StartError, Started};
use crate::settings::{Opening, SettingsError};

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

/// The failure a call answers with for a refused setting, on an engine
/// that runs as `identity`: the one translation of each refusal into its
/// Error code.
pub fn settings_error(error: &SettingsError, identity: &Identity) -> CallError {
    let message = error.to_string();
    match error {
        SettingsError::Unknown {
            key,
            layer,
            closest,
        } => CallError::new(
            &SETTING_UNKNOWN,
            message,
            &SettingUnknownDetails {
                key: key.clone(),
                layer: *layer,
                closest: closest.clone(),
            },
        ),
        SettingsError::Invalid { key, layer, reason } => CallError::new(
            &SETTING_INVALID,
            message,
            &SettingInvalidDetails {
                key: key.clone(),
                layer: *layer,
                reason: reason.clone(),
            },
        ),
        SettingsError::NotApplicable { key, layer, fix } => CallError::new(
            &SETTING_NOT_APPLICABLE,
            message,
            &SettingNotApplicableDetails {
                key: key.clone(),
                layer: *layer,
                fix: fix.clone(),
            },
        ),
        SettingsError::Locked {
            key,
            layer,
            limit,
            fix,
        } => CallError::new(
            &SETTING_LOCKED,
            message,
            &SettingLockedDetails {
                key: key.clone(),
                layer: *layer,
                limit: limit.clone(),
                fix: fix.clone(),
            },
        ),
        SettingsError::NotInProduct { key, needs } => {
            let (message, details) = identity.not_in_product(Requested::Name(key), *needs, false);
            CallError::new(&NOT_IN_PRODUCT, message, &details)
        }
    }
}

fn stopped() -> CallError {
    CallError::protocol(
        goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
        "the engine's Runs have stopped",
    )
}

fn run_error(error: RunError, identity: &Identity) -> CallError {
    match error {
        RunError::NotFound(run_id) => CallError::new(
            &RUN_NOT_FOUND,
            format!("No Run has the Run ID `{run_id}`."),
            &RunDetails { run_id, path: None },
        ),
        RunError::Busy { run_id, folder } => CallError::new(
            &RUN_BUSY,
            format!(
                "Another engine writes the Run `{run_id}`, and the owner the Run index names did not take the action; try again later."
            ),
            &RunDetails {
                run_id,
                path: Some(folder.display().to_string()),
            },
        ),
        RunError::NotOwned(run_id) => CallError::new(
            &RUN_NOT_OWNED,
            format!("This engine does not own the Run `{run_id}`."),
            &RunDetails { run_id, path: None },
        ),
        RunError::NotInProduct { needs, .. } => {
            let (message, details) =
                identity.not_in_product(Requested::Name(RUN_ABORT.name), needs, false);
            CallError::new(&NOT_IN_PRODUCT, message, &details)
        }
        RunError::Unavailable {
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
        RunError::Stopped => stopped(),
    }
}

async fn start(params: RunStartParams, context: CallContext) -> Result<RunStarted, CallError> {
    let run_id = run_id(params.run_id.as_deref())?;
    let (settings, _) = context
        .runs()
        .settings()
        .open(Opening {
            run: Some(Product::Collie),
            project: params.project,
            recipe: None,
            settings: params.settings,
            origin: Some(RunOrigin::Settings),
        })
        .await
        .map_err(|error| settings_error(&error, context.identity()))?;
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
            policy_version: settings.resolved().policy_version().map(str::to_owned),
            weakened_by: settings.resolved().weakened_by(),
            timeout: settings.resolved().duration(TIMEOUT_RUN.name),
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
        .map_err(|error| run_error(error, context.identity()))
}

async fn abort(params: RunAbortParams, context: CallContext) -> Result<RunReport, CallError> {
    let forwarded = if params.forwarded {
        Forwarded::Yes
    } else {
        Forwarded::No
    };
    context
        .runs()
        .abort(params.run_id, StoppedBy::Client, forwarded)
        .await
        .map_err(|error| run_error(error, context.identity()))
}
