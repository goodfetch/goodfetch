//! Encrypted per-install, per-major Saved identity store (ADRs 0015, 0035,
//! 0056, 0089 and 0098; ticket #736).
//!
//! Size: one cohesive store concern (schema, OS writer lock, per-identity AEAD
//! encryption, and CRUD/expiry queries for `saved-identities.sqlite`).
//!
//! Each Product major has one SQLite store (`saved-identities.sqlite`) shared
//! by both Collie and Foxhound, recording the lowest minor of its major that
//! can use it (`lowest_minor::Opener`). Every secret field (cookies,
//! `localStorage`, `IndexedDB`, OPFS) is sealed with XChaCha20-Poly1305 under
//! a per-identity random 256-bit data key, wrapped under HKDF-SHA256 of the
//! key-encryption key that `run_state.key_reference` names. `PRAGMA
//! secure_delete = ON` zeroes freed pages on replacement or deletion, files
//! are restricted to mode `0600` on Unix so other OS users cannot read them,
//! and writes serialize under an install-wide exclusive file lock
//! (`saved-identities.lock`) beside the store.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use chacha20poly1305::aead::{Aead as _, KeyInit as _, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use goodfetch_protocol::catalog::{ProtocolVersion, schema_of};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::{Compatibility, Format, Rule, Storage};
use goodfetch_protocol::page::identity::SavedIdentityInfo;
use goodfetch_protocol::session_export::SavedProxyBinding;
use hkdf::Hkdf;
use rusqlite::{Connection, OptionalExtension as _, Row, params};
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret as _, SecretString};
use sha2::Sha256;
use tokio::sync::watch;

use crate::lowest_minor::{self, Opener};
use crate::run_key::KeyError;
use crate::run_state::RunStateError;
use crate::sqlite_store::{
    self, Access, Durability, File, Folder, KeptStore, Opening, StoreFile, Wait,
};
use crate::stored_time::StoredTime;
use crate::tasks::{ActorStopped, TaskOwner};

/// The Saved identity store's format (ADR 0098): a shared store per Product
/// major that records the lowest minor of its major that can use it.
pub const SAVED_IDENTITY_STORE: Format = Format {
    name: "saved-identity-store",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::SharedStore,
    hello_from: &[],
    schema: schema_of::<SavedIdentityInfo>,
};

/// How long a store operation waits for another connection's SQLite lock.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// Sealing format layout byte.
const LAYOUT: u8 = 1;
const KEY_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 24;
const FIELD_DOMAIN: &[u8] = b"goodfetch saved_identity field";
const WRAP_DOMAIN: &[u8] = b"goodfetch saved_identity wrap";
const WRAPPING_KEY_INFO: &[u8] = b"goodfetch saved_identity key-encryption key";

const SCHEMA: &[&str] = &[
    lowest_minor::FORMAT_TABLE,
    "CREATE TABLE IF NOT EXISTS saved_identities (
        name TEXT NOT NULL PRIMARY KEY,
        access_mode TEXT NOT NULL,
        identity_seed INTEGER NOT NULL,
        browser_family TEXT NOT NULL,
        user_agent TEXT,
        locale TEXT,
        timezone_id TEXT,
        proxy_binding TEXT,
        machine_binding TEXT,
        was_lent INTEGER NOT NULL DEFAULT 0,
        source_kind TEXT,
        domains TEXT NOT NULL,
        cookie_count INTEGER NOT NULL,
        origin_count INTEGER NOT NULL,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        last_used_at TEXT NOT NULL,
        wrapped_key BLOB NOT NULL,
        key_reference TEXT NOT NULL,
        sealed_payload BLOB NOT NULL
    ) STRICT",
];

const COLUMNS: &str = "name, access_mode, identity_seed, browser_family, user_agent, locale, \
    timezone_id, proxy_binding, machine_binding, was_lent, source_kind, domains, cookie_count, \
    origin_count, created_at, updated_at, last_used_at, wrapped_key, key_reference, sealed_payload";

