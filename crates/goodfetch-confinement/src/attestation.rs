//! Offline Sigstore provenance attestation verification shared by the engine
//! (`plugin add`, `plugin setup`) and `goodfetch-filter-broker` (caller
//! release-attestation verification without linking `goodfetch-engine`,
//! ADRs 0033, 0039 and 0059).
//!
//! Uses Ed25519 over RFC 8785 canonical JSON with a domain separation tag,
//! avoiding any crate that brings its own HTTP/TUF stack outside ADR 0043's
//! single outbound path.

use std::collections::BTreeMap;

use data_encoding::{BASE64, HEXLOWER};
use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};
use goodfetch_protocol::plugin::add::AttestationFailureReason;
use goodfetch_protocol::plugin::index::{
    ProvenanceAttestation, ProvenanceStatement, SignerIdentity,
};
use sha2::{Digest as _, Sha256};

/// Domain separation prefix included in every provenance attestation signing
/// payload.
const ATTESTATION_DOMAIN_TAG: &[u8] = b"goodfetch-plugin-provenance-v1\n";

/// Expected Sigstore signer identity for official Goodfetch release artifacts
/// (engine executables, `goodfetch-sandbox`, `goodfetch-filter-broker`,
/// ADR 0039).
#[must_use]
pub fn release_signer() -> SignerIdentity {
    SignerIdentity {
        repository: "goodfetch/goodfetch".to_owned(),
        workflow: ".github/workflows/release.yml".to_owned(),
        issuer: "https://token.actions.githubusercontent.com".to_owned(),
    }
}

/// Computes the canonical `sha256:<hex>` digest of `bytes`.
#[must_use]
pub fn sha256_digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    format!("sha256:{}", HEXLOWER.encode(&hash))
}

/// A trust root holding the trusted Ed25519 verifying keys for Sigstore
/// provenance attestations (bundled with each release or injected as a value
/// in tests, ADR 0059).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttestationTrustRoot {
    /// Trusted Ed25519 verifying keys (32-byte raw public keys), by `key_id`.
    pub keys: BTreeMap<String, [u8; 32]>,
}

impl AttestationTrustRoot {
    /// Constructs an attestation trust root from explicit verifying keys.
    #[must_use]
    pub fn new(keys: BTreeMap<String, [u8; 32]>) -> Self {
        Self { keys }
    }

    /// The attestation trust root bundled with this release. Before the first
    /// published release signs attestations with production keys, the bundled
    /// key map is empty so no forged attestation verifies against it.
    #[must_use]
    pub fn bundled() -> Self {
        Self::default()
    }
}

/// A deterministic Ed25519 signer producing [`ProvenanceAttestation`] values
/// under a test or CI [`AttestationTrustRoot`].
#[derive(Clone)]
pub struct AttestationSigner {
    key_id: String,
    signing_key: SigningKey,
}

impl AttestationSigner {
    /// Creates a deterministic signer from `seed` and returns `(signer,
    /// trust_root)` trusting only this signer's key.
    #[must_use]
    pub fn for_root(key_id: impl Into<String>, seed: [u8; 32]) -> (Self, AttestationTrustRoot) {
        let key_id = key_id.into();
        let signing_key = SigningKey::from_bytes(&seed);
        let public_bytes = signing_key.verifying_key().to_bytes();
        let mut keys = BTreeMap::new();
        keys.insert(key_id.clone(), public_bytes);
        (
            Self {
                key_id,
                signing_key,
            },
            AttestationTrustRoot::new(keys),
        )
    }

    /// Signs `statement`, returning a [`ProvenanceAttestation`] bound to its
    /// RFC 8785 canonical JSON.
    ///
    /// # Errors
    ///
    /// Fails if `statement` cannot be serialized to RFC 8785 canonical JSON.
    pub fn sign(
        &self,
        statement: ProvenanceStatement,
    ) -> Result<ProvenanceAttestation, serde_json::Error> {
        let message = signing_message(&statement)?;
        let signature = self.signing_key.sign(&message);
        Ok(ProvenanceAttestation {
            statement,
            key_id: self.key_id.clone(),
            signature: BASE64.encode(&signature.to_bytes()),
        })
    }
}

