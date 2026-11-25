//! The shared Standard Webhooks vectors (ADR 0017; #666), committed once as
//! `test-support/fixtures/webhook-signatures.json`, which the TypeScript
//! and Python packages read too, so the engine's `webhook_signing` and the
//! SDKs' verification helpers sign and verify alike.
//!
//! `signed` holds what a signer with the given keys must send: the
//! specification's published `v1` and `v2a` vectors, each with its source,
//! and Goodfetch's own rotations. `verifications` holds what the reference
//! verifier must answer for a delivery at a given time.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The committed vectors, as their file holds them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WebhookSignatures {
    /// What a signer sends.
    pub signed: Vec<SignedVector>,
    /// What the verifier answers.
    pub verifications: Vec<VerificationCase>,
}

/// A message, the keys that sign it and the `webhook-signature` they give.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedVector {
    /// The case's name.
    pub name: String,
    /// Where the vector is published, for one the specification publishes.
    pub source: Option<String>,
    /// The signing keys, current first: `whsec_` or `whsk_`.
    pub keys: Vec<String>,
    /// The endpoint ID a `v2a` key signs for.
    pub endpoint: Option<String>,
    /// `webhook-id`.
    pub id: String,
    /// `webhook-timestamp`, in Unix seconds.
    pub timestamp: i64,
    /// The body, as sent.
    pub body: String,
    /// `webhook-signature`: each key's signature, in order.
    pub signature: String,
}

/// A delivery as received, the keys trusted and the verifier's answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerificationCase {
    /// The case's name.
    pub name: String,
    /// The trusted keys: `whsec_` or `whpk_`.
    pub trusted: Vec<String>,
    /// The endpoint ID a `v2a` signature is made for.
    pub endpoint: Option<String>,
    /// `webhook-id`, absent when missing.
    pub id: Option<String>,
    /// `webhook-timestamp` as received, absent when missing.
    pub timestamp: Option<String>,
    /// `webhook-signature`, absent when missing.
    pub signature: Option<String>,
    /// The body, as received.
    pub body: String,
    /// The verifier's time, in Unix seconds.
    pub now: i64,
    /// The verifier's answer.
    pub outcome: VerificationOutcome,
    /// The run epoch a verified Goodfetch delivery's body carries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_epoch: Option<u64>,
}

/// What the verifier answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerificationOutcome {
    /// A trusted key's signature matches.
    Verified,
    /// A `webhook-` header is missing.
    MissingHeader,
    /// `webhook-timestamp` is no Unix time in seconds.
    TimestampInvalid,
    /// The timestamp is more than five minutes in the past.
    TimestampTooOld,
    /// The timestamp is more than five minutes in the future.
    TimestampTooNew,
    /// No signature is a trusted key's.
    NoMatchingSignature,
}

/// The committed vectors' text.
const COMMITTED: &str = include_str!("../../../test-support/fixtures/webhook-signatures.json");

/// The committed vectors.
///
/// # Panics
///
/// When the committed file does not parse, which the Quick suite checks.
pub fn webhook_signatures() -> WebhookSignatures {
    serde_json::from_str(COMMITTED)
        .expect("invariant: the committed webhook vectors parse, which the Quick suite checks")
}

#[cfg(test)]
mod tests;
