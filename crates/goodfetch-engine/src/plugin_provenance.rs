//! Provenance attestation verification, signer identity and look-alike name
//! protection for `plugin add` (ADRs 0033 and 0059; issue 916).
//!
//! - **Look-alike names (`story:7-6`):** when `plugin add` receives a name or
//!   unpinned reference without an exact `@sha256:<hex>` digest, any name
//!   close to an entry in the Plugin index (typosquatting — separator/case
//!   confusables, one-character edits, or two-character edits on longer names)
//!   is refused with `plugin_name_ambiguous` (`ProgramError`). Giving an exact
//!   digest bypasses look-alike refusal; giving an exact bare index entry name
//!   resolves to that entry's pinned reference.
//! - **Provenance (`story:7-7`):** a third-party Plugin's OCI artifact must
//!   carry a provenance attestation (fetched through OCI 1.1 Referrers) whose
//!   Ed25519 signature verifies under the attestation [`AttestationTrustRoot`]
//!   (bundled with the release or injected as a value in tests, ADR 0059),
//!   whose `subject_digest` equals the artifact's image index digest, and
//!   whose [`SignerIdentity`] matches the signer recorded in the Plugin index.
//!   An unattested or mismatching third-party artifact is refused with
//!   `plugin_attestation_invalid` unless Operator policy explicitly allows its
//!   exact digest (`plugins.<name>.allow = true` and
//!   `plugins.<name>.digest = <digest>`).
//!
//! Dependency decision (STYLE.md dependency bar): as in [`crate::signed_set`],
//! `sigstore-verify` was checked and declined because it pulls in its own
//! TUF/HTTP/TLS stack outside ADR 0043's single outbound path; verification
//! uses the workspace's existing `ed25519-dalek`, `serde_json_canonicalizer`
//! and `data-encoding` crates, and OCI referrer pulls go through
//! `plugin_install` on the engine's own `http_client`.

use std::collections::BTreeMap;

use data_encoding::BASE64;
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use goodfetch_protocol::plugin::add::{
    AttestationFailureReason, PLUGIN_ATTESTATION_INVALID, PLUGIN_NAME_AMBIGUOUS,
    PluginAttestationInvalidDetails, PluginNameAmbiguousDetails,
};
use goodfetch_protocol::plugin::index::{
    IndexEntry, PluginIndex, ProvenanceAttestation, ProvenanceStatement, SignerIdentity,
};
use goodfetch_protocol::plugin::settings::{PLUGINS, PluginEntry};
use serde_json::Value;

use crate::call_error::CallError;
use crate::plugin_oci::{MalformedReference, REPOSITORY_PREFIX, Reference};
use crate::settings::Resolved;

/// Domain separation prefix included in every provenance attestation signing
/// payload.
const ATTESTATION_DOMAIN_TAG: &[u8] = b"goodfetch-plugin-provenance-v1\n";

/// Minimum normalized length for a 2-edit look-alike match (1-edit matches
/// apply to any name of length >= 2).
const TWO_EDIT_MIN_LEN: usize = 6;

/// A trust root holding the trusted Ed25519 verifying keys for Plugin
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

/// Outcome of verifying a Plugin artifact's provenance attestation(s).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttestationVerdict {
    /// At least one attestation verified under the trust root, matched the
    /// artifact's image index digest, and matched the index's expected signer.
    Verified(SignerIdentity),
    /// No attestation passed verification.
    Refused {
        /// Primary failure reason.
        reason: AttestationFailureReason,
        /// The signer identity expected by the Plugin index, if listed.
        expected_signer: Option<SignerIdentity>,
        /// The signer identity found in the attestation, if any was readable.
        found_signer: Option<SignerIdentity>,
    },
}

/// Verifies `attestations` against `expected_digest`, the Plugin index's
/// `expected_signer` and `trust`.
///
/// If multiple attestations are attached to an artifact (for example alongside
/// another referrer), any single valid matching attestation succeeds;
/// otherwise the most informative refusal is returned.
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

/// Verifies a single [`ProvenanceAttestation`].
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

