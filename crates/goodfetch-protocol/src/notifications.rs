//! Notification channels as settings (ADRs 0027, 0028 and 0129; spec
//! #608): `notifications.channels.<name>.*` is one family per channel, each
//! entry a [`ChannelSettings`], set only in user configuration or an
//! Operator policy, the machine's own layers. A Recipe, a program's code, a
//! Run and a flag are refused with `setting_not_applicable` naming those
//! files, since a channel names a destination and the secrets that reach it.
//! A destination's URL and every credential are Secret references, never
//! the value itself.

use schemars::JsonSchema;
use schemars::generate::SchemaSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::catalog::{
    Authority, Entries, ProtocolVersion, Setting, SharedType, SharedTypeValue, ValueKind,
    ValueShape,
};
use crate::inline_value::open_value_type;
use crate::run::{Completeness, Lifecycle, RunAxes, Verdict};
use crate::secret_reference::{REFERENCE_PATTERN, SecretReference};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The channel types Goodfetch ships (spec #608): any other `type` is a
/// Notification channel Plugin's short name.
pub const BUILT_IN_TYPES: [&str; 7] = [
    "slack", "teams", "discord", "telegram", "email", "sns", "webhook",
];

/// The `type` of the `webhook` channel, which posts a Standard Webhooks
/// delivery.
pub const WEBHOOK: &str = "webhook";

/// The `type` of the `email` channel.
const EMAIL: &str = "email";

/// What a `webhook` channel's delivery carries in its `data`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebhookPayload {
    /// The whole message: its IDs, event, Product, title, text and link.
    #[default]
    Full,
    /// Its IDs and link only, which Integrations use (spec #608).
    Thin,
}

/// A `webhook` channel's keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelWebhook {
    /// The Secret reference of the Standard Webhooks signing secret, a
    /// `whsec_` key for `v1` or a `whsk_` key for `v2a`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub secret: Option<String>,
    /// What a delivery's `data` carries: `full` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: Option<WebhookPayload>,
}

/// The provider an `email` channel sends through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EmailProvider {
    /// An SMTP server.
    Smtp,
    /// Amazon SES.
    Ses,
    /// Resend.
    Resend,
    /// Postmark.
    Postmark,
    /// SendGrid.
    Sendgrid,
    /// Mailgun.
    Mailgun,
}

/// How an SMTP connection is secured.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SmtpSecurity {
    /// TLS from the first byte.
    Tls,
    /// A plain connection upgraded with `STARTTLS`.
    Starttls,
}

/// An `email` channel's SMTP server.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelSmtp {
    /// Its host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: Option<String>,
    /// Its port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub port: Option<u16>,
    /// How the connection is secured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub security: Option<SmtpSecurity>,
    /// The Secret reference of the user name it logs in with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub username: Option<String>,
    /// The Secret reference of its password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub password: Option<String>,
}

/// An `email` channel's Amazon SES region.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelSes {
    /// The AWS region, such as `eu-west-1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub region: Option<String>,
}

/// An `email` channel's Mailgun domain and region.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelMailgun {
    /// The sending domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub domain: Option<String>,
    /// The region its domain lives in: `us` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub region: Option<MailgunRegion>,
}

/// The Mailgun region a sending domain lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MailgunRegion {
    /// The United States, at `api.mailgun.net`.
    Us,
    /// The European Union, at `api.eu.mailgun.net`.
    Eu,
}

/// An `email` channel's keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelEmail {
    /// The provider it sends through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub provider: Option<EmailProvider>,
    /// The addresses it sends to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub to: Option<Vec<String>>,
    /// The address it sends from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub from: Option<String>,
    /// The SMTP server, for the `smtp` provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub smtp: Option<ChannelSmtp>,
    /// The SES region, for the `ses` provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ses: Option<ChannelSes>,
    /// The Secret reference of the provider's API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub api_key: Option<String>,
    /// The Mailgun domain and region, for the `mailgun` provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub mailgun: Option<ChannelMailgun>,
}

