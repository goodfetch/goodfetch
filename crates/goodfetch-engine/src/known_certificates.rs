//! The known-certificates store (ADRs 0042, 0066 and 0098): one per-user
//! SQLite file in Goodfetch home, beside the Usage ledger, outside the
//! per-major folders, as SSH's `known_hosts` remembers hosts. Each row pins
//! one host for one person: the SHA-256 of the `SubjectPublicKeyInfo` of the
//! certificate a trust-on-first-use exception accepted, when it was pinned
//! and by which Run. On a shared Daemon every person's pins sit in their own
//! partition, so nothing reads or lists another person's.
//!
//! A pin is written once; only a person replaces or removes it (`certs
//! accept` and `certs forget`). A row the engine cannot read, its key's
//! bytes or its other fields damaged or written by a newer minor in a form
//! this one does not know, is [`Found::Unreadable`], never no pin, so its
//! host stops as a changed key and an older engine never trusts a pinned
//! host as unpinned.
//!
//! The store opens through a VFS named by a value, none in production, and
//! is of the Usage ledger's family: its major never changes and a newer
//! minor only adds. Its calls block: callers run them under their owner's
//! blocking policy.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use goodfetch_protocol::formats::{Compatibility, KNOWN_CERTIFICATES};
use goodfetch_protocol::run::{PointInTime, RunId};
use goodfetch_protocol::tls::CertificatePin;
use jiff::Timestamp;
use rusqlite::{Connection, OpenFlags, OptionalExtension as _, TransactionBehavior};
use sha2::{Digest as _, Sha256};

use crate::home::Home;

/// The kind of exception a host was reached through, as Provenance marks
/// what was fetched under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExceptionKind {
    /// Its domain was listed without a pin: the key pinned on first use.
    TrustOnFirstUse,
    /// Its domain was listed with `any`, and the host has no pin: any
    /// certificate, none pinned.
    Any,
}

impl ExceptionKind {
    /// Its name in the Audit log and Provenance.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TrustOnFirstUse => "trust_on_first_use",
            Self::Any => "any",
        }
    }
}

/// How long a write waits for another engine's, and a lookup while SQLite
/// answers busy, as while a crashed writer's journal is recovered.
const BUSY_TIMEOUT: Duration = Duration::from_secs(10);

/// The tables of this version. A newer minor only adds tables and columns,
/// each column with a default, so these statements keep working on its
/// files.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS format (name TEXT NOT NULL PRIMARY KEY, version TEXT NOT NULL) STRICT",
    "CREATE TABLE IF NOT EXISTS pins (
        person TEXT NOT NULL,
        host TEXT NOT NULL,
        spki_sha256 BLOB NOT NULL,
        pinned_at TEXT NOT NULL,
        run_id TEXT,
        PRIMARY KEY (person, host)
    ) STRICT",
    // The key a pinned host presented instead, which stopped it, waiting
    // for a person to accept it.
    "CREATE TABLE IF NOT EXISTS changes (
        person TEXT NOT NULL,
        host TEXT NOT NULL,
        spki_sha256 BLOB NOT NULL,
        seen_at TEXT NOT NULL,
        run_id TEXT,
        PRIMARY KEY (person, host)
    ) STRICT",
];

/// The SHA-256 of a certificate's `SubjectPublicKeyInfo`: what a pin holds,
/// written as `sha256/` and its standard base64.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct KeyHash([u8; 32]);

impl KeyHash {
    /// The hash of `spki`, a `SubjectPublicKeyInfo`'s DER, its outer
    /// `SEQUENCE` included.
    pub fn of_public_key(spki: &[u8]) -> Self {
        Self(Sha256::digest(spki).into())
    }

    /// The hash a pin row's key bytes hold; none unless they are 32 bytes.
    pub fn from_stored(bytes: &[u8]) -> Option<Self> {
        bytes.try_into().ok().map(Self)
    }

    /// Its bytes, as a pin row stores them.
    pub fn stored(&self) -> &[u8] {
        &self.0
    }

    /// The hash `text` writes, as `sha256/` and its standard base64; none
    /// for any other text.
    pub fn from_text(text: &str) -> Option<Self> {
        let encoded = text.strip_prefix("sha256/")?;
        let bytes = data_encoding::BASE64.decode(encoded.as_bytes()).ok()?;
        Self::from_stored(&bytes)
    }
}

impl std::fmt::Display for KeyHash {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "sha256/{}",
            data_encoding::BASE64.encode(&self.0)
        )
    }
}

impl std::fmt::Debug for KeyHash {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "KeyHash({self})")
    }
}

