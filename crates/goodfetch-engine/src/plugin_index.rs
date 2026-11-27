//! The Plugin index store and policy evaluator (ADRs 0033, 0047, 0052 and
//! 0059; issue 911): stores the last-seen freshness version and current
//! verified [`PluginIndex`] in `<home>/plugin-index/`, verifies updates
//! and offline imports through [`signed_set::verify`], refuses stale,
//! badly signed or expired indexes with `plugin_index_stale`, blocks
//! third-party loading while an index is expired (still allowing
//! first-party Plugins), and detects revoked digests/signers for
//! `plugin_revoked` refusal and Audit log recording.
#![expect(
    clippy::disallowed_methods,
    reason = "index persistence runs on the tasks module's blocking policy, never on an async worker"
)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::plugin::PluginKindName;
use goodfetch_protocol::plugin::approvals::Origin;
use goodfetch_protocol::plugin::index::{
    IndexEntry, IndexRefusalReason, PLUGIN_INDEX_IMPORTED_AUDIT, PLUGIN_INDEX_STALE,
    PLUGIN_REVOKED, PLUGIN_REVOKED_AUDIT, PluginIndex, PluginIndexImported,
    PluginIndexStaleDetails, PluginRevokedDetails, PluginSearchParams, PluginSearchResult,
    Revocation, SignedBundle, SignerIdentity,
};
use goodfetch_protocol::run::RunId;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::audit::{self, AuditLog};
use crate::call_error::CallError;
use crate::durable;
use crate::egress::ConfiguredKind;
use crate::pinned_download::Fetching;
use crate::signed_set::{self, FetchError, SignedSetPayload, SignedSetVerdict, TrustRoot};

/// File holding the verified signed Plugin index in `<home>/plugin-index/`.
const INDEX_FILE: &str = "index.json";

/// File holding the monotonic last-seen freshness version beside the index.
const LAST_SEEN_FILE: &str = "last-seen.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LastSeen {
    version: u64,
}

/// Persistent store of the verified Plugin index and its monotonic
/// last-seen freshness `version` (ADR 0033).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginIndexStore {
    folder: PathBuf,
}

/// Current state of the cached Plugin index at one instant `now`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexStatus {
    /// No verified Plugin index has been stored in this Goodfetch home yet.
    Absent,
    /// A verified index is present and not past its expiry.
    Fresh(PluginIndex),
    /// A verified index is present, and its `expires_at` has passed at
    /// `now`: third-party loading is blocked while first-party Plugins
    /// still load, and revocations already recorded still hold.
    Expired {
        /// The cached index (still consulted for origin and revocations).
        index: PluginIndex,
        /// The parsed expiry timestamp.
        expires_at: Timestamp,
    },
}

impl IndexStatus {
    /// The index held, whether fresh or expired.
    #[must_use]
    pub fn index(&self) -> Option<&PluginIndex> {
        match self {
            Self::Absent => None,
            Self::Fresh(index) | Self::Expired { index, .. } => Some(index),
        }
    }

    /// Whether third-party Plugin loading is blocked by an expired index.
    #[must_use]
    pub fn is_expired(&self) -> bool {
        matches!(self, Self::Expired { .. })
    }
}

/// Why reading or persisting the Plugin index failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IndexError {
    /// The signed index failed signature, signer, freshness or expiry
    /// verification: becomes `plugin_index_stale`.
    #[error("{0}")]
    Refused(Box<CallError>),
    /// The index file or folder could not be read or written.
    #[error("the Plugin index at `{path}`: {source}")]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying OS error.
        source: io::Error,
    },
    /// The persisted index JSON was corrupted.
    #[error("the Plugin index at `{path}` is malformed: {source}")]
    Malformed {
        /// Path that failed.
        path: PathBuf,
        /// Decode error.
        source: serde_json::Error,
    },
    /// Network or HTTP fetch failure when refreshing from `plugin.index.url`.
    #[error("{0}")]
    Fetch(#[from] FetchError),
}

