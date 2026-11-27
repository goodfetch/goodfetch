//! Shared verification and fetching for signed, versioned, expiring sets
//! (ADRs 0033, 0043, 0047 and 0059; issue 911): serves both the Plugin
//! index and area 4's Clean-up rules.
//!
//! Each set is carried as a [`SignedBundle<T>`] pairing its JSON payload
//! with a [`SignatureBundle`] binding the key ID, OIDC issuer and CI
//! workflow identity to the payload's RFC 8785 canonical JSON. Checking
//! verifies the Ed25519 signature against a [`TrustRoot`] (bundled with
//! the release or injected as a value in tests, ADR 0059), checks the
//! signer claims against the trust root, enforces monotonic freshness
//! (`version >= last_seen`), and checks expiry (`now <= expires_at`),
//! returning a pure [`SignedSetVerdict`] so each caller decides what
//! follows.
//!
//! Dependency decision (STYLE.md dependency bar): `sigstore-verify` was
//! checked and declined because it pulls in its own TUF/HTTP/TLS stack
//! outside ADR 0043's single outbound path; verification here uses the
//! workspace's existing `ed25519-dalek`, `serde_json_canonicalizer` and
//! `data-encoding` crates, and network pulls go through `rule_sources` on
//! the engine's own `http_client`. No TUF client (`tough`) is used.

use std::collections::BTreeMap;

use data_encoding::BASE64;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use goodfetch_protocol::plugin::index::{PluginIndex, SignatureBundle, SignedBundle};
use jiff::Timestamp;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::egress::ConfiguredKind;
use crate::pinned_download::Fetching;
use crate::rule_sources::{self, SourceFailure};

/// Domain separation prefix included in every signed-set message.
const DOMAIN_TAG: &[u8] = b"goodfetch-signed-set-v1\n";

/// Maximum size of a signed-set JSON document fetched over HTTPS (4 MiB).
pub const MAX_SET_BYTES: usize = 4 * 1024 * 1024;

/// Default OIDC issuer for GitHub Actions release workflows.
pub const DEFAULT_ISSUER: &str = "https://token.actions.githubusercontent.com";

/// Default CI workflow signing the official Goodfetch Plugin index.
pub const DEFAULT_INDEX_WORKFLOW: &str =
    "https://github.com/goodfetch/goodfetch/.github/workflows/plugin-index.yml@refs/heads/main";

/// A trust root against which a [`SignedBundle`] is verified: the expected
/// OIDC issuer and CI workflow identity, plus the trusted Ed25519
/// verifying keys keyed by `key_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrustRoot {
    /// Expected OIDC issuer in the bundle.
    pub issuer: String,
    /// Expected CI workflow identity in the bundle.
    pub workflow: String,
    /// Trusted Ed25519 verifying keys (32-byte raw public keys), by `key_id`.
    pub keys: BTreeMap<String, [u8; 32]>,
}

impl TrustRoot {
    /// Constructs a trust root from explicit values (for bundled release
    /// configuration or test injection, ADR 0059).
    #[must_use]
    pub fn new(
        issuer: impl Into<String>,
        workflow: impl Into<String>,
        keys: BTreeMap<String, [u8; 32]>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            workflow: workflow.into(),
            keys,
        }
    }

    /// The trust root bundled with this release for the official Plugin
    /// index. Before the first published release signs an index with a
    /// production release key, the bundled key map is empty so no forged
    /// signature verifies against it.
    #[must_use]
    pub fn bundled() -> Self {
        Self {
            issuer: DEFAULT_ISSUER.to_owned(),
            workflow: DEFAULT_INDEX_WORKFLOW.to_owned(),
            keys: BTreeMap::new(),
        }
    }
}

/// A deterministic Ed25519 signing key paired with its bundle identity,
/// used to produce [`SignedBundle`] values under a test or CI trust root.
#[derive(Clone)]
pub struct SignedSetSigner {
    key_id: String,
    issuer: String,
    workflow: String,
    signing_key: SigningKey,
}

