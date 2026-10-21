//! The `run.*` operations every Product carries (ADRs 0002, 0029, 0036,
//! 0102 and 0116): `run.start` begins a Run its program drives, or returns
//! the one its Run ID names; `run.finish` answers a Run's report once it
//! ended; `run.abort` stops it; `run.checkpoint` writes a Checkpoint;
//! `run.pause` and `run.migrate` checkpoint a Run and stop it, resumable;
//! `run.resume` takes the next run epoch of an interrupted Run and starts
//! its kind's driver again. Also the one translation of a start's, a Run's
//! or a setting's failure into its Error code, which a Product's own start,
//! such as Foxhound's `crawl.start`, answers with too.

use std::collections::BTreeMap;
use std::path::PathBuf;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::{
    CHECKPOINT_INTERVAL, FILE_WRITE_FAILED, FORMAT_VERSION_UNSUPPORTED, FileWriteFailedDetails,
    FormatVersionUnsupportedDetails, RUN_CHECKPOINT, RUN_MIGRATE, RUN_PAUSE, RUN_RESUME,
    RunCheckpointParams, RunResumeParams, SESSION_BOUND_TO_MACHINE, SessionBoundToMachineDetails,
    WriteSide,
};
use goodfetch_protocol::connection::StoppedBy;
use goodfetch_protocol::formats::{Compatibility, Format};
use goodfetch_protocol::run::{Checkpoint, Lifecycle};
use goodfetch_protocol::run::{
    RUN_ABORT, RUN_BUSY, RUN_CONFLICT, RUN_FINISH, RUN_ID_INVALID, RUN_ID_PATTERN, RUN_IN_USE,
    RUN_NOT_FOUND, RUN_NOT_OWNED, RUN_START, RUN_STATE_UNAVAILABLE, RunAbortParams,
    RunConflictDetails, RunDetails, RunId, RunIdInvalidDetails, RunKind, RunParams, RunReport,
    RunStartParams, RunStarted, StartedAs, StopReason,
};

use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::settings::{
    RunOrigin, SETTING_INVALID, SETTING_LOCKED, SETTING_NOT_APPLICABLE, SETTING_UNKNOWN,
    SettingInvalidDetails, SettingLockedDetails, SettingNotApplicableDetails,
    SettingUnknownDetails, TIMEOUT_RUN,
};

use crate::composition::{CallContext, CallError, Handler};
use crate::product::{Identity, Requested};
use crate::runs::{EndedBy, Forwarded, Resume, Resumed, RunError, Start, StartError, Started};
use crate::settings::{Opening, Resolved, SettingsError};

/// The `run.*` handlers.
pub(crate) fn handlers() -> [Handler; 7] {
    [
        Handler::new(&RUN_START, start),
        Handler::new(&RUN_FINISH, finish),
        Handler::new(&RUN_ABORT, abort),
        Handler::new(&RUN_CHECKPOINT, checkpoint),
        Handler::new(&RUN_PAUSE, pause),
        Handler::new(&RUN_MIGRATE, migrate),
        Handler::new(&RUN_RESUME, resume),
    ]
}

/// How often a Run whose settings resolved as `resolved` writes a
/// Checkpoint, from `checkpoint.interval`.
pub fn checkpoint_interval(resolved: &Resolved) -> Option<std::time::Duration> {
    resolved.duration(CHECKPOINT_INTERVAL.name)
}

/// The refusal of a file of `format` at the version `found`, which this
/// engine, reading `format`'s own version, makes `compatibility` of: what
/// to upgrade, or `upgrade-files` for an older major (ADRs 0030 and 0066).
pub fn format_refusal(format: &Format, found: &str, compatibility: Compatibility) -> CallError {
    let supported = format.version.to_string();
    let (message, fix) = match compatibility {
        Compatibility::OlderMajor => (
            format!(
                "The file is {} {found}, of an older major than this engine's {supported}; run `upgrade-files` to upgrade it.",
                format.name
            ),
            "Run `upgrade-files`, then try again.".to_owned(),
        ),
        Compatibility::NewerMinor | Compatibility::NewerMajor | Compatibility::Compatible => (
            format!(
                "The file is {} {found}, newer than this engine's {supported}; upgrade Goodfetch to read it.",
                format.name
            ),
            format!(
                "Upgrade Goodfetch to a version that reads {} {found}.",
                format.name
            ),
        ),
    };
    CallError::new(
        &FORMAT_VERSION_UNSUPPORTED,
        message,
        &FormatVersionUnsupportedDetails {
            format: format.name.to_owned(),
            found: found.to_owned(),
            supported,
            fix,
        },
    )
}

