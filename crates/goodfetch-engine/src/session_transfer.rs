//! Moving a Session between machines (ADRs 0015 and 0056): its export to a
//! session export bundle and its import from one. Each resolves its
//! passphrase or identity through the Run's Secrets, does the bundle's
//! scrypt work on the Secrets' owner, and is recorded in the Audit log
//! before its caller gets the bundle or the Session, without a value:
//! whom it was exported to or the reference it opened under, the bundle's
//! SHA-256 digest, which an export and its import share, and how many
//! cookies and origins it holds.

use goodfetch_protocol::audit::{SESSION_EXPORTED, SESSION_IMPORTED};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::session_export::SessionExport;
use goodfetch_protocol::settings::SESSION_SOURCE;
use secrecy::SecretString;
use serde_json::{Value, json};

use crate::audit::{AuditError, Origin};
use crate::call_error::CallError;
use crate::pinned_download::Digest;
use crate::run::secret_error;
use crate::secrets::{Purpose, RunSecrets, SecretError};
use crate::session_bundle::{self, Key, Limits, OpenError, PublicKey, To};

#[cfg(test)]
mod tests;

/// What an export's passphrase is resolved for, as the Audit log and the
/// mask name it: no setting holds it.
const EXPORT_PASSPHRASE: &str = "session_export.passphrase";

/// Whom an export is encrypted to.
pub enum Recipient {
    /// The holder of this public key's identity.
    PublicKey(PublicKey),
    /// Whoever knows the passphrase this reference gives.
    Passphrase {
        /// The Secret reference, never a raw passphrase.
        reference: SecretReference,
        /// The scrypt work, as [`To::Passphrase`]'s.
        work_factor: u8,
    },
}

/// Why a Session was not exported or imported.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    /// Its passphrase or identity gave no value.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// The bundle did not open.
    #[error(transparent)]
    Open(#[from] OpenError),
    /// It could not be recorded in the Audit log, so the bundle or the
    /// Session was dropped unused.
    #[error("the session {} cannot be recorded: {source}", direction.as_str())]
    Unrecorded {
        /// Which of the two it was.
        direction: Direction,
        /// The Audit log's failure.
        #[source]
        source: AuditError,
    },
}

/// Which way a Session moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// To a bundle.
    Export,
    /// From a bundle.
    Import,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Export => "export",
            Self::Import => "import",
        }
    }
}

impl TransferError {
    /// The failure a call answers with.
    pub fn to_call_error(&self) -> CallError {
        match self {
            Self::Secret(error) => secret_error(error),
            Self::Open(error) => error.to_call_error(),
            Self::Unrecorded { .. } => {
                tracing::error!("{self}");
                CallError::protocol(ProtocolFailure::InternalError, self.to_string())
            }
        }
    }
}

/// `session` as a bundle encrypted to `recipient`, recorded as
/// `session_exported` by `origin`.
///
/// # Errors
///
/// [`TransferError`], but [`TransferError::Open`].
///
/// # Cancel safety
///
/// Dropping the future may leave the export recorded, its bundle unread.
pub async fn export(
    session: SessionExport,
    recipient: Recipient,
    origin: &Origin,
    secrets: &RunSecrets,
) -> Result<Vec<u8>, TransferError> {
    let counts = Counts::of(&session);
    let (to, recorded_as) = match recipient {
        Recipient::PublicKey(key) => {
            let recorded_as = json!({ "public_key": key.to_string() });
            (To::PublicKey(key), recorded_as)
        }
        Recipient::Passphrase {
            reference,
            work_factor,
        } => {
            let passphrase = resolved(&reference, EXPORT_PASSPHRASE, origin, secrets).await?;
            (
                To::Passphrase {
                    passphrase,
                    work_factor,
                },
                json!({ "reference": reference.as_str() }),
            )
        }
    };
    let bundle = secrets
        .owner()
        .blocking(move || session_bundle::seal(&session, &to))
        .await;
    let details = counts.details(&recorded_as, &bundle);
    secrets
        .audit(origin, &SESSION_EXPORTED, details)
        .await
        .map_err(|source| TransferError::Unrecorded {
            direction: Direction::Export,
            source,
        })?;
    Ok(bundle)
}

/// The Session `bundle` holds, opened within `limits` under the value
/// `key` gives, the age identity or the passphrase it was exported to,
/// recorded as `session_imported` by `origin`.
///
/// # Errors
///
/// [`TransferError`].
///
/// # Cancel safety
///
/// Dropping the future may leave the import recorded, its Session unread.
pub async fn import(
    bundle: Vec<u8>,
    key: &SecretReference,
    limits: Limits,
    origin: &Origin,
    secrets: &RunSecrets,
) -> Result<SessionExport, TransferError> {
    let value = resolved(key, SESSION_SOURCE.name, origin, secrets).await?;
    let reference = key.clone();
    let (bundle, opened) = secrets
        .owner()
        .blocking(move || {
            let key = Key {
                reference: &reference,
                value: &value,
            };
            let opened = session_bundle::open(&bundle, &key, limits);
            (bundle, opened)
        })
        .await;
    let session = opened?;
    let details = Counts::of(&session).details(&json!({ "reference": key.as_str() }), &bundle);
    secrets
        .audit(origin, &SESSION_IMPORTED, details)
        .await
        .map_err(|source| TransferError::Unrecorded {
            direction: Direction::Import,
            source,
        })?;
    Ok(session)
}

/// The value `reference` gives, resolved now for `placeholder`, its use
/// recorded.
async fn resolved(
    reference: &SecretReference,
    placeholder: &str,
    origin: &Origin,
    secrets: &RunSecrets,
) -> Result<SecretString, SecretError> {
    secrets
        .resolve(
            reference,
            &Purpose {
                placeholder,
                domain: None,
                domains: &[],
                layer: origin.layer,
                run_id: origin.run_id.as_ref(),
            },
        )
        .await
}

/// How much a Session holds.
struct Counts {
    cookies: usize,
    origins: usize,
}

impl Counts {
    fn of(session: &SessionExport) -> Self {
        Self {
            cookies: session.cookies.len(),
            origins: session.origins.len(),
        }
    }

    /// The Audit entry's details of `bundle`, holding this much, under
    /// `key`, whom it was exported to or what it opened under.
    fn details(&self, key: &Value, bundle: &[u8]) -> Value {
        json!({
            "key": key,
            "bundle_sha256": Digest::of(bundle).hex(),
            "cookies": self.cookies,
            "origins": self.origins,
        })
    }
}
