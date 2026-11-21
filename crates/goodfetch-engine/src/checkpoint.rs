//! A Run's Checkpoints (ADRs 0002, 0003, 0056, 0066 and 0116): a
//! consistent, self-contained copy of its run file, taken through SQLite's
//! online backup from a reader beside the Run's writer, so the copy holds
//! the run file as of one commit, then written whole or not at all: into a
//! hidden temporary file beside its location, synced, then renamed over
//! it, its size and SHA-256 read from the bytes that landed. A write that
//! fails leaves no file and says why in `io::ErrorKind`'s words. Reading a
//! Checkpoint or a run file before resuming it checks its format's version
//! and reads what the resume decides on: the Run it holds, the Product it
//! needs, what binds it to a machine and where it is. A Checkpoint becomes
//! a Run's run file on another machine by the same whole-or-nothing write.
//!
//! The run file's reader opens through the run state's VFS, the crash VFS
//! in tests (ADR 0060); a Checkpoint is a file of the real filesystem.
#![expect(
    clippy::disallowed_methods,
    reason = "its calls block: the Runs registry runs them under its owner's blocking policy"
)]

use std::fs::File;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::checkpoint::RunBinding;
use goodfetch_protocol::formats::RUN_STATE;
use goodfetch_protocol::run::Lifecycle;
use rusqlite::backup::{Backup, StepResult};
use rusqlite::{Connection, ErrorCode};
use sha2::{Digest as _, Sha256};

use crate::durable;
use crate::run_files::{self, CarryError};
use crate::run_key::{self, StoredKey};
use crate::run_state::{RunStateError, open_reader};
use crate::sqlite_store::{self, Access};

/// The name of the Checkpoint the engine keeps in a Run's folder: the
/// latest one taken there.
pub(crate) const CHECKPOINT_FILE: &str = "checkpoint.sqlite";

/// What a Checkpoint that was written holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Written {
    /// Its size in bytes.
    pub(crate) size: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    pub(crate) digest: String,
    /// The run epoch of the writer whose commits it holds.
    pub(crate) run_epoch: u64,
    /// How many commits of the run file it holds.
    pub(crate) journal_position: u64,
}

/// Why a Checkpoint, or a run file made from one, was not written.
#[derive(Debug, thiserror::Error)]
pub(crate) enum WriteError {
    /// The storage failed: the file was not written, and none is left.
    #[error("{message}")]
    Storage {
        /// Why, as `io::ErrorKind`'s name in `snake_case`.
        reason: &'static str,
        /// The OS's own name for it, when known.
        os_code: Option<&'static str>,
        /// What failed.
        message: String,
    },
    /// The run file could not be read.
    #[error(transparent)]
    Source(#[from] RunStateError),
}

/// Writes a Checkpoint of the run file at `source`, read through the VFS
/// `vfs`, to `destination`, whole or not at all; `nonce` names its
/// temporary file. It blocks: run it under a blocking policy.
///
/// # Errors
///
/// `Source` when the run file cannot be read, `Storage` when the
/// Checkpoint cannot be written; neither leaves a file at `destination` or
/// beside it.
pub(crate) fn write(
    source: &Path,
    vfs: Option<&str>,
    destination: &Path,
    nonce: u64,
) -> Result<Written, WriteError> {
    let reader = open_reader(source, vfs)?;
    let temporary = Temporary::create(destination, nonce)?;
    let copied = copy_into(&reader, source, temporary.path());
    drop(reader);
    let (run_epoch, journal_position) = copied?;
    let (size, digest) = sync_and_digest(temporary.path())?;
    temporary.persist(destination)?;
    Ok(Written {
        size,
        digest,
        run_epoch,
        journal_position,
    })
}

/// Makes the Checkpoint at `checkpoint` the run file at `run_file`, opened
/// through the VFS `vfs`, replacing whatever it held: SQLite's online
/// backup restores it in one transaction of the run file, so a crash leaves
/// the earlier run file or the whole Checkpoint. Its kept files are written
/// into the run file's folder first, so a Checkpoint whose files cannot be
/// carried leaves the run file as it was. It blocks: run it under a
/// blocking policy.
///
/// # Errors
///
/// `Source` when the Checkpoint cannot be read, `Storage` when the run
/// file cannot be written.
pub(crate) fn install(
    checkpoint: &Path,
    run_file: &Path,
    vfs: Option<&str>,
) -> Result<(), WriteError> {
    let from = open_checkpoint(checkpoint)?;
    if let Some(folder) = run_file.parent() {
        std::fs::create_dir_all(folder).map_err(|error| storage(&error, "make its folder"))?;
        run_files::unpack(&from, folder).map_err(|error| carry_failed(error, checkpoint))?;
    }
    let failed = |error: rusqlite::Error| sqlite_storage(&error);
    let mut to = sqlite_store::File::new(run_file, vfs)
        .connect(Access::Create)
        .map_err(failed)?;
    {
        let backup = Backup::new(&from, &mut to).map_err(failed)?;
        match backup.step(-1).map_err(failed)? {
            StepResult::Done => {}
            other => {
                return Err(WriteError::Source(RunStateError::Sqlite {
                    path: checkpoint.to_owned(),
                    message: format!("the restore stopped early: {other:?}"),
                }));
            }
        }
    }
    run_files::drop_chunks(&to).map_err(failed)
}

/// The failure `error` of carrying the kept files of the Checkpoint or run
/// file at `path`.
fn carry_failed(error: CarryError, path: &Path) -> WriteError {
    match error {
        CarryError::Io(error) => storage(&error, "carry its kept files"),
        other => WriteError::Source(RunStateError::Sqlite {
            path: path.to_owned(),
            message: other.to_string(),
        }),
    }
}

/// What a run file or a Checkpoint says about its Run, read before a
/// resume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Held {
    /// The Run's ID, as written.
    pub(crate) run_id: String,
    /// The Product it needs.
    pub(crate) needs: Product,
    /// Where it is.
    pub(crate) lifecycle: Lifecycle,
    /// What binds it, if anything.
    pub(crate) binding: Option<RunBinding>,
    /// The run epoch of its last writer.
    pub(crate) run_epoch: u64,
    /// Its data key, wrapped, once it has one.
    pub(crate) key: Option<StoredKey>,
    /// The settings fixed for its life, as its start kept them: a JSON
    /// object of each key's masked value (ADRs 0102 and 0120).
    pub(crate) fixed_settings: String,
}

