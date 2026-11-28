//! The group store (ADR 0055): one per-major SQLite file in the Daemon's
//! own folder, mode `0700`, which the engine opens as a user store
//! ([`crate::user_store`]). On the Lead Daemon it keeps the group's
//! certificate authority, so a restart signs for the same group, the join
//! tokens minted and not yet redeemed, each by its SHA-256 only, the Group
//! members it admitted, and the serial numbers of the Group member
//! certificates it signed until they expire, so it can revoke them; on
//! every Daemon of a group, its own identity on the peer link and the
//! highest lead epoch it has recorded.
//!
//! A Group backup carries the authority's key wrapped by the Secret
//! provider's key-encryption key (ADR 0055); this file is never one.

use std::time::SystemTime;

use data_encoding::{BASE64, HEXLOWER};
use goodfetch_protocol::catalog::{ProtocolVersion, schema_of};
use goodfetch_protocol::formats::{Format, Rule, Storage};
use goodfetch_protocol::group::{
    ControlStateSnapshot, LeadEpoch, LeadEpochStaleDetails, StoredGroup, StoredReplicatedStore,
};
use goodfetch_protocol::home;
use rcgen::SerialNumber;
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, params};
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivatePkcs8KeyDer};

use super::identity::KeptIdentity;
use super::join_tokens::digest;
use super::{
    GroupAuthority, GroupMemberCertificate, JOIN_TOKEN_LIFETIME, JoinToken, JoinTokenRefused,
    KeptAuthority,
};
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
    "certificates",
    "replicated_stores",
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
    "CREATE TABLE IF NOT EXISTS certificates (
        serial TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        expires_at TEXT NOT NULL,
        revoked_at TEXT
    ) STRICT",
    "CREATE TABLE IF NOT EXISTS replicated_stores (
        store TEXT PRIMARY KEY,
        version INTEGER NOT NULL,
        content TEXT
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

/// A Group member recorded in the Lead Daemon's `members` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Member {
    /// Its Group member name.
    pub(crate) name: String,
    /// Where it listens for peer links.
    pub(crate) peer_address: String,
}

