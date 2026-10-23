//! The files themselves, in the Run's folder, and their records in its run
//! state. Its calls block: run them under the owner's blocking policy.

use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension as _};
use sha2::{Digest as _, Sha256};

use crate::run_state::RunStateError;

/// The folder of a Run's kept files, inside its folder.
const FILES: &str = "files";

/// What a kept file is to the program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// A Download's file (ADR 0085).
    Download,
    /// Bytes over 1 MiB a Key-value store keeps (ADR 0103), named by their
    /// key.
    KeyValue,
}

impl FileKind {
    /// Its name in run state.
    fn as_str(self) -> &'static str {
        match self {
            Self::Download => "download",
            Self::KeyValue => "key_value",
        }
    }
}

/// A file the Run keeps, as `Run::keep_file` answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    /// Its position in the Run, from 1.
    pub position: u64,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    pub sha256: String,
    /// Its size in bytes.
    pub size: u64,
}

/// What a Run's run state records of a file it keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    /// Whether it is a Download's file.
    pub kind: String,
    /// The name it was kept under, such as a Download's suggested file
    /// name.
    pub name: String,
    /// The SHA-256 of its bytes, in lowercase hexadecimal.
    pub sha256: String,
    /// Its size in bytes.
    pub size: u64,
}

/// Why a file was not kept.
#[derive(Debug, thiserror::Error)]
pub enum KeepError {
    /// The Run's folder could not take it: nothing of it is left there.
    #[error("{message}")]
    Storage {
        /// Why, as `io::ErrorKind`'s name in `snake_case`.
        reason: &'static str,
        /// The OS's own name for it, when known.
        os_code: Option<&'static str>,
        /// What failed.
        message: String,
    },
    /// Its record could not be committed to the Run's state.
    #[error(transparent)]
    State(#[from] RunStateError),
}

impl KeepError {
    /// The failure of `doing` with `error`.
    pub(crate) fn io(error: &io::Error, doing: &str) -> Self {
        Self::Storage {
            reason: super::reason_of(error.kind()),
            os_code: error.raw_os_error().and_then(super::os_code),
            message: format!("could not {doing}: {error}"),
        }
    }
}

/// The file of the bytes whose SHA-256 is `sha256` in the Run folder
/// `folder`, or `None` when `sha256` is no SHA-256 in lowercase hex.
pub(crate) fn file(folder: &Path, sha256: &str) -> Option<PathBuf> {
    let is_digest = sha256.len() == 64
        && sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    is_digest.then(|| folder.join(FILES).join(sha256))
}

/// Writes what `source` reads as a file of the Run folder `folder`, whole
/// before its name appears, and answers its SHA-256 and size.
///
/// # Errors
///
/// When `source` cannot be read or the file cannot be written; no partial
/// file is left under its name.
pub(crate) fn store(folder: &Path, source: &mut dyn Read) -> io::Result<(String, u64)> {
    let files = folder.join(FILES);
    std::fs::DirBuilder::new().recursive(true).create(&files)?;
    let mut partial = tempfile::Builder::new()
        .prefix(".")
        .suffix(".partial")
        .tempfile_in(&files)?;
    let mut hasher = Sha256::new();
    let mut size: u64 = 0;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = match source.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&buffer[..read]);
        partial.write_all(&buffer[..read])?;
        size += u64::try_from(read).unwrap_or(u64::MAX);
    }
    partial.as_file().sync_all()?;
    let sha256 = data_encoding::HEXLOWER.encode(&hasher.finalize());
    let kept = file(folder, &sha256).expect("invariant: a SHA-256 in lowercase hex names a file");
    partial.persist(&kept).map_err(|error| error.error)?;
    super::save::sync_folder(&files);
    Ok((sha256, size))
}

/// Records the kept file `name`, of `kind`, in the run state `connection`
/// holds, and answers its position.
///
/// # Errors
///
/// SQLite's.
pub(crate) fn record(
    connection: &Connection,
    kind: FileKind,
    name: &str,
    sha256: &str,
    size: u64,
) -> rusqlite::Result<u64> {
    connection.execute(
        "INSERT INTO run_files (kind, name, sha256, size) VALUES (?1, ?2, ?3, ?4)",
        (
            kind.as_str(),
            name,
            sha256,
            i64::try_from(size).unwrap_or(i64::MAX),
        ),
    )?;
    Ok(u64::try_from(connection.last_insert_rowid()).unwrap_or_default())
}

/// What the run state `connection` holds records of the file at
/// `position`, if any.
///
/// # Errors
///
/// SQLite's.
pub(crate) fn find(connection: &Connection, position: u64) -> rusqlite::Result<Option<Record>> {
    let Ok(position) = i64::try_from(position) else {
        return Ok(None);
    };
    connection
        .query_row(
            "SELECT kind, name, sha256, size FROM run_files WHERE position = ?1",
            [position],
            |row| {
                Ok(Record {
                    kind: row.get(0)?,
                    name: row.get(1)?,
                    sha256: row.get(2)?,
                    size: u64::try_from(row.get::<_, i64>(3)?).unwrap_or_default(),
                })
            },
        )
        .optional()
}

/// Removes the record of the kept file at `position` from the run state
/// `connection` holds, and answers the SHA-256 it named, if any. The file
/// stays until [`forget`] finds no record naming it.
///
/// # Errors
///
/// SQLite's.
pub(crate) fn unrecord(connection: &Connection, position: u64) -> rusqlite::Result<Option<String>> {
    let Ok(position) = i64::try_from(position) else {
        return Ok(None);
    };
    connection
        .query_row(
            "DELETE FROM run_files WHERE position = ?1 RETURNING sha256",
            [position],
            |row| row.get(0),
        )
        .optional()
}

/// Deletes the kept file whose SHA-256 is `sha256` from the Run folder
/// `folder` once no record of the run state `connection` holds names it:
/// files are content-addressed, so another record may share it. Run it in
/// the writer's order, after the commit that removed the last record, so
/// that a record of the same bytes written beside it either keeps the file
/// or finds it gone and writes it again. A file that cannot be deleted is
/// left in the Run's folder, which a discard removes.
///
/// # Errors
///
/// SQLite's.
#[expect(
    clippy::disallowed_methods,
    reason = "it runs in the run state's writer, a blocking actor of its own"
)]
pub(crate) fn forget(connection: &Connection, folder: &Path, sha256: &str) -> rusqlite::Result<()> {
    let named: bool = connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM run_files WHERE sha256 = ?1)",
        [sha256],
        |row| row.get(0),
    )?;
    if let (false, Some(kept)) = (named, file(folder, sha256)) {
        match std::fs::remove_file(&kept) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(file = %kept.display(), %error, "a kept file no record names could not be deleted");
            }
        }
    }
    Ok(())
}

/// The bytes of the kept file whose SHA-256 is `sha256` in the Run folder
/// `folder`.
///
/// # Errors
///
/// When the file cannot be read: `NotFound` once it was forgotten, and
/// `InvalidInput` when `sha256` is no SHA-256 in lowercase hex.
#[expect(
    clippy::disallowed_methods,
    reason = "its callers run it under their owner's blocking policy"
)]
pub(crate) fn read(folder: &Path, sha256: &str) -> io::Result<Vec<u8>> {
    let kept = file(folder, sha256)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no SHA-256 names the file"))?;
    std::fs::read(kept)
}
