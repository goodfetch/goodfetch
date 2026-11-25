//! The Accounts store (ADR 0149): one SQLite file per user and major,
//! beside the Operator policy, which only the Daemon of its major opens,
//! once, as it starts, when it has accounts. It holds the person's one
//! Account, its passkeys' public keys and the SHA-256 of each of its
//! recovery codes and Named tokens: what checks a credential, never a value
//! that signs anyone in, and no key file sits beside it. A recovery code or
//! a Named token is drawn from the OS generator with at least 128 bits, so
//! one SHA-256 of it is as hard to reverse as the value is to guess, and
//! checking one needs no key of its own.
//!
//! It keeps ADR 0030's default version rule: an engine reads its own major
//! up to its own minor. Its one connection is opened with every statement
//! prepared, so no call prepares one (ADR 0140), and its calls block:
//! callers run them under their owner's blocking policy.
#![expect(
    clippy::disallowed_types,
    reason = "one Mutex guards the store's one connection for the length of one blocking call, with nothing else under it and never across an `.await`, so there is no lock order for loom to test (ADR 0060)"
)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use goodfetch_protocol::accounts::StoredAccount;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::{ACCOUNTS_STORE, Compatibility};
use jiff::Timestamp;
use rusqlite::{Connection, OptionalExtension as _};
use secrecy::{ExposeSecret as _, SecretString};
use sha2::{Digest as _, Sha256};

use crate::home::Home;
use crate::sqlite_store::{
    self, Access, Durability, File, Folder, OpenError, Opening, Schema, Wait,
};

