//! A domain crate's own store (ADRs 0060 and 0098): one per-user, per-major
//! SQLite file in the major's folder of Goodfetch home, beside the Run
//! index, which a contribution declares by a [`UserStoreKind`] and its
//! handlers reach through [`EngineContext::user_store`]. The engine opens
//! each through a VFS named by a value, none in production, as it opens its
//! own stores: its writes run on a writer, an actor of the engine's owner
//! that keeps its connection, each in one `IMMEDIATE` transaction, and its
//! reads on that owner's blocking threads, on one read-only connection kept
//! beside the writer's.
//!
//! The file records its format's version in its `format` table when it is
//! made, and an engine refuses one of a version its format's rule does not
//! let it read. Its statements only name what they carry, so a row or a
//! table this version does not know stays as it is.
//!
//! [`EngineContext::user_store`]: crate::engine_context::EngineContext::user_store

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_protocol::formats::{Compatibility, Format};
use rusqlite::Connection;

use crate::sqlite_store::{
    self, Access, Durability, File, Folder, FormatRefusal, KeptStore, OpenError, Opening, Schema,
    StoreFile, Wait,
};
use crate::tasks::{ActorStopped, TaskOwner};

/// How long a write waits for another engine's, and a read while SQLite
/// answers busy, as while a crashed writer's journal is recovered.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// A store a contribution declares: its file in the major's folder, its
/// format, the statements that make its tables, and its writer's name.
pub struct UserStoreKind {
    file: &'static str,
    format: &'static Format,
    tables: &'static [&'static str],
    statements: &'static [&'static str],
    writer: &'static str,
}

impl UserStoreKind {
    /// The store in the file `file` of the major's folder, of the format
    /// `format`, whose `statements` make its `tables` when they are not
    /// there (`CREATE TABLE IF NOT EXISTS`), written by the actor `writer`.
    pub const fn new(
        file: &'static str,
        format: &'static Format,
        tables: &'static [&'static str],
        statements: &'static [&'static str],
        writer: &'static str,
    ) -> Self {
        Self {
            file,
            format,
            tables,
            statements,
            writer,
        }
    }

    /// Its file's name in the major's folder.
    pub fn file(&self) -> &'static str {
        self.file
    }

    /// Its format.
    pub fn format(&self) -> &'static Format {
        self.format
    }
}

impl std::fmt::Debug for UserStoreKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserStoreKind")
            .field("file", &self.file)
            .field("format", &self.format.name)
            .finish_non_exhaustive()
    }
}

/// Why a store could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum UserStoreError {
    /// SQLite failed; a write committed nothing.
    #[error("the store at `{path}`: {message}")]
    Sqlite {
        /// The store's file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// The engine's writer of the store stopped with its owner.
    #[error("the store at `{path}` is not running: {message}")]
    NotRunning {
        /// The store's file.
        path: PathBuf,
        /// Why.
        message: String,
    },
    /// The store is of a version this engine cannot read (ADR 0030).
    #[error("the store `{format}` is {found}, which this engine cannot read")]
    Format {
        /// Its format's name.
        format: &'static str,
        /// The store's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
}

/// A store of one user, as one engine opens it: its writer and the
/// connection its reads keep, which its clones share.
#[derive(Clone, Debug)]
pub struct UserStore {
    path: Arc<Path>,
    kind: &'static UserStoreKind,
    store: Arc<KeptStore<UserStoreFile>>,
}

impl UserStore {
    /// The store `kind` at `path`, opened through the SQLite VFS named
    /// `vfs` (none in production), its writer and reads under `owner`;
    /// nothing is opened before its first call.
    pub fn open(
        path: PathBuf,
        kind: &'static UserStoreKind,
        vfs: Option<String>,
        owner: Arc<TaskOwner>,
    ) -> Self {
        let file = UserStoreFile {
            path: path.clone(),
            kind,
            vfs,
        };
        Self {
            path: Arc::from(path),
            kind,
            store: Arc::new(KeptStore::of(file, kind.writer, owner)),
        }
    }

    /// Where it lies.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What store it is.
    pub fn kind(&self) -> &'static UserStoreKind {
        self.kind
    }

    /// Runs `work` on the store's read connection and answers what it
    /// returned; `missing` while the file is not there, which a read does
    /// not create.
    ///
    /// # Errors
    ///
    /// SQLite's, or the store's when it cannot be opened or is of a version
    /// this engine cannot read.
    pub async fn read<T, W>(&self, missing: T, work: W) -> Result<T, UserStoreError>
    where
        T: Send + 'static,
        W: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.store
            .read(Some(missing), move |file, connection| {
                work(connection).map_err(|error| file.failed(&error))
            })
            .await
    }

    /// Runs `work` in one `IMMEDIATE` transaction on the writer's
    /// connection, the file and its tables made when they are not there,
    /// and answers what it returned once it committed. A `work` that fails
    /// commits nothing.
    ///
    /// # Errors
    ///
    /// As [`Self::read`], and once the engine's writer stopped.
    pub async fn write<T, W>(&self, work: W) -> Result<T, UserStoreError>
    where
        T: Send + 'static,
        W: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        self.store
            .write(None, move |file, connection| {
                sqlite_store::immediate(connection, work).map_err(|error| file.failed(&error))
            })
            .await
    }
}