/// What the store holds for one person's host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// No pin: its next invalid certificate under an exception is its
    /// first.
    Unpinned,
    /// A pin of this key, the only one the host may present.
    Pinned(KeyHash),
    /// A row this engine cannot read, which stops the host as a changed
    /// key.
    Unreadable,
}

/// What [`KnownCertificates::pin`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pinned {
    /// The host had no pin: this is its first.
    First,
    /// Another engine pinned the host first: its row, which is kept.
    Already(Found),
}

/// What [`KnownCertificates::accept`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accepted {
    /// What the store held for the host before.
    pub previous: Found,
    /// The pin it holds now.
    pub pin: CertificatePin,
}

/// A change of a pinned host's key waiting for its person, as
/// [`KnownCertificates::waiting`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    /// The host.
    pub host: String,
    /// What the store holds for it: its pin, or a row it cannot read.
    pub pinned: Found,
    /// The key it presented instead.
    pub presented: KeyHash,
    /// When it presented it.
    pub seen_at: PointInTime,
    /// The Run that met it.
    pub run_id: Option<RunId>,
}

/// Why the store could not be read or written.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KnownCertificatesError {
    /// SQLite failed; a write committed nothing.
    #[error("the known-certificates store at `{path}`: {message}")]
    Sqlite {
        /// The store's file.
        path: PathBuf,
        /// SQLite's message.
        message: String,
    },
    /// No change waits for the host with the key a person accepted.
    #[error("no change of {host}'s key to that one waits to be accepted")]
    NoSuchChange {
        /// The host.
        host: String,
    },
    /// The store is of another major of the format (ADR 0030).
    #[error("the known-certificates store is {found}, which this engine cannot read")]
    Format {
        /// The store's version.
        found: String,
        /// What this engine makes of it.
        compatibility: Compatibility,
    },
}

/// The known-certificates store of one user: its file and the VFS it
/// opens through. Clones share whether this engine has found the store's
/// tables made.
#[derive(Clone, Debug)]
pub struct KnownCertificates {
    path: PathBuf,
    vfs: Option<String>,
    /// Set once this engine found every table of this version in the
    /// store, or made them: a lookup then opens the store read-only.
    made: Arc<OnceLock<()>>,
}

/// One row as SQLite gives it, before it is read.
struct Row {
    host: String,
    key: Vec<u8>,
    pinned_at: String,
    run_id: Option<String>,
}

/// A row read: its key, when it was pinned and by which Run.
struct Read {
    key: KeyHash,
    pinned_at: Timestamp,
    run_id: Option<RunId>,
}

impl Row {
    fn from_sql(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        // A value of another SQLite type than this minor writes is a row
        // this engine cannot read, not a failure of the store.
        let host: String = row.get(0)?;
        let key = row.get::<_, Vec<u8>>(1).ok();
        let pinned_at = row.get::<_, String>(2).ok();
        let run_id = row.get::<_, Option<String>>(3).ok();
        Ok(match (key, pinned_at, run_id) {
            (Some(key), Some(pinned_at), Some(run_id)) => Self {
                host,
                key,
                pinned_at,
                run_id,
            },
            _ => Self {
                host,
                key: Vec::new(),
                pinned_at: String::new(),
                run_id: None,
            },
        })
    }

    /// The row's fields, when every one reads.
    fn read(&self) -> Option<Read> {
        Some(Read {
            key: KeyHash::from_stored(&self.key)?,
            pinned_at: self.pinned_at.parse().ok()?,
            run_id: match &self.run_id {
                Some(run_id) => Some(RunId::parse(run_id).ok()?),
                None => None,
            },
        })
    }

    fn found(&self) -> Found {
        self.read()
            .map_or(Found::Unreadable, |read| Found::Pinned(read.key))
    }

    fn listed(self) -> CertificatePin {
        match self.read() {
            Some(read) => CertificatePin {
                host: self.host,
                spki_sha256: Some(read.key.to_string()),
                pinned_at: Some(PointInTime::from_rfc3339(read.pinned_at.to_string())),
                run_id: read.run_id,
                changed_spki_sha256: None,
            },
            None => CertificatePin {
                host: self.host,
                spki_sha256: None,
                pinned_at: None,
                run_id: None,
                changed_spki_sha256: None,
            },
        }
    }
}

const SELECT: &str = "SELECT host, spki_sha256, pinned_at, run_id FROM pins";

/// [`SELECT`]'s columns with the key of the change waiting for each pin.
const SELECT_LISTED: &str = "SELECT pins.host, pins.spki_sha256, pins.pinned_at, pins.run_id, changes.spki_sha256 FROM pins LEFT JOIN changes ON changes.person = pins.person AND changes.host = pins.host";

