//! A Notification as the driver hands it to a channel (ADR 0027): which
//! event it reports, about which Run or Schedule, and its English title,
//! text and link. It carries no Records and no page content, and no
//! channel renders more than these fields.

use std::time::SystemTime;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::notifications::NotificationEvent;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::schedule::ScheduleId;
use url::Url;

#[cfg(test)]
mod tests;

/// The longest event ID, in bytes.
pub const EVENT_ID_MAX: usize = 128;

/// What identifies one event for good: the delivery ledger's key, so a
/// resumed Run never sends it again, and a webhook's `webhook-id`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(String);

/// Why a text is no event ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "an event ID is 1 to {EVENT_ID_MAX} ASCII letters, digits, `_` or `-`, so a webhook signs it unambiguously"
)]
#[non_exhaustive]
pub struct EventIdInvalid;

impl EventId {
    /// Reads `text` as an event ID: 1 to [`EVENT_ID_MAX`] ASCII letters,
    /// digits, `_` or `-`, never a `.`, which would make a webhook's signed
    /// text ambiguous.
    ///
    /// # Errors
    ///
    /// [`EventIdInvalid`] for any other text.
    pub fn new(text: impl Into<String>) -> Result<Self, EventIdInvalid> {
        let text = text.into();
        let allowed = |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-');
        if (1..=EVENT_ID_MAX).contains(&text.len()) && text.bytes().all(allowed) {
            Ok(Self(text))
        } else {
            Err(EventIdInvalid)
        }
    }

    /// Its text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One Notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    /// The event's ID.
    pub event_id: EventId,
    /// The event's name, such as `run_failed`.
    pub event: NotificationEvent,
    /// The Run it is about, if any.
    pub run_id: Option<RunId>,
    /// The Schedule it is about, if any.
    pub schedule_id: Option<ScheduleId>,
    /// The Product whose engine raised it.
    pub product: Product,
    /// Its title, in English.
    pub title: String,
    /// Its text, in English.
    pub text: String,
    /// Where a person follows it up, such as the Run's page.
    pub link: Option<Url>,
    /// When the event happened.
    pub occurred_at: SystemTime,
}

impl Message {
    /// Its link without the user and password it may carry, which no
    /// channel's message carries.
    pub(crate) fn link_without_credentials(&self) -> Option<Url> {
        self.link.as_ref().map(|link| {
            let mut clean = link.clone();
            // Only a URL that cannot be a base refuses either, and it
            // carries neither.
            let _ = clean.set_username("");
            let _ = clean.set_password(None);
            clean
        })
    }
}

/// `text` with HTML's special characters escaped, as an email's HTML part
/// and a Telegram message hold it.
pub(crate) fn html_escaped(text: &str) -> String {
    text.chars().fold(
        String::with_capacity(text.len()),
        |mut escaped, character| {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&#39;"),
                other => escaped.push(other),
            }
            escaped
        },
    )
}

/// `text` cut to at most `limit` characters, a cut one ending in `…`, as a
/// party's field of that length holds it.
pub(crate) fn clipped(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_owned();
    }
    let mut kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    kept.push('…');
    kept
}