/// Every Group member the Lead Daemon last knew, in name order.
///
/// # Errors
///
/// The store's.
pub(crate) async fn members(store: &UserStore) -> Result<Vec<Member>, UserStoreError> {
    store
        .read(Vec::new(), |connection| {
            connection
                .prepare_cached("SELECT name, peer_address FROM members ORDER BY name")?
                .query_map([], |row| {
                    Ok(Member {
                        name: row.get(0)?,
                        peer_address: row.get(1)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
}

/// Durably admits `epoch` against the highest lead epoch recorded in `store`:
/// refuses a lower epoch with [`LeadEpochStaleDetails`], raises the stored
/// row when `epoch` is higher, and returns the highest recorded epoch.
///
/// # Errors
///
/// The store's, after which nothing changed.
pub(crate) async fn admit_epoch(
    store: &UserStore,
    epoch: LeadEpoch,
) -> Result<Result<LeadEpoch, LeadEpochStaleDetails>, UserStoreError> {
    store
        .write(move |connection| {
            let current: Option<i64> = connection
                .prepare_cached("SELECT highest FROM lead_epoch")?
                .query_row([], |row| row.get(0))
                .optional()?;
            let current = match current {
                Some(value) => LeadEpoch::new(u64::try_from(value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(0, Type::Integer, Box::new(error))
                })?),
                None => LeadEpoch::new(0),
            };
            if epoch < current {
                return Ok(Err(LeadEpochStaleDetails {
                    lead_epoch: epoch,
                    highest_lead_epoch: current,
                }));
            }
            if epoch > current {
                connection
                    .prepare_cached(
                        "INSERT INTO lead_epoch (only, highest) VALUES (1, ?1)
                         ON CONFLICT (only) DO UPDATE SET highest = excluded.highest",
                    )?
                    .execute([stored(epoch.get())?])?;
            }
            Ok(Ok(current.max(epoch)))
        })
        .await
}

/// Who asks to join: the Group member name it takes, the `host:port` it
/// listens at and the certificate the authority signed for it.
#[derive(Clone, Debug)]
pub(crate) struct Joining {
    /// Its Group member name.
    pub(crate) name: String,
    /// Where it listens for peer links.
    pub(crate) peer_address: String,
    /// Its certificate, which it receives once admitted.
    pub(crate) certificate: Certified,
}

/// What the Lead Daemon keeps of a Group member certificate it signed, to
/// revoke it until it expires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Certified {
    pub(crate) serial: SerialNumber,
    pub(crate) expires_at: SystemTime,
}

impl From<&GroupMemberCertificate> for Certified {
    fn from(signed: &GroupMemberCertificate) -> Self {
        Self {
            serial: signed.serial.clone(),
            expires_at: signed.not_after,
        }
    }
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
            record_certificate(connection, &joining.name, &joining.certificate, now)?;
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
/// keeps that identity still. The Lead Daemon keeps its own list, which
/// only [`revoke`] replaces, so a revocation made while it renewed stands.
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
                    "UPDATE identity SET key = ?1, certificate = ?2,
                         revocations = CASE WHEN EXISTS (SELECT 1 FROM authority)
                             THEN revocations ELSE ?3 END,
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

/// Records at `now` that the authority signed `certificate` for the Group
/// member `name`, if the Lead Daemon admitted it and has not revoked it:
/// whether it did, which hands it the certificate.
///
/// # Errors
///
/// The store's, after which nothing was recorded.
pub(crate) async fn certified(
    store: &UserStore,
    name: String,
    certificate: Certified,
    now: SystemTime,
) -> Result<bool, UserStoreError> {
    store
        .write(move |connection| {
            let member = connection
                .prepare_cached("SELECT 1 FROM members WHERE name = ?1")?
                .exists([&name])?;
            if member {
                record_certificate(connection, &name, &certificate, now)?;
            }
            Ok(member)
        })
        .await
}

/// What the Lead Daemon's store answers the revocation of a Group member.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Revocation {
    /// It is no Group member now, and the revocation list numbered so
    /// revokes every certificate signed for it that has not expired.
    Revoked(u64),
    /// This Daemon keeps no group's authority.
    NotLead,
    /// It is the Lead Daemon itself.
    Lead,
    /// The group has no Group member of that name.
    Unknown,
}

/// Removes the Group member `name` from the group at `now`, in one
/// transaction: revokes every certificate the authority signed for it in a
/// new revocation list, which the Lead Daemon's identity holds from then.
///
/// # Errors
///
/// The store's, or the authority's when it cannot sign the list, after
/// which nothing changed.
pub(crate) async fn revoke(
    store: &UserStore,
    name: String,
    now: SystemTime,
) -> Result<Revocation, UserStoreError> {
    store
        .write(move |connection| {
            let (Some(kept), Some(lead)) = (kept_authority(connection)?, kept_identity(connection)?)
            else {
                return Ok(Revocation::NotLead);
            };
            if lead.name == name {
                return Ok(Revocation::Lead);
            }
            let removed = connection
                .prepare_cached("DELETE FROM members WHERE name = ?1")?
                .execute([&name])?;
            if removed == 0 {
                return Ok(Revocation::Unknown);
            }
            connection
                .prepare_cached(
                    "UPDATE certificates SET revoked_at = ?1 WHERE name = ?2 AND revoked_at IS NULL",
                )?
                .execute(params![StoredTime(now), name])?;
            forget_expired_certificates(connection, now)?;
            let revoked = connection
                .prepare_cached("SELECT serial FROM certificates WHERE revoked_at IS NOT NULL")?
                .query_map([], |row| {
                    let text: String = row.get(0)?;
                    HEXLOWER
                        .decode(text.as_bytes())
                        .map(|bytes| SerialNumber::from_slice(&bytes))
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error))
                        })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let list = GroupAuthority::restore(kept)
                .and_then(|mut authority| authority.revocation_list(&revoked, now))
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            connection
                .prepare_cached("UPDATE authority SET lists_issued = ?1")?
                .execute([stored(list.number)?])?;
            connection
                .prepare_cached("UPDATE identity SET revocations = ?1")?
                .execute([BASE64.encode(&list.list)])?;
            Ok(Revocation::Revoked(list.number))
        })
        .await
}

/// Forgets the group this Daemon is in, every row of it: whether it was in
/// one.
///
/// # Errors
///
/// The store's, after which nothing changed.
pub(crate) async fn leave(store: &UserStore) -> Result<bool, UserStoreError> {
    store
        .write(|connection| {
            let was = in_a_group(connection)?;
            for table in TABLES {
                connection
                    .prepare_cached(&format!("DELETE FROM {table}"))?
                    .execute([])?;
            }
            Ok(was)
        })
        .await
}

/// Records at `now` the certificate `certified` the authority signed for
/// the Group member `name`, and forgets every one that expired.
fn record_certificate(
    connection: &Connection,
    name: &str,
    certified: &Certified,
    now: SystemTime,
) -> rusqlite::Result<()> {
    forget_expired_certificates(connection, now)?;
    connection
        .prepare_cached("INSERT INTO certificates (serial, name, expires_at) VALUES (?1, ?2, ?3)")?
        .execute(params![
            HEXLOWER.encode(&certified.serial.to_bytes()),
            name,
            StoredTime(certified.expires_at),
        ])?;
    Ok(())
}

/// Deletes the certificates that expired by `now`, which no revocation
/// list needs to name any more.
fn forget_expired_certificates(connection: &Connection, now: SystemTime) -> rusqlite::Result<()> {
    let recorded: Vec<(String, StoredTime)> = connection
        .prepare_cached("SELECT serial, expires_at FROM certificates")?
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let mut delete = connection.prepare_cached("DELETE FROM certificates WHERE serial = ?1")?;
    for (serial, StoredTime(expires)) in recorded {
        if expires <= now {
            delete.execute([serial])?;
        }
    }
    Ok(())
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

/// Loads every persisted read-mostly control-state snapshot held in the group
/// store (ADR 0055).
///
/// # Errors
///
/// The store's, or a row this version cannot read.
pub(crate) async fn load_replicated_stores(
    store: &UserStore,
) -> Result<Vec<StoredReplicatedStore>, UserStoreError> {
    store
        .read(Vec::new(), |connection| {
            connection
                .prepare_cached(
                    "SELECT store, version, content FROM replicated_stores ORDER BY store",
                )?
                .query_map([], |row| {
                    let version = u64::try_from(row.get::<_, i64>(1)?).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(1, Type::Integer, Box::new(error))
                    })?;
                    Ok(StoredReplicatedStore {
                        store: row.get(0)?,
                        version,
                        content: row.get(2)?,
                    })
                })?
                .collect()
        })
        .await
}

/// Persists `snapshot` in `replicated_stores` if and only if its version is
/// strictly greater than the version already stored for `snapshot.store`.
/// Returns `true` when the row was inserted or updated.
///
/// # Errors
///
/// The store's, after which nothing changed.
pub(crate) async fn save_replicated_store(
    store: &UserStore,
    snapshot: ControlStateSnapshot,
) -> Result<bool, UserStoreError> {
    store
        .write(move |connection| {
            let changed = connection
                .prepare_cached(
                    "INSERT INTO replicated_stores (store, version, content)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(store) DO UPDATE SET
                         version = excluded.version,
                         content = excluded.content
                     WHERE excluded.version > replicated_stores.version",
                )?
                .execute(params![
                    snapshot.store,
                    stored(snapshot.version)?,
                    snapshot.content,
                ])?;
            Ok(changed > 0)
        })
        .await
}

#[cfg(test)]
mod tests;
