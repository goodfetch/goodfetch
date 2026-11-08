//! The `run.*` operations every Product carries (ADRs 0002, 0029, 0036,
//! 0102 and 0116): `run.start` begins a Run its program drives, or returns
//! the one its Run ID names; `run.finish` answers a Run's report once it
//! ended; `run.abort` stops it; `run.checkpoint` writes a Checkpoint;
//! `run.pause` and `run.migrate` checkpoint a Run and stop it, resumable;
//! `run.resume` takes the next run epoch of an interrupted Run and starts
//! its kind's driver again; `run.set_progress_message` sets the program's
//! own status line about its Run; `run.progress` and `run.report` read a
//! Run, through the engine that works it when that is another (ADR 0102).
//! Also the one translation of a start's, a Run's
//! or a setting's failure into its Error code, which a Product's own start,
//! such as Foxhound's `crawl.start`, answers with too.
//!
//! Size: one concern, the `run.*` handlers over the Run registry, a few
//! lines each, and the one failure translation they all answer through.

use std::collections::BTreeMap;
use std::path::PathBuf;

use goodfetch_protocol::catalog::Operation;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::{
    CHECKPOINT_INTERVAL, FILE_WRITE_FAILED, FORMAT_VERSION_UNSUPPORTED, FileWriteFailedDetails,
    FormatVersionUnsupportedDetails, RUN_CHECKPOINT, RUN_MIGRATE, RUN_PAUSE, RUN_RESUME,
    RunCheckpointParams, RunResumeParams, SESSION_BOUND_TO_MACHINE, SessionBoundToMachineDetails,
    WriteSide,
};
use goodfetch_protocol::connection::{StoppedBy, WaitedFor};
use goodfetch_protocol::entries::operations_of;
use goodfetch_protocol::formats::{Compatibility, Format};
use goodfetch_protocol::run::{
    Checkpoint, Lifecycle, RESOURCES_EXHAUSTED, Resource, ResourcesExhaustedDetails,
};
use goodfetch_protocol::run::{
    RUN_ABORT, RUN_BUSY, RUN_CONFLICT, RUN_DISCARD, RUN_FINISH, RUN_ID_INVALID, RUN_ID_PATTERN,
    RUN_IN_USE, RUN_LIVE, RUN_NOT_FOUND, RUN_NOT_OWNED, RUN_SET_PROGRESS_MESSAGE, RUN_START,
    RUN_STATE_UNAVAILABLE, RUN_STOP, RUN_USAGE, ReachedLimit, RunAbortParams, RunConflictDetails,
    RunDetails, RunDiscarded, RunId, RunIdInvalidDetails, RunKind, RunParams, RunProgress,
    RunReport, RunSetProgressMessageParams, RunStartParams, RunStarted, RunUsage, RunWaitParams,
    StartedAs, StopReason,
};
use goodfetch_protocol::run::{
    RUN_INTERRUPTED, RUN_LIST, RUN_PROGRESS_READ, RUN_REPORT, RunInterruptedDetails, RunList,
    RunListParams, RunProgressParams,
};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::usage::{
    LIMIT_NOT_UNDERSTOOD, LIMIT_REACHED, Limit, LimitNotUnderstoodDetails,
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use goodfetch_protocol::connection::NOT_IN_PRODUCT;
use goodfetch_protocol::daemon::{DAEMON_DRAINING, DaemonDrainingDetails};
use goodfetch_protocol::secrets::{
    RUN_KEY_MISMATCH, RUN_STATE_KEY_REFERENCE, RunKeyMismatchDetails, SECRET_DOMAIN_NOT_ALLOWED,
    SECRET_INPUT_REFUSED, SECRET_UNAVAILABLE, SecretDomainNotAllowedDetails,
    SecretInputRefusedDetails, SecretUnavailableDetails,
};
use goodfetch_protocol::settings::{
    RunOrigin, SETTING_INVALID, SETTING_LOCKED, SETTING_NOT_APPLICABLE, SETTING_UNKNOWN,
    SettingInvalidDetails, SettingLockedDetails, SettingNotApplicableDetails,
    SettingUnknownDetails, SettingsLayer, TIMEOUT_CLOSE, TIMEOUT_RUN,
};

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::deadline::Deadline;
use crate::product::{Identity, Requested};
use crate::runs::{
    EndedBy, Forwarded, ListError, Resume, Resumed, RunError, Start, StartError, Started,
    StorageExhausted,
};
use crate::secrets::SecretError;
use crate::settings::{Opening, Resolved, SettingsError};

/// The `run.*` handlers.
pub(crate) fn handlers() -> [Handler; 14] {
    [
        Handler::new(&RUN_START, start),
        Handler::new(&RUN_FINISH, finish),
        Handler::new(&RUN_STOP, stop),
        Handler::new(&RUN_USAGE, usage),
        Handler::new(&RUN_ABORT, abort),
        Handler::new(&RUN_CHECKPOINT, checkpoint),
        Handler::new(&RUN_PAUSE, pause),
        Handler::new(&RUN_MIGRATE, migrate),
        Handler::new(&RUN_RESUME, resume),
        Handler::new(&RUN_SET_PROGRESS_MESSAGE, set_progress_message),
        Handler::new(&RUN_DISCARD, discard),
        Handler::new(&RUN_PROGRESS_READ, progress),
        Handler::new(&RUN_REPORT, report),
        Handler::new(&RUN_LIST, list),
    ]
}

/// How often a Run whose settings resolved as `resolved` writes a
/// Checkpoint, from `checkpoint.interval`.
pub fn checkpoint_interval(resolved: &Resolved) -> Option<std::time::Duration> {
    resolved.get(&CHECKPOINT_INTERVAL)
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
        StartError::LimitReached(reached) => limit_reached(&reached),
        StartError::LimitNotUnderstood(limit) => limit_not_understood(*limit),
        StartError::Internal { message, .. } => internal(message),
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
        StartError::Draining { run_id } => draining(run_id),
        StartError::Discarding { run_id, folder } => discarding(run_id, &folder),
        StartError::ResourcesExhausted(exhausted) => storage_exhausted(&exhausted),
        StartError::Secret(error) => secret_error(&error),
        StartError::Stopped => stopped(),
        StartError::Refused(refused) => *refused,
    }
}