/// Whether Operator policy in `resolved` explicitly allows the exact
/// `(name, digest)` pair (`plugins.<name>.allow = true` and
/// `plugins.<name>.digest = digest`).
///
/// Only Operator policy can set `allow` and `digest` (enforced by
/// `settings`, issue 900).
#[must_use]
pub fn policy_allows_exact_digest(resolved: &Resolved, name: &str, digest: &str) -> bool {
    match &resolved.value(&PLUGINS)[name] {
        Value::Null => false,
        entry => PluginEntry::read(entry).is_ok_and(|entry| {
            entry.allow == Some(true) && entry.digest.as_deref() == Some(digest)
        }),
    }
}

/// How a caller-provided `plugin add` argument resolved to a digest-pinned
/// [`Reference`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedReference {
    /// The caller provided an explicit `@sha256:<hex>` digest reference.
    Exact(Reference),
    /// The caller provided a bare Plugin name that uniquely matched an entry
    /// in the Plugin index, resolved to that entry's pinned reference.
    FromIndex {
        /// The index entry's digest-pinned OCI reference.
        reference: Reference,
        /// The matched index entry.
        entry: Box<IndexEntry>,
    },
}

impl ResolvedReference {
    /// The resolved digest-pinned OCI [`Reference`].
    #[must_use]
    pub fn reference(&self) -> &Reference {
        match self {
            Self::Exact(reference) | Self::FromIndex { reference, .. } => reference,
        }
    }
}

/// Why a `plugin add` argument could not be resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolveError {
    /// The name was given without an exact digest and is ambiguous or a
    /// look-alike of one or more Plugin index entries: becomes
    /// `plugin_name_ambiguous` (`ProgramError`).
    Ambiguous {
        /// The requested Plugin name.
        name: String,
        /// Close or matching index entry names, sorted.
        similar: Vec<String>,
    },
    /// The reference is not a valid digest-pinned OCI reference (and did not
    /// resolve to an index entry).
    Malformed(MalformedReference),
}

/// Resolves a `plugin add` reference string against `index` (ADR 0033,
/// issue 916):
///
/// - If `raw` contains `@`, it is parsed as an exact digest-pinned
///   [`Reference`] without look-alike refusal.
/// - Otherwise (no exact digest), the candidate Plugin name is checked against
///   `index`: an exact unique bare name resolves to the index entry's pinned
///   reference; multiple exact matches or a name close to an index entry
///   (typosquatting) is refused with [`ResolveError::Ambiguous`].
///
/// # Errors
///
/// [`ResolveError::Ambiguous`] for a look-alike or ambiguous name without an
/// exact digest, or [`ResolveError::Malformed`] when the string is not a valid
/// digest-pinned OCI reference.
pub fn resolve_reference(
    raw: &str,
    index: Option<&PluginIndex>,
) -> Result<ResolvedReference, ResolveError> {
    let trimmed = raw.trim();
    if trimmed.contains('@') {
        return trimmed
            .parse::<Reference>()
            .map(ResolvedReference::Exact)
            .map_err(ResolveError::Malformed);
    }

    let is_bare_name = !trimmed.contains('/') && !trimmed.contains(':');
    let candidate = extract_candidate_name(trimmed);

    if let Some(index) = index
        && !candidate.is_empty()
    {
        let exact: Vec<&IndexEntry> = index
            .plugins
            .iter()
            .filter(|entry| entry.name == candidate)
            .collect();
        if exact.len() > 1 {
            let mut similar: Vec<String> = exact.iter().map(|entry| entry.name.clone()).collect();
            similar.sort();
            similar.dedup();
            return Err(ResolveError::Ambiguous {
                name: candidate.to_owned(),
                similar,
            });
        }
        if let [entry] = exact.as_slice()
            && is_bare_name
        {
            let pinned = format!("{}@{}", entry.reference, entry.digest);
            let reference = pinned
                .parse::<Reference>()
                .map_err(ResolveError::Malformed)?;
            return Ok(ResolvedReference::FromIndex {
                reference,
                entry: Box::new((*entry).clone()),
            });
        }
        if exact.is_empty() {
            let similar = find_similar_names(candidate, index);
            if !similar.is_empty() {
                return Err(ResolveError::Ambiguous {
                    name: candidate.to_owned(),
                    similar,
                });
            }
        }
    }

    trimmed
        .parse::<Reference>()
        .map(ResolvedReference::Exact)
        .map_err(ResolveError::Malformed)
}

