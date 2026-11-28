//! The Notification channel, a Plugin kind (ADRs 0027, 0033 and 0129; issue
//! 1023): an implementation answers `notification.send` with `{ delivered }`
//! for a channel named by `notifications.channels.<name>` that Goodfetch does
//! not ship, and the Error codes of a channel that answered what the engine
//! cannot take or that no one answers.
//!
//! `notification.send` is Effect-class: an abandoned call is reported
//! uncertain at once and never repeated blindly. Every delivery runs through
//! ADR 0027's delivery ledger so a resumed Run never repeats an event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, PluginKind, PluginMethod, ProtocolVersion, Stability,
    schema_of,
};
use crate::notifications::{NOTIFICATIONS_CHANNELS, NotificationEvent};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The Notification channel's method, its wire name with its kind's.
pub const NOTIFICATION_SEND: &str = "notification.send";

/// What `notification.send` hands a Notification channel Plugin or SDK
/// handler: the event's idempotency key, event name, English text and
/// optional link. Fields a newer engine adds are ignored by the receiver
/// (ADRs 0077 and 0082).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationSendParams {
    /// The event ID, the channel's idempotency key.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event_id: String,
    /// The Notification event's `snake_case` name, such as `run_failed`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event: NotificationEvent,
    /// The Notification's text, in English.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// Where the Run or Schedule is shown, if anywhere.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub link: Option<String>,
}

/// What a Notification channel Plugin or SDK handler answers to
/// `notification.send`: whether the Notification was delivered, and no other
/// field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NotificationSendResult {
    /// Whether the channel delivered the Notification.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub delivered: bool,
}

/// What `notification_handler_failed` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationHandlerFailedDetails {
    /// The channel's name in `notifications.channels.<name>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub channel: String,
    /// The event ID of the Notification that was being sent.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event_id: String,
}

/// What `notification_channel_unavailable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotificationChannelUnavailableDetails {
    /// The channel's name in `notifications.channels.<name>`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub channel: String,
}

/// A Notification channel whose answer the engine cannot take (ADRs 0027,
/// 0104 and 0127).
pub const NOTIFICATION_HANDLER_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "notification_handler_failed",
    description: "The Notification channel `notifications.channels.<name>` names answered `notification.send` with what the engine cannot take: an answer it cannot read, a result whose shape is not `{ delivered }`, `delivered: false`, or an Error code the catalog does not know. Its text is logged, never returned. The details name the channel and the event ID. Fix the channel Plugin or SDK handler, or choose another channel.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Notification channel that no one answers for (ADRs 0027 and 0104).
pub const NOTIFICATION_CHANNEL_UNAVAILABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "notification_channel_unavailable",
    description: "No one answers for the Notification channel `notifications.channels.<name>` names: no built-in has its type and no installed Plugin of the Notification channel kind does, its Plugin cannot start, or it is a handler the program registered that left its call unanswered. The details name the channel. Install the Plugin or name another channel, or retry once the program reads its Notification channel's calls again.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The Notification channel kind.
pub const NOTIFICATION_CHANNEL: PluginKind = PluginKind {
    name: "notification",
    description: "Delivers a Notification to a channel Goodfetch does not ship, through ADR 0027's delivery ledger.",
    methods: &[PluginMethod {
        name: "send",
        description: "Delivers one Notification event and answers whether it was delivered.",
        params: schema_of::<NotificationSendParams>,
        result: schema_of::<NotificationSendResult>,
    }],
    chosen_by: Some(&NOTIFICATIONS_CHANNELS),
    stability: Stability::Experimental,
    arrived_in: ARRIVED_IN,
};

/// The Notification channel kind and its Error codes.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        NOTIFICATION_HANDLER_FAILED,
        NOTIFICATION_CHANNEL_UNAVAILABLE,
    ],
    plugin_kinds: &[NOTIFICATION_CHANNEL],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
