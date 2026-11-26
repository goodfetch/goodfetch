//! The chat channels (ADR 0027; issue 1387): a Notification posted in
//! English as a Slack incoming webhook's blocks ([`Chat::slack`]), a
//! Microsoft Teams Workflows webhook's Adaptive Card ([`Chat::teams`]), a
//! Discord webhook's embed ([`Chat::discord`]) or a Telegram bot's message
//! ([`Chat::telegram`]). Each delivery is one `POST` through the engine's
//! Notification channel endpoint, its own client and never a Session's
//! (ADR 0088), and each platform is a translation point of its own (ADR
//! 0127), whose table maps its statuses and error bodies to the driver's
//! [`Failure`]. A destination's URL, or a Telegram bot's, carries its
//! credential, so no channel formats it: its `Debug` names the host alone.

mod discord;
mod slack;
mod teams;
mod telegram;

use std::fmt::Write as _;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use hyper::body::Bytes;
use secrecy::SecretString;
use serde_json::Value;
use url::Url;

use crate::api::{self, Api, ApiFailure, ApiUnusable, Table};
use crate::driver::Channel;
use crate::failure::Failure;
use crate::message::Message;

/// How long a delivery waits for the platform's whole answer before it
/// counts as [`Failure::Timeout`].
pub const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// The platform a chat channel posts to, and what it posts to it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Platform {
    Slack,
    Teams,
    Discord,
    /// A Telegram bot's chat, by its ID or `@username`.
    Telegram {
        chat_id: String,
    },
}

impl Platform {
    /// The platform's translation table.
    fn table(&self) -> &'static Table {
        match self {
            Self::Slack => &slack::TABLE,
            Self::Teams => &teams::TABLE,
            Self::Discord => &discord::TABLE,
            Self::Telegram { .. } => &telegram::TABLE,
        }
    }

    /// The JSON body that reports `message` on the platform.
    fn body(&self, message: &Message) -> Value {
        match self {
            Self::Slack => slack::body(message),
            Self::Teams => teams::body(message),
            Self::Discord => discord::body(message),
            Self::Telegram { chat_id } => telegram::body(chat_id, message),
        }
    }
}

/// A `slack`, `teams`, `discord` or `telegram` channel.
pub struct Chat {
    name: String,
    platform: Platform,
    api: Api,
    clock: Clock,
}

/// Why a delivery got no success from the platform.
#[derive(Debug)]
#[non_exhaustive]
pub enum ChatFailure {
    /// No whole answer within [`SEND_TIMEOUT`].
    TimedOut,
    /// The platform's failure.
    Api(ApiFailure),
}

impl Chat {
    /// The `slack` channel `name`, posting to the incoming webhook `url`
    /// through `endpoint`.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable`] when `url` is no `http` or `https` URL with a host,
    /// or the egress policy refuses it (ADR 0017), so the configuration
    /// fails rather than each delivery.
    pub fn slack(
        name: impl Into<String>,
        url: &Url,
        endpoint: NotificationEndpoint,
        clock: Clock,
    ) -> Result<Self, ApiUnusable> {
        Self::posting(name, Platform::Slack, url, endpoint, clock)
    }

    /// The `teams` channel `name`, posting to the Workflows webhook `url`
    /// through `endpoint`.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable`] as [`Self::slack`]'s, and
    /// [`ApiUnusable::TeamsConnector`] for an Office 365 connector's URL,
    /// which takes no Adaptive Card and which Microsoft retired.
    pub fn teams(
        name: impl Into<String>,
        url: &Url,
        endpoint: NotificationEndpoint,
        clock: Clock,
    ) -> Result<Self, ApiUnusable> {
        if url.host_str().is_some_and(teams::is_connector) {
            return Err(ApiUnusable::TeamsConnector);
        }
        Self::posting(name, Platform::Teams, url, endpoint, clock)
    }

    /// The `discord` channel `name`, posting to the webhook `url` through
    /// `endpoint`.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable`] as [`Self::slack`]'s.
    pub fn discord(
        name: impl Into<String>,
        url: &Url,
        endpoint: NotificationEndpoint,
        clock: Clock,
    ) -> Result<Self, ApiUnusable> {
        Self::posting(name, Platform::Discord, url, endpoint, clock)
    }

    /// The `telegram` channel `name`, its bot `bot_token` posting to the
    /// chat `chat_id` through `endpoint`, at Telegram's Bot API, or at
    /// `base` when given: where a stand-in answers in Telegram's place; a
    /// Run gives none.
    ///
    /// # Errors
    ///
    /// [`ApiUnusable::ChatId`] for a chat that is no number nor
    /// `@username`, [`ApiUnusable::Key`] for a token that is no bot's, and
    /// [`ApiUnusable`] as [`Self::slack`]'s for the API's URL.
    pub fn telegram(
        name: impl Into<String>,
        chat_id: &str,
        bot_token: &SecretString,
        base: Option<Url>,
        endpoint: NotificationEndpoint,
        clock: Clock,
    ) -> Result<Self, ApiUnusable> {
        if !telegram::is_chat_id(chat_id) {
            return Err(ApiUnusable::ChatId);
        }
        let url = telegram::url(base, bot_token)?;
        let platform = Platform::Telegram {
            chat_id: chat_id.to_owned(),
        };
        Self::posting(name, platform, &url, endpoint, clock)
    }

    /// The channel `name` posting to `platform` at `url`.
    fn posting(
        name: impl Into<String>,
        platform: Platform,
        url: &Url,
        endpoint: NotificationEndpoint,
        clock: Clock,
    ) -> Result<Self, ApiUnusable> {
        Ok(Self {
            name: name.into(),
            platform,
            api: Api::new(url, "application/json", None, endpoint)?,
            clock,
        })
    }
}

impl std::fmt::Debug for Chat {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The URL, and a Telegram chat, stay out: the URL carries the
        // destination's credential.
        formatter
            .debug_struct("Chat")
            .field("name", &self.name)
            .field("platform", &self.platform.table().party)
            .field("host", &self.api.host())
            .finish_non_exhaustive()
    }
}

impl Channel for Chat {
    type Failure = ChatFailure;

    fn name(&self) -> &str {
        &self.name
    }

    fn destination(&self) -> &str {
        self.api.host()
    }

    async fn send(&self, message: &Message) -> Result<(), ChatFailure> {
        let body = Bytes::from(self.platform.body(message).to_string());
        let sent = self.api.post(None, body, self.clock, self.platform.table());
        tokio::select! {
            sent = sent => sent.map_err(ChatFailure::Api),
            () = self.clock.sleep(SEND_TIMEOUT) => Err(ChatFailure::TimedOut),
        }
    }

    fn map_failure(&self, failure: ChatFailure) -> Failure {
        match failure {
            ChatFailure::TimedOut => Failure::Timeout,
            ChatFailure::Api(failure) => api::translate(failure, self.platform.table()),
        }
    }
}

/// What every platform's message says under its text: the Product and the
/// event, and the Run or Schedule it is about.
fn context(message: &Message) -> String {
    let mut context = format!("{} · {}", message.product.name(), message.event);
    if let Some(run_id) = &message.run_id {
        let _ = write!(context, " · Run {run_id}");
    }
    if let Some(schedule_id) = &message.schedule_id {
        let _ = write!(context, " · Schedule {}", schedule_id.as_str());
    }
    context
}