/// A `telegram` channel's keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelTelegram {
    /// The chat it posts to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub chat_id: Option<String>,
    /// The Secret reference of the bot's token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub bot_token: Option<String>,
}

/// An `sns` channel's topic (#3680).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelSns {
    /// The topic's ARN.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub topic_arn: Option<String>,
    /// The topic's AWS region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub region: Option<String>,
}

/// One Notification channel, the entry of its name in
/// `notifications.channels` (spec #608).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChannelSettings {
    /// One of [`BUILT_IN_TYPES`] or a Notification channel Plugin's short
    /// name; left out for a program's own SDK handler of the entry's name.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub channel_type: Option<String>,
    /// The Secret reference of its destination's URL: a Slack, Teams or
    /// Discord webhook's, or a `webhook` channel's receiver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(REFERENCE_PATTERN), extend("arrived_in" = "0.1"))]
    pub url: Option<String>,
    /// The events it receives: the attention set when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub events: Option<Vec<String>>,
    /// The Runs it hears of: `all`, the default, or `tagged:<tag>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(pattern(RUNS_PATTERN), extend("arrived_in" = "0.1"))]
    pub runs: Option<String>,
    /// An `email` channel's keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub email: Option<ChannelEmail>,
    /// A `telegram` channel's keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub telegram: Option<ChannelTelegram>,
    /// An `sns` channel's keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sns: Option<ChannelSns>,
    /// A `webhook` channel's keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub webhook: Option<ChannelWebhook>,
}

/// What `runs` holds.
const RUNS_PATTERN: &str = "^(all|tagged:.+)$";

impl ChannelSettings {
    /// Reads `value` as a channel's entry.
    ///
    /// # Errors
    ///
    /// What is wrong with it, never repeating a Secret reference's place's
    /// text, since a value given there may be the secret itself.
    pub fn read(value: &Value) -> Result<Self, String> {
        let settings: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("expected a Notification channel's entry: {error}"))?;
        if settings.channel_type.as_deref() == Some("") {
            return Err("a Notification channel's `type` is not empty".to_owned());
        }
        if let Some(runs) = settings.runs.as_deref()
            && runs != "all"
            && runs.strip_prefix("tagged:").is_none_or(str::is_empty)
        {
            return Err(format!(
                "a Notification channel's `runs` is `all` or `tagged:<tag>`, found `{runs}`"
            ));
        }
        let email = settings.email.as_ref();
        let smtp = email.and_then(|email| email.smtp.as_ref());
        let references = [
            ("url", settings.url.as_deref()),
            (
                "webhook.secret",
                settings
                    .webhook
                    .as_ref()
                    .and_then(|webhook| webhook.secret.as_deref()),
            ),
            (
                "email.api_key",
                email.and_then(|email| email.api_key.as_deref()),
            ),
            (
                "email.smtp.username",
                smtp.and_then(|smtp| smtp.username.as_deref()),
            ),
            (
                "email.smtp.password",
                smtp.and_then(|smtp| smtp.password.as_deref()),
            ),
            (
                "telegram.bot_token",
                settings
                    .telegram
                    .as_ref()
                    .and_then(|telegram| telegram.bot_token.as_deref()),
            ),
        ];
        for (key, text) in references {
            reference(key, text)?;
        }
        if settings.channel_type.as_deref() == Some(WEBHOOK) {
            if settings.url.is_none() {
                return Err(
                    "a `webhook` channel names its `url`, a Secret reference such as `env:HOOK_URL`"
                        .to_owned(),
                );
            }
            if settings
                .webhook
                .as_ref()
                .is_none_or(|webhook| webhook.secret.is_none())
            {
                return Err(
                    "a `webhook` channel names its `webhook.secret`, a Secret reference such as `env:HOOK_SECRET`"
                        .to_owned(),
                );
            }
        }
        if settings.channel_type.as_deref() == Some(EMAIL) {
            email_needs(settings.email.as_ref())?;
        }
        Ok(settings)
    }
}

