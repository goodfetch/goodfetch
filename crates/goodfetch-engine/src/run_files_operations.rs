//! The program's calls on the files a Run keeps for it (ADRs 0001, 0085 and
//! 0116): `download.save_as`, which reads a Download's record from the
//! Run's committed state, so it serves a Run that ended or that no engine
//! runs, and then either writes the program's path itself, on an Attached
//! engine, under ADR 0116's rule, or, on any other engine, writes nothing
//! on its own machine and sends the kept file in `$/chunk` notifications,
//! one per credit, for the SDK to write. That save and that send are
//! public, [`save_kept`] and [`send_kept`], for the files Foxhound's
//! crates keep, such as an Export. And the Error code of a file a Run
//! could not keep, for the areas that keep one.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use goodfetch_protocol::checkpoint::{
    FILE_WRITE_FAILED, FileWriteFailedDetails, PATH_NOT_WRITABLE, PathNotWritableDetails, WriteSide,
};
use goodfetch_protocol::connection::{CALL_CANCELLED, CallCancelledDetails, StoppedBy};
use goodfetch_protocol::download::{
    DOWNLOAD_SAVE_AS, DownloadId, DownloadSaveAsParams, DownloadSaved,
};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::RunId;

use crate::call_error::CallError;
use crate::composition::{CallContext, Handler};
use crate::run::run_error;
use crate::run_files::{self, KeepError, SaveError};
use crate::runs::RunError;

/// The handlers of the calls on a Run's kept files.
pub(crate) fn handlers() -> [Handler; 1] {
    [Handler::new(&DOWNLOAD_SAVE_AS, save_as).streaming()]
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
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
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
    let mut file = tokio::fs::File::open(source).await.map_err(unreadable)?;
    context
        .stream()
        .send_file(&mut file)
        .await
        .map_err(unreadable)?;
    Ok(())
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
                by: StoppedBy::Client,
                closed: None,
            },
        ),
    }
}

/// What to change in the path argument, for `reason` (ADR 0116's `fix`).
fn path_fix(reason: &str) -> &'static str {
    match reason {
        "is_a_directory" => "Give `saveAs` a file name, not a folder.",
        "not_a_directory" => "Give `saveAs` a path whose parent folders are folders, not files.",
        "invalid_filename" => "Give `saveAs` a file name this filesystem accepts.",
        "file_too_large" => "Give `saveAs` a path on a filesystem that holds a file this size.",
        _ => "Give `saveAs` a path in a folder this user can write.",
    }
}
