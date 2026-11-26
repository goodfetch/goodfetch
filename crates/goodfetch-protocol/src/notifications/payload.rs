//! The body of a `webhook` channel's delivery (ADRs 0027 and 0030; issue
//! 1391): a Standard Webhooks envelope, `{ type, timestamp, format, data }`,
//! a format of its own whose major never changes and whose newer minors
//! only add, so a receiver reads every minor of its major: it ignores a
//! field it does not know and keeps an event it does not know. The engine's `webhook` channel writes it and each SDK's
//! verification helper reads it, raising [`WEBHOOK_INVALID`] when a call
//! is not one.

use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::NotificationEvent;
use crate::catalog::{ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};
use crate::point_in_time::PointInTime;
use crate::run::RunId;
use crate::schedule::ScheduleId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The version of the delivery's body, in its `format` field.
pub const NOTIFICATION_PAYLOAD_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A `webhook` channel's delivery: the Standard Webhooks envelope around
/// one Notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationPayload {
    /// `notification.` and the Notification's event, such as
    /// `notification.run_failed`.
    #[serde(rename = "type")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// When the Notification's event happened, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub timestamp: SystemTime,
    /// The body's `major.minor` version: a receiver reads every minor of
    /// its major.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format: ProtocolVersion,
    /// The Notification, whole or thin as the channel's `webhook.payload`
    /// says.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub data: NotificationData,
}

/// What a delivery's `data` holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum NotificationData {
    /// The whole Notification, a channel's `webhook.payload` of `full`.
    Full(FullNotification),
    /// Its IDs and link only, a channel's `webhook.payload` of `thin`.
    Thin(ThinNotification),
}

/// The whole Notification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FullNotification {
    /// The event's ID, the same on every delivery of it and the
    /// `webhook-id` header's value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event_id: String,
    /// What happened.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event: NotificationEvent,
    /// The Run it concerns, if any.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// The Schedule it concerns, if any.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule_id: Option<ScheduleId>,
    /// The Product whose engine sent it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its one-line title.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub title: String,
    /// Its text.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// Where a person acts on it, such as the Run's page, if anywhere.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub link: Option<String>,
}

/// A thin Notification: its IDs and link only, which an Integration reads
/// the rest from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ThinNotification {
    /// The event's ID, the same on every delivery of it and the
    /// `webhook-id` header's value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event_id: String,
    /// The Run it concerns, if any.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// The Schedule it concerns, if any.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schedule_id: Option<ScheduleId>,
    /// Where a person acts on it, such as the Run's page, if anywhere.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub link: Option<String>,
}

/// A call that is no verified delivery of a `webhook` channel: raised by an
/// SDK's verification helper, so no engine sends it.
pub const WEBHOOK_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "webhook_invalid",
    description: "The call is no verified delivery of a `webhook` channel: a Standard Webhooks header is missing, its timestamp is more than 5 minutes from now, no signature matches a secret given, or its body is no Notification payload of a major this SDK reads. Answer the call with an error and keep nothing of it; when every delivery fails, check that the secret given is the channel's `webhook.secret`.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Why a call is no verified delivery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebhookInvalidReason {
    /// No signature of `webhook-signature` matches a secret given.
    Signature,
    /// `webhook-timestamp` is no whole number of seconds, or more than 5
    /// minutes from now: a delayed or replayed call.
    Timestamp,
    /// The body is no Notification payload of a major the SDK reads.
    Malformed,
    /// `webhook-id`, `webhook-timestamp` or `webhook-signature` is
    /// missing.
    MissingHeader,
}

/// What `webhook_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WebhookInvalidDetails {
    /// Why the call was refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: WebhookInvalidReason,
}