/// A store's file, its kind and the VFS it opens through.
#[derive(Debug)]
struct UserStoreFile {
    path: PathBuf,
    kind: &'static UserStoreKind,
    vfs: Option<String>,
}

impl StoreFile for UserStoreFile {
    type Error = UserStoreError;

    fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the file is there, asked of its VFS when it has one, whose
    /// files need not lie on the disk; an error other than its absence
    /// counts it there, so that opening it reports that error.
    fn exists(&self) -> bool {
        match self.vfs.as_deref() {
            None => self.path.exists(),
            Some(vfs) => !matches!(
                File::new(&self.path, Some(vfs)).connect_existing(Access::ReadOnly),
                Ok(None)
            ),
        }
    }

    /// Opens the store, creating it and its tables when it does not exist,
    /// and refuses one of a version this engine cannot read.
    fn open(&self) -> Result<Connection, UserStoreError> {
        File::new(&self.path, self.vfs.as_deref())
            .open(&Opening {
                folder: Folder::Make,
                access: Access::Create,
                wait: Wait::Busy(BUSY_TIMEOUT),
                durability: Durability::SYNCED,
                counting: None,
                schema: Some(Schema {
                    format: self.kind.format,
                    statements: &[self.kind.statements],
                }),
            })
            .map_err(|error| match error {
                OpenError::Format(refusal) => self.refused(refusal),
                other => self.failed(&other),
            })
    }

    /// Opens the existing store for its reads, refusing one of a version
    /// this engine cannot read: read-only, so it takes no write lock. A
    /// store whose tables of this version are not all there yet has them
    /// made, through [`StoreFile::open`], whose connection then reads.
    fn open_read(&self) -> Result<Connection, UserStoreError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let connection = File::new(&self.path, self.vfs.as_deref())
            .connect(Access::ReadOnly)
            .map_err(|error| failed(&error))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| failed(&error))?;
        let mut present = 0_usize;
        for table in std::iter::once(&"format").chain(self.kind.tables) {
            let found: i64 = connection
                .prepare_cached(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                )
                .and_then(|mut statement| statement.query_row([table], |row| row.get(0)))
                .map_err(|error| failed(&error))?;
            present += usize::from(found > 0);
        }
        if present <= self.kind.tables.len() {
            drop(connection);
            return self.open();
        }
        let found: String = connection
            .prepare_cached("SELECT version FROM format WHERE name = ?1")
            .and_then(|mut statement| {
                statement.query_row([self.kind.format.name], |row| row.get(0))
            })
            .map_err(|error| failed(&error))?;
        sqlite_store::check_format(self.kind.format, &found)
            .map_err(|refusal| self.refused(refusal))?;
        Ok(connection)
    }

    fn stopped(&self, stopped: &ActorStopped) -> UserStoreError {
        UserStoreError::NotRunning {
            path: self.path.clone(),
            message: stopped.to_string(),
        }
    }
}

impl UserStoreFile {
    fn failed(&self, error: &dyn std::fmt::Display) -> UserStoreError {
        UserStoreError::Sqlite {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }

    fn refused(&self, refusal: FormatRefusal) -> UserStoreError {
        UserStoreError::Format {
            format: self.kind.format.name,
            found: refusal.found,
            compatibility: refusal.compatibility,
        }
    }
}
