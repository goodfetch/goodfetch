//! Notifications (ADRs 0027 and 0129; spec #608): messages about the
//! progress or outcome of a Run or a Schedule, sent to a person or a
//! system, which carry no Records. The Notification channel driver
//! ([`driver`]) owns
//! what every channel shares: the delivery ledger ([`ledger`]), keyed by
//! event ID so a resumed Run never sends an event again, retries on the
//! delivery schedule, the rate limit of each destination ([`rate_limit`])
//! and the failure enum ([`failure`]) each channel's translation point
//! maps its party's answers to. The channels fill it: [`webhook`] posts a
//! signed Standard Webhooks delivery, [`email`] sends a two-part message
//! through an SMTP server, Amazon SES or an email provider's HTTP API,
//! [`chat`] posts to Slack, Microsoft Teams, Discord or Telegram, [`sns`]
//! publishes to an Amazon SNS topic, and
//! [`plugin_channel`] hands the Notification to a channel Plugin or an SDK
//! handler. The channels reached over an HTTP API share [`api`].

pub mod api;
pub mod chat;
pub mod dispatcher;
pub mod driver;
pub mod email;
pub mod failure;
pub mod ledger;
pub mod message;
pub mod plugin_channel;
pub mod rate_limit;
pub mod sns;
pub mod subscriptions;
pub mod webhook;

/// What `goodfetch-notification` contributes to an engine composition: its
/// [`NotificationDispatcher`](goodfetch_engine::notification::NotificationDispatcher)
/// and Integration Notification subscriptions (`notification.subscribe` /
/// `notification.unsubscribe`, ADRs 0017, 0027 and 0087).
#[must_use]
pub fn contribution() -> goodfetch_engine::Contribution {
    subscriptions::contribution().with_notification_dispatcher(dispatcher::new_dispatcher())
}