/// The failure a call answers with for a Secret reference that gave no
/// value: each names the placeholder and the reference, never a value
/// (ADR 0092).
pub fn secret_error(error: &SecretError) -> CallError {
    let message = error.to_string();
    match error {
        SecretError::Unavailable {
            placeholder,
            reference,
            failure,
            fix,
        } => CallError::new(
            &SECRET_UNAVAILABLE,
            message,
            &SecretUnavailableDetails {
                placeholder: placeholder.clone(),
                reference: reference.to_string(),
                provider: reference.provider().as_str().to_owned(),
                failure: *failure,
                fix: fix.clone(),
            },
        ),
        SecretError::DomainNotAllowed {
            placeholder,
            reference,
            domain,
            domains,
        } => CallError::new(
            &SECRET_DOMAIN_NOT_ALLOWED,
            message,
            &SecretDomainNotAllowedDetails {
                placeholder: placeholder.clone(),
                reference: reference.to_string(),
                domain: domain.clone(),
                domains: domains.clone(),
            },
        ),
        SecretError::InputRefused {
            placeholder,
            reference,
            domain,
            reason,
        } => CallError::new(
            &SECRET_INPUT_REFUSED,
            message,
            &SecretInputRefusedDetails {
                placeholder: placeholder.clone(),
                reference: reference.to_string(),
                domain: domain.clone(),
                reason: *reason,
            },
        ),
        SecretError::Unrecorded { .. } => {
            tracing::error!("{error}");
            internal(message)
        }
    }
}

/// `limit_reached`, naming the Limit's scope, Usage kind and period.
pub fn limit_reached(reached: &ReachedLimit) -> CallError {
    let period = reached
        .period
        .as_ref()
        .map(|period| format!(" this {} ({})", period.every.as_str(), period.time_zone))
        .unwrap_or_default();
    CallError::new(
        &LIMIT_REACHED,
        format!(
            "The {} Limit on {} has nothing left{period}; wait for its next period, or ask a person to raise it.",
            reached.scope.kind, reached.usage_kind
        ),
        reached,
    )
}

