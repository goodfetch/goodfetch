//! "A Plugin quarantined" Notification delivery through ADR 0027's ledger
//! (ADRs 0027 and 0033, issue 837): maps a [`QuarantineEvent`] to a
//! deterministic [`EventId`] per Run and Plugin and a [`NotificationEvent::QUARANTINED`]
//! [`Message`], delivering it once through [`Driver`] so a resumed Run or
//! repeated trigger never sends a duplicate.

use std::fmt;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_engine::plugin_host::instance::{QuarantineEvent, QuarantineNotifier};
use goodfetch_protocol::notifications::NotificationEvent;
use goodfetch_protocol::run::RunId;
use sha2::{Digest as _, Sha256};

use crate::driver::{Channel, Driver};
use crate::message::{EVENT_ID_MAX, EventId, Message};

/// Builds the deterministic ADR 0027 ledger [`EventId`] for a quarantined
/// Plugin within a Run: unique per `(run_id, plugin)` and guaranteed to fit
/// [`EVENT_ID_MAX`] with only ASCII alphanumeric, `_` and `-` characters.
#[must_use]
pub fn event_id(run_id: Option<&RunId>, plugin: &str) -> EventId {
    let run_part = run_id.map_or("task", RunId::as_str);
    let sanitized: String = plugin
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-') {
                char::from(b)
            } else {
                '_'
            }
        })
        .collect();
    let candidate = format!("plugin_quarantined-{run_part}-{sanitized}");
    if candidate.len() <= EVENT_ID_MAX
        && let Ok(id) = EventId::new(candidate)
    {
        return id;
    }
    let mut hasher = Sha256::new();
    hasher.update(run_part.as_bytes());
    hasher.update(b"\0");
    hasher.update(plugin.as_bytes());
    let hex = data_encoding::HEXLOWER.encode(&hasher.finalize());
    EventId::new(format!("plugin_quarantined-{hex}"))
        .expect("invariant: hashed event ID is 83 ASCII characters")
}

/// Builds the [`NotificationEvent::QUARANTINED`] [`Message`] for `event`.
#[must_use]
pub fn message(event: &QuarantineEvent) -> Message {
    Message {
        event_id: event_id(event.run_id.as_ref(), &event.plugin),
        event: NotificationEvent::QUARANTINED,
        run_id: event.run_id.clone(),
        schedule_id: None,
        product: event.product,
        title: format!("Plugin `{}` quarantined", event.plugin),
        text: format!(
            "Plugin `{}` crashed {} times within 10 minutes (last failure: {}) and is quarantined for the rest of this run.",
            event.plugin, event.crashes, event.reason,
        ),
        link: None,
        occurred_at: event.occurred_at,
    }
}

/// Delivers [`QuarantineEvent`] Notifications to a configured [`Channel`]
/// through ADR 0027's [`Driver`] and delivery ledger, guaranteeing at most one
/// delivery per Run and Plugin.
pub struct LedgerQuarantineNotifier<C: Channel + 'static> {
    driver: Driver,
    channel: Arc<C>,
}

impl<C: Channel + 'static> fmt::Debug for LedgerQuarantineNotifier<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LedgerQuarantineNotifier")
            .field("channel", &self.channel.name())
            .finish_non_exhaustive()
    }
}

impl<C: Channel + 'static> LedgerQuarantineNotifier<C> {
    /// Creates a notifier delivering through `driver` onto `channel`.
    #[must_use]
    pub fn new(driver: Driver, channel: Arc<C>) -> Self {
        Self { driver, channel }
    }
}

impl<C: Channel + 'static> QuarantineNotifier for LedgerQuarantineNotifier<C> {
    fn notify(&self, event: QuarantineEvent) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let driver = self.driver.clone();
        let channel = Arc::clone(&self.channel);
        let msg = message(&event);
        Box::pin(async move {
            if let Err(error) = driver.deliver(channel.as_ref(), &msg).await {
                tracing::warn!(
                    plugin = %event.plugin,
                    channel = channel.name(),
                    %error,
                    "could not record or deliver `quarantined` Notification"
                );
            }
        })
    }
}

#[cfg(test)]
#[expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]
mod tests {
    use std::time::SystemTime;

    use goodfetch_engine::clock::Clock;
    use goodfetch_engine::run_state::{RunState, RunStateOptions};
    use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
    use goodfetch_protocol::catalog::Product;
    use goodfetch_protocol::run::RunId;
    use goodfetch_test_support::TestEnv;
    use tokio::sync::Mutex;

    use super::*;
    use crate::failure::Failure;
    use crate::ledger::{Ledger, TABLES};
    use crate::rate_limit::RateLimit;

    struct CountingChannel {
        sent: Arc<Mutex<Vec<NotificationEvent>>>,
    }

    impl Channel for CountingChannel {
        type Failure = ();

        fn name(&self) -> &'static str {
            "alerts"
        }

        fn destination(&self) -> &'static str {
            "alerts.example"
        }

        async fn send(&self, message: &Message) -> Result<(), Self::Failure> {
            self.sent.lock().await.push(message.event);
            Ok(())
        }

        fn map_failure(&self, (): Self::Failure) -> Failure {
            Failure::Rejected
        }
    }

    /// ADR 0027's ledger sends one `quarantined` Notification per Run and
    /// Plugin, never a second one even if triggered again.
    ///
    /// Covers: story:7-35
    #[tokio::test(start_paused = true)]
    async fn quarantine_notification_sends_once_per_run_and_plugin_through_ledger() {
        let env = TestEnv::in_memory();
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let owner = TaskOwner::root(BlockingPolicy::Threads);
        let state = RunState::open(
            env.folder().join("run.sqlite"),
            &RunStateOptions::default(),
            vec![TABLES],
            1,
            &owner,
        )
        .await
        .unwrap();
        let driver = Driver::new(Ledger::new(state), Arc::new(RateLimit::new(clock)), clock);
        let sent = Arc::new(Mutex::new(Vec::new()));
        let channel = Arc::new(CountingChannel {
            sent: Arc::clone(&sent),
        });
        let notifier = LedgerQuarantineNotifier::new(driver, channel);
        let event = QuarantineEvent {
            plugin: "flaky-plugin".to_owned(),
            run_id: Some(RunId::parse("quarantine-ledger-test").unwrap()),
            product: Product::Collie,
            crashes: 3,
            reason: "plugin_crashed".to_owned(),
            occurred_at: SystemTime::UNIX_EPOCH,
        };

        notifier.notify(event.clone()).await;
        notifier.notify(event).await;

        let delivered = sent.lock().await;
        assert_eq!(delivered.as_slice(), &[NotificationEvent::QUARANTINED]);
        drop(delivered);
        owner.stop().await;
    }
}
