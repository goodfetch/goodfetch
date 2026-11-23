//! A Run's files the engine holds for the program (ADRs 0001, 0002, 0085,
//! 0116 and 0122): the files of its Downloads and its Key-value store's
//! bytes over 1 MiB now, and through Foxhound's
//! crates its Exports and WARC files. Each is kept content-addressed in the
//! Run's folder, under `files/`, named by the lowercase hex SHA-256 of its
//! bytes and written whole before its name appears, and recorded in a table
//! of the Run's run state, by its position in the Run, so a resume finds
//! it. A Checkpoint carries the bytes of every file its run state records,
//! in chunks of a table of its own, and the file a Checkpoint becomes on
//! another machine has them back in its Run's folder, so a kept file
//! survives a resume there too. A file is deleted with its Run, when the
//! Run is discarded, since it lives in the Run's folder, and before then
//! once no record of the Run names it, as when a key whose bytes it holds
//! is deleted or replaced.
//!
//! An Export is kept the same way, under the Run folder's `exports/`, in a
//! folder named by the run epoch of the state it was written from (ADR
//! 0003), but recorded in no table: it is written from the Run's committed
//! state, a Run that ended having no writer, so its ID names its file by
//! that run epoch and its SHA-256. It is deleted with its Run, and a Checkpoint does not carry it.
//!
//! A kept file reaches the program's path through [`save`], under ADR
//! 0116's rule: a hidden temporary file beside the path,
//! `.<name>.<random>.partial`, synced, then renamed over the path, so the
//! path holds the whole file or what it held before, and a failure says
//! whether the path (`path_not_writable`) or the storage
//! (`file_write_failed`) failed. [`Saving`] writes a path the same way
//! from bytes that arrive in parts, such as the `$/chunk` notifications of
//! an Export a CLI command receives from the Daemon.
//!
//! A live Run's [`RunFiles`] stores its page bodies and keeps its files
//! (ADR 0145).

mod carry;
mod live;
mod save;
mod store;

pub(crate) use carry::{CarryError, drop_chunks, pack, unpack};
pub use live::RunFiles;
pub use save::{SaveError, Saving};
pub(crate) use save::{os_code, reason_of, save};
pub use store::{FileKind, KeepError, Kept, KeptExport, Record, export_file};
pub(crate) use store::{file, find, forget, read, record, stage, store, store_export, unrecord};

use crate::run_state::RunStateTables;

/// The tables of a Run's files, which every run file holds: what the Run
/// keeps, and the chunks of their bytes, which only a Checkpoint holds.
pub const TABLES: RunStateTables = RunStateTables {
    name: "run-files",
    statements: &[
        "CREATE TABLE IF NOT EXISTS run_files (position INTEGER NOT NULL PRIMARY KEY, kind TEXT NOT NULL, name TEXT NOT NULL, sha256 TEXT NOT NULL, size INTEGER NOT NULL) STRICT",
        "CREATE TABLE IF NOT EXISTS run_file_chunks (sha256 TEXT NOT NULL, offset INTEGER NOT NULL, data BLOB NOT NULL, PRIMARY KEY (sha256, offset)) STRICT",
    ],
};
