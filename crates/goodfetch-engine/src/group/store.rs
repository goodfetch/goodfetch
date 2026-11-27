//! The group store (ADR 0055): one per-major SQLite file in the Daemon's
//! own folder, mode `0700`, which the engine opens as a user store
//! ([`crate::user_store`]). On the Lead Daemon it keeps the group's
//! certificate authority, so a restart signs for the same group, the join
//! tokens minted and not yet redeemed, each by its SHA-256 only, and the
//! Group members it admitted; on every Daemon of a group, its own identity
//! on the peer link and the highest lead epoch it has recorded.
//!
//! A Group backup carries the authority's key wrapped by the Secret
//! provider's key-encryption key (ADR 0055); this file is never one.

use std::time::SystemTime;

use data_encoding::BASE64;
use goodfetch_protocol::catalog::{ProtocolVersion, schema_of};
use goodfetch_protocol::formats::{Format, Rule, Storage};
use goodfetch_protocol::group::{LeadEpoch, StoredGroup};
use goodfetch_protocol::home;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, params};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivatePkcs8KeyDer};

use super::identity::KeptIdentity;
use super::join_tokens::digest;
use super::{JOIN_TOKEN_LIFETIME, JoinToken, JoinTokenRefused, KeptAuthority};
use crate::StoredTime;
use crate::user_store::{UserStore, UserStoreError, UserStoreKind};

/// The group store's format. Only the Daemon of its major opens it; its
/// registration in the format list and its compatibility corpus are the
/// formats ticket's, as the Schedule store's are.
pub(crate) const GROUP_STORE: Format = Format {
    name: "group-store",
    version: ProtocolVersion { major: 0, minor: 1 },
    storage: Storage::Sqlite,
    rule: Rule::Default,
    hello_from: &[],
    schema: schema_of::<StoredGroup>,
};

/// The store's tables of this version.
const TABLES: &[&str] = &[
    "authority",
    "lead_epoch",
    "join_tokens",
    "identity",
    "members",
];

/// The statements that make them.
const STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS authority (
        only INTEGER PRIMARY KEY CHECK (only = 1),
        name TEXT NOT NULL,
        key TEXT NOT NULL,
        certificate TEXT NOT NULL,
        lists_issued INTEGER NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS lead_epoch (
        only INTEGER PRIMARY KEY CHECK (only = 1),
        highest INTEGER NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS join_tokens (
        digest TEXT PRIMARY KEY,
        expires_at TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS identity (
        only INTEGER PRIMARY KEY CHECK (only = 1),
        name TEXT NOT NULL,
        key TEXT NOT NULL,
        certificate TEXT NOT NULL,
        authority TEXT NOT NULL,
        revocations TEXT NOT NULL,
        peer_address TEXT NOT NULL,
        lead_address TEXT NOT NULL,
        lead_name TEXT NOT NULL,
        certificate_expires_at TEXT NOT NULL,
        renew_at TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS members (
        name TEXT PRIMARY KEY,
        peer_address TEXT NOT NULL,
        joined_at TEXT NOT NULL
    ) STRICT",
];

/// The group store, as the group's contribution declares it.
pub(crate) static STORE: UserStoreKind = UserStoreKind::new(
    home::GROUP_STORE,
    &GROUP_STORE,
    TABLES,
    STATEMENTS,
    "group store",
);

/// Forms the group around `authority`, at lead epoch 1, with `lead` as its
/// Lead Daemon and first Group member at `now`, unless this Daemon is in a
/// group already, formed or joined, which its recorded lead epoch shows:
/// the lead epoch it formed it at, or none.
///
/// # Errors
///
/// The store's, after which nothing was recorded.
pub(crate) async fn form(
    store: &UserStore,
    authority: KeptAuthority,
    lead: KeptIdentity,
    now: SystemTime,
) -> Result<Option<LeadEpoch>, UserStoreError> {
    store
        .write(move |connection| {
            if in_a_group(connection)? {
                return Ok(None);
            }
            connection
                .prepare_cached(
                    "INSERT INTO authority (only, name, key, certificate, lists_issued)
                     VALUES (1, ?1, ?2, ?3, ?4)",
                )?
                .execute(params![
                    authority.name,
                    BASE64.encode(authority.key.secret_pkcs8_der()),
                    BASE64.encode(&authority.certificate),
                    stored(authority.lists_issued)?,
                ])?;
            insert_member(connection, &lead.name, &lead.peer_address, now)?;
            let first = LeadEpoch::new(1);
            enter(connection, &lead, first)?;
            Ok(Some(first))
        })
        .await
}

/// Records that this Daemon joined a group as `identity`, at the group's
/// `lead_epoch`, unless it is in a group already: whether it recorded it.
///
/// # Errors
///
/// The store's, after which nothing was recorded.
pub(crate) async fn joined(
    store: &UserStore,
    identity: KeptIdentity,
    lead_epoch: LeadEpoch,
) -> Result<bool, UserStoreError> {
    store
        .write(move |connection| {
            if in_a_group(connection)? {
                return Ok(false);
            }
            enter(connection, &identity, lead_epoch)?;
            Ok(true)
        })
        .await
}

/// Whether this Daemon is in a group: formed or joined.
///
/// # Errors
///
/// The store's.
pub(crate) async fn is_in_a_group(store: &UserStore) -> Result<bool, UserStoreError> {
    store.read(false, in_a_group).await
}

/// The identity this Daemon keeps in its group, and the highest lead
/// epoch it recorded, if it is in one.
///
/// # Errors
///
/// The store's, or a row this version cannot read.
pub(crate) async fn identity(
    store: &UserStore,
) -> Result<Option<(KeptIdentity, LeadEpoch)>, UserStoreError> {
    store
        .read(None, |connection| {
            let Some(identity) = kept_identity(connection)? else {
                return Ok(None);
            };
            let highest: Option<i64> = connection
                .prepare_cached("SELECT highest FROM lead_epoch")?
                .query_row([], |row| row.get(0))
                .optional()?;
            highest
                .map(|highest| {
                    u64::try_from(highest)
                        .map(|highest| (identity, LeadEpoch::new(highest)))
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                Type::Integer,
                                Box::new(error),
                            )
                        })
                })
                .transpose()
        })
        .await
}

