//! The program's calls on the files a Run keeps for it (ADRs 0001, 0085 and
//! 0116): `download.save_as`, which reads a Download's record from the
//! Run's committed state, so it serves a Run that ended or that no engine
//! runs, and then either writes the program's path itself, on an Attached
//! engine, under ADR 0116's rule, or, on any other engine, writes nothing
//! on its own machine and sends the kept file in `$/chunk` notifications,
//! one per credit, for the SDK to write. That save and that send are
//! public, [`save_kept`] and [`send_kept`], with [`kept_size`], for the
//! files Foxhound's crates keep, such as an Export. And the Error code of
//! a file a Run could not keep, for the areas that keep one.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_protocol::checkpoint::{
    CHECKPOINT_SAVE_AS, CHECKPOINT_STREAM, CheckpointSaveAsParams, CheckpointSaved,
    CheckpointStreamParams, CheckpointStreamed, FILE_WRITE_FAILED, FileWriteFailedDetails,
    PATH_NOT_WRITABLE, PathNotWritableDetails, WriteSide,
};
use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, CancelledBy};
use goodfetch_protocol::download::{
    DOWNLOAD_SAVE_AS, DownloadId, DownloadSaveAsParams, DownloadSaved,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{
    RECORDING_SAVE_AS, RecordingSaveAsParams, RecordingSaved, RunId, RunParams,
};
use goodfetch_protocol::run_trace::{
    RUN_TRACE, RunTraceResult, TRACE_SAVE_AS, TRACE_STREAM, TraceSaveAsParams, TraceSaved,
    TraceStreamParams, TraceStreamed,
};
use rusqlite::OptionalExtension as _;
use sha2::{Digest as _, Sha256};

use crate::call_error::CallError;
use crate::checkpoint;
use crate::composition::{CallContext, Handler};
use crate::response_recording;
use crate::run::{run_error, unwritten};
use crate::run_files::{self, KeepError, SaveError, Saving};
use crate::runs::RunError;

/// The handlers of the calls on a Run's kept files, Checkpoints, trace copies
/// and Response recordings.
pub(crate) fn handlers() -> [Handler; 7] {
    [
        Handler::new(&DOWNLOAD_SAVE_AS, save_as).streaming(),
        Handler::new(&RECORDING_SAVE_AS, recording_save_as).streaming(),
        Handler::new(&CHECKPOINT_SAVE_AS, checkpoint_save_as).streaming(),
        Handler::new(&CHECKPOINT_STREAM, checkpoint_stream).streaming(),
        Handler::new(&RUN_TRACE, run_trace),
        Handler::new(&TRACE_SAVE_AS, trace_save_as).streaming(),
        Handler::new(&TRACE_STREAM, trace_stream).streaming(),
    ]
}

/// `file_write_failed`, side `engine`, for a file the Run `run_id`, whose
/// folder is `folder`, could not keep, or `run_state_unavailable` when its
/// record could not be committed.
pub fn keep_error(run_id: &RunId, folder: &Path, error: KeepError) -> CallError {
    match error {
        KeepError::Storage {
            reason,
            os_code,
            message,
        } => CallError::new(
            &FILE_WRITE_FAILED,
            format!(
                "The Run `{run_id}` could not keep a file ({message}); free space where the engine keeps its Runs, or ask the Daemon's operator."
            ),
            &FileWriteFailedDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: Some(folder.display().to_string()),
                side: WriteSide::Engine,
            },
        ),
        KeepError::State(error) => CallError::new(
            &goodfetch_protocol::run::RUN_STATE_UNAVAILABLE,
            format!("The state of the Run `{run_id}` is unavailable: {error}"),
            &goodfetch_protocol::run::RunDetails {
                run_id: run_id.clone(),
                path: Some(folder.display().to_string()),
            },
        ),
    }
}

/// A Download ID no Run's state names.
fn no_download(download_id: &DownloadId) -> CallError {
    CallError::protocol(
        ProtocolFailure::InvalidParams,
        format!(
            "no Run keeps a Download `{}`: a Download ID is the one the engine named",
            download_id.0
        ),
    )
}

