//! The Notification channel (ADRs 0027, 0033 and 0129; issue 1023): its one
//! primitive, `notification.send`, which delivers one Notification event to a
//! channel Goodfetch does not ship, and the driver that calls it as an
//! Effect-class call under `timeout.plugin`, checks its `{ delivered }`
//! answer, accounts for the call to the Run, and reports an abandoned call
//! uncertain at once so it is never repeated blindly.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_protocol::catalog::OperationClass;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::notifications::NOTIFICATIONS_CHANNELS;
use goodfetch_protocol::plugin::notification::{
    NOTIFICATION_CHANNEL_UNAVAILABLE, NOTIFICATION_HANDLER_FAILED, NOTIFICATION_SEND,
    NotificationChannelUnavailableDetails, NotificationHandlerFailedDetails,
    NotificationSendParams, NotificationSendResult,
};

use super::{Accounts, Failure, PluginCall, Primitive};
use crate::call_error::{CallError, named};
use crate::clock::Clock;
use crate::deadline::Deadline;
use crate::plugin_host::instance::{
    CallFailure, CallOptions, PluginInstance, UncertainCall, UncertainReporter,
};
use crate::sync::{AtomicU64, Ordering};
use crate::wire::Received;

/// What every Notification channel implementation fills: the one primitive of
/// its kind, answered without any of the driver's checks.
pub trait NotificationPrimitives: Send + Sync {
    /// Its Plugin or handler name, under which Usage accounts for it.
    fn name(&self) -> &str;

    /// Answers `notification.send` for `params`.
    fn send<'call>(
        &'call self,
        params: &'call NotificationSendParams,
    ) -> BoxFuture<'call, Result<Received, CallFailure>>;

    /// Answers `notification.send` for `params` under `deadline` as an
    /// Effect-class call directly when the implementation enforces deadlines
    /// and uncertain reporting itself (such as a process Plugin sending
    /// `$/cancel` and reporting abandoned Effects). Defaults to `None`, so
    /// [`NotificationDriver`] enforces `deadline` and reports uncertainty.
    fn send_under<'call>(
        &'call self,
        params: &'call NotificationSendParams,
        deadline: &'call Deadline,
        uncertain: Arc<dyn UncertainReporter>,
    ) -> Option<BoxFuture<'call, Result<Received, CallFailure>>> {
        let _ = (params, deadline, uncertain);
        None
    }
}

/// A Notification channel a process Plugin implements, through its instance.
#[derive(Clone, Debug)]
pub struct ProcessNotificationChannel {
    name: String,
    instance: PluginInstance,
}

impl ProcessNotificationChannel {
    /// The Notification channel Plugin `name`, which `instance` answers for.
    #[must_use]
    pub fn new(name: String, instance: PluginInstance) -> Self {
        Self { name, instance }
    }

    /// Its Plugin name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl NotificationPrimitives for ProcessNotificationChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn send<'call>(
        &'call self,
        params: &'call NotificationSendParams,
    ) -> BoxFuture<'call, Result<Received, CallFailure>> {
        Box::pin(self.instance.call_under(
            NOTIFICATION_SEND,
            params,
            CallOptions {
                class: OperationClass::Effect,
                ..CallOptions::default()
            },
        ))
    }

    fn send_under<'call>(
        &'call self,
        params: &'call NotificationSendParams,
        deadline: &'call Deadline,
        uncertain: Arc<dyn UncertainReporter>,
    ) -> Option<BoxFuture<'call, Result<Received, CallFailure>>> {
        Some(Box::pin(self.instance.call_under(
            NOTIFICATION_SEND,
            params,
            CallOptions::effect(deadline.clone(), uncertain),
        )))
    }
}

/// Why a `notification.send` call through [`NotificationDriver`] did not
/// confirm delivery.
#[derive(Clone, Debug)]
pub enum NotificationSendFailure {
    /// The Effect-class call was abandoned in flight (past its deadline,
    /// cancelled, or its Plugin crashed/hit a resource limit while holding
    /// it): it may have landed, has been reported uncertain, and must not be
    /// repeated blindly.
    Uncertain(Failure),
    /// The call failed cleanly (refused before sending, answered with an
    /// Error code, malformed answer, or `delivered: false`).
    Failed(Failure),
}

impl NotificationSendFailure {
    /// The underlying kind-driver [`Failure`].
    #[must_use]
    pub fn failure(&self) -> &Failure {
        match self {
            Self::Uncertain(failure) | Self::Failed(failure) => failure,
        }
    }

