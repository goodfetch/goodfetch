//! Session export bundles (ADRs 0015 and 0056): a Session's cookies and
//! storage, as the registry's `session-export` document, encrypted with
//! `age` to one public key or to one passphrase and ASCII-armored, so a
//! login moves to another machine, such as CI, without showing in transit.
//!
//! Opening one is a decoder of files from elsewhere (ADR 0067), which the
//! `session_bundle` fuzz target reaches: an armored or a binary age file,
//! within the [`Limits`] the caller allows. Every failure names the Secret
//! reference given as the key, never its value.

use std::fmt;
use std::io::{Read as _, Write as _};
use std::iter;
use std::str::FromStr;

use age::armor::{ArmoredReader, ArmoredWriter, Format as Armor};
use age::{DecryptError, Decryptor, Encryptor, scrypt, x25519};
use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::checkpoint::{FORMAT_VERSION_UNSUPPORTED, FormatVersionUnsupportedDetails};
use goodfetch_protocol::formats::{Compatibility, SESSION_EXPORT};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{
    SESSION_EXPORT_OPEN_FAILED, SessionExportFailure, SessionExportOpenFailedDetails,
};
use goodfetch_protocol::session_export::SessionExport;
use secrecy::zeroize::Zeroizing;
use secrecy::{ExposeSecret as _, SecretString};

use crate::call_error::CallError;

#[cfg(test)]
mod tests;

/// The scrypt work factor, as `log2(N)`, an export to a passphrase takes:
/// 18, about a second and 256 MiB, age's own target on a typical machine.
pub const EXPORT_WORK_FACTOR: u8 = 18;

/// The most an import spends on a bundle, which may be hostile.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// The most scrypt work for a passphrase, as `log2(N)`.
    pub work_factor: u8,
    /// The most the bundle may hold once decrypted; a larger one is refused
    /// before it is parsed.
    pub document_bytes: u64,
}

/// What every import allows: 20 of scrypt work, 1 GiB, so a bundle age
/// exported on a faster machine still opens while a hostile one cannot make
/// the engine spend hours or tens of GiB; and 32 MiB, since a Session's
/// cookies and storage are far less.
pub const IMPORT_LIMITS: Limits = Limits {
    work_factor: 20,
    document_bytes: 32 * 1024 * 1024,
};

/// An age public key (`age1…`) a bundle is exported to.
#[derive(Clone)]
pub struct PublicKey(x25519::Recipient);

impl FromStr for PublicKey {
    type Err = NotAPublicKey;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        x25519::Recipient::from_str(text.trim())
            .map(Self)
            .map_err(|_| NotAPublicKey)
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Text that is no age public key.
#[derive(Debug, thiserror::Error)]
#[error("not an age public key, which starts with `age1`")]
pub struct NotAPublicKey;

/// Whom a bundle is encrypted to: one recipient, since age never mixes a
/// passphrase with other recipients.
pub enum To {
    /// The holder of this public key's identity.
    PublicKey(PublicKey),
    /// Whoever knows this passphrase, at this scrypt work factor.
    Passphrase {
        /// The passphrase, resolved from its Secret reference.
        passphrase: SecretString,
        /// `log2(N)`, from 1 to 63: [`EXPORT_WORK_FACTOR`] outside tests.
        work_factor: u8,
    },
}

/// The key a bundle is opened with: the Secret reference given as its
/// `key`, and the value it resolved to, an age identity or a passphrase.
pub struct Key<'a> {
    /// The reference, which every failure names.
    pub reference: &'a SecretReference,
    /// Its value, which no failure shows.
    pub value: &'a SecretString,
}

/// Why a bundle did not open.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    /// It does not open under its key: `session_export_open_failed`.
    #[error("the session export bundle did not open under `{reference}`: {}", explain(*failure))]
    Failed {
        /// The Secret reference given as its key.
        reference: SecretReference,
        /// What failed.
        failure: SessionExportFailure,
    },
    /// It opened, and holds a version of the format this engine does not
    /// read: `format_version_unsupported`.
    #[error("the session export bundle holds `session-export` {found}, which this engine does not read (it reads up to {})", SESSION_EXPORT.version)]
    Version {
        /// The version it holds.
        found: ProtocolVersion,
        /// How it compares with the engine's.
        compatibility: Compatibility,
    },
}