/// Opens the Checkpoint at `path` to read it, on the real filesystem.
///
/// # Errors
///
/// `Sqlite` when it cannot be opened.
pub(crate) fn open_checkpoint(path: &Path) -> Result<Connection, RunStateError> {
    open_reader(path, None)
}

/// Reads what the run file or Checkpoint `connection` holds of its Run,
/// refusing one of a format version this engine cannot read first.
///
/// # Errors
///
/// `Format` for a version this engine cannot read, `Sqlite` for a file
/// that holds no Run or no readable one.
pub(crate) fn read(connection: &Connection, path: &Path) -> Result<Held, RunStateError> {
    let damaged = |message: String| RunStateError::Sqlite {
        path: path.to_owned(),
        message,
    };
    let found: String = connection
        .prepare_cached("SELECT version FROM format WHERE name = ?1")
        .and_then(|mut statement| statement.query_row([RUN_STATE.name], |row| row.get(0)))
        .map_err(|error| damaged(error.to_string()))?;
    sqlite_store::check_format(&RUN_STATE, &found)?;
    let (run_id, needs, lifecycle, binding, run_epoch, fixed_settings): (
        String,
        String,
        String,
        Option<String>,
        i64,
        String,
    ) = connection
        .prepare_cached("SELECT run_id, needs, lifecycle, binding, epoch, fixed_settings FROM run")
        .and_then(|mut statement| {
            statement.query_row([], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            })
        })
        .map_err(|error| damaged(error.to_string()))?;
    let text = |value: &str| serde_json::Value::String(value.to_owned());
    Ok(Held {
        run_id,
        needs: serde_json::from_value(text(&needs))
            .map_err(|error| damaged(format!("`needs`: {error}")))?,
        lifecycle: serde_json::from_value(text(&lifecycle))
            .map_err(|error| damaged(format!("`lifecycle`: {error}")))?,
        binding: binding
            .map(|binding| serde_json::from_str(&binding))
            .transpose()
            .map_err(|error| damaged(format!("`binding`: {error}")))?,
        run_epoch: u64::try_from(run_epoch).unwrap_or_default(),
        key: run_key::stored(connection).map_err(|error| damaged(error.to_string()))?,
        fixed_settings,
    })
}

/// The fuzz target's body for Checkpoint files (ADR 0067): reads `data` as
/// a Checkpoint, in memory, as a resume reads one; it may refuse it, never
/// panic.
pub fn fuzz_checkpoint(data: &[u8]) {
    let Ok(mut connection) = Connection::open_in_memory() else {
        return;
    };
    if connection
        .deserialize_read_exact(rusqlite::MAIN_DB, data, data.len(), true)
        .is_err()
    {
        return;
    }
    let _ = read(&connection, Path::new("fuzzed"));
}

