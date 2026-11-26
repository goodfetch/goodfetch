//! Why a channel did not deliver a Notification (ADR 0127): the failure
//! enum every channel's translation point maps its party's answers to.
//! A transient failure is sent again on the delivery schedule; a permanent
//! one fails the delivery at once.

use std::time::Duration;

use goodfetch_engine::secrets::PartyText;

/// Why an attempt did not deliver.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Failure {
    /// No connection, or one that broke: transient.
    Unreachable,
    /// No answer in time: transient.
    Timeout,
    /// The destination asked to be sent less: transient, the next attempt
    /// no sooner than its `Retry-After`.
    RateLimited {
        /// How long it asked to be left alone, if it said.
        retry_after: Option<Duration>,
    },
    /// The destination failed on its side: transient.
    ServerError,
    /// The destination refused the credentials: permanent.
    Unauthorized,
    /// The destination no longer exists: permanent.
    Gone,
    /// The destination refused the message itself: permanent.
    Rejected,
    /// A failure the channel's translation point does not know, logged at
    /// `warn` with its text: transient.
    Unrecognized(PartyText),
}

/// A channel's party's own text about a failure, as its `send` returns it
/// to its translation point (ADR 0127): read only there, through
/// [`ChannelText::unrecognized`], and redacted wherever it is formatted.
#[derive(Clone, PartialEq, Eq)]
pub struct ChannelText(String);

impl ChannelText {
    /// The party's text `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// [`Failure::Unrecognized`] for this text, logged at `warn` with the
    /// text and `what` the translation point did not know, so a new answer
    /// is noticed and added to its table.
    pub(crate) fn unrecognized(self, what: &str) -> Failure {
        tracing::warn!(text = %self.0, "{what}");
        Failure::Unrecognized(PartyText::new(self.0))
    }
}

impl std::fmt::Debug for ChannelText {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "ChannelText(<{} bytes, redacted>)", self.0.len())
    }
}

impl Failure {
    /// Whether the delivery is tried again.
    pub fn is_transient(&self) -> bool {
        !matches!(self, Self::Unauthorized | Self::Gone | Self::Rejected)
    }

    /// The wait the destination asked for before the next attempt.
    pub fn retry_after(&self) -> Option<Duration> {
        match self {
            Self::RateLimited { retry_after } => *retry_after,
            _ => None,
        }
    }

    /// Its name in the delivery ledger and the logs.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unreachable => "unreachable",
            Self::Timeout => "timeout",
            Self::RateLimited { .. } => "rate_limited",
            Self::ServerError => "server_error",
            Self::Unauthorized => "unauthorized",
            Self::Gone => "gone",
            Self::Rejected => "rejected",
            Self::Unrecognized(_) => "unrecognized",
        }
    }
}
