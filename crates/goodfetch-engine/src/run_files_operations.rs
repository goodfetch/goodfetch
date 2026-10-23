//! The program's calls on the files a Run keeps for it (ADRs 0001, 0085 and
//! 0116): `download.save_as`, which reads a Download's record from the
//! Run's committed state, so it serves a Run that ended or that no engine
//! runs, and then either writes the program's path itself, on an Attached
//! engine, under ADR 0116's rule, or, on any other engine, writes nothing
//! on its own machine and sends the kept file in `$/chunk` notifications,
//! one per credit, for the SDK to write. And the Error code of a file a
//! Run could not keep, for the areas that keep one.

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
use goodfetch_protocol::page::{CHUNK, CHUNK_BYTES, Chunk};
use goodfetch_protocol::run::RunId;
use tokio::io::AsyncReadExt as _;

use crate::composition::{CallContext, CallError, Handler};
use crate::rpc::notification_line;
use crate::run::run_error;
use crate::run_files::{self, KeepError, Record, SaveError};
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
    let failed = |error: RunError| run_error(error, context.identity());
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
    match params.path {
        Some(path) if context.daemon().is_none() => {
            written(&context, &folder, source, path, &record).await
        }
        _ => sent(&context, &folder, &source, &record).await,
    }
}

/// Writes the kept file `source` of the Run folder `folder` at the
/// program's `path`, as an Attached engine on the program's machine may.
async fn written(
    context: &CallContext,
    folder: &Path,
    source: PathBuf,
    path: String,
    record: &Record,
) -> Result<DownloadSaved, CallError> {
    let owner = Arc::clone(&context.runs().limits().owner);
    let token = context.cancellation().clone();
    let target = PathBuf::from(&path);
    owner
        .blocking(move || run_files::save(&source, &target, &|| token.is_cancelled()))
        .await
        .map_err(|error| save_error(error, &path, folder))?;
    Ok(DownloadSaved {
        size: record.size,
        sha256: record.sha256.clone(),
        written: true,
    })
}

/// Sends the kept file `source` in `$/chunk` notifications, one per credit,
/// writing nothing on the engine's machine.
async fn sent(
    context: &CallContext,
    folder: &Path,
    source: &Path,
    record: &Record,
) -> Result<DownloadSaved, CallError> {
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
    let stream = context.stream();
    let mut offset: u64 = 0;
    let mut chunk = vec![0; CHUNK_BYTES];
    loop {
        let read = read_chunk(&mut file, &mut chunk)
            .await
            .map_err(unreadable)?;
        if read == 0 {
            break;
        }
        let _ = stream.credits.spend(1).await;
        let line = notification_line(
            CHUNK,
            &Chunk {
                id: stream.id.clone(),
                offset,
                data: data_encoding::BASE64.encode(&chunk[..read]),
            },
        );
        if stream.outgoing.send(line).await.is_err() {
            break;
        }
        offset += u64::try_from(read).unwrap_or(u64::MAX);
    }
    Ok(DownloadSaved {
        size: record.size,
        sha256: record.sha256.clone(),
        written: false,
    })
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

/// Fills `chunk` from `file` as far as it goes, and answers how many bytes
/// it read: fewer than its length only at the file's end.
async fn read_chunk(file: &mut tokio::fs::File, chunk: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < chunk.len() {
        let read = file.read(&mut chunk[filled..]).await?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}