/// Extracts the Plugin name portion from an unpinned input (bare name,
/// `goodfetch-plugin-<name>`, or `<registry>/<path>/<name>[:tag]`).
fn extract_candidate_name(raw: &str) -> &str {
    let last_segment = raw.rsplit('/').next().unwrap_or(raw);
    let without_tag = last_segment
        .split_once(':')
        .map_or(last_segment, |(name, _)| name);
    without_tag
        .strip_prefix(REPOSITORY_PREFIX)
        .unwrap_or(without_tag)
}

/// Finds index entry names that closely resemble `name` (typosquatting /
/// look-alike check, ADR 0033, issue 916), excluding exact matches.
///
/// Matches:
/// - case or separator (`-`, `_`, `.`) confusables (e.g. `foo_bar` vs
///   `foo-bar`, `foo-bar` vs `foobar`),
/// - optimal string alignment (Damerau-Levenshtein) distance `<= 1` for names
///   of length >= 2 (single insertion, deletion, substitution or adjacent
///   transposition),
/// - distance `<= 2` when both normalized names have length >= 6.
#[must_use]
pub fn find_similar_names(name: &str, index: &PluginIndex) -> Vec<String> {
    let norm = normalize_name(name);
    if norm.is_empty() {
        return Vec::new();
    }
    let compact = strip_separators(&norm);
    let mut similar = Vec::new();
    for entry in &index.plugins {
        if entry.name == name {
            continue;
        }
        let entry_norm = normalize_name(&entry.name);
        if entry_norm.is_empty() {
            continue;
        }
        let entry_compact = strip_separators(&entry_norm);
        let is_close = if norm == entry_norm || (!compact.is_empty() && compact == entry_compact) {
            true
        } else {
            let distance = osa_distance(&norm, &entry_norm);
            let min_len = norm.chars().count().min(entry_norm.chars().count());
            (min_len >= 2 && distance <= 1) || (min_len >= TWO_EDIT_MIN_LEN && distance <= 2)
        };
        if is_close {
            similar.push(entry.name.clone());
        }
    }
    similar.sort();
    similar.dedup();
    similar
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            '_' | '.' => '-',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

fn strip_separators(name: &str) -> String {
    name.chars().filter(|&ch| ch != '-').collect()
}

/// Optimal string alignment (restricted Damerau-Levenshtein) edit distance
/// between `left` and `right` in Unicode scalar values.
fn osa_distance(left: &str, right: &str) -> usize {
    let a: Vec<char> = left.chars().collect();
    let b: Vec<char> = right.chars().collect();
    let m = a.len();
    let n = b.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev2 = vec![0usize; n + 1];
    let mut prev1: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (prev1[j] + 1).min(curr[j - 1] + 1).min(prev1[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(prev2[j - 2] + 1);
            }
            curr[j] = best;
        }
        std::mem::swap(&mut prev2, &mut prev1);
        std::mem::swap(&mut prev1, &mut curr);
    }
    prev1[n]
}