async fn save_as(
    params: DownloadSaveAsParams,
    context: CallContext,
) -> Result<DownloadSaved, CallError> {
    let download_id = params.download_id;
    let Some((run_id, position)) = download_id.split() else {
        return Err(no_download(&download_id));
    };
    let failed = |error: RunError| run_error(error, &context);
    let mut reader = context
        .runs()
        .reader(run_id.clone())
        .await
        .map_err(failed)?;
    let record = reader
        .read(move |connection| run_files::find(connection, position))
        .await
        .map_err(failed)?
        .filter(|record| record.kind == "download")
        .ok_or_else(|| no_download(&download_id))?;
    let folder = context.runs().folder(run_id).await.map_err(failed)?;
    let source = run_files::file(&folder, &record.sha256).ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the Run's state names the file `{}`", record.sha256),
        )
    })?;
    let written = save_kept(&context, &folder, source, params.path).await?;
    Ok(DownloadSaved {
        size: record.size,
        sha256: record.sha256,
        written,
    })
}

/// Whether the Run's committed `run` row names a `mode: "record"` Response
/// recording in its inputs or report.
fn is_record_mode_run(connection: &rusqlite::Connection) -> rusqlite::Result<bool> {
    let row: Option<(String, Option<String>)> = connection
        .prepare_cached("SELECT inputs, report FROM run")?
        .query_row([], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    let Some((inputs, report)) = row else {
        return Ok(false);
    };
    let has_record = |text: &str| -> bool {
        serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .and_then(|value| {
                value
                    .get("recording")?
                    .get("mode")?
                    .as_str()
                    .map(str::to_owned)
            })
            .is_some_and(|mode| mode == "record")
    };
    Ok(has_record(&inputs) || report.as_deref().is_some_and(has_record))
}

async fn recording_save_as(
    params: RecordingSaveAsParams,
    context: CallContext,
) -> Result<RecordingSaved, CallError> {
    let run_id = params.run_id;
    let display = params
        .path
        .as_deref()
        .or(params.filename.as_deref())
        .unwrap_or("")
        .to_owned();
    let filename = params.filename.clone().or_else(|| {
        params.path.as_deref().and_then(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
    });
    let gzip_per_record = filename.as_deref().is_some_and(|name| {
        Path::new(name)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("gz"))
    });
    let software = format!(
        "{} {}",
        context.identity().product.name(),
        context.identity().product_version
    );
    let now = context.engine().clock().timestamp();
    let id_seed = context.engine().draw_u64();

    let failed = |error: RunError| run_error(error, &context);
    let folder = context
        .runs()
        .folder(run_id.clone())
        .await
        .map_err(failed)?;

    // Flush any staged exchanges if the Run is still live with a Recorder.
    let live_recorder = context.engine().recordings().recorder(&run_id);
    let has_live_recorder = live_recorder.is_some();
    if let (Some(recorder), Ok(live)) = (live_recorder, context.runs().live(run_id.clone()).await) {
        live.state
            .write(move |connection| recorder.flush_staged(connection))
            .await
            .map_err(|error| unwritten(&live.identity, &error, &context))?;
    }

    let mut reader = context
        .runs()
        .reader(run_id.clone())
        .await
        .map_err(failed)?;
    let run_id_for_read = run_id.clone();
    let encoded = reader
        .read(move |connection| {
            let allow_empty = has_live_recorder || is_record_mode_run(connection)?;
            response_recording::encode_warc(
                connection,
                &run_id_for_read,
                &software,
                filename,
                gzip_per_record,
                now,
                id_seed,
                allow_empty,
            )
        })
        .await
        .map_err(failed)?;
    let Some(bytes) = encoded else {
        return Err(response_recording::recording_invalid_error(
            &display,
            "the Run recorded no responses",
        ));
    };
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let written =
        deliver_recording_bytes(bytes, size, params.path, &display, &folder, &context).await?;
    Ok(RecordingSaved {
        size,
        digest,
        written,
    })
}

async fn deliver_recording_bytes(
    bytes: Vec<u8>,
    size: u64,
    path: Option<String>,
    display: &str,
    folder: &Path,
    context: &CallContext,
) -> Result<bool, CallError> {
    match path {
        Some(path) if context.daemon().is_none() => {
            let target = PathBuf::from(&path);
            let owner = Arc::clone(context.engine().owner());
            let token = context.cancellation().clone();
            owner
                .blocking(move || {
                    if token.is_cancelled() {
                        return Err(SaveError::Cancelled);
                    }
                    let mut saving = Saving::create(&target)?;
                    for chunk in bytes.chunks(64 * 1024) {
                        if token.is_cancelled() {
                            return Err(SaveError::Cancelled);
                        }
                        saving.write(chunk)?;
                    }
                    if token.is_cancelled() {
                        return Err(SaveError::Cancelled);
                    }
                    saving.finish()
                })
                .await
                .map_err(|error| save_error(error, &path, folder))?;
            Ok(true)
        }
        _ => {
            let sent = context.stream().send_bytes(&bytes).await;
            if sent < size {
                let error = if context.cancellation().is_cancelled() {
                    SaveError::Cancelled
                } else {
                    SaveError::Source {
                        reason: "broken_pipe",
                        os_code: None,
                        message: format!("the recording stream ended after {sent} of {size} bytes"),
                    }
                };
                return Err(save_error(error, display, folder));
            }
            Ok(false)
        }
    }
}

/// A Checkpoint path not found or not belonging to the named Run.
fn no_checkpoint(run_id: &RunId, location: &str) -> CallError {
    CallError::protocol(
        ProtocolFailure::InvalidParams,
        format!(
            "the Run `{run_id}` keeps no Checkpoint at `{location}`: give the `location` that `run.checkpoint` answered"
        ),
    )
}

/// A verified Checkpoint file on the engine's machine.
struct KeptCheckpoint {
    folder: PathBuf,
    file: PathBuf,
    size: u64,
    digest: String,
    run_epoch: u64,
}

async fn kept_checkpoint(
    run_id: &RunId,
    location: &str,
    context: &CallContext,
) -> Result<KeptCheckpoint, CallError> {
    let failed = |error: RunError| run_error(error, context);
    let folder = context
        .runs()
        .folder(run_id.clone())
        .await
        .map_err(failed)?;
    let source = PathBuf::from(location);
    let expected_run_id = run_id.as_str().to_owned();
    let verified = context
        .engine()
        .owner()
        .blocking(move || read_checkpoint_meta(&source, &expected_run_id))
        .await;
    let Some((file, size, digest, run_epoch)) = verified else {
        return Err(no_checkpoint(run_id, location));
    };
    Ok(KeptCheckpoint {
        folder,
        file,
        size,
        digest,
        run_epoch,
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "runs on the engine TaskOwner blocking policy"
)]
fn read_checkpoint_meta(
    source: &Path,
    expected_run_id: &str,
) -> Option<(PathBuf, u64, String, u64)> {
    let connection = checkpoint::open_checkpoint(source).ok()?;
    let held = checkpoint::read(&connection, source).ok()?;
    if held.run_id != expected_run_id {
        return None;
    }
    drop(connection);
    let mut file = std::fs::File::open(source).ok()?;
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += u64::try_from(read).unwrap_or(u64::MAX);
    }
    let digest = data_encoding::HEXLOWER.encode(&hasher.finalize());
    Some((source.to_owned(), size, digest, held.position.run_epoch))
}

async fn checkpoint_save_as(
    params: CheckpointSaveAsParams,
    context: CallContext,
) -> Result<CheckpointSaved, CallError> {
    let kept = kept_checkpoint(&params.run_id, &params.location, &context).await?;
    let written = save_kept(&context, &kept.folder, kept.file, params.path).await?;
    Ok(CheckpointSaved {
        size: kept.size,
        digest: kept.digest,
        written,
        run_epoch: kept.run_epoch,
    })
}

async fn checkpoint_stream(
    params: CheckpointStreamParams,
    context: CallContext,
) -> Result<CheckpointStreamed, CallError> {
    let kept = kept_checkpoint(&params.run_id, &params.location, &context).await?;
    send_kept(&context, &kept.folder, &kept.file).await?;
    Ok(CheckpointStreamed {
        size: kept.size,
        digest: kept.digest,
        run_epoch: kept.run_epoch,
    })
}

/// A trace copy path not kept for the named Run.
fn no_trace_copy(run_id: &RunId, location: &str) -> CallError {
    CallError::protocol(
        ProtocolFailure::InvalidParams,
        format!(
            "the Run `{run_id}` keeps no trace copy at `{location}`: give the `location` that `run.trace` answered"
        ),
    )
}

/// A verified trace copy file in its Run's folder.
struct KeptTrace {
    folder: PathBuf,
    file: PathBuf,
    size: u64,
    digest: String,
}

async fn kept_trace(
    run_id: &RunId,
    location: &str,
    context: &CallContext,
) -> Result<KeptTrace, CallError> {
    let failed = |error: RunError| run_error(error, context);
    let folder = context
        .runs()
        .folder(run_id.clone())
        .await
        .map_err(failed)?;
    let given = Path::new(location);
    let digest = given
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| no_trace_copy(run_id, location))?;
    let file = run_files::trace_copy_file(&folder, digest)
        .ok_or_else(|| no_trace_copy(run_id, location))?;
    if given != file {
        return Err(no_trace_copy(run_id, location));
    }
    let size = kept_size(context, &file)
        .await
        .map_err(|_| no_trace_copy(run_id, location))?;
    Ok(KeptTrace {
        folder,
        file,
        size,
        digest: digest.to_owned(),
    })
}

