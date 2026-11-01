//! The accounts store (spec #592): one SQLite file per user and major,
//! beside the Operator policy, which only the Daemon of its major opens. It
//! keeps the Console's sign-ins, each by its identifier with a verifier of
//! its cookie, never the cookie, its person, when it was created and last
//! used, and its wrapping key. It keeps ADR 0030's default version rule:
//! an engine reads its own major up to its own minor. Its calls block:
//! callers run them under their owner's blocking policy.

use std::path::{Path, PathBuf};
use std::time::Duration;

use goodfetch_protocol::formats::{ACCOUNTS_STORE, Compatibility};
use goodfetch_protocol::people::SignInRow;
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, TransactionBehavior};

/// How long a write waits for another connection's.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The tables of this version. A newer minor only adds tables and columns,
/// each column with a default.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT",
    "CREATE TABLE IF NOT EXISTS sign_ins (
        id TEXT NOT NULL PRIMARY KEY,
        verifier TEXT NOT NULL,
        person TEXT NOT NULL,
        created TEXT NOT NULL,
        last_used TEXT NOT NULL,
        wrapping_key TEXT NOT NULL
    ) STRICT",
];

/// Why the accounts store could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// SQLite or the file system failed; the write committed nothing.
    #[error("the accounts store at `{path}`: {message}")]
    Sqlite {
        /// The store's file.
        path: PathBuf,
        /// What failed.
        message: String,
    },
    /// The store is of a version this engine cannot read (ADR 0030).
    #[error("the accounts store is {found}, which this engine cannot read")]
    Format {
        /// The store's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
}

/// The accounts store at one path.
#[derive(Clone, Debug)]
pub struct AccountsStore {
    path: PathBuf,
}

impl AccountsStore {
    /// The store at `path`, created when first written.
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Its file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Adds `row`.
    pub fn insert(&self, row: &SignInRow) -> Result<(), StoreError> {
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO sign_ins (id, verifier, person, created, last_used, wrapping_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    &row.id,
                    &row.verifier,
                    &row.person,
                    &row.created,
                    &row.last_used,
                    &row.wrapping_key,
                ),
            )
            .map_err(|error| self.failed(&error))?;
        Ok(())
    }

    /// The sign-in `id`, if the store holds it. A store never written holds
    /// none, and is not created.
    pub fn find(&self, id: &str) -> Result<Option<SignInRow>, StoreError> {
        if !self.path.exists() {
            return Ok(None);
        }
        let connection = self.open()?;
        connection
            .query_row(
                "SELECT id, verifier, person, created, last_used, wrapping_key
                 FROM sign_ins WHERE id = ?1",
                [id],
                row,
            )
            .optional()
            .map_err(|error| self.failed(&error))
    }

    /// Every sign-in, oldest first.
    pub fn all(&self) -> Result<Vec<SignInRow>, StoreError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let connection = self.open()?;
        let mut statement = connection
            .prepare(
                "SELECT id, verifier, person, created, last_used, wrapping_key
                 FROM sign_ins ORDER BY created, id",
            )
            .map_err(|error| self.failed(&error))?;
        statement
            .query_map([], row)
            .and_then(Iterator::collect)
            .map_err(|error| self.failed(&error))
    }

    /// Records that `id` was used at `last_used`.
    pub fn touch(&self, id: &str, last_used: &str) -> Result<(), StoreError> {
        let connection = self.open()?;
        connection
            .execute(
                "UPDATE sign_ins SET last_used = ?2 WHERE id = ?1",
                (id, last_used),
            )
            .map_err(|error| self.failed(&error))?;
        Ok(())
    }

    /// Removes the sign-in `id`: whether the store held it.
    pub fn remove(&self, id: &str) -> Result<bool, StoreError> {
        if !self.path.exists() {
            return Ok(false);
        }
        let connection = self.open()?;
        connection
            .execute("DELETE FROM sign_ins WHERE id = ?1", [id])
            .map(|removed| removed > 0)
            .map_err(|error| self.failed(&error))
    }

    /// Removes every sign-in: how many it held.
    pub fn clear(&self) -> Result<u64, StoreError> {
        if !self.path.exists() {
            return Ok(0);
        }
        let connection = self.open()?;
        connection
            .execute("DELETE FROM sign_ins", [])
            .map(|removed| u64::try_from(removed).unwrap_or(u64::MAX))
            .map_err(|error| self.failed(&error))
    }

    /// Opens the store, creating it and its tables when it does not exist,
    /// and refuses one of a version this engine cannot read.
    fn open(&self) -> Result<Connection, StoreError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        if let Some(folder) = self.path.parent() {
            #[expect(
                clippy::disallowed_methods,
                reason = "the accounts store's calls run under their caller's blocking policy"
            )]
            std::fs::create_dir_all(folder).map_err(|error| failed(&error))?;
        }
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut connection =
            Connection::open_with_flags(&self.path, flags).map_err(|error| failed(&error))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| failed(&error))?;
        crate::sqlite_wal::enter(&connection, BUSY_TIMEOUT).map_err(|error| failed(&error))?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed(&error))?;
        for statement in SCHEMA {
            transaction
                .execute(statement, [])
                .map_err(|error| failed(&error))?;
        }
        transaction
            .execute(
                "INSERT INTO format (name, version) VALUES (?1, ?2) ON CONFLICT (name) DO NOTHING",
                (ACCOUNTS_STORE.name, ACCOUNTS_STORE.version.to_string()),
            )
            .map_err(|error| failed(&error))?;
        let found: String = transaction
            .query_row(
                "SELECT version FROM format WHERE name = ?1",
                [ACCOUNTS_STORE.name],
                |row| row.get(0),
            )
            .map_err(|error| failed(&error))?;
        transaction.commit().map_err(|error| failed(&error))?;
        check_format(&found)?;
        Ok(connection)
    }

    fn failed(&self, error: &dyn std::fmt::Display) -> StoreError {
        StoreError::Sqlite {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }
}

/// A row of `sign_ins` as its schema's type.
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SignInRow> {
    Ok(SignInRow {
        id: row.get(0)?,
        verifier: row.get(1)?,
        person: row.get(2)?,
        created: row.get(3)?,
        last_used: row.get(4)?,
        wrapping_key: row.get(5)?,
    })
}

/// Refuses a store of a version the default rule does not let this
/// engine read.
fn check_format(found: &str) -> Result<(), StoreError> {
    let refused = |compatibility| StoreError::Format {
        found: found.to_owned(),
        compatibility,
    };
    let version = found
        .parse()
        .map_err(|_| refused(Compatibility::NewerMajor))?;
    match ACCOUNTS_STORE
        .rule
        .compatibility(ACCOUNTS_STORE.version, version)
    {
        Compatibility::Compatible => Ok(()),
        other => Err(refused(other)),
    }
}