/// Error from the Saved identity store.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// SQLite or store-format failure (including `lowest_minor` refusal).
    #[error(transparent)]
    State(#[from] RunStateError),
    /// The install-wide lock file could not be acquired.
    #[error("could not acquire Saved identity store lock `{path}`: {message}")]
    Lock {
        /// Lock file path.
        path: PathBuf,
        /// OS error message.
        message: String,
    },
    /// Sealing or unsealing a secret payload or data key failed.
    #[error(transparent)]
    Crypto(#[from] KeyError),
    /// Stored metadata JSON was corrupt.
    #[error("corrupt Saved identity metadata for `{name}`: {message}")]
    Corrupt {
        /// Identity name.
        name: String,
        /// Parse error message.
        message: String,
    },
}

/// Metadata + opaque secret payload bytes to save into the store.
#[derive(Clone, Debug)]
pub struct NewSavedIdentity {
    /// Unique name within this Product major.
    pub name: String,
    /// Access mode (`standard`, `stealth`, `identified`).
    pub access_mode: String,
    /// Identity profile seed.
    pub identity_seed: u64,
    /// Browser family (`chromium`, `firefox`, `webkit`).
    pub browser_family: String,
    /// Effective user agent, if recorded.
    pub user_agent: Option<String>,
    /// Effective BCP 47 locale, if recorded.
    pub locale: Option<String>,
    /// Effective IANA timezone ID, if recorded.
    pub timezone_id: Option<String>,
    /// Proxy binding, or `None` for direct egress.
    pub proxy_binding: Option<SavedProxyBinding>,
    /// Machine identity binding, when direct-egress imported.
    pub machine_binding: Option<String>,
    /// Whether replacing an existing identity should preserve its
    /// `machine_binding` when `self.machine_binding` is `None` (ADR 0035).
    pub preserve_existing_machine_binding: bool,
    /// Whether the Session was lent via `page.lend()` (ADR 0089).
    pub was_lent: bool,
    /// Source kind when imported, `None` when saved from a Session.
    pub source_kind: Option<String>,
    /// Distinct non-secret domain names held in cookies/origins.
    pub domains: BTreeSet<String>,
    /// Cookie count.
    pub cookie_count: u64,
    /// Origin count.
    pub origin_count: u64,
    /// Plaintext secret payload bytes (cookies + storage JSON) to encrypt.
    pub payload_plaintext: Zeroizing<Vec<u8>>,
}

/// A decrypted Saved identity read from the store.
#[derive(Clone)]
pub struct LoadedSavedIdentity {
    /// Non-secret metadata as exposed by `identity.list`.
    pub info: SavedIdentityInfo,
    /// Identity profile seed.
    pub identity_seed: u64,
    /// Recorded user agent, if any.
    pub user_agent: Option<String>,
    /// Proxy binding, if any.
    pub proxy_binding: Option<SavedProxyBinding>,
    /// Machine binding, if any.
    pub machine_binding: Option<String>,
    /// The Secret reference that wrapped the identity's data key.
    pub key_reference: String,
    /// Decrypted secret payload bytes (zeroized on drop).
    pub payload_plaintext: Zeroizing<Vec<u8>>,
}

/// A raw encrypted row read from SQLite before unwrapping its data key.
#[derive(Clone)]
pub struct SealedSavedIdentity {
    /// Non-secret metadata.
    pub info: SavedIdentityInfo,
    /// Identity profile seed.
    pub identity_seed: u64,
    /// Recorded user agent, if any.
    pub user_agent: Option<String>,
    /// Proxy binding, if any.
    pub proxy_binding: Option<SavedProxyBinding>,
    /// Machine binding, if any.
    pub machine_binding: Option<String>,
    /// The Secret reference that wrapped `wrapped_key`.
    pub key_reference: String,
    wrapped_key: Vec<u8>,
    sealed_payload: Vec<u8>,
}

impl SealedSavedIdentity {
    /// Unwraps the identity's data key under `kek` and decrypts its secret
    /// payload.
    ///
    /// # Errors
    ///
    /// [`KeyError::DoesNotOpen`] when `kek` does not match or the ciphertext
    /// was moved or tampered with.
    pub fn open(self, kek: &SecretString) -> Result<LoadedSavedIdentity, KeyError> {
        let data_key = unwrap_key(&self.info.name, &self.wrapped_key, kek)?;
        let plaintext = open_payload(&data_key, &self.info.name, &self.sealed_payload)?;
        Ok(LoadedSavedIdentity {
            info: self.info,
            identity_seed: self.identity_seed,
            user_agent: self.user_agent,
            proxy_binding: self.proxy_binding,
            machine_binding: self.machine_binding,
            key_reference: self.key_reference,
            payload_plaintext: plaintext,
        })
    }
}

/// The per-user, per-major Saved identity store as one engine opens it.
#[derive(Clone, Debug)]
pub struct SavedIdentityStore {
    store: Arc<KeptStore<IdentityStoreFile>>,
    locked_out: Arc<watch::Sender<bool>>,
    owner: Arc<TaskOwner>,
}

impl SavedIdentityStore {
    /// Creates a handle to the per-major Saved identity store at `path` with
    /// install-wide writer lock `lock_path`. Nothing is created on disk until
    /// the first write.
    pub fn of(
        path: PathBuf,
        lock_path: PathBuf,
        engine: &ProductVersion,
        vfs: Option<String>,
        owner: Arc<TaskOwner>,
    ) -> Self {
        let file = IdentityStoreFile::new(path, lock_path, opener(engine), vfs);
        let locked_out = Arc::clone(&file.locked_out);
        Self {
            store: Arc::new(KeptStore::of(
                file,
                "saved-identities-writer",
                Arc::clone(&owner),
            )),
            locked_out,
            owner,
        }
    }

    /// Reads an import source file on the engine's blocking pool up to
    /// `max_bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error description if the file cannot be read or exceeds
    /// `max_bytes`.
    pub async fn read_file_capped(&self, path: String, max_bytes: u64) -> Result<Vec<u8>, String> {
        self.owner
            .blocking(move || read_capped_blocking(&path, max_bytes))
            .await
    }

    /// Whether a call found the store's `lowest_minor` raised past this engine.
    pub fn locked_out(&self) -> watch::Receiver<bool> {
        self.locked_out.subscribe()
    }

    /// Encrypts `new_identity` under a fresh per-identity data key wrapped by
    /// `kek` (from `key_reference`), acquires the install-wide writer lock,
    /// replaces any existing identity of the same name, and returns its
    /// [`SavedIdentityInfo`].
    ///
    /// # Errors
    ///
    /// [`StoreError`] if encryption, locking, format verification, or SQLite fails.
    pub async fn save(
        &self,
        mut new_identity: NewSavedIdentity,
        key_reference: String,
        kek: SecretString,
        now: SystemTime,
        max_unused_age: Option<Duration>,
    ) -> Result<SavedIdentityInfo, StoreError> {
        let mut data_key = Zeroizing::new([0_u8; KEY_LENGTH]);
        getrandom::fill(data_key.as_mut_slice()).map_err(KeyError::NoRandomness)?;
        let wrapped_key = wrap_key(&data_key, &new_identity.name, &kek)?;
        let sealed_payload = seal_payload(
            &data_key,
            &new_identity.name,
            &new_identity.payload_plaintext,
        )?;
        let domains: Vec<String> = std::mem::take(&mut new_identity.domains)
            .into_iter()
            .collect();
        let domains_json =
            serde_json::to_string(&domains).map_err(|error| StoreError::Corrupt {
                name: new_identity.name.clone(),
                message: error.to_string(),
            })?;
        let proxy_json = new_identity
            .proxy_binding
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| StoreError::Corrupt {
                name: new_identity.name.clone(),
                message: error.to_string(),
            })?;
        let seed_i64 = i64::from_le_bytes(new_identity.identity_seed.to_le_bytes());
        let cookie_count_i64 = i64::try_from(new_identity.cookie_count).unwrap_or(i64::MAX);
        let origin_count_i64 = i64::try_from(new_identity.origin_count).unwrap_or(i64::MAX);
        let proxy_bound = new_identity.proxy_binding.is_some();

        let prepared = PreparedSave {
            new_identity,
            key_reference,
            wrapped_key,
            sealed_payload,
            domains,
            domains_json,
            proxy_json,
            proxy_bound,
            seed_i64,
            cookie_count_i64,
            origin_count_i64,
            now,
            max_unused_age,
        };
        self.store
            .write(None, move |file, connection| {
                let _lock = file.lock_exclusive()?;
                let result = file.in_transaction(connection, true, |connection| {
                    upsert_row(connection, prepared)
                });
                file.restrict_permissions();
                result
            })
            .await
    }

    /// Prunes any identities unused for at least `max_unused_age` at `now` and
    /// returns the names that expired (for Audit logging). If the store file
    /// does not exist yet, returns empty without creating it.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on SQLite, lock, or format failure.
    pub async fn prune_expired(
        &self,
        now: SystemTime,
        max_unused_age: Option<Duration>,
    ) -> Result<Vec<String>, StoreError> {
        let Some(max_age) = max_unused_age else {
            return Ok(Vec::new());
        };
        let expired = self
            .store
            .read(Some(Vec::new()), move |file, connection| {
                file.in_transaction(connection, false, |connection| {
                    find_expired_names(connection, now, max_age)
                })
            })
            .await?;
        if expired.is_empty() {
            return Ok(Vec::new());
        }
        self.store
            .write(None, move |file, connection| {
                let _lock = file.lock_exclusive()?;
                let deleted = file.in_transaction(connection, true, |connection| {
                    let names = find_expired_names(connection, now, max_age)?;
                    let mut stmt = connection
                        .prepare_cached("DELETE FROM saved_identities WHERE name = ?1")?;
                    for name in &names {
                        stmt.execute([name.as_str()])?;
                    }
                    Ok(names)
                })?;
                file.restrict_permissions();
                Ok(deleted)
            })
            .await
    }

    /// Lists all non-expired Saved identities (metadata only, never decrypting
    /// secret payloads), pruning expired entries first and returning
    /// `(identities, expired_names)`.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on SQLite, format, or metadata failure.
    pub async fn list(
        &self,
        now: SystemTime,
        max_unused_age: Option<Duration>,
    ) -> Result<(Vec<SavedIdentityInfo>, Vec<String>), StoreError> {
        let expired = self.prune_expired(now, max_unused_age).await?;
        let rows = self
            .store
            .read(Some(Vec::new()), move |file, connection| {
                file.in_transaction(connection, false, |connection| {
                    let mut stmt = connection.prepare_cached(&format!(
                        "SELECT {COLUMNS} FROM saved_identities ORDER BY name"
                    ))?;
                    let mapped = stmt.query_map([], |row| decode_sealed(row, max_unused_age))?;
                    let mut out = Vec::new();
                    for item in mapped {
                        out.push(item?.info);
                    }
                    Ok(out)
                })
            })
            .await?;
        Ok((rows, expired))
    }

    /// Loads the encrypted row for `name` after pruning expired entries. When
    /// `touch_last_used` is `true`, updates `last_used_at = now` under the
    /// install-wide lock so reusing an identity resets its 30-day inactivity
    /// window. Returns `(Option<SealedSavedIdentity>, expired_names)`.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on SQLite, lock, format, or metadata failure.
    pub async fn load_sealed(
        &self,
        name: String,
        now: SystemTime,
        max_unused_age: Option<Duration>,
        touch_last_used: bool,
    ) -> Result<(Option<SealedSavedIdentity>, Vec<String>), StoreError> {
        let expired = self.prune_expired(now, max_unused_age).await?;
        if !touch_last_used {
            let loaded = self
                .store
                .read(Some(None), move |file, connection| {
                    file.in_transaction(connection, false, |connection| {
                        let mut stmt = connection.prepare_cached(&format!(
                            "SELECT {COLUMNS} FROM saved_identities WHERE name = ?1"
                        ))?;
                        stmt.query_row([name.as_str()], |row| decode_sealed(row, max_unused_age))
                            .optional()
                    })
                })
                .await?;
            return Ok((loaded, expired));
        }
        // Check existence first on the read connection so missing names do not
        // create an empty store file.
        if !self.exists(&name).await? {
            return Ok((None, expired));
        }
        let loaded = self
            .store
            .write(None, move |file, connection| {
                let _lock = file.lock_exclusive()?;
                let loaded = file.in_transaction(connection, true, |connection| {
                    let updated = connection
                        .prepare_cached(
                            "UPDATE saved_identities SET last_used_at = ?2 WHERE name = ?1",
                        )?
                        .execute(params![name.as_str(), StoredTime(now)])?;
                    if updated == 0 {
                        return Ok(None);
                    }
                    let mut stmt = connection.prepare_cached(&format!(
                        "SELECT {COLUMNS} FROM saved_identities WHERE name = ?1"
                    ))?;
                    stmt.query_row([name.as_str()], |row| decode_sealed(row, max_unused_age))
                        .optional()
                })?;
                file.restrict_permissions();
                Ok(loaded)
            })
            .await?;
        Ok((loaded, expired))
    }

    async fn exists(&self, name: &str) -> Result<bool, StoreError> {
        let name = name.to_owned();
        self.store
            .read(Some(false), move |file, connection| {
                file.in_transaction(connection, false, |connection| {
                    let found: Option<i64> = connection
                        .prepare_cached("SELECT 1 FROM saved_identities WHERE name = ?1")?
                        .query_row([name.as_str()], |row| row.get(0))
                        .optional()?;
                    Ok(found.is_some())
                })
            })
            .await
    }

    /// Deletes the Saved identity `name` under the install-wide lock. Returns
    /// `true` if a row existed and was deleted. If the store does not exist
    /// yet, returns `false` without creating it.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on SQLite, lock, or format failure.
    pub async fn delete(&self, name: String) -> Result<bool, StoreError> {
        if !self.exists(&name).await? {
            return Ok(false);
        }
        self.store
            .write(None, move |file, connection| {
                let _lock = file.lock_exclusive()?;
                let deleted = file.in_transaction(connection, true, |connection| {
                    let count = connection
                        .prepare_cached("DELETE FROM saved_identities WHERE name = ?1")?
                        .execute([name.as_str()])?;
                    Ok(count > 0)
                })?;
                file.restrict_permissions();
                Ok(deleted)
            })
            .await
    }
}