impl IndexError {
    /// Converts this error into a protocol [`CallError`], preserving
    /// `plugin_index_stale` refusals.
    #[must_use]
    pub fn into_call_error(self) -> CallError {
        match self {
            Self::Refused(error) => *error,
            other => CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                other.to_string(),
            ),
        }
    }
}

impl PluginIndexStore {
    /// Opens the Plugin index store rooted at `folder`
    /// (`<home>/plugin-index`).
    #[must_use]
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }

    /// The store's folder.
    #[must_use]
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Returns the highest freshness `version` previously accepted, if any.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::Io`] or [`IndexError::Malformed`] if the
    /// persisted file exists and cannot be read.
    pub fn last_seen_version(&self) -> Result<Option<u64>, IndexError> {
        let path = self.folder.join(LAST_SEEN_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(IndexError::Io { path, source }),
        };
        let state: LastSeen = serde_json::from_slice(&bytes)
            .map_err(|source| IndexError::Malformed { path, source })?;
        Ok(Some(state.version))
    }

    /// Loads the stored [`SignedBundle<PluginIndex>`] if present.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::Io`] or [`IndexError::Malformed`] if present
    /// and unreadable.
    pub fn load_bundle(&self) -> Result<Option<SignedBundle<PluginIndex>>, IndexError> {
        let path = self.folder.join(INDEX_FILE);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(IndexError::Io { path, source }),
        };
        let bundle: SignedBundle<PluginIndex> = serde_json::from_slice(&bytes)
            .map_err(|source| IndexError::Malformed { path, source })?;
        Ok(Some(bundle))
    }

    /// Reads the cached index's status at `now`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::Io`] or [`IndexError::Malformed`] if the
    /// persisted index cannot be read.
    pub fn status(&self, now: Timestamp) -> Result<IndexStatus, IndexError> {
        let Some(bundle) = self.load_bundle()? else {
            return Ok(IndexStatus::Absent);
        };
        let index = bundle.payload;
        match index.expires_at() {
            Ok(expires_at) if now > expires_at => Ok(IndexStatus::Expired { index, expires_at }),
            Ok(_) => Ok(IndexStatus::Fresh(index)),
            Err(_) => Ok(IndexStatus::Expired {
                index,
                expires_at: Timestamp::UNIX_EPOCH,
            }),
        }
    }

    /// Verifies `signed` against `trust`, the stored monotonic `last_seen`
    /// version, and `now`. If valid, atomically persists the index and
    /// advances `last_seen`; otherwise refuses with `plugin_index_stale`
    /// leaving the stored index and `last_seen` untouched.
    ///
    /// # Errors
    ///
    /// [`IndexError::Refused`] with `plugin_index_stale` on bad signature,
    /// untrusted signer, stale version or expiry; [`IndexError::Io`] on
    /// filesystem failure.
    pub fn apply(
        &self,
        signed: SignedBundle<PluginIndex>,
        trust: &TrustRoot,
        now: Timestamp,
    ) -> Result<PluginIndex, IndexError> {
        let last_seen = self.last_seen_version()?;
        match signed_set::verify(&signed, trust, last_seen, now) {
            SignedSetVerdict::Valid { version, .. } => {
                self.persist(&signed, version)?;
                Ok(signed.payload)
            }
            verdict => Err(IndexError::Refused(Box::new(stale_error(
                &verdict,
                Some(signed.payload.version),
                last_seen,
                Some(signed.payload.expires_at),
            )))),
        }
    }

    /// Fetches a signed Plugin index from `url` through the engine's own
    /// HTTP client (`ConfiguredKind::PluginIndex`), verifies it against
    /// `trust` and `last_seen`, and persists it when valid.
    ///
    /// # Errors
    ///
    /// [`IndexError::Fetch`] on network/HTTP/decode errors,
    /// [`IndexError::Refused`] with `plugin_index_stale` on verification
    /// refusal, or [`IndexError::Io`] on filesystem failure.
    pub async fn refresh(
        &self,
        url: &str,
        fetching: &Fetching,
        trust: &TrustRoot,
        now: Timestamp,
    ) -> Result<PluginIndex, IndexError> {
        let last_seen = self.last_seen_version()?;
        let (signed, verdict) = signed_set::fetch::<PluginIndex>(
            url,
            ConfiguredKind::PluginIndex,
            fetching,
            trust,
            last_seen,
            now,
        )
        .await?;
        match verdict {
            SignedSetVerdict::Valid { version, .. } => {
                self.persist(&signed, version)?;
                Ok(signed.payload)
            }
            verdict => Err(IndexError::Refused(Box::new(stale_error(
                &verdict,
                Some(signed.payload.version),
                last_seen,
                Some(signed.payload.expires_at),
            )))),
        }
    }

    fn persist(&self, signed: &SignedBundle<PluginIndex>, version: u64) -> Result<(), IndexError> {
        fs::create_dir_all(&self.folder).map_err(|source| IndexError::Io {
            path: self.folder.clone(),
            source,
        })?;
        let index_path = self.folder.join(INDEX_FILE);
        let index_bytes =
            serde_json::to_vec_pretty(signed).map_err(|source| IndexError::Malformed {
                path: index_path.clone(),
                source,
            })?;
        durable::replace(&index_path, &index_bytes).map_err(|source| IndexError::Io {
            path: index_path,
            source,
        })?;
        let seen_path = self.folder.join(LAST_SEEN_FILE);
        let seen_bytes =
            serde_json::to_vec(&LastSeen { version }).map_err(|source| IndexError::Malformed {
                path: seen_path.clone(),
                source,
            })?;
        durable::replace(&seen_path, &seen_bytes).map_err(|source| IndexError::Io {
            path: seen_path,
            source,
        })?;
        Ok(())
    }
}