/// Constructs a `plugin_attestation_invalid` [`CallError`].
#[must_use]
pub fn attestation_invalid_error(
    name: &str,
    digest: &str,
    reason: AttestationFailureReason,
    expected_signer: Option<SignerIdentity>,
    found_signer: Option<SignerIdentity>,
) -> CallError {
    let message = match reason {
        AttestationFailureReason::Missing => format!(
            "The Plugin `{name}` at {digest} carries no provenance attestation: nothing was installed."
        ),
        AttestationFailureReason::BadSignature => format!(
            "The provenance attestation for Plugin `{name}` at {digest} failed signature verification: nothing was installed."
        ),
        AttestationFailureReason::SignerMismatch => match (&expected_signer, &found_signer) {
            (Some(expected), Some(found)) => format!(
                "The provenance attestation for Plugin `{name}` at {digest} was signed by {found}, not the index-recorded signer {expected}: nothing was installed."
            ),
            (None, Some(found)) => format!(
                "The Plugin `{name}` at {digest} (signed by {found}) has no signer recorded in the Plugin index: nothing was installed."
            ),
            _ => format!(
                "The provenance attestation for Plugin `{name}` at {digest} did not match the signer recorded in the Plugin index: nothing was installed."
            ),
        },
        AttestationFailureReason::DigestMismatch => format!(
            "The provenance attestation for Plugin `{name}` does not cover its image index digest {digest}: nothing was installed."
        ),
        AttestationFailureReason::Malformed => format!(
            "The provenance attestation for Plugin `{name}` at {digest} could not be parsed: nothing was installed."
        ),
    };
    let details = PluginAttestationInvalidDetails {
        name: name.to_owned(),
        digest: digest.to_owned(),
        reason,
        expected_signer,
        found_signer,
        fix: format!(
            "Install a Plugin release attested by its index-recorded signer, or allow its exact digest in Operator policy (`plugins.{name}.allow = true` and `plugins.{name}.digest = \"{digest}\"`)"
        ),
    };
    CallError::new(&PLUGIN_ATTESTATION_INVALID, message, &details)
}

