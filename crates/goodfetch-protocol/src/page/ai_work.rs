//! How AI work ends on the wire: what its model claimed, kept apart from
//! how it ended and its Verdict (ADRs 0079, 0100 and 0117), and the Error
//! codes it alone fails with: `not_verified`, which an SDK raises itself
//! when a program asks for an output its Verdict does not prove, and
//! `page_navigated_during_call`, when a navigation the AI verb did not
//! start replaces the document it read (ADR 0114).

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

/// An AI verb whose document a navigation it did not start replaced
/// (ADR 0114).
pub const PAGE_NAVIGATED_DURING_CALL: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "page_navigated_during_call",
    description: "The page's main frame committed a navigation to a new document while an `act`, `observe` or `extract` ran, between the call's first read of the page and its return (an `act` until its first input), and the call did not start it: the program's own driver, the page's script, a person or an extension may have. Nothing was acted on, and no Plan entry was written. The details name the document the call read, `url`, and the one that replaced it, `navigated_to`; the fix says to await the Collie call before the program's own driver acts on the page. A retry may find the page where the call needs it.",
    group: ErrorGroup::Page,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `page_navigated_during_call` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageNavigatedDuringCallDetails {
    /// The document the call read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// The document that replaced it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub navigated_to: String,
}