/// Evaluates whether a Plugin (`name`, `digest`, `signer`) may start under
/// `status`: returns its resolved [`Origin`] (`FirstParty`, `ThirdParty` or
/// `Private`), or a protocol [`CallError`] (`plugin_revoked` when revoked,
/// or `plugin_index_stale` when an expired index blocks third-party
/// loading).
///
/// # Errors
///
/// `plugin_revoked` if the Plugin's digest or signer is on the revocation
/// list (even when the index is expired), or `plugin_index_stale` if the
/// index is expired and the Plugin is not first-party.
pub fn check_load(
    status: &IndexStatus,
    name: &str,
    digest: Option<&str>,
    signer: Option<&SignerIdentity>,
) -> Result<Origin, LoadRefusal> {
    if let Some(index) = status.index()
        && let Some(revocation) = index.revocation_for(digest, signer)
    {
        return Err(LoadRefusal::Revoked {
            revocation: Box::new(revocation.clone()),
            error: Box::new(revoked_error(name, digest, revocation)),
        });
    }
    let origin = status
        .index()
        .map_or(Origin::ThirdParty, |index| index.origin_of(name, digest));
    if let IndexStatus::Expired { index, .. } = status
        && origin != Origin::FirstParty
    {
        return Err(LoadRefusal::ExpiredIndex(Box::new(expired_loading_error(
            name, index,
        ))));
    }
    Ok(origin)
}

/// Why the Plugin index refused loading a Plugin.
#[derive(Debug)]
pub enum LoadRefusal {
    /// The Plugin's digest or signer is on the revocation list.
    Revoked {
        /// The matching revocation entry.
        revocation: Box<Revocation>,
        /// The `plugin_revoked` refusal to return.
        error: Box<CallError>,
    },
    /// The cached index is past its expiry, blocking third-party loading.
    ExpiredIndex(Box<CallError>),
}

impl LoadRefusal {
    /// The underlying protocol [`CallError`].
    #[must_use]
    pub fn into_call_error(self) -> CallError {
        match self {
            Self::Revoked { error, .. } | Self::ExpiredIndex(error) => *error,
        }
    }
}