async fn run_trace(params: RunParams, context: CallContext) -> Result<RunTraceResult, CallError> {
    let failed = |error: RunError| run_error(error, &context);
    let folder = context
        .runs()
        .folder(params.run_id.clone())
        .await
        .map_err(failed)?;
    let trace = context
        .runs()
        .keep_trace_copy(params.run_id.clone())
        .await
        .map_err(|error| keep_error(&params.run_id, &folder, error))?;
    Ok(RunTraceResult { trace })
}

async fn trace_save_as(
    params: TraceSaveAsParams,
    context: CallContext,
) -> Result<TraceSaved, CallError> {
    let kept = kept_trace(&params.run_id, &params.location, &context).await?;
    let written = save_kept(&context, &kept.folder, kept.file, params.path).await?;
    Ok(TraceSaved {
        size: kept.size,
        digest: kept.digest,
        written,
    })
}

async fn trace_stream(
    params: TraceStreamParams,
    context: CallContext,
) -> Result<TraceStreamed, CallError> {
    let kept = kept_trace(&params.run_id, &params.location, &context).await?;
    send_kept(&context, &kept.folder, &kept.file).await?;
    Ok(TraceStreamed {
        size: kept.size,
        digest: kept.digest,
    })
}

/// Saves the file `source` that the Run whose folder is `folder` keeps,
/// a Download's or an Export's, for the call `context` serves (ADR 0116):
/// an Attached engine given the program's `path` writes it there itself,
/// through a hidden temporary file renamed over it; any other engine
/// writes nothing on its own machine and sends the file in `$/chunk`
/// notifications, one per credit, for the SDK to write. Answers whether
/// the engine wrote the path.
///
/// # Errors
///
/// `path_not_writable` or `file_write_failed` with side `program` when the
/// path could not be written, `file_write_failed` with side `engine` when
/// the kept file could not be read, and `call_cancelled`.
///
/// # Cancel safety
///
/// Dropping the future does not stop a write of the path already
/// started: it runs to its end, the hidden temporary file renamed over the
/// path only once whole. Chunks already sent to the SDK stay sent.
pub async fn save_kept(
    context: &CallContext,
    folder: &Path,
    source: PathBuf,
    path: Option<String>,
) -> Result<bool, CallError> {
    match path {
        Some(path) if context.daemon().is_none() => {
            written(context, folder, source, path).await?;
            Ok(true)
        }
        _ => {
            send_kept(context, folder, &source).await?;
            Ok(false)
        }
    }
}

