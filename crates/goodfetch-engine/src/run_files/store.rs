//! The files themselves, in the Run's folder, and their records in its run
//! state. Its calls block: run them under the owner's blocking policy.

use std::io::{self, Read, Write as _};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension as _};
use sha2::{Digest as _, Sha256};

use crate::run_state::RunStateError;

/// The folder of a Run's kept files, inside its folder.
const FILES: &str = "files";

/// The folder of a Run's Exports, inside its folder: files no record of
/// its run state names, each named by its SHA-256 (ADR 0116).
const EXPORTS: &str = "exports";

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

/// An Export a Run keeps, as `Runs::keep_export` answers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptExport {
    /// The SHA-256 of its bytes, in lowercase hexadecimal, which names it.
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
    is_digest(sha256).then(|| folder.join(FILES).join(sha256))
}

/// The Export whose bytes' SHA-256 is `sha256` in the Run folder `folder`,
/// or `None` when `sha256` is no SHA-256 in lowercase hex.
pub fn export_file(folder: &Path, sha256: &str) -> Option<PathBuf> {
    is_digest(sha256).then(|| folder.join(EXPORTS).join(sha256))
}

/// Whether `sha256` is a SHA-256 in lowercase hexadecimal.
fn is_digest(sha256: &str) -> bool {
    sha256.len() == 64
        && sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// A file's bytes written whole and synced under a hidden name in a Run
/// folder's kept files, not yet under the SHA-256 that names them: its
/// record's write places it, so that a [`forget`] of the same bytes
/// ordered before the record can never delete the file the record names.
/// Dropped unplaced, it leaves nothing.
#[derive(Debug)]
pub(crate) struct Staged {
    partial: tempfile::NamedTempFile,
    files: PathBuf,
    /// The SHA-256 of its bytes, in lowercase hex.
    pub(crate) sha256: String,
    /// Its size, in bytes.
    pub(crate) size: u64,
}

impl Staged {
    /// Renames it to the SHA-256 of its bytes, over a file of the same
    /// bytes when one is there. Run it in the writer's order, in the write
    /// that records it: a [`forget`] ordered before has then deleted only
    /// the file it replaces.
    ///
    /// # Errors
    ///
    /// When the rename fails; the hidden file then goes.
    pub(crate) fn place(self) -> io::Result<()> {
        let kept = self.files.join(&self.sha256);
        self.partial.persist(&kept).map_err(|error| error.error)?;
        crate::durable::sync_folder(&self.files);
        Ok(())
    }

    /// [`Staged::place`], answering its SHA-256 and size.
    fn placed(self) -> io::Result<(String, u64)> {
        let (sha256, size) = (self.sha256.clone(), self.size);
        self.place()?;
        Ok((sha256, size))
    }
}

/// Writes what `source` reads as a kept file of the Run folder `folder`,
/// whole and synced, for its record's write to place.
///
/// # Errors
///
/// When `source` cannot be read or the file cannot be written; no partial
/// file is left.
pub(crate) fn stage(folder: &Path, source: &mut dyn Read) -> io::Result<Staged> {
    stage_in(&folder.join(FILES), source)
}

/// Writes what `source` reads as a kept file of the Run folder `folder`
/// and places it at once, answering its SHA-256 and size: for a write the
/// writer runs, whose record commits in the same order as any forget.
///
/// # Errors
///
/// As [`stage`]'s and [`Staged::place`]'s.
pub(crate) fn store(folder: &Path, source: &mut dyn Read) -> io::Result<(String, u64)> {
    stage(folder, source)?.placed()
}

/// Writes what `source` reads as an Export of the Run folder `folder`,
/// whole before its name appears, under `exports/`, and answers its
/// SHA-256 and size. No record names an Export, so nothing forgets it.
///
/// # Errors
///
/// When `source` cannot be read or the file cannot be written; no partial
/// file is left under its name.
pub(crate) fn store_export(folder: &Path, source: &mut dyn Read) -> io::Result<(String, u64)> {
    stage_in(&folder.join(EXPORTS), source)?.placed()
}

/// Writes what `source` reads into the folder `files`, through a hidden
/// temporary file, synced, which [`Staged::place`] renames to the SHA-256
/// of its bytes.
fn stage_in(files: &Path, source: &mut dyn Read) -> io::Result<Staged> {
    std::fs::DirBuilder::new().recursive(true).create(files)?;
    let mut partial = tempfile::Builder::new()
        .prefix(".")
        .suffix(".partial")
        .tempfile_in(files)?;
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
        // Through the raw file, whose error keeps its OS code; the
        // temporary file's own writer wraps it with its path.
        partial.as_file_mut().write_all(&buffer[..read])?;
        size += u64::try_from(read).unwrap_or(u64::MAX);
    }
    partial.as_file().sync_all()?;
    Ok(Staged {
        partial,
        files: files.to_owned(),
        sha256: data_encoding::HEXLOWER.encode(&hasher.finalize()),
        size,
    })
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
    connection
        .prepare_cached("INSERT INTO run_files (kind, name, sha256, size) VALUES (?1, ?2, ?3, ?4)")?
        .execute((
            kind.as_str(),
            name,
            sha256,
            i64::try_from(size).unwrap_or(i64::MAX),
        ))?;
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
        .prepare_cached("SELECT kind, name, sha256, size FROM run_files WHERE position = ?1")?
        .query_row([position], |row| {
            Ok(Record {
                kind: row.get(0)?,
                name: row.get(1)?,
                sha256: row.get(2)?,
                size: u64::try_from(row.get::<_, i64>(3)?).unwrap_or_default(),
            })
        })
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
        .prepare_cached("DELETE FROM run_files WHERE position = ?1 RETURNING sha256")?
        .query_row([position], |row| row.get(0))
        .optional()
}

/// Deletes the kept file whose SHA-256 is `sha256` from the Run folder
/// `folder` once no record of the run state `connection` holds names it:
/// files are content-addressed, so another record may share it. Run it in
/// the writer's order, after the commit that removed the last record, so
/// that a record of the same bytes written beside it, which places its
/// [`Staged`] file in the same write, keeps the file. A file that cannot be deleted is
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
    let named: bool = connection
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM run_files WHERE sha256 = ?1)")?
        .query_row([sha256], |row| row.get(0))?;
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