/// Constructs a `plugin_revoked` [`CallError`] for `plugin` and `digest`.
#[must_use]
pub fn revoked_error(plugin: &str, digest: Option<&str>, revocation: &Revocation) -> CallError {
    let details = PluginRevokedDetails {
        plugin: plugin.to_owned(),
        digest: digest.map(str::to_owned),
        reason: revocation.reason.clone(),
        fix: format!(
            "remove the revoked Plugin with `goodfetch plugin remove {plugin}` or update to an unrevoked release"
        ),
    };
    CallError::new(
        &PLUGIN_REVOKED,
        format!("the Plugin `{plugin}` is revoked: {}", revocation.reason),
        &details,
    )
}

/// Records a `plugin_revoked` entry in `audit`, logging a warning if the
/// audit append fails.
pub async fn audit_revocation(
    audit: &AuditLog,
    plugin: &str,
    digest: Option<&str>,
    revocation: &Revocation,
    run_id: Option<RunId>,
    stopped_running: bool,
    now: Timestamp,
) {
    let origin = audit::Origin {
        actor: Actor::Engine,
        surface: None,
        layer: None,
        run_id,
    };
    let details = json!({
        "plugin": plugin,
        "digest": digest,
        "reason": revocation.reason,
        "revoked_at": revocation.revoked_at,
        "stopped_running": stopped_running,
    });
    if let Err(error) = audit
        .record(&origin, &PLUGIN_REVOKED_AUDIT, details, now)
        .await
    {
        tracing::warn!(
            plugin,
            %error,
            "the Audit log could not record a Plugin revocation",
        );
    }
}

fn stale_error(
    verdict: &SignedSetVerdict,
    version: Option<u64>,
    last_seen: Option<u64>,
    expires_at: Option<String>,
) -> CallError {
    let (reason, message) = match verdict {
        SignedSetVerdict::BadSignature | SignedSetVerdict::MalformedExpiry(_) => (
            IndexRefusalReason::BadSignature,
            "the Plugin index failed signature verification".to_owned(),
        ),
        SignedSetVerdict::UntrustedSigner { issuer, workflow } => (
            IndexRefusalReason::UntrustedSigner,
            format!(
                "the Plugin index was signed by untrusted workflow `{workflow}` (issuer `{issuer}`)"
            ),
        ),
        SignedSetVerdict::Stale { version, last_seen } => (
            IndexRefusalReason::Stale,
            format!(
                "the Plugin index version {version} is older than the last seen version {last_seen}"
            ),
        ),
        SignedSetVerdict::Expired { expires_at, .. } => (
            IndexRefusalReason::Expired,
            format!("the Plugin index expired at {expires_at}"),
        ),
        SignedSetVerdict::Valid { .. } => (
            IndexRefusalReason::Stale,
            "the Plugin index was refused".to_owned(),
        ),
    };
    let details = PluginIndexStaleDetails {
        reason,
        version,
        last_seen,
        expires_at,
        fix: "refresh the Plugin index or import a fresh signed index with `goodfetch plugin index import <file>`".to_owned(),
    };
    CallError::new(&PLUGIN_INDEX_STALE, message, &details)
}

fn expired_loading_error(plugin: &str, index: &PluginIndex) -> CallError {
    let details = PluginIndexStaleDetails {
        reason: IndexRefusalReason::Expired,
        version: Some(index.version),
        last_seen: Some(index.version),
        expires_at: Some(index.expires_at.clone()),
        fix: "refresh the Plugin index or import a fresh signed index with `goodfetch plugin index import <file>` before loading third-party Plugins".to_owned(),
    };
    CallError::new(
        &PLUGIN_INDEX_STALE,
        format!(
            "cannot load third-party Plugin `{plugin}`: the Plugin index (version {}) expired at {}",
            index.version, index.expires_at
        ),
        &details,
    )
}