/// Whether an `email` channel's keys name its provider, its addresses and
/// what its provider sends with, or the first key it misses.
fn email_needs(email: Option<&ChannelEmail>) -> Result<(), String> {
    let missing = |key: &str, example: &str| {
        Err(format!(
            "an `email` channel names its `{key}`, such as {example}"
        ))
    };
    let Some((email, provider)) = email.and_then(|email| Some((email, email.provider?))) else {
        return missing("email.provider", "`smtp`, `ses` or `resend`");
    };
    if email.to.as_ref().is_none_or(Vec::is_empty) {
        return missing("email.to", "`[\"ops@example.com\"]`");
    }
    if email.from.is_none() {
        return missing("email.from", "`goodfetch@example.com`");
    }
    match provider {
        EmailProvider::Smtp if email.smtp.as_ref().is_none_or(|smtp| smtp.host.is_none()) => {
            missing("email.smtp.host", "`smtp.example.com`")
        }
        EmailProvider::Ses if email.ses.as_ref().is_none_or(|ses| ses.region.is_none()) => {
            missing("email.ses.region", "`eu-west-1`")
        }
        EmailProvider::Resend
        | EmailProvider::Postmark
        | EmailProvider::Sendgrid
        | EmailProvider::Mailgun
            if email.api_key.is_none() =>
        {
            missing("email.api_key", "`env:EMAIL_API_KEY`")
        }
        EmailProvider::Mailgun
            if email
                .mailgun
                .as_ref()
                .is_none_or(|mailgun| mailgun.domain.is_none()) =>
        {
            missing("email.mailgun.domain", "`mg.example.com`")
        }
        EmailProvider::Smtp
        | EmailProvider::Ses
        | EmailProvider::Resend
        | EmailProvider::Postmark
        | EmailProvider::Sendgrid
        | EmailProvider::Mailgun => Ok(()),
    }
}

/// Whether `text`, the value of the key `key` names, is a Secret reference
/// when given, or why not, never repeating it.
fn reference(key: &str, text: Option<&str>) -> Result<(), String> {
    match text.map(SecretReference::parse) {
        Some(Err(error)) => Err(format!(
            "a Notification channel's `{key}` is a Secret reference such as `env:NAME`: {error}; a value is never taken in its place"
        )),
        None | Some(Ok(_)) => Ok(()),
    }
}

impl crate::catalog::SettingJson for ChannelSettings {
    fn into_setting_json(self) -> Value {
        crate::catalog::shape_json(self)
    }
}

/// A value of `notifications.channels`: one channel's entry.
pub const CHANNEL_SHAPE: ValueShape = ValueShape {
    name: "NotificationChannelSettings",
    check: |value| ChannelSettings::read(value).map(drop),
    schema: || {
        let schema = SchemaSettings::draft2020_12()
            .with(|settings| settings.inline_subschemas = true)
            .into_generator()
            .into_root_schema_for::<ChannelSettings>();
        let mut schema = serde_json::to_value(schema).expect("invariant: a JSON Schema is JSON");
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            object.remove("title");
        }
        schema
    },
    // The first gives every key a value: the settings corpus of a
    // format's compatibility set takes a map's entry from it alone.
    examples: || {
        vec![
            json!({
                "type": "email",
                "url": "env:CHANNEL_URL",
                "events": ["run_failed", "human_pause_waiting"],
                "runs": "tagged:nightly",
                "email": {
                    "provider": "smtp",
                    "to": ["ops@example.com"],
                    "from": "goodfetch@example.com",
                    "smtp": {
                        "host": "smtp.example.com",
                        "port": 465,
                        "security": "tls",
                        "username": "env:SMTP_USER",
                        "password": "env:SMTP_PASSWORD",
                    },
                    "ses": { "region": "eu-west-1" },
                    "api_key": "env:EMAIL_API_KEY",
                    "mailgun": { "domain": "mg.example.com", "region": "eu" },
                },
                "telegram": { "chat_id": "-1001234567890", "bot_token": "env:TELEGRAM_TOKEN" },
                "sns": { "topic_arn": "arn:aws:sns:eu-west-1:123456789012:goodfetch", "region": "eu-west-1" },
                "webhook": { "secret": "env:HOOK_SECRET", "payload": "thin" },
            }),
            json!({
                "type": "webhook",
                "url": "env:HOOK_URL",
                "webhook": { "secret": "env:HOOK_SECRET" },
            }),
            json!({
                "type": "slack",
                "url": "env:SLACK_WEBHOOK_URL",
                "runs": "all",
            }),
            json!({ "type": "pagerduty" }),
            json!({}),
        ]
    },
};