impl OpenError {
    /// The failure a call answers with.
    pub fn to_call_error(&self) -> CallError {
        let message = self.to_string();
        match self {
            Self::Failed { reference, failure } => CallError::new(
                &SESSION_EXPORT_OPEN_FAILED,
                message,
                &SessionExportOpenFailedDetails {
                    reference: reference.to_string(),
                    failure: *failure,
                    fix: fix(*failure).to_owned(),
                },
            ),
            Self::Version {
                found,
                compatibility,
            } => CallError::new(
                &FORMAT_VERSION_UNSUPPORTED,
                message,
                &FormatVersionUnsupportedDetails {
                    format: SESSION_EXPORT.name.to_owned(),
                    found: found.to_string(),
                    supported: SESSION_EXPORT.version.to_string(),
                    fix: match compatibility {
                        Compatibility::OlderMajor => {
                            "Export the Session again with this Goodfetch, then import that bundle."
                                .to_owned()
                        }
                        Compatibility::NewerMinor
                        | Compatibility::NewerMajor
                        | Compatibility::Compatible => format!(
                            "Upgrade Goodfetch to a version that reads `session-export` {found}."
                        ),
                    },
                },
            ),
        }
    }
}

fn explain(failure: SessionExportFailure) -> &'static str {
    match failure {
        SessionExportFailure::WrongKey => "its value opens none of the bundle's recipients",
        SessionExportFailure::NotAnIdentity => {
            "the bundle was exported to a public key, and its value is no age identity"
        }
        SessionExportFailure::NotABundle => "the file is no session export bundle, whole",
        SessionExportFailure::ExcessiveWork => {
            "its passphrase asks more scrypt work than an import does"
        }
        SessionExportFailure::TooLarge => "it holds more than 32 MiB",
    }
}

fn fix(failure: SessionExportFailure) -> &'static str {
    match failure {
        SessionExportFailure::WrongKey => {
            "Give the reference of the identity or the passphrase the bundle was exported to."
        }
        SessionExportFailure::NotAnIdentity => {
            "Give the reference of the age identity (`AGE-SECRET-KEY-1…`) of the public key the bundle was exported to."
        }
        SessionExportFailure::NotABundle => {
            "Give the bundle a session export wrote, unchanged, or export the Session again."
        }
        SessionExportFailure::ExcessiveWork => {
            "Export the Session again with Goodfetch, whose exports any import opens."
        }
        SessionExportFailure::TooLarge => "Export a Session that holds less than 32 MiB.",
    }
}

/// `export` as the format's document: JSON, one line per field.
pub fn document(export: &SessionExport) -> Zeroizing<Vec<u8>> {
    let mut bytes = Zeroizing::new(
        serde_json::to_vec_pretty(export).expect("invariant: a Session export serializes"),
    );
    bytes.push(b'\n');
    bytes
}

/// `export` encrypted to `to`, as an ASCII-armored age file. It blocks for
/// the scrypt work of a passphrase: run it under a blocking policy.
pub fn seal(export: &SessionExport, to: &To) -> Vec<u8> {
    seal_document(&document(export), to)
}

/// `document` encrypted to `to`, as [`seal`] does.
fn seal_document(document: &[u8], to: &To) -> Vec<u8> {
    let passphrase;
    let recipient: &dyn age::Recipient = match to {
        To::PublicKey(PublicKey(key)) => key,
        To::Passphrase {
            passphrase: value,
            work_factor,
        } => {
            let mut recipient = scrypt::Recipient::new(value.clone());
            recipient.set_work_factor(*work_factor);
            passphrase = recipient;
            &passphrase
        }
    };
    let encryptor = Encryptor::with_recipients(iter::once(recipient))
        .expect("invariant: one recipient encrypts");
    let mut bundle = Vec::new();
    let armored = ArmoredWriter::wrap_output(&mut bundle, Armor::AsciiArmor)
        .expect("invariant: memory takes the armor");
    let mut writer = encryptor
        .wrap_output(armored)
        .expect("invariant: memory takes the header");
    writer
        .write_all(document)
        .and_then(|()| writer.finish())
        .and_then(ArmoredWriter::finish)
        .expect("invariant: memory takes the payload");
    bundle
}