    /// Consumes the outcome and returns the underlying [`Failure`].
    #[must_use]
    pub fn into_failure(self) -> Failure {
        match self {
            Self::Uncertain(failure) | Self::Failed(failure) => failure,
        }
    }

    /// Whether the delivery outcome is uncertain and must not be retried.
    #[must_use]
    pub fn is_uncertain(&self) -> bool {
        matches!(self, Self::Uncertain(_))
    }
}

/// Records whether an Effect-class call was reported uncertain while
/// forwarding every [`UncertainCall`] to an optional outer reporter.
#[derive(Debug)]
struct TrackingReporter {
    reported: AtomicU64,
    inner: Option<Arc<dyn UncertainReporter>>,
}

impl TrackingReporter {
    fn new(inner: Option<Arc<dyn UncertainReporter>>) -> Self {
        Self {
            reported: AtomicU64::new(0),
            inner,
        }
    }

    fn was_reported(&self) -> bool {
        self.reported.load(Ordering::Acquire) != 0
    }
}

impl UncertainReporter for TrackingReporter {
    fn report(&self, call: UncertainCall) {
        self.reported.store(1, Ordering::Release);
        if let Some(inner) = &self.inner {
            inner.report(call);
        }
    }
}

/// The Notification channel's driver: it calls one implementation with
/// `notification.send` as an Effect-class call under `timeout.plugin`, checks
/// its `{ delivered }` answer, accounts for the call to the Run, and reports
/// any abandoned in-flight call uncertain at once.
pub struct NotificationDriver<'a> {
    channel: &'a str,
    implementation: &'a dyn NotificationPrimitives,
    accounts: &'a dyn Accounts,
    clock: &'a Clock,
    deadline: &'a Deadline,
    uncertain: Option<Arc<dyn UncertainReporter>>,
}