/// How long a write waits for another connection's, as a Daemon handing
/// over to the next one's.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The tables of this version. A newer minor only adds tables and columns,
/// each column with a default.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS account (
        id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
        name TEXT NOT NULL,
        created TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS passkeys (
        credential_id TEXT NOT NULL PRIMARY KEY,
        public_key TEXT NOT NULL,
        added TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS recovery_codes (
        sha256 TEXT NOT NULL PRIMARY KEY,
        used TEXT
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS named_tokens (
        name TEXT NOT NULL PRIMARY KEY,
        sha256 TEXT NOT NULL UNIQUE,
        created TEXT NOT NULL
    ) STRICT",
];

const ACCOUNT: &str = "SELECT name, created FROM account WHERE id = 1";
const MAKE_ACCOUNT: &str =
    "INSERT INTO account (id, name, created) VALUES (1, ?1, ?2) ON CONFLICT (id) DO NOTHING";
const ADD_PASSKEY: &str =
    "INSERT INTO passkeys (credential_id, public_key, added) VALUES (?1, ?2, ?3)";
const PASSKEY: &str = "SELECT public_key FROM passkeys WHERE credential_id = ?1";
const FORGET_RECOVERY_CODES: &str = "DELETE FROM recovery_codes";
const ADD_RECOVERY_CODE: &str =
    "INSERT INTO recovery_codes (sha256) VALUES (?1) ON CONFLICT (sha256) DO NOTHING";
const USE_RECOVERY_CODE: &str =
    "UPDATE recovery_codes SET used = ?2 WHERE sha256 = ?1 AND used IS NULL";
const ADD_NAMED_TOKEN: &str =
    "INSERT INTO named_tokens (name, sha256, created) VALUES (?1, ?2, ?3)";
const NAMED_TOKEN: &str = "SELECT name FROM named_tokens WHERE sha256 = ?1";
const ROLLBACK: &str = "ROLLBACK";

/// Every statement the store runs but the `BEGIN` and `COMMIT` of its
/// transactions, each prepared as it opens.
const STATEMENTS: &[&str] = &[
    ACCOUNT,
    MAKE_ACCOUNT,
    ADD_PASSKEY,
    PASSKEY,
    FORGET_RECOVERY_CODES,
    ADD_RECOVERY_CODE,
    USE_RECOVERY_CODE,
    ADD_NAMED_TOKEN,
    NAMED_TOKEN,
    ROLLBACK,
];

/// Why the Accounts store could not be opened, read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AccountsStoreError {
    /// SQLite or the file system failed; a write committed nothing.
    #[error("the Accounts store at `{path}`: {message}")]
    Sqlite {
        /// The store's file.
        path: PathBuf,
        /// What failed.
        message: String,
    },
    /// The store is of a version this engine cannot read (ADR 0030).
    #[error("the Accounts store is {found}, which this engine cannot read")]
    Format {
        /// The store's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
}

/// The Accounts store, open.
#[derive(Debug)]
pub struct AccountsStore {
    path: PathBuf,
    connection: Mutex<Connection>,
}

impl AccountsStore {
    /// Opens the store of `home`'s major of `version`, creating it when it
    /// does not exist.
    ///
    /// # Errors
    ///
    /// As [`AccountsStore::open`].
    pub fn of(home: &Home, version: &ProductVersion) -> Result<Self, AccountsStoreError> {
        Self::open(&home.accounts_store(version), None)
    }

    /// Opens the store at `path`, through the SQLite VFS named `vfs` (none
    /// in production, ADR 0060), creating it and its folder when they do
    /// not exist, and prepares every statement it runs.
    ///
    /// # Errors
    ///
    /// `Format` for a store of a version this engine cannot read, `Sqlite`
    /// when it cannot be opened.
    pub fn open(path: &Path, vfs: Option<&str>) -> Result<Self, AccountsStoreError> {
        let failed = |error: &dyn std::fmt::Display| sqlite_error(path, error);
        let connection = File::new(path, vfs)
            .open(&Opening {
                folder: Folder::Make,
                access: Access::Create,
                wait: Wait::Busy(BUSY_TIMEOUT),
                durability: Durability::SYNCED,
                counting: None,
                schema: Some(Schema {
                    format: &ACCOUNTS_STORE,
                    statements: &[SCHEMA],
                }),
            })
            .map_err(|error| match error {
                OpenError::Format(refusal) => AccountsStoreError::Format {
                    found: refusal.found,
                    compatibility: refusal.compatibility,
                },
                other => failed(&other),
            })?;
        // A forgotten recovery code's or Named token's page is zeroed, so
        // the file keeps no hash its Account no longer holds.
        connection
            .pragma_update(None, "secure_delete", "ON")
            .map_err(|error| failed(&error))?;
        // `BEGIN` and `COMMIT` are the two beside `STATEMENTS`.
        connection.set_prepared_statement_cache_capacity(STATEMENTS.len() + 2);
        sqlite_store::prepare_immediate(&connection, STATEMENTS).map_err(|error| failed(&error))?;
        Ok(Self {
            path: path.to_owned(),
            connection: Mutex::new(connection),
        })
    }

    /// Its file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Its Account, if its person made one.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be read.
    pub fn account(&self) -> Result<Option<StoredAccount>, AccountsStoreError> {
        self.on(|connection| {
            connection
                .prepare_cached(ACCOUNT)?
                .query_row([], |row| {
                    Ok(StoredAccount {
                        name: row.get(0)?,
                        created: row.get(1)?,
                    })
                })
                .optional()
        })
    }

    /// Makes its Account, named `name`, at `now`, unless it holds one:
    /// whether it made it.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be written.
    pub fn make_account(&self, name: &str, now: Timestamp) -> Result<bool, AccountsStoreError> {
        self.on(|connection| {
            let made = connection
                .prepare_cached(MAKE_ACCOUNT)?
                .execute((name, now.to_string()))?;
            Ok(made == 1)
        })
    }

    /// Adds the passkey `credential_id`, whose COSE public key is
    /// `public_key`, at `now`.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be written, as when it holds that passkey
    /// already.
    pub fn add_passkey(
        &self,
        credential_id: &[u8],
        public_key: &[u8],
        now: Timestamp,
    ) -> Result<(), AccountsStoreError> {
        self.on(|connection| {
            connection.prepare_cached(ADD_PASSKEY)?.execute((
                BASE64URL_NOPAD.encode(credential_id),
                BASE64URL_NOPAD.encode(public_key),
                now.to_string(),
            ))?;
            Ok(())
        })
    }

    /// The COSE public key of the passkey `credential_id`, if it holds it.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be read, or the key it holds is no base64url.
    pub fn passkey(&self, credential_id: &[u8]) -> Result<Option<Vec<u8>>, AccountsStoreError> {
        let stored: Option<String> = self.on(|connection| {
            connection
                .prepare_cached(PASSKEY)?
                .query_row([BASE64URL_NOPAD.encode(credential_id)], |row| row.get(0))
                .optional()
        })?;
        stored
            .map(|key| BASE64URL_NOPAD.decode(key.as_bytes()))
            .transpose()
            .map_err(|error| sqlite_error(&self.path, &error))
    }

    /// Replaces its recovery codes with `codes`, each kept as its hash.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be written; it then keeps the codes it held.
    pub fn replace_recovery_codes(&self, codes: &[SecretString]) -> Result<(), AccountsStoreError> {
        self.on(|connection| {
            sqlite_store::immediate(connection, |connection| {
                connection
                    .prepare_cached(FORGET_RECOVERY_CODES)?
                    .execute([])?;
                let mut add = connection.prepare_cached(ADD_RECOVERY_CODE)?;
                for code in codes {
                    add.execute([hash(code)])?;
                }
                Ok(())
            })
        })
    }

    /// Uses the recovery code `code` at `now`: whether it is one of its
    /// codes, unused until now. A code is used once.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be written.
    pub fn use_recovery_code(
        &self,
        code: &SecretString,
        now: Timestamp,
    ) -> Result<bool, AccountsStoreError> {
        self.on(|connection| {
            let used = connection
                .prepare_cached(USE_RECOVERY_CODE)?
                .execute((hash(code), now.to_string()))?;
            Ok(used == 1)
        })
    }

    /// Adds the Named token `name`, whose value is `value`, kept as its
    /// hash, at `now`.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be written, as when it holds a token of that
    /// name already.
    pub fn add_named_token(
        &self,
        name: &str,
        value: &SecretString,
        now: Timestamp,
    ) -> Result<(), AccountsStoreError> {
        self.on(|connection| {
            connection.prepare_cached(ADD_NAMED_TOKEN)?.execute((
                name,
                hash(value),
                now.to_string(),
            ))?;
            Ok(())
        })
    }

    /// The name of its Named token whose value is `value`, if it holds one.
    ///
    /// # Errors
    ///
    /// `Sqlite` when it cannot be read.
    pub fn named_token(&self, value: &SecretString) -> Result<Option<String>, AccountsStoreError> {
        self.on(|connection| {
            connection
                .prepare_cached(NAMED_TOKEN)?
                .query_row([hash(value)], |row| row.get(0))
                .optional()
        })
    }

    /// Runs `work` on its connection, held for the length of the call.
    fn on<T>(
        &self,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, AccountsStoreError> {
        let connection = self.connection();
        work(&connection).map_err(|error| sqlite_error(&self.path, &error))
    }

    /// Its connection. One a call that panicked held is taken back, its
    /// transaction, if one was left open, rolled back.
    fn connection(&self) -> MutexGuard<'_, Connection> {
        self.connection.lock().unwrap_or_else(|poisoned| {
            let connection = poisoned.into_inner();
            if !connection.is_autocommit() {
                let _ = sqlite_store::run_cached(&connection, ROLLBACK);
            }
            connection
        })
    }
}

/// What the store keeps of a recovery code or a Named token: its SHA-256,
/// in lowercase hex.
fn hash(secret: &SecretString) -> String {
    HEXLOWER.encode(&Sha256::digest(secret.expose_secret().as_bytes()))
}

fn sqlite_error(path: &Path, error: &dyn std::fmt::Display) -> AccountsStoreError {
    AccountsStoreError::Sqlite {
        path: path.to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests;