/// The Session `bundle` holds, opened under `key`: as a passphrase when the
/// bundle was exported to one, else as an age identity, the first
/// `AGE-SECRET-KEY-1` line of its value, within `limits`. It blocks for the
/// scrypt work: run it under a blocking policy.
///
/// # Errors
///
/// [`OpenError::Failed`] for each [`SessionExportFailure`], and
/// [`OpenError::Version`] for a document of a version this engine does not
/// read.
pub fn open(bundle: &[u8], key: &Key<'_>, limits: Limits) -> Result<SessionExport, OpenError> {
    let failed = |failure| OpenError::Failed {
        reference: key.reference.clone(),
        failure,
    };
    let decryptor = Decryptor::new_buffered(ArmoredReader::new(bundle))
        .map_err(|error| failed(failure_of(&error)))?;
    let identity: Box<dyn age::Identity> = if decryptor.is_scrypt() {
        let mut identity = scrypt::Identity::new(key.value.clone());
        identity.set_max_work_factor(limits.work_factor);
        Box::new(identity)
    } else {
        let line = key
            .value
            .expose_secret()
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("AGE-SECRET-KEY-1"))
            .ok_or_else(|| failed(SessionExportFailure::NotAnIdentity))?;
        Box::new(
            x25519::Identity::from_str(line)
                .map_err(|_| failed(SessionExportFailure::NotAnIdentity))?,
        )
    };
    let reader = decryptor
        .decrypt(iter::once(identity.as_ref()))
        .map_err(|error| failed(failure_of(&error)))?;
    let mut document = Zeroizing::new(Vec::new());
    reader
        .take(limits.document_bytes.saturating_add(1))
        .read_to_end(&mut document)
        .map_err(|_| failed(SessionExportFailure::NotABundle))?;
    if u64::try_from(document.len()).unwrap_or(u64::MAX) > limits.document_bytes {
        return Err(failed(SessionExportFailure::TooLarge));
    }
    read_document(&document).map_err(|refused| match refused {
        Refused::Malformed => failed(SessionExportFailure::NotABundle),
        Refused::Version {
            found,
            compatibility,
        } => OpenError::Version {
            found,
            compatibility,
        },
    })
}

/// Why a decrypted document is not read.
enum Refused {
    Malformed,
    Version {
        found: ProtocolVersion,
        compatibility: Compatibility,
    },
}

/// The Session `document` holds: its version decides first, so a document
/// of a newer minor with fields this engine does not know is refused for
/// its version, not as malformed.
fn read_document(document: &[u8]) -> Result<SessionExport, Refused> {
    let value: serde_json::Value =
        serde_json::from_slice(document).map_err(|_| Refused::Malformed)?;
    let found: ProtocolVersion = value
        .get("format_version")
        .and_then(|version| serde_json::from_value(version.clone()).ok())
        .ok_or(Refused::Malformed)?;
    match SESSION_EXPORT
        .rule
        .compatibility(SESSION_EXPORT.version, found)
    {
        Compatibility::Compatible => serde_json::from_value(value).map_err(|_| Refused::Malformed),
        compatibility => Err(Refused::Version {
            found,
            compatibility,
        }),
    }
}

/// What an age failure means for the person importing the bundle.
fn failure_of(error: &DecryptError) -> SessionExportFailure {
    match error {
        DecryptError::NoMatchingKeys
        | DecryptError::KeyDecryptionFailed
        | DecryptError::DecryptionFailed => SessionExportFailure::WrongKey,
        DecryptError::ExcessiveWork { .. } => SessionExportFailure::ExcessiveWork,
        _ => SessionExportFailure::NotABundle,
    }
}

/// The fuzz target's body for session export bundles (ADR 0067): opens
/// `data` under a fixed passphrase at little scrypt work, as an import
/// does, and reads it as a decrypted document; a document it reads must
/// come back the same through a seal and an open. It may refuse `data`,
/// never panic.
pub fn fuzz_session_bundle(data: &[u8]) {
    let reference =
        SecretReference::parse("env:FUZZ_PASSPHRASE").expect("invariant: a Secret reference");
    let passphrase = SecretString::from("fuzz passphrase");
    let key = Key {
        reference: &reference,
        value: &passphrase,
    };
    let limits = Limits {
        work_factor: 2,
        ..IMPORT_LIMITS
    };
    let _ = open(data, &key, limits);
    if let Ok(export) = read_document(data) {
        let sealed = seal(
            &export,
            &To::Passphrase {
                passphrase: passphrase.clone(),
                work_factor: 1,
            },
        );
        let reopened = open(&sealed, &key, limits).expect("invariant: a sealed bundle opens");
        assert_eq!(
            document(&reopened).as_slice(),
            document(&export).as_slice(),
            "a document read back differs after a seal and an open"
        );
    }
}