/// Constructs a `plugin_name_ambiguous` [`CallError`] (`ProgramError`).
#[must_use]
pub fn name_ambiguous_error(name: &str, similar: Vec<String>) -> CallError {
    let listed = similar.join(", ");
    let details = PluginNameAmbiguousDetails {
        name: name.to_owned(),
        similar,
        fix: format!(
            "Use the exact Plugin index name ({listed}), or pass the full OCI reference with an exact `@sha256:<hex>` digest"
        ),
    };
    CallError::new(
        &PLUGIN_NAME_AMBIGUOUS,
        format!(
            "The Plugin name `{name}` is ambiguous or too close to listed Plugin(s) ({listed}) without an exact digest: nothing was installed."
        ),
        &details,
    )
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::plugin::PluginKindName;
    use goodfetch_protocol::plugin::approvals::Origin;

    use super::*;

    const DIGEST_A: &str =
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DIGEST_B: &str =
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn signer(repo: &str, workflow: &str) -> SignerIdentity {
        SignerIdentity {
            issuer: "https://token.actions.githubusercontent.com".to_owned(),
            workflow: workflow.to_owned(),
            repository: repo.to_owned(),
        }
    }

    fn index_with(names: &[&str]) -> PluginIndex {
        PluginIndex {
            version: 1,
            signed_at: "2026-11-27T00:00:00Z".to_owned(),
            expires_at: "2026-12-11T00:00:00Z".to_owned(),
            plugins: names
                .iter()
                .map(|name| IndexEntry {
                    name: (*name).to_owned(),
                    reference: format!("ghcr.io/goodfetch/goodfetch-plugin-{name}"),
                    digest: DIGEST_A.to_owned(),
                    kinds: vec![PluginKindName::EXTRACT],
                    signer: signer(
                        &format!("github.com/vendor/{name}"),
                        &format!(
                            "https://github.com/vendor/{name}/.github/workflows/release.yml@refs/tags/v1.0.0"
                        ),
                    ),
                    origin: Origin::ThirdParty,
                    conformance: Vec::new(),
                })
                .collect(),
            revocations: Vec::new(),
        }
    }

    #[test]
    fn find_similar_names_catches_one_letter_typos_and_confusables() {
        let index = index_with(&["kettle", "web-vault"]);
        assert_eq!(find_similar_names("ketle", &index), vec!["kettle"]);
        assert_eq!(find_similar_names("kettlee", &index), vec!["kettle"]);
        assert_eq!(find_similar_names("kettla", &index), vec!["kettle"]);
        assert_eq!(find_similar_names("kettel", &index), vec!["kettle"]);
        assert_eq!(find_similar_names("web_vault", &index), vec!["web-vault"]);
        assert_eq!(find_similar_names("webvault", &index), vec!["web-vault"]);
        assert_eq!(find_similar_names("kettle", &index), Vec::<String>::new());
        assert_eq!(
            find_similar_names("totally-different", &index),
            Vec::<String>::new()
        );
    }

    #[test]
    fn resolve_reference_refuses_look_alike_without_digest_and_accepts_with_digest() {
        let index = index_with(&["kettle"]);

        // Bare one-letter look-alike refused.
        let err = resolve_reference("ketle", Some(&index)).unwrap_err();
        assert_eq!(
            err,
            ResolveError::Ambiguous {
                name: "ketle".to_owned(),
                similar: vec!["kettle".to_owned()],
            }
        );

        // Unpinned registry path with look-alike name also refused.
        let err =
            resolve_reference("ghcr.io/evil/goodfetch-plugin-ketle", Some(&index)).unwrap_err();
        assert_eq!(
            err,
            ResolveError::Ambiguous {
                name: "ketle".to_owned(),
                similar: vec!["kettle".to_owned()],
            }
        );

        // Exact digest bypasses look-alike check.
        let exact = format!("ghcr.io/evil/goodfetch-plugin-ketle@{DIGEST_B}");
        let resolved = resolve_reference(&exact, Some(&index)).unwrap();
        assert!(matches!(resolved, ResolvedReference::Exact(_)));
        assert_eq!(resolved.reference().plugin(), "ketle");

        // Exact bare name resolves to the index entry's pinned reference.
        let resolved = resolve_reference("kettle", Some(&index)).unwrap();
        assert!(matches!(resolved, ResolvedReference::FromIndex { .. }));
        assert_eq!(resolved.reference().index().to_string(), DIGEST_A);
    }

    #[test]
    fn verify_attestations_enforces_signature_digest_and_signer() {
        let (att_signer, trust) = AttestationSigner::for_root("att-key-1", [0x42; 32]);
        let expected = signer(
            "github.com/vendor/kettle",
            "https://github.com/vendor/kettle/.github/workflows/release.yml@refs/tags/v1.0.0",
        );
        let other = signer(
            "github.com/attacker/kettle",
            "https://github.com/attacker/kettle/.github/workflows/release.yml@refs/tags/v1.0.0",
        );

        // Matching attestation verifies.
        let good = att_signer
            .sign(ProvenanceStatement {
                subject_digest: DIGEST_A.to_owned(),
                signer: expected.clone(),
            })
            .unwrap();
        assert_eq!(
            verify_attestations(
                std::slice::from_ref(&good),
                DIGEST_A,
                Some(&expected),
                &trust
            ),
            AttestationVerdict::Verified(expected.clone())
        );

        // Missing attestation is refused.
        assert_eq!(
            verify_attestations(&[], DIGEST_A, Some(&expected), &trust),
            AttestationVerdict::Refused {
                reason: AttestationFailureReason::Missing,
                expected_signer: Some(expected.clone()),
                found_signer: None,
            }
        );

        // Wrong signer is refused.
        let wrong_signer = att_signer
            .sign(ProvenanceStatement {
                subject_digest: DIGEST_A.to_owned(),
                signer: other.clone(),
            })
            .unwrap();
        assert_eq!(
            verify_attestations(&[wrong_signer], DIGEST_A, Some(&expected), &trust),
            AttestationVerdict::Refused {
                reason: AttestationFailureReason::SignerMismatch,
                expected_signer: Some(expected.clone()),
                found_signer: Some(other),
            }
        );

        // Wrong digest is refused.
        assert_eq!(
            verify_attestations(
                std::slice::from_ref(&good),
                DIGEST_B,
                Some(&expected),
                &trust
            ),
            AttestationVerdict::Refused {
                reason: AttestationFailureReason::DigestMismatch,
                expected_signer: Some(expected.clone()),
                found_signer: Some(expected.clone()),
            }
        );

        // Tampered signature is refused.
        let mut tampered = good;
        tampered.signature = BASE64.encode(&[0u8; 64]);
        assert_eq!(
            verify_attestations(&[tampered], DIGEST_A, Some(&expected), &trust),
            AttestationVerdict::Refused {
                reason: AttestationFailureReason::BadSignature,
                expected_signer: Some(expected.clone()),
                found_signer: Some(expected),
            }
        );
    }
}