impl SignedSetSigner {
    /// Creates a deterministic signer from `seed` and returns `(signer,
    /// trust_root)` trusting only this signer's key and workflow identity.
    #[must_use]
    pub fn for_root(
        key_id: impl Into<String>,
        issuer: impl Into<String>,
        workflow: impl Into<String>,
        seed: [u8; 32],
    ) -> (Self, TrustRoot) {
        let key_id = key_id.into();
        let issuer = issuer.into();
        let workflow = workflow.into();
        let signing_key = SigningKey::from_bytes(&seed);
        let public_bytes = signing_key.verifying_key().to_bytes();
        let mut keys = BTreeMap::new();
        keys.insert(key_id.clone(), public_bytes);
        let root = TrustRoot::new(issuer.clone(), workflow.clone(), keys);
        let signer = Self {
            key_id,
            issuer,
            workflow,
            signing_key,
        };
        (signer, root)
    }

    /// Signs `payload` under this signer's `key_id`, `issuer` and
    /// `workflow`.
    ///
    /// # Errors
    ///
    /// Fails if `payload` cannot be serialized to RFC 8785 canonical JSON.
    pub fn sign<T: Serialize>(&self, payload: T) -> Result<SignedBundle<T>, serde_json::Error> {
        self.sign_with_claims(payload, &self.key_id, &self.issuer, &self.workflow)
    }

    /// Signs `payload` while stamping explicit bundle claims (`key_id`,
    /// `issuer`, `workflow`), allowing tests to construct a validly signed
    /// bundle from an untrusted workflow or issuer.
    ///
    /// # Errors
    ///
    /// Fails if `payload` cannot be serialized to RFC 8785 canonical JSON.
    pub fn sign_with_claims<T: Serialize>(
        &self,
        payload: T,
        key_id: &str,
        issuer: &str,
        workflow: &str,
    ) -> Result<SignedBundle<T>, serde_json::Error> {
        let message = signing_message(&payload, issuer, workflow)?;
        let signature = self.signing_key.sign(&message);
        Ok(SignedBundle {
            payload,
            bundle: SignatureBundle {
                key_id: key_id.to_owned(),
                issuer: issuer.to_owned(),
                workflow: workflow.to_owned(),
                signature: BASE64.encode(&signature.to_bytes()),
            },
        })
    }
}

/// Any versioned, expiring set payload verified by [`verify`]: both the
/// Plugin index and area 4's Clean-up rules implement this trait
/// (ADR 0047).
pub trait SignedSetPayload: Serialize {
    /// Its monotonic freshness version.
    fn version(&self) -> u64;

    /// Its expiry timestamp, parsed from RFC 3339.
    ///
    /// # Errors
    ///
    /// Returns a message describing why the expiry timestamp could not be
    /// parsed.
    fn expires_at(&self) -> Result<Timestamp, String>;
}

impl SignedSetPayload for PluginIndex {
    fn version(&self) -> u64 {
        self.version
    }

    fn expires_at(&self) -> Result<Timestamp, String> {
        self.expires_at
            .parse::<Timestamp>()
            .map_err(|error| format!("invalid expires_at `{}`: {error}", self.expires_at))
    }
}

/// The pure verdict [`verify`] reports for a [`SignedBundle`], leaving each
/// caller to decide how to act on it (ADR 0047).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignedSetVerdict {
    /// Signature verified under `trust`, signer identity matched, `version`
    /// is at least `last_seen`, and `now` is not past `expires_at`.
    Valid {
        /// The set's freshness version.
        version: u64,
        /// The set's expiry timestamp.
        expires_at: Timestamp,
    },
    /// The signature failed verification, the `key_id` is unknown in
    /// `trust`, or the canonical payload did not encode.
    BadSignature,
    /// The signature verified under a trusted key, but the bundle's OIDC
    /// `issuer` or `workflow` did not match `trust`.
    UntrustedSigner {
        /// The bundle's issuer.
        issuer: String,
        /// The bundle's workflow.
        workflow: String,
    },
    /// The set's `version` is strictly older than `last_seen`.
    Stale {
        /// The received version.
        version: u64,
        /// The last version previously accepted.
        last_seen: u64,
    },
    /// The set is past its `expires_at` timestamp at `now`.
    Expired {
        /// The received version.
        version: u64,
        /// The set's expiry timestamp.
        expires_at: Timestamp,
    },
    /// The set's `expires_at` field could not be parsed as RFC 3339.
    MalformedExpiry(String),
}