/// `limit_not_understood`, naming the Limit.
pub fn limit_not_understood(limit: Limit) -> CallError {
    CallError::new(
        &LIMIT_NOT_UNDERSTOOD,
        format!(
            "A newer engine set a {} Limit on {} that this engine cannot evaluate; upgrade it.",
            limit.scope.kind, limit.usage_kind
        ),
        &LimitNotUnderstoodDetails { limit },
    )
}

/// A run a draining Daemon refuses to start or resume (ADR 0049).
fn draining(run_id: Option<RunId>) -> CallError {
    CallError::new(
        &DAEMON_DRAINING,
        "The Daemon is draining on `SIGTERM` and takes no run; start or resume it on the Daemon that replaces this one.",
        &DaemonDrainingDetails {
            run_id: run_id.map(|run_id| run_id.to_string()),
        },
    )
}

/// The failure a call on `context`'s Connection answers with for a refused
/// setting: the one translation of each refusal into its Error code. A
/// setting the engine's Product lacks is worded for Goodfetch Cloud's
/// gateway when the Connection's `hello` came from it, naming the run's
/// Product rather than an install that would not help there (ADR 0096).
pub fn settings_error(error: &SettingsError, context: &CallContext) -> CallError {
    translated(error, context.identity(), context.behind_gateway())
}

/// The failure a command of the executable itself answers with for a
/// refused setting, outside any Connection: no gateway relays it, so a
/// setting the engine's Product lacks names the install that carries it.
pub fn local_settings_error(error: &SettingsError, identity: &Identity) -> CallError {
    translated(error, identity, false)
}

/// [`settings_error`]'s translation, for the engine `identity` names,
/// behind Goodfetch Cloud's gateway or not.
fn translated(error: &SettingsError, identity: &Identity, behind_gateway: bool) -> CallError {
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
            let (message, details) =
                identity.not_in_product(Requested::Name(key), *needs, behind_gateway);
            CallError::new(&NOT_IN_PRODUCT, message, &details)
        }
        SettingsError::Format {
            format,
            found,
            supported,
            install,
            ..
        } => CallError::new(
            &FORMAT_VERSION_UNSUPPORTED,
            message,
            &FormatVersionUnsupportedDetails {
                format: (*format).to_owned(),
                found: found.clone(),
                supported: supported.clone(),
                fix: match install {
                    Some(install) => {
                        format!("Install Goodfetch {install} or newer, then try again.")
                    }
                    None if error_is_older_major(error) => {
                        "Run `upgrade-files`, then try again.".to_owned()
                    }
                    None => format!("Upgrade Goodfetch to a version that reads {format} {found}."),
                },
            },
        ),
        SettingsError::OlderMajorPolicy { major, current, .. } => CallError::new(
            &FORMAT_VERSION_UNSUPPORTED,
            message,
            &FormatVersionUnsupportedDetails {
                format: goodfetch_protocol::formats::OPERATOR_POLICY.name.to_owned(),
                found: major.clone(),
                supported: current.clone(),
                fix: "Run `upgrade-files` with this Goodfetch, then try again.".to_owned(),
            },
        ),
    }
}

/// `resources_exhausted` for a Run whose filesystem is at the `critical`
/// storage level (ADR 0122): its folder, its free space and the setting,
/// with a fix that names the folder to free or the setting to lower.
fn storage_exhausted(exhausted: &StorageExhausted) -> CallError {
    let folder = exhausted.folder.display().to_string();
    let filesystem = &exhausted.filesystem;
    CallError::new(
        &RESOURCES_EXHAUSTED,
        format!(
            "The filesystem of `{folder}` has {} bytes free, below `storage.critical_free`: the Run `{}` cannot write there.",
            filesystem.free, exhausted.run_id
        ),
        &ResourcesExhaustedDetails {
            resource: Resource::Storage,
            run_id: Some(exhausted.run_id.clone()),
            path: Some(folder.clone()),
            free_bytes: Some(filesystem.free),
            limit: filesystem.limit.clone(),
            fix: format!(
                "Free space on the filesystem of `{folder}`, or lower `storage.critical_free`, then start the Run `{}` again.",
                exhausted.run_id
            ),
        },
    )
}

