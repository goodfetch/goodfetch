//! AI work that can claim success (ADRs 0079, 0100 and 0117): what its
//! model claimed, kept apart from how it ended and its Verdict, and
//! `not_verified`, which an SDK raises itself when a program asks for an
//! output its Verdict does not prove.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ARRIVED_IN;
use crate::catalog::{ErrorCodeDefinition, ErrorGroup};
use crate::open_value::UnknownValue;
use crate::run::{AxisReason, Verdict};

/// What a model said of the work it was given (ADR 0100): a signal, never
/// proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum Claim {
    /// It said it was done.
    Done,
    /// It said it gave up, which stops the work with the reason `gave_up`
    /// (ADR 0117).
    GaveUp,
    /// A claim a newer engine reports that this reader does not know, kept
    /// as received (ADR 0079).
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

/// The model's claim and its message, absent from a result when no model
/// claimed anything: the work stopped before a claim, or was replayed whole
/// (ADR 0100).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Reported {
    /// What it claimed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub claim: Claim,
    /// What it said with the claim, when it said something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: Option<String>,
}

/// What `not_verified` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NotVerifiedDetails {
    /// The work's Verdict, which is not `verified`; one this reader does not
    /// know is kept as received.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub verdict: Verdict,
    /// Why it is not `verified`: failed, missing or switched-off checks, an
    /// early end, uncertain Effects.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reasons: Vec<AxisReason>,
    /// The model's claim, when it made one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reported: Option<Reported>,
}

/// A program asked AI work for its output with `requireVerified()`
/// (`require_verified()`), and its Verdict is not `verified` (ADR 0079).
pub const NOT_VERIFIED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "not_verified",
    description: "The program asked AI work for its output only if verified, and its Verdict is not `verified`, so no output is returned. The details hold the Verdict, the reasons it is not `verified` and the model's claim, which is never proof; the result itself still holds how the work ended.",
    group: ErrorGroup::Page,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};
