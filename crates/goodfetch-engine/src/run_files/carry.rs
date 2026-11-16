//! The kept files a Checkpoint carries (ADRs 0002 and 0085): a Checkpoint
//! is one SQLite file, so the bytes of every file its run state records go
//! into its `run_file_chunks` table, a chunk of at most 1 MiB a row and an
//! empty file one empty chunk, and
//! the run file a Checkpoint becomes has them written back into its Run's
//! folder, each checked against its SHA-256, then dropped from the run
//! file, whose live copy never holds them. Its calls block: run them under
//! the owner's blocking policy.
#![expect(
    clippy::disallowed_methods,
    reason = "its calls block: their callers run them under their owner's blocking policy"
)]

use std::io::{self, Read as _, Write as _};
use std::path::Path;

use rusqlite::{Connection, OptionalExtension as _};
use sha2::{Digest as _, Sha256};

use super::store::file;
use crate::durable::sync_folder;

/// The most bytes one chunk row holds.
const CHUNK: usize = 1024 * 1024;

/// Why the kept files were not carried.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CarryError {
    /// A kept file could not be read or written.
    #[error("a kept file could not be carried: {0}")]
    Io(#[from] io::Error),
    /// The SQLite file could not be read or written.
    #[error("the kept files' chunks could not be carried: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// A Checkpoint's chunks are not a kept file's bytes.
    #[error("the Checkpoint's kept files are damaged: {0}")]
    Damaged(String),
}

/// Copies into the Checkpoint `copy` the bytes of every file its run state
/// records, from the Run folder `folder`.
///
/// # Errors
///
/// When a recorded file cannot be read or the chunks cannot be written.
pub(crate) fn pack(copy: &Connection, folder: &Path) -> Result<(), CarryError> {
    if !has_table(copy, "run_files")? {
        return Ok(());
    }
    let digests: Vec<String> = copy
        .prepare("SELECT DISTINCT sha256 FROM run_files ORDER BY sha256")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut insert =
        copy.prepare("INSERT INTO run_file_chunks (sha256, offset, data) VALUES (?1, ?2, ?3)")?;
    let mut chunk = vec![0_u8; CHUNK];
    for sha256 in digests {
        let path = file(folder, &sha256)
            .ok_or_else(|| CarryError::Damaged(format!("`{sha256}` is no SHA-256")))?;
        let mut kept = std::fs::File::open(&path)?;
        let mut offset: i64 = 0;
        loop {
            let read = fill(&mut kept, &mut chunk)?;
            // An empty file still takes one chunk, so that it is carried.
            if read == 0 && offset > 0 {
                break;
            }
            insert.execute((&sha256, offset, &chunk[..read]))?;
            if read < chunk.len() {
                break;
            }
            offset += i64::try_from(read).unwrap_or(i64::MAX);
        }
    }
    Ok(())
}

/// Writes into the Run folder `folder` every kept file whose chunks the
/// Checkpoint `checkpoint` holds, checking each against its SHA-256; one
/// already there is left as it is.
///
/// # Errors
///
/// `Damaged` when the chunks are not whole or do not hash to their name,
/// and when a file cannot be written; no partial file is left under a
/// kept file's name.
pub(crate) fn unpack(checkpoint: &Connection, folder: &Path) -> Result<(), CarryError> {
    if !has_table(checkpoint, "run_file_chunks")? {
        return Ok(());
    }
    let digests: Vec<String> = checkpoint
        .prepare("SELECT DISTINCT sha256 FROM run_file_chunks ORDER BY sha256")?
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let mut chunks = checkpoint
        .prepare("SELECT offset, data FROM run_file_chunks WHERE sha256 = ?1 ORDER BY offset")?;
    for sha256 in digests {
        let target = file(folder, &sha256)
            .ok_or_else(|| CarryError::Damaged(format!("`{sha256}` is no SHA-256")))?;
        if std::fs::exists(&target)? {
            continue;
        }
        let files = target
            .parent()
            .expect("invariant: a kept file is inside its folder");
        std::fs::create_dir_all(files)?;
        let mut partial = tempfile::Builder::new()
            .prefix(".")
            .suffix(".partial")
            .tempfile_in(files)?;
        let mut hasher = Sha256::new();
        let mut size: i64 = 0;
        let mut rows = chunks.query([&sha256])?;
        while let Some(row) = rows.next()? {
            let offset: i64 = row.get(0)?;
            let data: Vec<u8> = row.get(1)?;
            if offset != size {
                return Err(CarryError::Damaged(format!(
                    "the chunks of `{sha256}` skip from {size} to {offset}"
                )));
            }
            hasher.update(&data);
            partial.write_all(&data)?;
            size += i64::try_from(data.len()).unwrap_or(i64::MAX);
        }
        if data_encoding::HEXLOWER.encode(&hasher.finalize()) != sha256 {
            return Err(CarryError::Damaged(format!(
                "the chunks named `{sha256}` hash to another SHA-256"
            )));
        }
        partial.as_file().sync_all()?;
        partial.persist(&target).map_err(|error| error.error)?;
        sync_folder(files);
    }
    Ok(())
}

/// Drops the kept files' chunks from the run file `connection` holds, once
/// its folder has them back.
///
/// # Errors
///
/// SQLite's.
pub(crate) fn drop_chunks(connection: &Connection) -> rusqlite::Result<()> {
    if has_table(connection, "run_file_chunks")? {
        crate::sqlite_store::run_cached(connection, "DELETE FROM run_file_chunks")?;
    }
    Ok(())
}

/// Whether `connection`'s database has the table `name`.
fn has_table(connection: &Connection, name: &str) -> rusqlite::Result<bool> {
    Ok(connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1")?
        .query_row([name], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Fills `chunk` from `file` as far as it goes, and answers how many bytes
/// it read: fewer than its length only at the file's end.
fn fill(file: &mut std::fs::File, chunk: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < chunk.len() {
        match file.read(&mut chunk[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}