/// The refusal of a shared store of `format` at the version `found` whose
/// lowest minor is newer than this engine (ADR 0098): the Product version
/// `install` and newer can use it.
pub fn store_refusal(format: &Format, found: &str, install: &str) -> CallError {
    CallError::new(
        &FORMAT_VERSION_UNSUPPORTED,
        format!(
            "The {} ({found}) was last written by a newer engine and can be used from Goodfetch {install} on, newer than this engine; install Goodfetch {install} or newer to use it.",
            format.name
        ),
        &FormatVersionUnsupportedDetails {
            format: format.name.to_owned(),
            found: found.to_owned(),
            supported: format.version.to_string(),
            fix: format!("Install Goodfetch {install} or newer, then try again."),
        },
    )
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
        StartError::Format {
            format,
            found,
            compatibility,
            install,
        } => {
            let format = goodfetch_protocol::formats::FORMATS
                .iter()
                .find(|known| known.name == format)
                .copied()
                .unwrap_or(&goodfetch_protocol::formats::RUN_INDEX);
            match install {
                Some(install) => store_refusal(format, &found, &install),
                None => format_refusal(format, &found, compatibility),
            }
        }
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

/// The Error code of a Run's failure to be finished, aborted or read.
pub fn run_error(error: RunError, identity: &Identity) -> CallError {
    run_error_of(error, identity, RUN_ABORT.name)
}

/// The Error code a Run's failure answers with, for a call of `operation`.
fn run_error_of(error: RunError, identity: &Identity, operation: &'static str) -> CallError {
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
                identity.not_in_product(Requested::Name(operation), needs, false);
            CallError::new(&NOT_IN_PRODUCT, message, &details)
        }
        RunError::InUse { run_id, folder } => CallError::new(
            &RUN_IN_USE,
            format!("Another engine writes the Run `{run_id}`."),
            &RunDetails {
                run_id,
                path: Some(folder.display().to_string()),
            },
        ),
        RunError::Format {
            format,
            found,
            compatibility,
            install,
            ..
        } => {
            let format = goodfetch_protocol::formats::FORMATS
                .iter()
                .find(|known| known.name == format)
                .copied()
                .unwrap_or(&goodfetch_protocol::formats::RUN_STATE);
            match install {
                Some(install) => store_refusal(format, &found, &install),
                None => format_refusal(format, &found, compatibility),
            }
        }
        RunError::BoundToMachine(run_id) => CallError::new(
            &SESSION_BOUND_TO_MACHINE,
            format!(
                "The Run `{run_id}` is bound to the machine its Session source is on; resume it there, or serve its Session source through a Browser host."
            ),
            &SessionBoundToMachineDetails { run_id },
        ),
        RunError::WriteFailed {
            run_id,
            folder,
            reason,
            os_code,
            message,
        } => CallError::new(
            &FILE_WRITE_FAILED,
            format!(
                "A Checkpoint of the Run `{run_id}` could not be written ({message}); free space where the engine keeps its Runs, or ask the Daemon's operator, then checkpoint again."
            ),
            &FileWriteFailedDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: Some(folder.display().to_string()),
                side: WriteSide::Engine,
            },
        ),
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
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: checkpoint_interval(settings.resolved()),
            timeout: settings.resolved().duration(TIMEOUT_RUN.name),
            settings: Some(settings.clone()),
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

async fn checkpoint(
    params: RunCheckpointParams,
    context: CallContext,
) -> Result<Checkpoint, CallError> {
    context
        .runs()
        .checkpoint(params.run_id, params.location.map(PathBuf::from))
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_CHECKPOINT.name))
}

async fn pause(params: RunParams, context: CallContext) -> Result<RunReport, CallError> {
    context
        .runs()
        .stop(params.run_id, StopReason::Pause, None)
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_PAUSE.name))
}

async fn migrate(
    params: RunCheckpointParams,
    context: CallContext,
) -> Result<RunReport, CallError> {
    context
        .runs()
        .stop(
            params.run_id,
            StopReason::Migrate,
            params.location.map(PathBuf::from),
        )
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_MIGRATE.name))
}

async fn resume(params: RunResumeParams, context: CallContext) -> Result<RunStarted, CallError> {
    let (settings, _) = context
        .runs()
        .settings()
        .open(Opening {
            run: None,
            project: params.project,
            recipe: None,
            settings: params.settings,
            origin: Some(RunOrigin::Settings),
        })
        .await
        .map_err(|error| settings_error(&error, context.identity()))?;
    let resumed = context
        .runs()
        .resume(Resume {
            run_id: params.run_id,
            checkpoint: params.checkpoint.map(PathBuf::from),
            checkpoint_interval: checkpoint_interval(settings.resolved()),
            timeout: settings.resolved().duration(TIMEOUT_RUN.name),
            settings: Some(settings.clone()),
        })
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_RESUME.name))?;
    Ok(match resumed {
        Resumed::New { run, kind, needs } => {
            let started = RunStarted {
                run_id: run.id().clone(),
                started_as: StartedAs::Resumed,
                lifecycle: Lifecycle::Running,
                needs,
            };
            if let Some(driver) = context.runs().driver(kind) {
                run.spawn(driver(run.clone(), context.client().clone(), settings));
            }
            started
        }
        Resumed::Existing(existing) => existing,
    })
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