/// A failure only a bug in Goodfetch causes, such as a transition a Run's
/// table refuses, already logged at `error` (ADR 0128).
fn internal(message: String) -> CallError {
    CallError::protocol(
        goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
        message,
    )
}

fn error_is_older_major(error: &SettingsError) -> bool {
    matches!(
        error,
        SettingsError::Format {
            compatibility: Compatibility::OlderMajor,
            ..
        }
    )
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
pub fn run_error_of(error: RunError, identity: &Identity, operation: &'static str) -> CallError {
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
        } => state_unavailable(run_id, &folder, &error),
        RunError::Internal { message, .. } => internal(message),
        RunError::Draining(run_id) => draining(Some(run_id)),
        RunError::Secret { error, .. } => secret_error(&error),
        RunError::KeyMismatch {
            run_id,
            reference,
            wrapped_by,
        } => key_mismatch(run_id, &reference, wrapped_by),
        RunError::ResourcesExhausted(exhausted) => storage_exhausted(&exhausted),
        RunError::Live(run_id) => live(run_id),
        RunError::Interrupted(run_id) => interrupted(run_id),
        RunError::Discarding { run_id, folder } => discarding(run_id, &folder),
        RunError::Forwarded(failure) => forwarded(*failure),
        RunError::Stopped => stopped(),
    }
}

/// `run_state_unavailable`: the state of `run_id` in `folder` failed with
/// `error`.
fn state_unavailable(run_id: RunId, folder: &std::path::Path, error: &str) -> CallError {
    CallError::new(
        &RUN_STATE_UNAVAILABLE,
        format!("The state of the Run `{run_id}` is unavailable: {error}"),
        &RunDetails {
            run_id,
            path: Some(folder.display().to_string()),
        },
    )
}

/// The failure the owner of a Run answered a forwarded read with.
fn forwarded(failure: goodfetch_protocol::jsonrpc::ErrorObject) -> CallError {
    match goodfetch_protocol::entries::error_code(&failure.data.code) {
        Some(definition) => CallError::new(definition, failure.message, &failure.data.details),
        None => internal(format!("{}: {}", failure.data.code, failure.message)),
    }
}

/// `run_in_use`: this engine discards the Run `run_id`, holding its lock.
fn discarding(run_id: RunId, folder: &std::path::Path) -> CallError {
    CallError::new(
        &RUN_IN_USE,
        format!(
            "This engine is discarding the Run `{run_id}`; once the discard ends, the Run is gone."
        ),
        &RunDetails {
            run_id,
            path: Some(folder.display().to_string()),
        },
    )
}

/// `run_interrupted`: `run_id` has no live writer and has not ended.
fn interrupted(run_id: RunId) -> CallError {
    CallError::new(
        &RUN_INTERRUPTED,
        format!("The Run `{run_id}` has no live writer and has not ended, so it has no report."),
        &RunInterruptedDetails {
            fix: format!("Start the Run with its Run ID `{run_id}` to resume it."),
            run_id,
        },
    )
}

/// Reads `operation` with `params` of the Run `run_id` through the engine
/// that works it, when that is another engine (ADR 0102): its answer, or
/// `None` for the read to go on here, from the Run's committed state. A
/// read never takes the Run's writer lock.
///
/// # Errors
///
/// The failure the owner answered.
///
/// # Cancel safety
///
/// Dropping the future abandons the read; the owner may still answer it,
/// to no one.
pub async fn through_owner<R: DeserializeOwned>(
    context: &CallContext,
    operation: &'static Operation,
    run_id: &RunId,
    params: &impl Serialize,
) -> Result<Option<R>, CallError> {
    let needs = if operations_of(Product::Collie).any(|known| known.name == operation.name) {
        Product::Collie
    } else {
        Product::Foxhound
    };
    let params = serde_json::to_value(params).expect("invariant: params encode");
    let answer = context
        .runs()
        .through_owner(run_id, needs, operation.name, params)
        .await
        .map_err(|error| run_error_of(error, context.identity(), operation.name))?;
    answer
        .map(|answer| {
            serde_json::from_value(answer).map_err(|error| {
                internal(format!(
                    "the Run's owner answered `{}` with no value of its result: {error}",
                    operation.name
                ))
            })
        })
        .transpose()
}

