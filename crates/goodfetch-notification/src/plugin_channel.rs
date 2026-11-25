//! The driver's side of a channel Plugin or an SDK handler (ADR 0129;
//! issue 1385): the driver hands each Notification's event ID, event, text
//! and link to `notification.send` through a [`NotificationCall`], and the
//! answer's translation point maps it to the driver's
//! [`Failure`](crate::failure::Failure). A `GoodfetchError` keeps what its
//! own `retryable` says; any other error the program raises is transient.
//! The call itself, its catalog method and its Plugin host are the engine's
//! (issue 1023).

use std::future::Future;
use std::time::Duration;

use goodfetch_engine::secrets::PartyText;
use url::Url;

use crate::driver::Channel;
use crate::failure::Failure;
use crate::message::Message;

/// What `notification.send` carries: the Notification without what only
/// the built-in channels render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params<'a> {
    /// The event ID, the program's idempotency key.
    pub event_id: &'a str,
    /// The event's name, such as `run_failed`.
    pub event: &'a str,
    /// The Notification's text, in English.
    pub text: &'a str,
    /// Where the Run or Schedule is shown, if anywhere.
    pub link: Option<&'a Url>,
}

/// Why `notification.send` failed, as its caller read the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CallFailure {
    /// The program raised a `GoodfetchError`.
    Goodfetch {
        /// Its code.
        code: String,
        /// Whether it says the call may be tried again.
        retryable: bool,
        /// The wait it asks for before the next try, if it gave one.
        retry_after: Option<Duration>,
    },
    /// The program failed in any other way: another exception, or no
    /// answer the caller could read. Kept as party text.
    Other(String),
}

/// The call that sends a Notification to a channel Plugin or an SDK
/// handler.
pub trait NotificationCall: Send + Sync {
    /// Calls `notification.send` with `params`.
    fn send(&self, params: Params<'_>) -> impl Future<Output = Result<(), CallFailure>> + Send;
}

/// A channel a Plugin or an SDK handler delivers.
#[derive(Debug)]
pub struct PluginChannel<C> {
    name: String,
    destination: String,
    call: C,
}

impl<C: NotificationCall> PluginChannel<C> {
    /// The channel `name`, rate-limited as `destination`, sending through
    /// `call`.
    pub fn new(name: impl Into<String>, destination: impl Into<String>, call: C) -> Self {
        Self {
            name: name.into(),
            destination: destination.into(),
            call,
        }
    }
}

impl<C: NotificationCall> Channel for PluginChannel<C> {
    type Failure = CallFailure;

    fn name(&self) -> &str {
        &self.name
    }

    fn destination(&self) -> &str {
        &self.destination
    }

    async fn send(&self, message: &Message) -> Result<(), CallFailure> {
        self.call
            .send(Params {
                event_id: message.event_id.as_str(),
                event: &message.event,
                text: &message.text,
                link: message.link.as_ref(),
            })
            .await
    }

    fn map_failure(&self, failure: CallFailure) -> Failure {
        translate(failure)
    }
}

/// The plugin channel's translation point (ADR 0127): a retryable
/// `GoodfetchError` that asks for a wait is [`Failure::RateLimited`], one
/// that does not is [`Failure::ServerError`], one that is not retryable is
/// [`Failure::Rejected`], and any other failure is
/// [`Failure::Unrecognized`], logged at `warn` without its text.
fn translate(failure: CallFailure) -> Failure {
    match failure {
        CallFailure::Goodfetch {
            retryable: true,
            retry_after: Some(retry_after),
            ..
        } => Failure::RateLimited {
            retry_after: Some(retry_after),
        },
        CallFailure::Goodfetch {
            retryable: true, ..
        } => Failure::ServerError,
        CallFailure::Goodfetch {
            retryable: false, ..
        } => Failure::Rejected,
        CallFailure::Other(text) => {
            tracing::warn!(
                "a channel Plugin or SDK handler failed with an error that is no GoodfetchError"
            );
            Failure::Unrecognized(PartyText::new(text))
        }
    }
}

#[cfg(test)]
mod tests;