/// Filters `index`'s listed Plugins by optional `query` (case-insensitive
/// substring across name, OCI reference, kinds and signer identity) and
/// optional `kind`, returning matching entries sorted by name (ADR 0033,
/// issue 919).
#[must_use]
pub fn search_entries(
    index: &PluginIndex,
    query: Option<&str>,
    kind: Option<&PluginKindName>,
) -> Vec<IndexEntry> {
    let needle = query
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_ascii_lowercase);
    let mut matches: Vec<IndexEntry> = index
        .plugins
        .iter()
        .filter(|entry| kind.is_none_or(|wanted| entry.kinds.contains(wanted)))
        .filter(|entry| {
            let Some(needle) = needle.as_deref() else {
                return true;
            };
            entry.name.to_ascii_lowercase().contains(needle)
                || entry.reference.to_ascii_lowercase().contains(needle)
                || entry
                    .signer
                    .repository
                    .to_ascii_lowercase()
                    .contains(needle)
                || entry.signer.workflow.to_ascii_lowercase().contains(needle)
                || entry
                    .kinds
                    .iter()
                    .any(|entry_kind| entry_kind.as_str().to_ascii_lowercase().contains(needle))
        })
        .cloned()
        .collect();
    matches.sort_by(|left, right| left.name.cmp(&right.name));
    matches
}

/// Builds a [`PluginSearchResult`] from a cached [`IndexStatus`] and
/// [`PluginSearchParams`] (ADR 0033, issue 919).
#[must_use]
pub fn search_status(status: &IndexStatus, params: &PluginSearchParams) -> PluginSearchResult {
    match status {
        IndexStatus::Absent => PluginSearchResult {
            index_version: None,
            expires_at: None,
            expired: false,
            plugins: Vec::new(),
        },
        IndexStatus::Fresh(index) => PluginSearchResult {
            index_version: Some(index.version),
            expires_at: Some(index.expires_at.clone()),
            expired: false,
            plugins: search_entries(index, params.query.as_deref(), params.kind.as_ref()),
        },
        IndexStatus::Expired { index, .. } => PluginSearchResult {
            index_version: Some(index.version),
            expires_at: Some(index.expires_at.clone()),
            expired: true,
            plugins: search_entries(index, params.query.as_deref(), params.kind.as_ref()),
        },
    }
}

/// Records a `plugin_index_imported` Audit entry for `applied` and returns
/// the [`PluginIndexImported`] summary (ADR 0033, issue 919).
///
/// # Errors
///
/// An internal error if the Audit log cannot record the entry.
pub async fn record_import(
    audit: &AuditLog,
    origin: &audit::Origin,
    applied: &PluginIndex,
    now: Timestamp,
) -> Result<PluginIndexImported, CallError> {
    let plugins = u64::try_from(applied.plugins.len()).unwrap_or(u64::MAX);
    let revocations = u64::try_from(applied.revocations.len()).unwrap_or(u64::MAX);
    let details = json!({
        "version": applied.version,
        "signed_at": applied.signed_at,
        "expires_at": applied.expires_at,
        "plugins": plugins,
        "revocations": revocations,
    });
    audit
        .record(origin, &PLUGIN_INDEX_IMPORTED_AUDIT, details, now)
        .await
        .map_err(|error| {
            CallError::protocol(
                goodfetch_protocol::jsonrpc::ProtocolFailure::InternalError,
                format!("the Audit log could not record the imported Plugin index: {error}"),
            )
        })?;
    Ok(PluginIndexImported {
        version: applied.version,
        signed_at: applied.signed_at.clone(),
        expires_at: applied.expires_at.clone(),
        plugins,
        revocations,
    })
}

/// Verifies `bundle` offline against `trust`, persists it in `store`, and
/// records `plugin_index_imported` in `audit` (ADR 0033, issue 919).
///
/// # Errors
///
/// `plugin_index_stale` when verification fails (leaving `store` untouched),
/// or an internal error if the store or Audit log cannot be written.
pub async fn import_and_audit(
    store: &PluginIndexStore,
    audit: &AuditLog,
    origin: &audit::Origin,
    bundle: SignedBundle<PluginIndex>,
    trust: &TrustRoot,
    now: Timestamp,
) -> Result<PluginIndexImported, CallError> {
    let applied = store
        .apply(bundle, trust, now)
        .map_err(IndexError::into_call_error)?;
    record_import(audit, origin, &applied, now).await
}