async fn progress(
    params: RunProgressParams,
    context: CallContext,
) -> Result<RunProgress, CallError> {
    if let Some(answer) =
        through_owner(&context, &RUN_PROGRESS_READ, &params.run_id, &params).await?
    {
        return Ok(answer);
    }
    context
        .runs()
        .progress(params.run_id, params.after)
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_PROGRESS_READ.name))
}

async fn list(params: RunListParams, context: CallContext) -> Result<RunList, CallError> {
    match context.runs().list(params).await {
        Ok(runs) => Ok(RunList { runs }),
        Err(ListError::Stopped) => Err(stopped()),
        Err(ListError::Format {
            format,
            found,
            compatibility,
            install,
        }) => {
            let format = goodfetch_protocol::formats::FORMATS
                .iter()
                .find(|known| known.name == format)
                .copied()
                .unwrap_or(&goodfetch_protocol::formats::RUN_INDEX);
            Err(match install {
                Some(install) => store_refusal(format, &found, &install),
                None => format_refusal(format, &found, compatibility),
            })
        }
        Err(error @ ListError::Unreadable(_)) => Err(internal(error.to_string())),
    }
}

async fn report(params: RunParams, context: CallContext) -> Result<RunReport, CallError> {
    if let Some(answer) = through_owner(&context, &RUN_REPORT, &params.run_id, &params).await? {
        return Ok(answer);
    }
    context
        .runs()
        .report(params.run_id)
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_REPORT.name))
}

/// `run_live`: this engine still runs `run_id`, so it is not discarded.
fn live(run_id: RunId) -> CallError {
    CallError::new(
        &RUN_LIVE,
        format!(
            "This engine still runs the Run `{run_id}`; stop it with `run.stop` or `run.abort`, or let it finish, then discard it."
        ),
        &RunDetails { run_id, path: None },
    )
}

/// `run_key_mismatch`: the key-encryption key `reference` does not open
/// the data key of `run_id`, which the key `wrapped_by` names wrapped.
fn key_mismatch(run_id: RunId, reference: &SecretReference, wrapped_by: String) -> CallError {
    CallError::new(
        &RUN_KEY_MISMATCH,
        format!(
            "The key-encryption key `{reference}` does not open the data key of the Run `{run_id}`, which `{wrapped_by}` wrapped where the Run began."
        ),
        &RunKeyMismatchDetails {
            run_id,
            setting: RUN_STATE_KEY_REFERENCE.name.to_owned(),
            reference: reference.to_string(),
            wrapped_by,
            fix: format!(
                "Set `{}` to a Secret reference that gives the key that wrapped it, in project configuration or configuration, then resume again.",
                RUN_STATE_KEY_REFERENCE.name
            ),
        },
    )
}