/// Checks `signed` against `trust`, `last_seen` and `now`, returning a pure
/// [`SignedSetVerdict`] without side effects.
#[must_use]
pub fn verify<T: SignedSetPayload>(
    signed: &SignedBundle<T>,
    trust: &TrustRoot,
    last_seen: Option<u64>,
    now: Timestamp,
) -> SignedSetVerdict {
    if !signature_valid(signed, trust) {
        return SignedSetVerdict::BadSignature;
    }
    if signed.bundle.issuer != trust.issuer || signed.bundle.workflow != trust.workflow {
        return SignedSetVerdict::UntrustedSigner {
            issuer: signed.bundle.issuer.clone(),
            workflow: signed.bundle.workflow.clone(),
        };
    }
    let version = signed.payload.version();
    if let Some(seen) = last_seen
        && version < seen
    {
        return SignedSetVerdict::Stale {
            version,
            last_seen: seen,
        };
    }
    let expires_at = match signed.payload.expires_at() {
        Ok(timestamp) => timestamp,
        Err(reason) => return SignedSetVerdict::MalformedExpiry(reason),
    };
    if now > expires_at {
        return SignedSetVerdict::Expired {
            version,
            expires_at,
        };
    }
    SignedSetVerdict::Valid {
        version,
        expires_at,
    }
}

/// Why fetching a signed set from its HTTPS URL did not produce a decoded
/// [`SignedBundle`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// Transport, offline, egress or size failure from `rule_sources`.
    #[error("{0}")]
    Source(#[from] SourceFailure),
    /// The server answered with a non-200 HTTP status.
    #[error("the signed set endpoint answered HTTP {0}")]
    HttpStatus(u16),
    /// The body was not valid JSON matching the expected signed bundle.
    #[error("the signed set did not decode: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// Fetches a [`SignedBundle<T>`] from `url` as a configured endpoint of
/// `kind` through the engine's own HTTP client and checks it against
/// `trust`, `last_seen` and `now`.
///
/// # Errors
///
/// [`FetchError`] when transport, egress, offline, HTTP status or JSON
/// decoding fails. A signature, signer, freshness or expiry failure is
/// returned in [`SignedSetVerdict`] so the caller decides what follows.
pub async fn fetch<T: DeserializeOwned + SignedSetPayload>(
    url: &str,
    kind: ConfiguredKind,
    fetching: &Fetching,
    trust: &TrustRoot,
    last_seen: Option<u64>,
    now: Timestamp,
) -> Result<(SignedBundle<T>, SignedSetVerdict), FetchError> {
    let fetched = rule_sources::get_for(url, kind, &[], fetching, MAX_SET_BYTES).await?;
    if fetched.status != 200 {
        return Err(FetchError::HttpStatus(fetched.status));
    }
    let signed: SignedBundle<T> = serde_json::from_slice(&fetched.body)?;
    let verdict = verify(&signed, trust, last_seen, now);
    Ok((signed, verdict))
}

fn signature_valid<T: Serialize>(signed: &SignedBundle<T>, trust: &TrustRoot) -> bool {
    let Some(public_bytes) = trust.keys.get(&signed.bundle.key_id) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(public_bytes) else {
        return false;
    };
    let Ok(sig_bytes) = BASE64.decode(signed.bundle.signature.as_bytes()) else {
        return false;
    };
    let Ok(sig_array): Result<[u8; 64], _> = sig_bytes.try_into() else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_array);
    let Ok(message) = signing_message(
        &signed.payload,
        &signed.bundle.issuer,
        &signed.bundle.workflow,
    ) else {
        return false;
    };
    verifying_key.verify(&message, &signature).is_ok()
}

fn signing_message<T: Serialize>(
    payload: &T,
    issuer: &str,
    workflow: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    let canonical = serde_json_canonicalizer::to_vec(payload)?;
    let mut message = Vec::with_capacity(
        DOMAIN_TAG.len() + issuer.len() + 1 + workflow.len() + 1 + canonical.len(),
    );
    message.extend_from_slice(DOMAIN_TAG);
    message.extend_from_slice(issuer.as_bytes());
    message.push(0);
    message.extend_from_slice(workflow.as_bytes());
    message.push(0);
    message.extend_from_slice(&canonical);
    Ok(message)
}