fn find_expired_names(
    connection: &Connection,
    now: SystemTime,
    max_age: Duration,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = connection
        .prepare_cached("SELECT name, last_used_at FROM saved_identities ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        let name: String = row.get(0)?;
        let last_used: StoredTime = row.get(1)?;
        Ok((name, last_used.0))
    })?;
    let mut expired = Vec::new();
    for item in rows {
        let (name, last_used) = item?;
        if now
            .duration_since(last_used)
            .is_ok_and(|elapsed| elapsed >= max_age)
        {
            expired.push(name);
        }
    }
    Ok(expired)
}

fn decode_sealed(
    row: &Row<'_>,
    max_unused_age: Option<Duration>,
) -> rusqlite::Result<SealedSavedIdentity> {
    let name: String = row.get(0)?;
    let access_mode: String = row.get(1)?;
    let seed_i64: i64 = row.get(2)?;
    let identity_seed = u64::from_le_bytes(seed_i64.to_le_bytes());
    let browser_family: String = row.get(3)?;
    let user_agent: Option<String> = row.get(4)?;
    let locale: Option<String> = row.get(5)?;
    let timezone_id: Option<String> = row.get(6)?;
    let proxy_json: Option<String> = row.get(7)?;
    let proxy_binding: Option<SavedProxyBinding> = proxy_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let machine_binding: Option<String> = row.get(8)?;
    let was_lent_i64: i64 = row.get(9)?;
    let was_lent = was_lent_i64 != 0;
    let source_kind: Option<String> = row.get(10)?;
    let domains_json: String = row.get(11)?;
    let domains: Vec<String> = serde_json::from_str(&domains_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, Box::new(error))
    })?;
    let cookie_count = u64::try_from(row.get::<_, i64>(12)?).unwrap_or(0);
    let origin_count = u64::try_from(row.get::<_, i64>(13)?).unwrap_or(0);
    let created_at = row.get::<_, StoredTime>(14)?.0;
    let updated_at = row.get::<_, StoredTime>(15)?.0;
    let last_used_at = row.get::<_, StoredTime>(16)?.0;
    let wrapped_key: Vec<u8> = row.get(17)?;
    let key_reference: String = row.get(18)?;
    let sealed_payload: Vec<u8> = row.get(19)?;
    let expires_at = max_unused_age.and_then(|age| last_used_at.checked_add(age));

    Ok(SealedSavedIdentity {
        info: SavedIdentityInfo {
            name,
            access_mode,
            browser_family,
            locale,
            timezone_id,
            domains,
            cookie_count,
            origin_count,
            was_lent,
            machine_bound: machine_binding.is_some(),
            proxy_bound: proxy_binding.is_some(),
            source_kind,
            created_at,
            updated_at,
            last_used_at,
            expires_at,
            warnings: Vec::new(),
        },
        identity_seed,
        user_agent,
        proxy_binding,
        machine_binding,
        key_reference,
        wrapped_key,
        sealed_payload,
    })
}