/// A change waiting for a person: the key the host presented instead, and
/// the Run that met it.
struct Change {
    key: KeyHash,
    run_id: Option<RunId>,
}

impl KnownCertificates {
    /// The store of `home`, opened through the SQLite VFS named `vfs` (none
    /// in production, ADR 0060).
    pub fn of(home: &Home, vfs: Option<String>) -> Self {
        Self {
            path: home.known_certificates(),
            vfs,
            made: Arc::default(),
        }
    }

    /// Its file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the store holds for `person`'s `host`. A store never written
    /// holds nothing, and is not created.
    ///
    /// # Errors
    ///
    /// When the store cannot be read, or is of another major.
    pub fn find(&self, person: &str, host: &str) -> Result<Found, KnownCertificatesError> {
        if !self.path.exists() {
            return Ok(Found::Unpinned);
        }
        let connection = self.open_read()?;
        find_in(&connection, person, host).map_err(|error| self.failed(&error))
    }

    /// Pins `key` for `person`'s `host`, at `now`, by `run_id`, unless the
    /// host already has a row, which is kept.
    ///
    /// # Errors
    ///
    /// When the store cannot be written, or is of another major.
    pub fn pin(
        &self,
        person: &str,
        host: &str,
        key: &KeyHash,
        run_id: Option<&RunId>,
        now: Timestamp,
    ) -> Result<Pinned, KnownCertificatesError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed(&error))?;
        let found = find_in(&transaction, person, host).map_err(|error| failed(&error))?;
        if found != Found::Unpinned {
            return Ok(Pinned::Already(found));
        }
        transaction
            .execute(
                "INSERT INTO pins (person, host, spki_sha256, pinned_at, run_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                (
                    person,
                    host,
                    key.stored(),
                    now.to_string(),
                    run_id.map(RunId::as_str),
                ),
            )
            .map_err(|error| failed(&error))?;
        transaction.commit().map_err(|error| failed(&error))?;
        Ok(Pinned::First)
    }

    /// Records `key`, which `person`'s pinned `host` presented at `now` in
    /// `run_id` and was stopped for, as the change waiting for a person; it
    /// replaces any change recorded before.
    ///
    /// # Errors
    ///
    /// When the store cannot be written, or is of another major.
    pub fn record_change(
        &self,
        person: &str,
        host: &str,
        key: &KeyHash,
        run_id: Option<&RunId>,
        now: Timestamp,
    ) -> Result<(), KnownCertificatesError> {
        let connection = self.open()?;
        connection
            .execute(
                "INSERT INTO changes (person, host, spki_sha256, seen_at, run_id) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (person, host) DO UPDATE SET spki_sha256 = excluded.spki_sha256, seen_at = excluded.seen_at, run_id = excluded.run_id",
                (
                    person,
                    host,
                    key.stored(),
                    now.to_string(),
                    run_id.map(RunId::as_str),
                ),
            )
            .map_err(|error| self.failed(&error))?;
        Ok(())
    }

    /// Pins `key` for `person`'s `host` in place of its pin, dated `now`
    /// and by the Run that met it, once `key` is the change waiting for the
    /// host, which then waits no more.
    ///
    /// # Errors
    ///
    /// [`KnownCertificatesError::NoSuchChange`] unless `key` is the change
    /// waiting for the host; otherwise when the store cannot be written,
    /// or is of another major.
    pub fn accept(
        &self,
        person: &str,
        host: &str,
        key: &KeyHash,
        now: Timestamp,
    ) -> Result<Accepted, KnownCertificatesError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed(&error))?;
        let waiting = change_in(&transaction, person, host).map_err(|error| failed(&error))?;
        let Some(Change {
            key: changed,
            run_id,
        }) = waiting.filter(|change| change.key == *key)
        else {
            return Err(KnownCertificatesError::NoSuchChange {
                host: host.to_owned(),
            });
        };
        let previous = find_in(&transaction, person, host).map_err(|error| failed(&error))?;
        transaction
            .execute(
                "INSERT INTO pins (person, host, spki_sha256, pinned_at, run_id) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (person, host) DO UPDATE SET spki_sha256 = excluded.spki_sha256, pinned_at = excluded.pinned_at, run_id = excluded.run_id",
                (
                    person,
                    host,
                    changed.stored(),
                    now.to_string(),
                    run_id.as_ref().map(RunId::as_str),
                ),
            )
            .map_err(|error| failed(&error))?;
        transaction
            .execute(
                "DELETE FROM changes WHERE person = ?1 AND host = ?2",
                [person, host],
            )
            .map_err(|error| failed(&error))?;
        transaction.commit().map_err(|error| failed(&error))?;
        Ok(Accepted {
            previous,
            pin: CertificatePin {
                host: host.to_owned(),
                spki_sha256: Some(changed.to_string()),
                pinned_at: Some(PointInTime::from_rfc3339(now.to_string())),
                run_id,
                changed_spki_sha256: None,
            },
        })
    }

    /// Removes `person`'s pin of `host` and the change waiting for it;
    /// whether there was a pin. A store never written holds none, and is
    /// not created.
    ///
    /// # Errors
    ///
    /// When the store cannot be written, or is of another major.
    pub fn forget(&self, person: &str, host: &str) -> Result<bool, KnownCertificatesError> {
        if !self.path.exists() {
            return Ok(false);
        }
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let mut connection = self.open()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| failed(&error))?;
        let removed = transaction
            .execute(
                "DELETE FROM pins WHERE person = ?1 AND host = ?2",
                [person, host],
            )
            .map_err(|error| failed(&error))?;
        transaction
            .execute(
                "DELETE FROM changes WHERE person = ?1 AND host = ?2",
                [person, host],
            )
            .map_err(|error| failed(&error))?;
        transaction.commit().map_err(|error| failed(&error))?;
        Ok(removed > 0)
    }

    /// `person`'s pins, by host; a row this engine cannot read is listed
    /// with its host alone. A store never written lists none, and is not
    /// created.
    ///
    /// # Errors
    ///
    /// When the store cannot be read, or is of another major.
    pub fn list(&self, person: &str) -> Result<Vec<CertificatePin>, KnownCertificatesError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let connection = self.open_read()?;
        let mut statement = connection
            .prepare(&format!(
                "{SELECT_LISTED} WHERE pins.person = ?1 ORDER BY pins.host"
            ))
            .map_err(|error| failed(&error))?;
        let rows = statement
            .query_map([person], |row| {
                // A change whose key this engine cannot read waits for
                // nothing a person could accept.
                let changed = row
                    .get::<_, Option<Vec<u8>>>(4)
                    .ok()
                    .flatten()
                    .and_then(|bytes| KeyHash::from_stored(&bytes));
                Ok((Row::from_sql(row)?, changed))
            })
            .map_err(|error| failed(&error))?;
        let mut pins = Vec::new();
        for row in rows {
            let (row, changed) = row.map_err(|error| failed(&error))?;
            pins.push(CertificatePin {
                changed_spki_sha256: changed.map(|key| key.to_string()),
                ..row.listed()
            });
        }
        Ok(pins)
    }

    /// The changes waiting for `person`, by host: each pinned host that
    /// presented another key, with its pin. A change whose key this engine
    /// cannot read waits for nothing a person could accept, and is left
    /// out. A store never written lists none, and is not created.
    ///
    /// # Errors
    ///
    /// When the store cannot be read, or is of another major.
    pub fn waiting(&self, person: &str) -> Result<Vec<Waiting>, KnownCertificatesError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let connection = self.open_read()?;
        let mut statement = connection
            .prepare(
                "SELECT host, spki_sha256, seen_at, run_id FROM changes WHERE person = ?1 ORDER BY host",
            )
            .map_err(|error| failed(&error))?;
        let rows = statement
            .query_map([person], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1).ok(),
                    row.get::<_, String>(2).ok(),
                    row.get::<_, Option<String>>(3).ok().flatten(),
                ))
            })
            .map_err(|error| failed(&error))?;
        let mut waiting = Vec::new();
        for row in rows {
            let (host, key, seen_at, run_id) = row.map_err(|error| failed(&error))?;
            let (Some(presented), Some(seen_at)) = (
                key.as_deref().and_then(KeyHash::from_stored),
                seen_at.and_then(|seen_at| seen_at.parse::<Timestamp>().ok()),
            ) else {
                continue;
            };
            let pinned = find_in(&connection, person, &host).map_err(|error| failed(&error))?;
            waiting.push(Waiting {
                pinned,
                presented,
                seen_at: PointInTime::from_rfc3339(seen_at.to_string()),
                run_id: run_id.and_then(|run_id| RunId::parse(&run_id).ok()),
                host,
            });
        }
        Ok(waiting)
    }

    /// Opens the existing store for a lookup, refusing one of another major:
    /// read-only, so it takes no write lock and, in WAL mode, never waits
    /// for a writer. The first time this engine finds a table of this
    /// version missing, it makes the tables through [`Self::open`] instead.
    fn open_read(&self) -> Result<Connection, KnownCertificatesError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        if self.made.get().is_none() {
            let connection = self.connect(OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let tables: i64 = connection
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('format', 'pins', 'changes')",
                    [],
                    |row| row.get(0),
                )
                .map_err(|error| failed(&error))?;
            if tables < 3 {
                drop(connection);
                return self.open();
            }
            let _ = self.made.set(());
        }
        let connection = self.connect(OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let found: String = connection
            .query_row(
                "SELECT version FROM format WHERE name = ?1",
                [KNOWN_CERTIFICATES.name],
                |row| row.get(0),
            )
            .map_err(|error| failed(&error))?;
        check_format(&found)?;
        Ok(connection)
    }

    /// A connection to the store opened with `access`, waiting up to
    /// [`BUSY_TIMEOUT`] where SQLite answers busy.
    fn connect(&self, access: OpenFlags) -> Result<Connection, KnownCertificatesError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        let flags = access | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = match &self.vfs {
            Some(vfs) => Connection::open_with_flags_and_vfs(&self.path, flags, vfs.as_str()),
            None => Connection::open_with_flags(&self.path, flags),
        }
        .map_err(|error| failed(&error))?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(|error| failed(&error))?;
        Ok(connection)
    }

    /// Opens the store, creating it and its tables when it does not exist,
    /// and refuses one of another major.
    fn open(&self) -> Result<Connection, KnownCertificatesError> {
        let failed = |error: &dyn std::fmt::Display| self.failed(error);
        if let Some(folder) = self.path.parent() {
            #[expect(
                clippy::disallowed_methods,
                reason = "the store's calls run under their caller's blocking policy"
            )]
            std::fs::create_dir_all(folder).map_err(|error| failed(&error))?;
        }
        let mut connection =
            self.connect(OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE)?;
        connection
            .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
            .map_err(|error| failed(&error))?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|error| failed(&error))?;
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
                (
                    KNOWN_CERTIFICATES.name,
                    KNOWN_CERTIFICATES.version.to_string(),
                ),
            )
            .map_err(|error| failed(&error))?;
        let found: String = transaction
            .query_row(
                "SELECT version FROM format WHERE name = ?1",
                [KNOWN_CERTIFICATES.name],
                |row| row.get(0),
            )
            .map_err(|error| failed(&error))?;
        transaction.commit().map_err(|error| failed(&error))?;
        check_format(&found)?;
        let _ = self.made.set(());
        Ok(connection)
    }

    fn failed(&self, error: &dyn std::fmt::Display) -> KnownCertificatesError {
        KnownCertificatesError::Sqlite {
            path: self.path.clone(),
            message: error.to_string(),
        }
    }
}