/// Writes the kept file `source` of the Run folder `folder` at the
/// program's `path`, as an Attached engine on the program's machine may.
async fn written(
    context: &CallContext,
    folder: &Path,
    source: PathBuf,
    path: String,
) -> Result<(), CallError> {
    let owner = Arc::clone(context.engine().owner());
    let token = context.cancellation().clone();
    let target = PathBuf::from(&path);
    owner
        .blocking(move || run_files::save(&source, &target, &|| token.is_cancelled()))
        .await
        .map_err(|error| save_error(error, &path, folder))
}

/// Sends the file `source` that the Run whose folder is `folder` keeps in
/// `$/chunk` notifications of the call `context` serves, in order, one per
/// credit, writing nothing on the engine's machine.
///
/// # Errors
///
/// `file_write_failed` with side `engine` when the kept file could not be
/// read.
///
/// # Cancel safety
///
/// Dropping the future stops the chunks; those sent stay sent.
pub async fn send_kept(
    context: &CallContext,
    folder: &Path,
    source: &Path,
) -> Result<(), CallError> {
    let unreadable = |error: std::io::Error| {
        save_error(
            SaveError::Source {
                reason: run_files::reason_of(error.kind()),
                os_code: error.raw_os_error().and_then(run_files::os_code),
                message: format!("could not read the kept file: {error}"),
            },
            "",
            folder,
        )
    };
    let owner = context.engine().owner();
    let source = source.to_owned();
    let file = owner
        .blocking(move || {
            #[expect(
                clippy::disallowed_methods,
                reason = "ADR 0140: a file opened on no counted row yet, on the tasks module's blocking policy (issue 2106); the Shared paths audit moves it onto its counted function"
            )]
            std::fs::File::open(source)
        })
        .await
        .map_err(unreadable)?;
    context
        .stream()
        .send_file(file, owner)
        .await
        .map_err(unreadable)?;
    Ok(())
}