fn opener(engine: &ProductVersion) -> Opener {
    Opener::new(&SAVED_IDENTITY_STORE, engine.major_minor())
}

#[derive(Debug)]
struct IdentityStoreFile {
    path: PathBuf,
    lock_path: PathBuf,
    opener: Opener,
    vfs: Option<String>,
    locked_out: Arc<watch::Sender<bool>>,
}

enum Failure {
    Sqlite(rusqlite::Error),
    Refused(RunStateError),
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

impl IdentityStoreFile {
    fn new(path: PathBuf, lock_path: PathBuf, opener: Opener, vfs: Option<String>) -> Self {
        Self {
            path,
            lock_path,
            opener,
            vfs,
            locked_out: Arc::new(watch::Sender::new(false)),
        }
    }

    fn failed(&self, error: &impl std::fmt::Display) -> StoreError {
        StoreError::State(RunStateError::Sqlite {
            path: self.path.clone(),
            message: error.to_string(),
        })
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "install-wide writer lock runs exclusively on the store's blocking writer thread"
    )]
    fn lock_exclusive(&self) -> Result<std::fs::File, StoreError> {
        if let Some(folder) = self.lock_path.parent() {
            std::fs::create_dir_all(folder).map_err(|error| StoreError::Lock {
                path: self.lock_path.clone(),
                message: error.to_string(),
            })?;
        }
        let file = open_private_lock(&self.lock_path).map_err(|error| StoreError::Lock {
            path: self.lock_path.clone(),
            message: error.to_string(),
        })?;
        file.lock().map_err(|error| StoreError::Lock {
            path: self.lock_path.clone(),
            message: error.to_string(),
        })?;
        Ok(file)
    }