async fn start(params: RunStartParams, context: CallContext) -> Result<RunStarted, CallError> {
    let run_id = run_id(params.run_id.as_deref())?;
    let (settings, _) = context
        .engine()
        .settings()
        .open(Opening {
            run: Some(Product::Collie),
            project: params.project,
            recipe: None,
            settings: params.settings,
            origin: Some(RunOrigin::Settings),
            acting_for_model: context.surface().acts_for_model(),
        })
        .await
        .map_err(|error| settings_error(&error, &context))?;
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
            weakened: settings.resolved().weakened(),
            binding: None,
            clean_up_digest: None,
            checkpoint_interval: checkpoint_interval(settings.resolved()),
            timeout: settings.resolved().get(&TIMEOUT_RUN),
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

async fn finish(params: RunWaitParams, context: CallContext) -> Result<RunReport, CallError> {
    let finishing = context.runs().finish(params.run_id);
    let report = match params.timeout {
        Some(millis) => {
            Deadline::given(&TIMEOUT_RUN, millis, SettingsLayer::Run)
                .within(&context.engine().clock(), WaitedFor::RUN_END, finishing)
                .await?
        }
        None => finishing.await,
    };
    ending(report.map_err(|error| run_error_of(error, context.identity(), RUN_FINISH.name))?)
}

/// `run.stop`: the Run stops at its next safe point as an abort by its
/// program; the wait is bounded by `timeout.close`, or the call's own
/// `timeout`, and only the wait ends past it (ADR 0102).
async fn stop(params: RunWaitParams, context: CallContext) -> Result<RunReport, CallError> {
    // The stop is asked before the call's first await, so a call its Client
    // cancels still stops the Run; only the wait ends with it (#1743).
    let stopping = context
        .runs()
        .stop_at_safe_point(params.run_id.clone(), StoppedBy::Program);
    let deadline = match params.timeout {
        Some(millis) => Deadline::given(&TIMEOUT_CLOSE, millis, SettingsLayer::Run),
        None => close_deadline(&context, &params.run_id).await,
    };
    let report = deadline
        .within(
            &context.engine().clock(),
            WaitedFor::RUN_SAFE_POINT,
            stopping,
        )
        .await?;
    ending(report.map_err(|error| run_error_of(error, context.identity(), RUN_STOP.name))?)
}

/// The `timeout.close` the live Run `run_id` resolved, or its default.
async fn close_deadline(context: &CallContext, run_id: &RunId) -> Deadline {
    let resolved = match context.runs().live(run_id.clone()).await {
        Ok(run) => run.settings().map(|settings| settings.resolved().clone()),
        Err(_) => None,
    };
    Deadline::of_resolved(resolved.as_ref(), &TIMEOUT_CLOSE)
}

/// What a call that ends a Run answers once it ended: its report, or the
/// error its ending raises, the one that failed a `failed` Run or a write
/// at its end that failed (ADRs 0102 and 0116).
fn ending(report: RunReport) -> Result<RunReport, CallError> {
    let Some(error) = &report.error else {
        return Ok(report);
    };
    Err(match goodfetch_protocol::entries::error_code(&error.code) {
        Some(definition) => CallError::new(definition, error.message.clone(), &error.details),
        None => internal(format!("{}: {}", error.code, error.message)),
    })
}

async fn usage(params: RunParams, context: CallContext) -> Result<RunUsage, CallError> {
    context
        .runs()
        .usage(params.run_id)
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_USAGE.name))
}

async fn discard(params: RunParams, context: CallContext) -> Result<RunDiscarded, CallError> {
    let run_id = params.run_id;
    context
        .runs()
        .discard(run_id.clone())
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_DISCARD.name))?;
    Ok(RunDiscarded { run_id })
}

async fn set_progress_message(
    params: RunSetProgressMessageParams,
    context: CallContext,
) -> Result<RunProgress, CallError> {
    context
        .runs()
        .set_progress_message(params.run_id, params.message)
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_SET_PROGRESS_MESSAGE.name))
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
        .engine()
        .settings()
        .open(Opening {
            run: None,
            project: params.project,
            recipe: None,
            settings: params.settings,
            origin: Some(RunOrigin::Settings),
            acting_for_model: context.surface().acts_for_model(),
        })
        .await
        .map_err(|error| settings_error(&error, &context))?;
    let resumed = context
        .runs()
        .resume(Resume {
            run_id: params.run_id,
            checkpoint: params.checkpoint.map(PathBuf::from),
            checkpoint_interval: checkpoint_interval(settings.resolved()),
            timeout: settings.resolved().get(&TIMEOUT_RUN),
            settings: Some(settings.clone()),
            retry_failed: false,
        })
        .await
        .map_err(|error| run_error_of(error, context.identity(), RUN_RESUME.name))?;
    Ok(match resumed {
        Resumed::New {
            run, kind, needs, ..
        } => {
            let started = RunStarted {
                run_id: run.id().clone(),
                started_as: StartedAs::Resumed,
                lifecycle: Lifecycle::Running,
                needs,
            };
            if let Some(driver) = context.driver(kind) {
                run.spawn(driver(*run.clone(), context.clone(), settings));
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
