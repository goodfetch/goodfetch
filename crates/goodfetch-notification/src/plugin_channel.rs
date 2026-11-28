//! The driver's side of a channel Plugin or an SDK handler (ADR 0129;
//! issue 1385): the driver hands each Notification's event ID, event, text
//! and link to `notification.send` through a [`NotificationCall`], and the
//! answer's translation point maps it to the driver's
//! [`Failure`](crate::failure::Failure). A `GoodfetchError` keeps what its
//! own `retryable` says; any other error the program raises is transient.
//! The call itself, its catalog method and its Plugin host are the engine's
//! (issue 1023).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::plugin_host::instance::UncertainReporter;
use goodfetch_engine::plugin_kinds::Accounts;
use goodfetch_engine::plugin_kinds::notification::{
    NotificationDriver, NotificationPrimitives, NotificationSendFailure,
};
use goodfetch_protocol::notifications::NotificationEvent;
use goodfetch_protocol::plugin::notification::NotificationSendParams;
use url::Url;

use crate::driver::Channel;
use crate::failure::{ChannelText, Failure};
use crate::message::Message;

/// What `notification.send` carries: the Notification without what only
/// the built-in channels render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Params<'a> {
    /// The event ID, the program's idempotency key.
    pub event_id: &'a str,
    /// The event's name, such as `run_failed`.
    pub event: NotificationEvent,
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
    /// answer the caller could read.
    Other(ChannelText),
    /// The Effect-class `notification.send` call was abandoned in flight
    /// (past its deadline or crashed while holding the call): it may still
    /// have landed, has been reported uncertain, and must not be retried
    /// blindly (ADRs 0027, 0029 and 0033; issue 1023).
    Uncertain,
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
                event: message.event,
                text: &message.text,
                link: message.link.as_ref(),
            })
            .await
    }

    fn map_failure(&self, failure: CallFailure) -> Failure {
        translate(failure)
    }
}

/// A [`NotificationCall`] backed by any [`NotificationPrimitives`]
/// implementation (a process Plugin or an SDK handler) driven through
/// [`NotificationDriver`] as an Effect-class call under `deadline` (ADRs
/// 0027, 0033 and 0129; issue 1023).
pub struct PluginNotificationCall<P> {
    channel: String,
    implementation: P,
    accounts: Arc<dyn Accounts>,
    clock: Clock,
    deadline: Deadline,
    uncertain: Option<Arc<dyn UncertainReporter>>,
}

impl<P: NotificationPrimitives> PluginNotificationCall<P> {
    /// Creates a [`NotificationCall`] for channel `channel` backed by
    /// `implementation`, accounted to `accounts` and waiting on `clock` until
    /// `deadline`.
    #[must_use]
    pub fn new(
        channel: impl Into<String>,
        implementation: P,
        accounts: Arc<dyn Accounts>,
        clock: Clock,
        deadline: Deadline,
    ) -> Self {
        Self {
            channel: channel.into(),
            implementation,
            accounts,
            clock,
            deadline,
            uncertain: None,
        }
    }

    /// Attaches an [`UncertainReporter`] that receives every abandoned
    /// Effect-class `notification.send` call at once.
    #[must_use]
    pub fn with_uncertain(mut self, uncertain: Arc<dyn UncertainReporter>) -> Self {
        self.uncertain = Some(uncertain);
        self
    }
}

impl<P: NotificationPrimitives> NotificationCall for PluginNotificationCall<P> {
    async fn send(&self, params: Params<'_>) -> Result<(), CallFailure> {
        let driver = NotificationDriver::new(
            &self.channel,
            &self.implementation,
            self.accounts.as_ref(),
            &self.clock,
            &self.deadline,
        )
        .with_uncertain_opt(self.uncertain.clone());
        let wire = NotificationSendParams {
            event_id: params.event_id.to_owned(),
            event: params.event,
            text: params.text.to_owned(),
            link: params.link.map(|url| url.as_str().to_owned()),
        };
        match driver.send(&wire).await {
            Ok(()) => Ok(()),
            Err(NotificationSendFailure::Uncertain(_)) => Err(CallFailure::Uncertain),
            Err(NotificationSendFailure::Failed(failure)) => {
                let error = failure.into_error();
                let retry_after = error
                    .details()
                    .and_then(|details| details.get("retry_after"))
                    .and_then(serde_json::Value::as_u64)
                    .map(Duration::from_millis);
                Err(CallFailure::Goodfetch {
                    code: error.code().to_owned(),
                    retryable: error.retryable(),
                    retry_after,
                })
            }
        }
    }
}

/// The plugin channel's translation point (ADR 0127): a retryable
/// `GoodfetchError` that asks for a wait is [`Failure::RateLimited`], one
/// that does not is [`Failure::ServerError`], one that is not retryable is
/// [`Failure::Rejected`], an abandoned Effect-class call is
/// [`Failure::Uncertain`], and any other failure is
/// [`Failure::Unrecognized`], logged at `warn` with its text.
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
        CallFailure::Uncertain => Failure::Uncertain,
        CallFailure::Other(text) => text.unrecognized(
            "a channel Plugin or SDK handler failed with an error that is no GoodfetchError",
        ),
    }
}

#[cfg(test)]
mod tests;
