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
    /// `warn` without its text: transient.
    Unrecognized(PartyText),
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