/// Outcome of verifying an artifact's provenance attestation(s).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttestationVerdict {
    /// At least one attestation verified under the trust root, matched the
    /// artifact's digest, and matched the expected signer.
    Verified(SignerIdentity),
    /// No attestation passed verification.
    Refused {
        /// Primary failure reason.
        reason: AttestationFailureReason,
        /// The expected signer identity, if any.
        expected_signer: Option<SignerIdentity>,
        /// The signer identity found in the attestation, if any was readable.
        found_signer: Option<SignerIdentity>,
    },
}

/// Verifies `attestations` against `expected_digest`, `expected_signer` and
/// `trust`.
///
/// If multiple attestations are supplied, any single valid matching attestation
/// succeeds; otherwise the first refusal is returned.
#[must_use]
pub fn verify_attestations(
    attestations: &[ProvenanceAttestation],
    expected_digest: &str,
    expected_signer: Option<&SignerIdentity>,
    trust: &AttestationTrustRoot,
) -> AttestationVerdict {
    if attestations.is_empty() {
        return AttestationVerdict::Refused {
            reason: AttestationFailureReason::Missing,
            expected_signer: expected_signer.cloned(),
            found_signer: None,
        };
    }
    let mut first_refusal: Option<AttestationVerdict> = None;
    for attestation in attestations {
        let verdict = verify_one(attestation, expected_digest, expected_signer, trust);
        if matches!(verdict, AttestationVerdict::Verified(_)) {
            return verdict;
        }
        if first_refusal.is_none() {
            first_refusal = Some(verdict);
        }
    }
    first_refusal.expect("invariant: attestations is non-empty")
}

fn verify_one(
    attestation: &ProvenanceAttestation,
    expected_digest: &str,
    expected_signer: Option<&SignerIdentity>,
    trust: &AttestationTrustRoot,
) -> AttestationVerdict {
    if !signature_valid(attestation, trust) {
        return AttestationVerdict::Refused {
            reason: AttestationFailureReason::BadSignature,
            expected_signer: expected_signer.cloned(),
            found_signer: Some(attestation.statement.signer.clone()),
        };
    }
    if attestation.statement.subject_digest != expected_digest {
        return AttestationVerdict::Refused {
            reason: AttestationFailureReason::DigestMismatch,
            expected_signer: expected_signer.cloned(),
            found_signer: Some(attestation.statement.signer.clone()),
        };
    }
    match expected_signer {
        Some(expected) if &attestation.statement.signer == expected => {
            AttestationVerdict::Verified(attestation.statement.signer.clone())
        }
        expected => AttestationVerdict::Refused {
            reason: AttestationFailureReason::SignerMismatch,
            expected_signer: expected.cloned(),
            found_signer: Some(attestation.statement.signer.clone()),
        },
    }
}

fn signature_valid(attestation: &ProvenanceAttestation, trust: &AttestationTrustRoot) -> bool {
    let Some(raw_key) = trust.keys.get(&attestation.key_id) else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(raw_key) else {
        return false;
    };
    let Ok(sig_bytes) = BASE64.decode(attestation.signature.as_bytes()) else {
        return false;
    };
    let Ok(sig_array): Result<[u8; 64], _> = sig_bytes.try_into() else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_array);
    let Ok(message) = signing_message(&attestation.statement) else {
        return false;
    };
    verifying_key.verify(&message, &signature).is_ok()
}

fn signing_message(statement: &ProvenanceStatement) -> Result<Vec<u8>, serde_json::Error> {
    let canonical = serde_json_canonicalizer::to_vec(statement)?;
    let mut message = Vec::with_capacity(ATTESTATION_DOMAIN_TAG.len() + canonical.len());
    message.extend_from_slice(ATTESTATION_DOMAIN_TAG);
    message.extend_from_slice(&canonical);
    Ok(message)
}