/// Copies `reader`'s database into the empty file at `into` in one step of
/// SQLite's online backup, so it holds one commit's state, and answers its
/// run epoch and journal position.
fn copy_into(reader: &Connection, source: &Path, into: &Path) -> Result<(u64, u64), WriteError> {
    let failed = |error: rusqlite::Error| sqlite_storage(&error);
    let mut copy = sqlite_store::File::new(into, None)
        .connect(Access::ReadWrite)
        .map_err(failed)?;
    {
        let backup = Backup::new(reader, &mut copy).map_err(failed)?;
        match backup.step(-1).map_err(failed)? {
            StepResult::Done => {}
            other => {
                return Err(WriteError::Source(RunStateError::Sqlite {
                    path: source.to_owned(),
                    message: format!("the online backup stopped early: {other:?}"),
                }));
            }
        }
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "the copy is this call's own new file, which no other connection opens"
    )]
    copy.pragma_update_and_check(None, "journal_mode", "DELETE", |_| Ok(()))
        .map_err(failed)?;
    // A Checkpoint holds one commit's whole state, so it is never "unclean"
    // (ADR 0138): a Run at `off` resumes from it.
    sqlite_store::run_cached(&copy, "DELETE FROM unclean").map_err(failed)?;
    if let Some(folder) = source.parent() {
        run_files::pack(&copy, folder).map_err(|error| carry_failed(error, source))?;
    }
    let (epoch, position): (i64, i64) = copy
        .prepare_cached("SELECT epoch, journal_position FROM run")
        .and_then(|mut statement| statement.query_row([], |row| Ok((row.get(0)?, row.get(1)?))))
        .map_err(|error| {
            WriteError::Source(RunStateError::Sqlite {
                path: source.to_owned(),
                message: error.to_string(),
            })
        })?;
    copy.close().map_err(|(_, error)| failed(error))?;
    Ok((
        u64::try_from(epoch).unwrap_or_default(),
        u64::try_from(position).unwrap_or_default(),
    ))
}

/// Syncs the file at `path` and answers its size and SHA-256.
fn sync_and_digest(path: &Path) -> Result<(u64, String), WriteError> {
    let mut file = File::options()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| storage(&error, "sync the Checkpoint"))?;
    file.sync_all()
        .map_err(|error| storage(&error, "sync the Checkpoint"))?;
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| storage(&error, "read the Checkpoint back"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += u64::try_from(read).unwrap_or(u64::MAX);
    }
    Ok((size, format!("{:x}", hasher.finalize())))
}

/// A hidden temporary file beside a destination, `.<name>.<nonce>.partial`,
/// removed unless it is persisted.
struct Temporary {
    path: PathBuf,
    persisted: bool,
}

impl Temporary {
    fn create(destination: &Path, nonce: u64) -> Result<Self, WriteError> {
        let folder = destination.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(folder).map_err(|error| storage(&error, "make its folder"))?;
        let name = destination
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let path = folder.join(format!(".{name}.{nonce:016x}.partial"));
        File::create_new(&path)
            .and_then(|mut file| file.flush())
            .map_err(|error| storage(&error, "create the file"))?;
        Ok(Self {
            path,
            persisted: false,
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    /// Renames it over `destination` and syncs the folder, so the rename
    /// survives a power cut.
    fn persist(mut self, destination: &Path) -> Result<(), WriteError> {
        std::fs::rename(&self.path, destination)
            .map_err(|error| storage(&error, "rename it into place"))?;
        self.persisted = true;
        durable::sync_folder(destination.parent().unwrap_or(Path::new(".")));
        Ok(())
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.persisted {
            let _ = std::fs::remove_file(&self.path);
            let mut journal = self.path.as_os_str().to_owned();
            journal.push("-journal");
            let _ = std::fs::remove_file(PathBuf::from(journal));
        }
    }
}

fn storage(error: &io::Error, doing: &str) -> WriteError {
    WriteError::Storage {
        reason: run_files::reason_of(error.kind()),
        os_code: error.raw_os_error().and_then(run_files::os_code),
        message: format!("could not {doing}: {error}"),
    }
}

fn sqlite_storage(error: &rusqlite::Error) -> WriteError {
    let reason = match error.sqlite_error_code() {
        Some(ErrorCode::DiskFull) => "storage_full",
        Some(ErrorCode::ReadOnly) => "read_only_filesystem",
        Some(ErrorCode::PermissionDenied) => "permission_denied",
        Some(ErrorCode::TooBig) => "file_too_large",
        _ => "other",
    };
    WriteError::Storage {
        reason,
        os_code: None,
        message: format!("could not write the Checkpoint: {error}"),
    }
}

#[cfg(test)]
mod tests;