/// Who asks to join: the Group member name it takes and the `host:port`
/// it listens at.
#[derive(Clone, Debug)]
pub(crate) struct Joining {
    /// Its Group member name.
    pub(crate) name: String,
    /// Where it listens for peer links.
    pub(crate) peer_address: String,
}

/// What the Lead Daemon's store answers a Daemon that joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Admission {
    /// It is a Group member now, and its join token is spent.
    Admitted,
    /// This Daemon keeps no group's authority.
    NotLead,
    /// Its join token is refused; an expired one is spent.
    Refused(JoinTokenRefused),
    /// Another Group member has its name; its join token stays valid.
    NameTaken,
}

/// Admits `joining` into the group at `now` with the join token `given`,
/// in one transaction: an unknown or expired token is refused before the
/// name is looked at, so a Daemon without a token learns no Group
/// member's name.
///
/// # Errors
///
/// The store's, after which nothing changed.
pub(crate) async fn admit(
    store: &UserStore,
    given: &str,
    joining: Joining,
    now: SystemTime,
) -> Result<Admission, UserStoreError> {
    let digest = digest(given);
    store
        .write(move |connection| {
            if kept_authority(connection)?.is_none() {
                return Ok(Admission::NotLead);
            }
            let expires: Option<StoredTime> = connection
                .prepare_cached("SELECT expires_at FROM join_tokens WHERE digest = ?1")?
                .query_row([&digest], |row| row.get(0))
                .optional()?;
            let mut spend =
                connection.prepare_cached("DELETE FROM join_tokens WHERE digest = ?1")?;
            match expires {
                None => return Ok(Admission::Refused(JoinTokenRefused::Unknown)),
                Some(StoredTime(expires)) if expires <= now => {
                    spend.execute([&digest])?;
                    return Ok(Admission::Refused(JoinTokenRefused::Expired));
                }
                Some(_) => {}
            }
            let taken = connection
                .prepare_cached("SELECT 1 FROM members WHERE name = ?1")?
                .exists([&joining.name])?;
            if taken {
                return Ok(Admission::NameTaken);
            }
            spend.execute([&digest])?;
            insert_member(connection, &joining.name, &joining.peer_address, now)?;
            Ok(Admission::Admitted)
        })
        .await
}

/// Whether a lead epoch is recorded, which forming or joining a group does.
fn in_a_group(connection: &Connection) -> rusqlite::Result<bool> {
    connection
        .prepare_cached("SELECT 1 FROM lead_epoch")?
        .exists([])
}

/// Records `identity` and `lead_epoch`, the rows of a Daemon in a group.
fn enter(
    connection: &Connection,
    identity: &KeptIdentity,
    lead_epoch: LeadEpoch,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO identity (only, name, key, certificate, authority, revocations,
                 peer_address, lead_address, lead_name, certificate_expires_at, renew_at)
             VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?
        .execute(params![
            identity.name,
            BASE64.encode(identity.key.secret_pkcs8_der()),
            BASE64.encode(&identity.certificate),
            BASE64.encode(&identity.authority),
            BASE64.encode(&identity.revocations),
            identity.peer_address,
            identity.lead_address,
            identity.lead_name,
            StoredTime(identity.certificate_expires_at),
            StoredTime(identity.renew_at),
        ])?;
    connection
        .prepare_cached("INSERT INTO lead_epoch (only, highest) VALUES (1, ?1)")?
        .execute([stored(lead_epoch.get())?])?;
    Ok(())
}

/// Records the Group member `name`, listening at `peer_address`, as joined
/// at `now`.
fn insert_member(
    connection: &Connection,
    name: &str,
    peer_address: &str,
    now: SystemTime,
) -> rusqlite::Result<()> {
    connection
        .prepare_cached("INSERT INTO members (name, peer_address, joined_at) VALUES (?1, ?2, ?3)")?
        .execute(params![name, peer_address, StoredTime(now)])?;
    Ok(())
}

