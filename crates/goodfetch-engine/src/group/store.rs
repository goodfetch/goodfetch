//! The group store (ADR 0055): one per-major SQLite file in the Daemon's
//! own folder, mode `0700`, which the engine opens as a user store
//! ([`crate::user_store`]). On the Lead Daemon it keeps the group's
//! certificate authority, so a restart signs for the same group, and the
//! join tokens minted and not yet redeemed, each by its SHA-256 only; on
//! every Daemon, the highest lead epoch it has recorded.
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
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

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
const TABLES: &[&str] = &["authority", "lead_epoch", "join_tokens"];

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
];

/// The group store, as the group's contribution declares it.
pub(crate) static STORE: UserStoreKind = UserStoreKind::new(
    home::GROUP_STORE,
    &GROUP_STORE,
    TABLES,
    STATEMENTS,
    "group store",
);

/// Forms the group around `authority`, at lead epoch 1, unless the store
/// keeps an authority already: the lead epoch it formed it at, or none.
///
/// # Errors
///
/// The store's, after which nothing was recorded.
pub(crate) async fn form(
    store: &UserStore,
    authority: KeptAuthority,
) -> Result<Option<LeadEpoch>, UserStoreError> {
    store
        .write(move |connection| {
            if kept_authority(connection)?.is_some() {
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
            let first = LeadEpoch::new(1);
            connection
                .prepare_cached(
                    "INSERT INTO lead_epoch (only, highest) VALUES (1, ?1)
                     ON CONFLICT (only) DO UPDATE SET highest = max(highest, ?1)",
                )?
                .execute([stored(first.get())?])?;
            Ok(Some(first))
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

/// Redeems the join token `given` at `now`: once it is pending, it is
/// forgotten whether or not its lifetime has ended.
///
/// # Errors
///
/// The store's, after which the token is still pending.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "`group.join`, the next part of issue 242, redeems"
    )
)]
pub(crate) async fn redeem(
    store: &UserStore,
    given: &str,
    now: SystemTime,
) -> Result<Result<(), JoinTokenRefused>, UserStoreError> {
    let digest = digest(given);
    store
        .write(move |connection| {
            let expires: Option<StoredTime> = connection
                .prepare_cached("DELETE FROM join_tokens WHERE digest = ?1 RETURNING expires_at")?
                .query_row([digest], |row| row.get(0))
                .optional()?;
            Ok(match expires {
                None => Err(JoinTokenRefused::Unknown),
                Some(StoredTime(expires)) if expires <= now => Err(JoinTokenRefused::Expired),
                Some(_) => Ok(()),
            })
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
