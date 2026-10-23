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
}

impl FileKind {
    /// Its name in run state.
    fn as_str(self) -> &'static str {
        match self {
            Self::Download => "download",
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
