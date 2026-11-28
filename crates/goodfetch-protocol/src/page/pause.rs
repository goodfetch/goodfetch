//! The Human pause (ADRs 0092 and 0113): `page.pause` waits for a person in
//! the headful window or the Live view, stopping every enclosing clock while
//! Limits keep counting, holding the page's later Gestures while reads and
//! waits run, ending when `until` holds or a person answers via
//! `page.pause_answer`, and returning [`HumanPauseResult`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::route::UrlMatcher;
use super::{ARRIVED_IN, PageId};
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind, Reach, SharedType,
    SharedTypeValue, Stability, schema_of,
};

/// The condition that ends a Human pause by itself (ADR 0092): a locator
/// becoming visible on the page, or the page's URL matching a `UrlMatcher`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PauseUntil {
    /// A locator on the page becomes visible.
    Visible {
        /// The locator's selector.
        #[schemars(extend("arrived_in" = "0.1"))]
        selector: String,
    },
    /// The page's URL matches `url`.
    Url {
        /// The URL matcher, as `page.wait_for_url` takes it.
        #[schemars(extend("arrived_in" = "0.1"))]
        url: UrlMatcher,
    },
}

/// What `page.pause` takes: the page, why it waits, the optional `until`
/// condition, and its optional `timeout`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PagePauseParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// Why the program paused, shown to the person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: Option<String>,
    /// The condition that ends the pause by itself; a person's "Done" or
    /// "Hand back" is accepted only while it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub until: Option<PauseUntil>,
    /// How long the pause may wait, in milliseconds, at least 1: absent for
    /// `timeout.human_pause`, `null` for no limit (ADR 0080).
    #[serde(
        default,
        deserialize_with = "crate::settings::present",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(extend("arrived_in" = "0.1", "setting" = "timeout.human_pause"))]
    pub timeout: Option<Option<u64>>,
}

/// What ended a Human pause that succeeded (ADR 0092).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PauseResolvedBy {
    /// Its `until` condition held.
    Condition,
    /// A person answered "Done" or "Hand back".
    Person,
}

/// What `page.pause` answers once its condition holds or a person answers
/// "Done" or "Hand back" (ADR 0092).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HumanPauseResult {
    /// Whether the `until` condition or a person's answer ended the pause.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resolved_by: PauseResolvedBy,
    /// Who answered, when a person ended it; absent when `until` ended it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub person: Option<String>,
    /// How long the pause waited, in milliseconds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub waited: u64,
}

/// A person's answer to a waiting Human pause (ADR 0092).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PauseAnswer {
    /// The person finished the step: accepted only while `until` holds (or
    /// at once when the pause has no `until`).
    Done,
    /// The person handed control back: ends a program's pause as `done`
    /// does.
    HandBack,
    /// The person declined the step: ends the pause with
    /// `human_pause_declined` carrying their note.
    Decline,
}

/// What `page.pause_answer` takes: the paused page, the person's answer, and
/// an optional note for `decline`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PagePauseAnswerParams {
    /// The paused page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The person's answer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub answer: PauseAnswer,
    /// The person's note, carried by `human_pause_declined` when declining.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub note: Option<String>,
}

/// What `page.pause_answer` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PagePauseAnswered {
    /// Whether the waiting pause accepted the answer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub accepted: bool,
    /// Why the answer was not accepted, such as when `until` does not hold
    /// yet or no pause waits on the page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: Option<String>,
}

/// A person declined the step a Human pause asked for (ADR 0092).
pub const HUMAN_PAUSE_DECLINED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "human_pause_declined",
    description: "A person declined the step `page.pause()` asked for, rather than finishing it. The details name the person and their note, when they gave one; the page stays open.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `human_pause_declined` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HumanPauseDeclinedDetails {
    /// Who declined the pause, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub person: Option<String>,
    /// The note the person gave when declining.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub note: Option<String>,
}

/// `page.pause`.
pub const PAGE_PAUSE: Operation = Operation {
    name: "page.pause",
    description: "Waits for a person in the headful window or the Live view, stopping every enclosing clock while Limits keep counting, holding the page's later Gestures while reads and waits run, and ending when `until` holds or a person answers `page.pause_answer` (ADRs 0092 and 0113). Closing the page's window or quitting the browser ends the pause with `call_cancelled` naming what closed, never `human_pause_declined`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<PagePauseParams>,
    result: schema_of::<HumanPauseResult>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `page.pause_answer`.
pub const PAGE_PAUSE_ANSWER: Operation = Operation {
    name: "page.pause_answer",
    description: "Answers a Human pause waiting on a page as a person: `done` or `hand_back` (accepted only while `until` holds, or at once when there is no `until`) ends the pause and returns `HumanPauseResult`; `decline` ends it with `human_pause_declined` carrying the note. Offered only on a person's Client.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<PagePauseAnswerParams>,
    result: schema_of::<PagePauseAnswered>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// The "Needs you" kind for a Client acting for a model or a Recipe run
/// waiting for a person's approval (ADR 0056; ticket #745).
pub const APPROVAL_REQUESTED_KIND: &str = "approval_requested";

/// Shared-type registration for `approval_requested` in `needs_you_kind`.
pub const APPROVAL_REQUESTED: SharedTypeValue = SharedTypeValue {
    shared_type: SharedType::NeedsYouKind,
    name: APPROVAL_REQUESTED_KIND,
    description: "A Client acting for a model or a Recipe run waits for a person's approval of an unlisted domain navigation or a login import, borrow, save, deletion, or export (ADR 0056).",
    arrived_in: ARRIVED_IN,
};

/// What `needs_you.answer_approval` takes: the approval item's ID, whether the
/// person approves, and an optional note when declining (ADR 0056; #745).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NeedsYouAnswerApprovalParams {
    /// The pending approval item's ID from `needs_you.list`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// `true` to approve and continue; `false` to decline with `human_pause_declined`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub approve: bool,
    /// Optional note carried by `human_pause_declined` when declining.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub note: Option<String>,
}

/// What `needs_you.answer_approval` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalAnswered {
    /// Whether the pending approval accepted the answer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub accepted: bool,
    /// Why the answer was not accepted, when `accepted` is false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: Option<String>,
}

/// `needs_you.answer_approval` (ADR 0056; ticket #745).
pub const NEEDS_YOU_ANSWER_APPROVAL: Operation = Operation {
    name: "needs_you.answer_approval",
    description: "Approves or declines a pending Human pause waiting for a person's approval (such as a model's or Recipe's unlisted domain navigation or login import, borrow, save, deletion, or export, ADR 0056). Offered only on a person's Client.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::PersonOnly,
    params: schema_of::<NeedsYouAnswerApprovalParams>,
    result: schema_of::<ApprovalAnswered>,
    events: &[&crate::needs_you::NEEDS_YOU_CHANGED],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};