/// What `connection`'s store holds for `person`'s `host`.
fn find_in(connection: &Connection, person: &str, host: &str) -> rusqlite::Result<Found> {
    let row = connection
        .query_row(
            &format!("{SELECT} WHERE person = ?1 AND host = ?2"),
            [person, host],
            Row::from_sql,
        )
        .optional()?;
    Ok(row.map_or(Found::Unpinned, |row| row.found()))
}

/// The change waiting for `person`'s `host` in `connection`'s store; none
/// when none waits, or its row cannot be read.
fn change_in(
    connection: &Connection,
    person: &str,
    host: &str,
) -> rusqlite::Result<Option<Change>> {
    let row = connection
        .query_row(
            "SELECT spki_sha256, run_id FROM changes WHERE person = ?1 AND host = ?2",
            [person, host],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0).ok(),
                    row.get::<_, Option<String>>(1).ok(),
                ))
            },
        )
        .optional()?;
    Ok(row.and_then(|(key, run_id)| {
        Some(Change {
            key: KeyHash::from_stored(&key?)?,
            run_id: match run_id? {
                Some(run_id) => Some(RunId::parse(&run_id).ok()?),
                None => None,
            },
        })
    }))
}

/// Refuses a store of another major (ADR 0030): every minor of this one
/// opens.
fn check_format(found: &str) -> Result<(), KnownCertificatesError> {
    let refused = |compatibility| KnownCertificatesError::Format {
        found: found.to_owned(),
        compatibility,
    };
    let version = found
        .parse()
        .map_err(|_| refused(Compatibility::NewerMajor))?;
    match KNOWN_CERTIFICATES
        .rule
        .compatibility(KNOWN_CERTIFICATES.version, version)
    {
        Compatibility::Compatible => Ok(()),
        other => Err(refused(other)),
    }
}

#[cfg(test)]
mod tests;