fn no_channels() -> Value {
    json!({})
}

/// `notifications.channels` (ADRs 0027 and 0129; spec #608).
pub const NOTIFICATIONS_CHANNELS: Setting = Setting {
    name: "notifications.channels",
    description: "The Notification channels the engine sends a Run's and a Schedule's Notifications to, each by its name, as `notifications.channels.<name>.*`: `type`, `slack`, `teams` (Workflows webhooks only), `discord`, `telegram`, `email`, `sns` or `webhook`, or a Notification channel Plugin's short name, left out for a program's own SDK handler of that name; `url`, the Secret reference of a Slack, Teams, Discord or webhook destination; `events`, the events it receives, the attention set when left out; `runs`, `all` (the default) or `tagged:<tag>`; and `email`, `telegram`, `sns` and `webhook`, which only their types take. A URL and every credential, `webhook.secret` included, is a Secret reference: a value given in its place is refused with `setting_invalid`. Each destination is an Operator-configured endpoint, so `egress.no_private_endpoints` refuses a private one. None by default. Set only in user configuration or an Operator policy: a project file, a Recipe, a program's code, a Run and a flag are refused with `setting_not_applicable`.",
    kind: ValueKind::Map(&ValueKind::Shape(&CHANNEL_SHAPE)),
    default: no_channels,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    fixed_for_run: false,
    unbuilt: Some("notifications"),
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

open_value_type!(
    /// A Notification event's `snake_case` name (ADRs 0027 and 0029), such
    /// as `run_failed`: one of Collie's, or Foxhound's or a newer engine's,
    /// which a reader keeps as received (ADRs 0079 and 0095).
    NotificationEvent,
    "a Notification event's name"
);

/// Defines each Collie event as an associated constant of
/// [`NotificationEvent`] and as a value of the catalog's open value list.
macro_rules! collie_events {
    ($($constant:ident = $name:literal: $description:literal;)*) => {
        impl NotificationEvent {
            $(
                #[doc = $description]
                pub const $constant: Self = Self::from_static($name);
            )*

            /// Collie's events.
            pub const COLLIE: &[Self] = &[$(Self::$constant),*];
        }

        /// Collie's events in the catalog's open value list.
        const EVENT_VALUES: &[SharedTypeValue] = &[$(
            SharedTypeValue {
                shared_type: SharedType::NotificationEvent,
                name: $name,
                description: $description,
                arrived_in: ARRIVED_IN,
            },
        )*];
    };
}

collie_events! {
    RUN_STARTED = "run_started": "A Run started.";
    RUN_FINISHED = "run_finished": "A Run finished.";
    RUN_FAILED = "run_failed": "A Run ended on an error.";
    RUN_STOPPED = "run_stopped": "A Run was stopped, by a Limit, a person or a program.";
    VERDICT_REJECTED = "verdict_rejected": "A Run's success check rejected its result.";
    VERDICT_NEEDS_REVIEW = "verdict_needs_review": "A Run's result waits for a person's review.";
    RUN_INCOMPLETE = "run_incomplete": "A Run ended with its Completeness `incomplete`.";
    EFFECT_UNCERTAIN = "effect_uncertain": "A Run reported an Effect whose outcome is uncertain.";
    RECONCILIATION_REQUESTED = "reconciliation_requested": "A person is asked to reconcile an uncertain Effect.";
    EFFECT_RECONCILED = "effect_reconciled": "An uncertain Effect was reconciled.";
    APPROVAL_REQUESTED = "approval_requested": "A Run waits for a person's approval.";
    HUMAN_PAUSE_WAITING = "human_pause_waiting": "A Human pause waits for a person, with a link to the Live view.";
    LIMIT_THRESHOLD = "limit_threshold": "A Limit reached its warning threshold.";
    SCHEDULE_AUTO_PAUSED = "schedule_auto_paused": "A Schedule paused itself.";
    SCHEDULE_OCCURRENCES_SKIPPED = "schedule_occurrences_skipped": "A Limit skipped a Schedule's occurrences, notified once per period.";
    SCHEDULE_OCCURRENCES_MISSED = "schedule_occurrences_missed": "A Schedule's occurrences were missed, notified once per wake.";
    IDENTITY_COOKIE_EXPIRY = "identity_cookie_expiry": "A Saved identity's earliest cookie expiry entered its warning window.";
    QUARANTINED = "quarantined": "Code mode or a Plugin was quarantined.";
}

impl NotificationEvent {
    /// The attention set: the events a channel with no `events` receives
    /// (ADRs 0027 and 0029), what needs a person rather than what went
    /// well.
    pub const ATTENTION: [Self; 13] = [
        Self::RUN_FAILED,
        Self::RUN_STOPPED,
        Self::VERDICT_REJECTED,
        Self::VERDICT_NEEDS_REVIEW,
        Self::RUN_INCOMPLETE,
        Self::EFFECT_UNCERTAIN,
        Self::RECONCILIATION_REQUESTED,
        Self::APPROVAL_REQUESTED,
        Self::HUMAN_PAUSE_WAITING,
        Self::LIMIT_THRESHOLD,
        Self::SCHEDULE_AUTO_PAUSED,
        Self::IDENTITY_COOKIE_EXPIRY,
        Self::QUARANTINED,
    ];

    /// The events a Run's end raises, from its axes as it ended (ADR
    /// 0029): its Lifecycle's, then a `rejected` or `needs_review`
    /// Verdict's and an `incomplete` Completeness's; none while it runs.
    #[must_use]
    pub fn of_run_end(axes: &RunAxes) -> Vec<Self> {
        let lifecycle = match axes.lifecycle {
            Lifecycle::Running => return Vec::new(),
            Lifecycle::Finished => Self::RUN_FINISHED,
            Lifecycle::Stopped => Self::RUN_STOPPED,
            Lifecycle::Failed => Self::RUN_FAILED,
        };
        let verdict = match axes.verdict {
            Some(Verdict::Rejected) => Some(Self::VERDICT_REJECTED),
            Some(Verdict::NeedsReview) => Some(Self::VERDICT_NEEDS_REVIEW),
            _ => None,
        };
        let completeness =
            (axes.completeness == Some(Completeness::Incomplete)).then_some(Self::RUN_INCOMPLETE);
        [Some(lifecycle), verdict, completeness]
            .into_iter()
            .flatten()
            .collect()
    }
}

impl ChannelSettings {
    /// Whether the channel receives `event`: one its `events` names, or
    /// one of the attention set when it names none.
    #[must_use]
    pub fn receives(&self, event: NotificationEvent) -> bool {
        match &self.events {
            Some(events) => events.iter().any(|name| name == event.as_str()),
            None => NotificationEvent::ATTENTION.contains(&event),
        }
    }
}

/// Notifications' entries: their settings and Collie's events.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[NOTIFICATIONS_CHANNELS],
    hook_stages: &[],
    shared_type_values: EVENT_VALUES,
};

#[cfg(test)]
mod tests;