    /// Sets mode `0600` on Unix on the SQLite store and its `-wal`/`-shm`
    /// companions so no other OS user can read the encrypted store (ADR 0035).
    fn restrict_permissions(&self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let base = self.path.as_os_str();
            let mut wal = base.to_os_string();
            wal.push("-wal");
            let mut shm = base.to_os_string();
            shm.push("-shm");
            for candidate in [self.path.as_path(), Path::new(&wal), Path::new(&shm)] {
                let _ = std::fs::set_permissions(candidate, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    fn in_transaction<T>(
        &self,
        connection: &Connection,
        writes: bool,
        work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, StoreError> {
        let checked = |connection: &Connection| -> Result<T, Failure> {
            let found = lowest_minor::read_sqlite(connection, SAVED_IDENTITY_STORE.name)?
                .unwrap_or_else(|| self.opener.created());
            self.opener.check(&found).map_err(|refusal| {
                if matches!(refusal.compatibility, Compatibility::NewerMinor) {
                    self.locked_out
                        .send_if_modified(|locked_out| !std::mem::replace(locked_out, true));
                }
                Failure::Refused(RunStateError::refused(refusal))
            })?;
            let answer = work(connection)?;
            if writes {
                let marked = self.opener.written(found, None);
                if marked != found {
                    lowest_minor::write_sqlite(connection, SAVED_IDENTITY_STORE.name, marked)?;
                }
            }
            Ok(answer)
        };
        let done = if writes {
            sqlite_store::immediate(connection, checked)
        } else {
            sqlite_store::deferred(connection, checked)
        };
        done.map_err(|failure| match failure {
            Failure::Sqlite(error) => self.failed(&error),
            Failure::Refused(refused) => StoreError::State(refused),
        })
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "runs on the store's blocking writer thread"
)]
fn open_private_lock(path: &Path) -> io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(file)
}

impl StoreFile for IdentityStoreFile {
    type Error = StoreError;

    fn path(&self) -> &Path {
        &self.path
    }

    fn exists(&self) -> bool {
        match self.vfs.as_deref() {
            None => self.path.exists(),
            Some(vfs) => !matches!(
                File::new(&self.path, Some(vfs)).connect_existing(Access::ReadOnly),
                Ok(None)
            ),
        }
    }

    fn open(&self) -> Result<Connection, StoreError> {
        let connection = File::new(&self.path, self.vfs.as_deref())
            .open(&Opening {
                folder: Folder::Make,
                access: Access::Create,
                wait: Wait::Busy(BUSY_TIMEOUT),
                durability: Durability::SYNCED,
                counting: None,
                schema: None,
            })
            .map_err(|error| self.failed(&error))?;
        connection
            .pragma_update(None, "secure_delete", "ON")
            .map_err(|error| self.failed(&error))?;
        for statement in SCHEMA {
            sqlite_store::run_cached(&connection, statement)
                .map_err(|error| self.failed(&error))?;
        }
        let marked = lowest_minor::read_sqlite(&connection, SAVED_IDENTITY_STORE.name)
            .map_err(|error| self.failed(&error))?;
        if marked.is_none() {
            let created = self.opener.created();
            connection
                .prepare_cached(
                    "INSERT INTO format (name, version, lowest_minor) VALUES (?1, ?2, ?3) \
                     ON CONFLICT (name) DO NOTHING",
                )
                .and_then(|mut statement| {
                    statement.execute((
                        SAVED_IDENTITY_STORE.name,
                        created.version.to_string(),
                        i64::from(created.lowest_minor),
                    ))
                })
                .map_err(|error| self.failed(&error))?;
        }
        self.restrict_permissions();
        Ok(connection)
    }

    fn open_read(&self) -> Result<Connection, StoreError> {
        let connection = File::new(&self.path, self.vfs.as_deref())
            .connect(Access::ReadOnly)
            .map_err(|error| self.failed(&error))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| self.failed(&error))?;
        Ok(connection)
    }

    fn stopped(&self, stopped: &ActorStopped) -> StoreError {
        self.failed(stopped)
    }
}

fn associated(domain: &[u8], fields: &[&str]) -> Vec<u8> {
    let mut data =
        Vec::with_capacity(1 + domain.len() + fields.iter().map(|f| 4 + f.len()).sum::<usize>());
    data.push(LAYOUT);
    data.extend_from_slice(domain);
    for field in fields {
        let len = u32::try_from(field.len()).expect("invariant: a name is under 4 GiB");
        data.extend_from_slice(&len.to_be_bytes());
        data.extend_from_slice(field.as_bytes());
    }
    data
}

fn wrapping_key(kek: &SecretString) -> Zeroizing<[u8; KEY_LENGTH]> {
    let mut key = Zeroizing::new([0_u8; KEY_LENGTH]);
    Hkdf::<Sha256>::new(None, kek.expose_secret().as_bytes())
        .expand(WRAPPING_KEY_INFO, key.as_mut_slice())
        .expect("invariant: 32 bytes is a valid HKDF-SHA256 output length");
    key
}

fn aead_seal(key: &[u8; KEY_LENGTH], aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
    let mut nonce = [0_u8; NONCE_LENGTH];
    getrandom::fill(&mut nonce).map_err(KeyError::NoRandomness)?;
    let ciphertext = XChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("invariant: XChaCha20-Poly1305 seals any value under 256 GiB");
    let mut sealed = Vec::with_capacity(1 + NONCE_LENGTH + ciphertext.len());
    sealed.push(LAYOUT);
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

fn aead_open(
    key: &[u8; KEY_LENGTH],
    aad: &[u8],
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KeyError> {
    let Some((&LAYOUT, rest)) = sealed.split_first() else {
        return Err(KeyError::DoesNotOpen);
    };
    let Some((nonce, ciphertext)) = rest.split_at_checked(NONCE_LENGTH) else {
        return Err(KeyError::DoesNotOpen);
    };
    XChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| KeyError::DoesNotOpen)
}

fn wrap_key(key: &[u8; KEY_LENGTH], name: &str, kek: &SecretString) -> Result<Vec<u8>, KeyError> {
    aead_seal(&wrapping_key(kek), &associated(WRAP_DOMAIN, &[name]), key)
}

fn unwrap_key(
    name: &str,
    wrapped: &[u8],
    kek: &SecretString,
) -> Result<Zeroizing<[u8; KEY_LENGTH]>, KeyError> {
    let opened = aead_open(
        &wrapping_key(kek),
        &associated(WRAP_DOMAIN, &[name]),
        wrapped,
    )?;
    if opened.len() != KEY_LENGTH {
        return Err(KeyError::DoesNotOpen);
    }
    let mut key = Zeroizing::new([0_u8; KEY_LENGTH]);
    key.copy_from_slice(&opened);
    Ok(key)
}

fn seal_payload(key: &[u8; KEY_LENGTH], name: &str, plaintext: &[u8]) -> Result<Vec<u8>, KeyError> {
    aead_seal(
        key,
        &associated(FIELD_DOMAIN, &[name, "payload"]),
        plaintext,
    )
}

fn open_payload(
    key: &[u8; KEY_LENGTH],
    name: &str,
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, KeyError> {
    aead_open(key, &associated(FIELD_DOMAIN, &[name, "payload"]), sealed)
}

struct PreparedSave {
    new_identity: NewSavedIdentity,
    key_reference: String,
    wrapped_key: Vec<u8>,
    sealed_payload: Vec<u8>,
    domains: Vec<String>,
    domains_json: String,
    proxy_json: Option<String>,
    proxy_bound: bool,
    seed_i64: i64,
    cookie_count_i64: i64,
    origin_count_i64: i64,
    now: SystemTime,
    max_unused_age: Option<Duration>,
}

const UPSERT_SQL: &str = "INSERT INTO saved_identities (
    name, access_mode, identity_seed, browser_family, user_agent, locale, timezone_id,
    proxy_binding, machine_binding, was_lent, source_kind, domains, cookie_count,
    origin_count, created_at, updated_at, last_used_at, wrapped_key, key_reference, sealed_payload
) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)
ON CONFLICT (name) DO UPDATE SET
    access_mode=excluded.access_mode, identity_seed=excluded.identity_seed,
    browser_family=excluded.browser_family, user_agent=excluded.user_agent,
    locale=excluded.locale, timezone_id=excluded.timezone_id,
    proxy_binding=excluded.proxy_binding, machine_binding=excluded.machine_binding,
    was_lent=excluded.was_lent, source_kind=excluded.source_kind, domains=excluded.domains,
    cookie_count=excluded.cookie_count, origin_count=excluded.origin_count,
    created_at=excluded.created_at, updated_at=excluded.updated_at,
    last_used_at=excluded.last_used_at, wrapped_key=excluded.wrapped_key,
    key_reference=excluded.key_reference, sealed_payload=excluded.sealed_payload";

fn upsert_row(connection: &Connection, p: PreparedSave) -> rusqlite::Result<SavedIdentityInfo> {
    let existing: Option<(StoredTime, Option<String>)> = connection
        .prepare_cached("SELECT created_at, machine_binding FROM saved_identities WHERE name = ?1")?
        .query_row([p.new_identity.name.as_str()], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    let (created_at, existing_machine) = existing.map_or((p.now, None), |(c, m)| (c.0, m));
    let effective_machine = match p.new_identity.machine_binding {
        Some(bound) => Some(bound),
        None if p.new_identity.preserve_existing_machine_binding => existing_machine,
        None => None,
    };
    connection.prepare_cached(UPSERT_SQL)?.execute(params![
        p.new_identity.name.as_str(),
        p.new_identity.access_mode.as_str(),
        p.seed_i64,
        p.new_identity.browser_family.as_str(),
        p.new_identity.user_agent.as_deref(),
        p.new_identity.locale.as_deref(),
        p.new_identity.timezone_id.as_deref(),
        p.proxy_json.as_deref(),
        effective_machine.as_deref(),
        i64::from(p.new_identity.was_lent),
        p.new_identity.source_kind.as_deref(),
        p.domains_json.as_str(),
        p.cookie_count_i64,
        p.origin_count_i64,
        StoredTime(created_at),
        StoredTime(p.now),
        StoredTime(p.now),
        p.wrapped_key.as_slice(),
        p.key_reference.as_str(),
        p.sealed_payload.as_slice(),
    ])?;
    Ok(SavedIdentityInfo {
        name: p.new_identity.name,
        access_mode: p.new_identity.access_mode,
        browser_family: p.new_identity.browser_family,
        locale: p.new_identity.locale,
        timezone_id: p.new_identity.timezone_id,
        domains: p.domains,
        cookie_count: p.new_identity.cookie_count,
        origin_count: p.new_identity.origin_count,
        was_lent: p.new_identity.was_lent,
        machine_bound: effective_machine.is_some(),
        proxy_bound: p.proxy_bound,
        source_kind: p.new_identity.source_kind,
        created_at,
        updated_at: p.now,
        last_used_at: p.now,
        expires_at: p.max_unused_age.and_then(|age| p.now.checked_add(age)),
        warnings: Vec::new(),
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "runs exclusively inside `TaskOwner::blocking` for `identity.import`"
)]
fn read_capped_blocking(path: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let meta = std::fs::metadata(path)
        .map_err(|error| format!("could not read state_file `{path}`: {error}"))?;
    if meta.len() > max_bytes {
        return Err(format!("state_file `{path}` exceeds the 32 MiB limit"));
    }
    std::fs::read(path).map_err(|error| format!("could not read state_file `{path}`: {error}"))
}

#[cfg(test)]
mod tests;