/// The size of the kept file `source`, read on the engine's blocking work
/// (issue 2106).
///
/// # Errors
///
/// When the file cannot be read, such as one no Run keeps.
///
/// # Cancel safety
///
/// Dropping the future loses only the answer.
pub async fn kept_size(context: &CallContext, source: &Path) -> std::io::Result<u64> {
    let source = source.to_owned();
    context
        .engine()
        .owner()
        .blocking(move || {
            #[expect(
                clippy::disallowed_methods,
                reason = "it runs on the tasks module's blocking policy"
            )]
            std::fs::metadata(source).map(|metadata| metadata.len())
        })
        .await
}

/// The Error code of a save to the program's `path` that failed, the Run
/// folder being `folder`.
fn save_error(error: SaveError, path: &str, folder: &Path) -> CallError {
    match error {
        SaveError::Path {
            reason,
            os_code,
            message,
        } => CallError::new(
            &PATH_NOT_WRITABLE,
            format!("The path `{path}` cannot be written ({message}); nothing was written there."),
            &PathNotWritableDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: path.to_owned(),
                fix: path_fix(reason).to_owned(),
            },
        ),
        SaveError::Storage {
            reason,
            os_code,
            message,
        } => CallError::new(
            &FILE_WRITE_FAILED,
            format!(
                "The file could not be written at `{path}` ({message}); the path holds what it held before. Free space there, then call `saveAs` again: the engine still keeps the file."
            ),
            &FileWriteFailedDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: Some(path.to_owned()),
                side: WriteSide::Program,
            },
        ),
        SaveError::Source {
            reason,
            os_code,
            message,
        } => CallError::new(
            &FILE_WRITE_FAILED,
            format!(
                "The kept file could not be read where the engine keeps it ({message}); check the storage where the engine keeps its Runs, or ask the Daemon's operator, then call `saveAs` again."
            ),
            &FileWriteFailedDetails {
                reason: reason.to_owned(),
                os_code: os_code.map(str::to_owned),
                path: Some(folder.display().to_string()),
                side: WriteSide::Engine,
            },
        ),
        SaveError::Cancelled => CallError::new(
            &CALL_CANCELLED,
            "The call was cancelled by its Client.",
            &CallCancelledDetails {
                by: CancelledBy::Client,
                closed: None,
                person: None,
            },
        ),
    }
}

/// What to change in the path argument, for `reason` (ADR 0116's `fix`).
fn path_fix(reason: &str) -> &'static str {
    match reason {
        "is_a_directory" => "Give a file name, not a folder.",
        "not_a_directory" => "Give a path whose parent folders are folders, not files.",
        "invalid_filename" => "Give a file name this filesystem accepts.",
        "file_too_large" => "Give a path on a filesystem that holds a file this size.",
        _ => "Give a path in a folder this user can write.",
    }
}