impl<'a> NotificationDriver<'a> {
    /// The driver of `implementation` serving channel `channel`, whose calls
    /// go to `accounts`, the Run's, and wait on `clock` until `deadline`,
    /// which `timeout.plugin` sets.
    #[must_use]
    pub fn new(
        channel: &'a str,
        implementation: &'a dyn NotificationPrimitives,
        accounts: &'a dyn Accounts,
        clock: &'a Clock,
        deadline: &'a Deadline,
    ) -> Self {
        Self {
            channel,
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

    /// Attaches an optional [`UncertainReporter`].
    #[must_use]
    pub fn with_uncertain_opt(mut self, uncertain: Option<Arc<dyn UncertainReporter>>) -> Self {
        self.uncertain = uncertain;
        self
    }

    /// Sends one Notification through the implementation. The call is charged
    /// before it goes and recorded once it ended, however it ended
    /// ([`Accounts`]).
    ///
    /// # Errors
    ///
    /// [`NotificationSendFailure::Uncertain`] when the Effect-class call was
    /// abandoned in flight (past its deadline or crashed while in flight), or
    /// [`NotificationSendFailure::Failed`] when Usage refused the call, the
    /// implementation answered an error, or the answer was malformed or
    /// reported `delivered: false`.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future while a process-Plugin call is in flight abandons
    /// the call through `PluginInstance`, which reports it uncertain at once
    /// and refuses any late answer; the call stays charged with no time and
    /// no line in the Run trace.
    pub async fn send(
        &self,
        params: &NotificationSendParams,
    ) -> Result<(), NotificationSendFailure> {
        let call = PluginCall::of(self.implementation.name(), Primitive::NotificationSend);
        self.accounts
            .call_starts(&call)
            .await
            .map_err(|refused| NotificationSendFailure::Failed(Failure::Refused(refused)))?;
        let started = self.clock.now();
        let outcome = self.deliver(params).await;
        let failed = outcome
            .as_ref()
            .err()
            .map(|failure| failure.failure().error().code());
        let ended = call.ended(self.clock.now().duration_since(started), failed);
        match (outcome, self.accounts.call_ended(&ended).await) {
            (Ok(()), Err(refused)) => {
                Err(NotificationSendFailure::Failed(Failure::Refused(refused)))
            }
            (outcome, _) => outcome,
        }
    }

    /// Calls `notification.send`, checks `{ delivered: true }`, and splits
    /// uncertain abandonments from clean failures.
    async fn deliver(
        &self,
        params: &NotificationSendParams,
    ) -> Result<(), NotificationSendFailure> {
        let tracker = Arc::new(TrackingReporter::new(self.uncertain.clone()));
        let reporter: Arc<dyn UncertainReporter> = Arc::clone(&tracker) as _;
        let handler_failed = || self.handler_failed(&params.event_id);
        let received = if let Some(under) =
            self.implementation
                .send_under(params, self.deadline, reporter)
        {
            match under.await {
                Ok(received) => received,
                Err(failure) => {
                    let kind_failure = Failure::of(failure, handler_failed);
                    return Err(
                        if tracker.was_reported()
                            || matches!(
                                kind_failure,
                                Failure::DeadlineExceeded(_) | Failure::Crashed(_)
                            )
                        {
                            NotificationSendFailure::Uncertain(kind_failure)
                        } else {
                            NotificationSendFailure::Failed(kind_failure)
                        },
                    );
                }
            }
        } else {
            let answer = match self
                .deadline
                .within(
                    self.clock,
                    WaitedFor::PLUGIN_ANSWER,
                    self.implementation.send(params),
                )
                .await
            {
                Ok(answer) => answer,
                Err(deadline_error) => {
                    tracker.report(UncertainCall {
                        plugin: self.implementation.name().to_owned(),
                        call_id: RequestId::Text(params.event_id.clone()),
                        method: NOTIFICATION_SEND.to_owned(),
                        reason: UncertainReason::Interrupted,
                    });
                    return Err(NotificationSendFailure::Uncertain(
                        Failure::DeadlineExceeded(deadline_error),
                    ));
                }
            };
            match answer {
                Ok(received) => received,
                Err(failure) => {
                    let kind_failure = Failure::of(failure, handler_failed);
                    if matches!(
                        kind_failure,
                        Failure::DeadlineExceeded(_) | Failure::Crashed(_)
                    ) {
                        tracker.report(UncertainCall {
                            plugin: self.implementation.name().to_owned(),
                            call_id: RequestId::Text(params.event_id.clone()),
                            method: NOTIFICATION_SEND.to_owned(),
                            reason: UncertainReason::Interrupted,
                        });
                        return Err(NotificationSendFailure::Uncertain(kind_failure));
                    }
                    return Err(NotificationSendFailure::Failed(kind_failure));
                }
            }
        };
        let result = received.read::<NotificationSendResult>().map_err(|error| {
            tracing::warn!(
                channel = self.channel,
                plugin = self.implementation.name(),
                event_id = params.event_id,
                %error,
                "a Notification channel's result is not `{{ delivered }}`"
            );
            NotificationSendFailure::Failed(Failure::Malformed(handler_failed()))
        })?;
        if !result.delivered {
            tracing::warn!(
                channel = self.channel,
                plugin = self.implementation.name(),
                event_id = params.event_id,
                "a Notification channel answered `delivered: false`"
            );
            return Err(NotificationSendFailure::Failed(Failure::Malformed(
                handler_failed(),
            )));
        }
        Ok(())
    }

    /// The `notification_handler_failed` of a call for `event_id`, which
    /// names the channel and event ID and never holds raw Plugin text.
    fn handler_failed(&self, event_id: &str) -> CallError {
        CallError::new(
            &NOTIFICATION_HANDLER_FAILED,
            format!(
                "The Notification channel `{}` that {} names answered what the engine cannot take; its answer is in the log at `warn`.",
                self.channel,
                named::setting(NOTIFICATIONS_CHANNELS.name),
            ),
            &NotificationHandlerFailedDetails {
                channel: self.channel.to_owned(),
                event_id: event_id.to_owned(),
            },
        )
    }
}

/// The `notification_channel_unavailable` of the channel `channel`, which no
/// built-in and no installed Plugin of the Notification channel kind answers
/// for, or whose Plugin cannot start. It names the channel, never a path.
#[must_use]
pub fn unavailable(channel: &str) -> CallError {
    CallError::new(
        &NOTIFICATION_CHANNEL_UNAVAILABLE,
        format!(
            "No one answers for the Notification channel `{channel}` that {} names: no built-in has its type, and no installed Plugin of the Notification channel kind does or its Plugin cannot start; why is in the log at `warn`.",
            named::setting(NOTIFICATIONS_CHANNELS.name),
        ),
        &NotificationChannelUnavailableDetails {
            channel: channel.to_owned(),
        },
    )
}