/// The identity the `identity` row keeps, if any.
fn kept_identity(connection: &Connection) -> rusqlite::Result<Option<KeptIdentity>> {
    connection
        .prepare_cached(
            "SELECT name, key, certificate, authority, revocations, peer_address, lead_address,
                 lead_name, certificate_expires_at, renew_at
             FROM identity",
        )?
        .query_row([], |row| {
            let StoredTime(certificate_expires_at) = row.get(8)?;
            let StoredTime(renew_at) = row.get(9)?;
            Ok(KeptIdentity {
                name: row.get(0)?,
                key: PrivatePkcs8KeyDer::from(decoded(row, 1)?),
                certificate: CertificateDer::from(decoded(row, 2)?),
                authority: CertificateDer::from(decoded(row, 3)?),
                revocations: CertificateRevocationListDer::from(decoded(row, 4)?),
                peer_address: row.get(5)?,
                lead_address: row.get(6)?,
                lead_name: row.get(7)?,
                certificate_expires_at,
                renew_at,
            })
        })
        .optional()
}

/// Keeps `renewed`'s key, certificate and revocation list in place of
/// those of the identity of the Group member it names: whether this Daemon
/// keeps that identity still.
///
/// # Errors
///
/// The store's, after which nothing was recorded.
pub(crate) async fn renew(
    store: &UserStore,
    renewed: KeptIdentity,
) -> Result<bool, UserStoreError> {
    store
        .write(move |connection| {
            let updated = connection
                .prepare_cached(
                    "UPDATE identity SET key = ?1, certificate = ?2, revocations = ?3,
                         certificate_expires_at = ?4, renew_at = ?5
                     WHERE name = ?6",
                )?
                .execute(params![
                    BASE64.encode(renewed.key.secret_pkcs8_der()),
                    BASE64.encode(&renewed.certificate),
                    BASE64.encode(&renewed.revocations),
                    StoredTime(renewed.certificate_expires_at),
                    StoredTime(renewed.renew_at),
                    renewed.name,
                ])?;
            Ok(updated == 1)
        })
        .await
}

/// Whether the Lead Daemon admitted a Group member named `name`.
///
/// # Errors
///
/// The store's.
pub(crate) async fn is_member(store: &UserStore, name: String) -> Result<bool, UserStoreError> {
    store
        .read(false, move |connection| {
            connection
                .prepare_cached("SELECT 1 FROM members WHERE name = ?1")?
                .exists([name])
        })
        .await
}

/// The group's certificate authority, if this Daemon keeps it.
///
/// # Errors
///
/// The store's, or a row this version cannot read.
pub(crate) async fn authority(store: &UserStore) -> Result<Option<KeptAuthority>, UserStoreError> {
    store.read(None, kept_authority).await
}

/// Records the join token `minted` at `now`, if this Daemon keeps the
/// group's authority, and forgets every token that expired a lifetime ago
/// or more: whether it recorded it.
///
/// # Errors
///
/// The store's, after which nothing was recorded.
pub(crate) async fn record_token(
    store: &UserStore,
    minted: &JoinToken,
    now: SystemTime,
) -> Result<bool, UserStoreError> {
    let digest = minted.digest();
    let expires = minted.expires;
    store
        .write(move |connection| {
            if kept_authority(connection)?.is_none() {
                return Ok(false);
            }
            forget_old_tokens(connection, now)?;
            connection
                .prepare_cached("INSERT INTO join_tokens (digest, expires_at) VALUES (?1, ?2)")?
                .execute(params![digest, StoredTime(expires)])?;
            Ok(true)
        })
        .await
}

/// Deletes the join tokens that expired a lifetime or more before `now`.
fn forget_old_tokens(connection: &Connection, now: SystemTime) -> rusqlite::Result<()> {
    let pending: Vec<(String, StoredTime)> = connection
        .prepare_cached("SELECT digest, expires_at FROM join_tokens")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut delete = connection.prepare_cached("DELETE FROM join_tokens WHERE digest = ?1")?;
    for (digest, StoredTime(expires)) in pending {
        if expires + JOIN_TOKEN_LIFETIME <= now {
            delete.execute([digest])?;
        }
    }
    Ok(())
}

/// The authority the `authority` row keeps, if any.
fn kept_authority(connection: &Connection) -> rusqlite::Result<Option<KeptAuthority>> {
    connection
        .prepare_cached("SELECT name, key, certificate, lists_issued FROM authority")?
        .query_row([], |row| {
            Ok(KeptAuthority {
                name: row.get(0)?,
                key: PrivatePkcs8KeyDer::from(decoded(row, 1)?),
                certificate: CertificateDer::from(decoded(row, 2)?),
                lists_issued: u64::try_from(row.get::<_, i64>(3)?).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(3, Type::Integer, Box::new(error))
                })?,
            })
        })
        .optional()
}

/// The INTEGER column's value of the count `count`.
fn stored(count: u64) -> rusqlite::Result<i64> {
    i64::try_from(count).map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))
}

/// The bytes of the base64 text in `column`.
fn decoded(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<Vec<u8>> {
    let text: String = row.get(column)?;
    BASE64.decode(text.as_bytes()).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
    })
}

#[cfg(test)]
mod tests;
