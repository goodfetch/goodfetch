//! Integration Notification subscriptions (`notification.subscribe` and
//! `notification.unsubscribe`, ADRs 0017 and 0027; spec #608, issue 1403).
//!
//! Registers and removes `webhook` channels named for an Integration's trigger
//! with [`WebhookPayload::Thin`] and a Standard Webhooks `whsec_` signing
//! secret returned once on creation. Both operations are idempotent writes,
//! offered on the HTTP listener and the CLI, and refused on agent surfaces.
//! Each destination URL passes ADR 0017's endpoint rule through
//! [`NotificationEndpoint::judge`], so `egress.no_private_endpoints` refuses a
//! private address.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use data_encoding::BASE64;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::runs::{RunEndHandler, RunEndObserver, RunEndedContext};
use goodfetch_engine::webhook_signing::{Signer, SigningKey};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::notifications::{
    NOTIFICATION_SUBSCRIBE, NOTIFICATION_UNSUBSCRIBE, NotificationEvent,
    NotificationSubscribeParams, NotificationSubscribeResult, NotificationUnsubscribeParams,
    NotificationUnsubscribeResult,
};
use goodfetch_protocol::run::{RunAxes, RunId};
use secrecy::SecretString;
use sha2::{Digest as _, Sha256};
use tokio::sync::Mutex;
use url::Url;

use crate::driver::Driver;
use crate::ledger::{Entry, Ledger};
use crate::message::{EVENT_ID_MAX, EventId, Message};
use crate::rate_limit::RateLimit;
use crate::webhook::{Payload, Webhook};

/// Maximum number of active `notification.subscribe` channels held in one
/// [`SubscriptionStore`] (STYLE.md: every collection that grows with load has
/// a bound).
pub const MAX_SUBSCRIPTIONS: usize = 256;

/// A single subscribed Integration webhook channel.
#[derive(Clone, Debug)]
pub struct Subscription {
    /// Channel name (named for the Integration's trigger).
    pub channel: String,
    /// Destination webhook URL.
    pub url: Url,
    /// Events this subscription receives; `None` means the attention set.
    pub events: Option<Vec<NotificationEvent>>,
    /// The generated `whsec_` signing secret (zeroed on drop).
    secret: SecretString,
}

impl Subscription {
    /// Whether this subscription receives `event`.
    #[must_use]
    pub fn receives(&self, event: &NotificationEvent) -> bool {
        match &self.events {
            Some(list) => list.iter().any(|item| item == event),
            None => NotificationEvent::ATTENTION.contains(event),
        }
    }

    /// Builds a [`Signer`] from this subscription's signing secret.
    #[must_use]
    pub fn signer(&self) -> Signer {
        let key = SigningKey::from_secret(&self.secret)
            .expect("invariant: stored subscription secret was validated on creation");
        Signer::new(vec![key], None).expect("invariant: single valid v1 SigningKey builds a Signer")
    }
}

/// Shared store of active `notification.subscribe` webhook channels.
#[derive(Clone, Debug)]
pub struct SubscriptionStore {
    inner: Arc<Mutex<BTreeMap<String, Subscription>>>,
    max_subscriptions: usize,
}

impl Default for SubscriptionStore {
    fn default() -> Self {
        Self::with_capacity(MAX_SUBSCRIPTIONS)
    }
}

impl SubscriptionStore {
    /// Creates an empty subscription store bounded by [`MAX_SUBSCRIPTIONS`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty subscription store holding at most `max_subscriptions`
    /// distinct channels.
    #[must_use]
    pub fn with_capacity(max_subscriptions: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BTreeMap::new())),
            max_subscriptions,
        }
    }

    /// Whether the store holds no active subscriptions.
    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }

    /// Lists all active subscriptions.
    pub async fn list(&self) -> Vec<Subscription> {
        self.inner.lock().await.values().cloned().collect()
    }

    /// Delivers `message` to every matching subscribed channel using `ledger`
    /// so a resumed Run never delivers the same `(event_id, channel)` twice.
    ///
    /// # Errors
    ///
    /// Returns a run-state error if the delivery ledger cannot be read or written.
    pub async fn deliver(
        &self,
        message: &Message,
        run_state: &RunState,
        endpoint: &NotificationEndpoint,
        rate_limit: &Arc<RateLimit>,
        clock: &goodfetch_engine::clock::Clock,
    ) -> Result<Vec<(String, Entry)>, goodfetch_engine::run_state::RunStateError> {
        let subscriptions: Vec<Subscription> = self
            .list()
            .await
            .into_iter()
            .filter(|sub| sub.receives(&message.event))
            .collect();
        if subscriptions.is_empty() {
            return Ok(Vec::new());
        }
        let ledger = Ledger::new(run_state.clone());
        let driver = Driver::new(ledger, Arc::clone(rate_limit), *clock);
        let mut outcomes = Vec::with_capacity(subscriptions.len());
        for sub in subscriptions {
            let Ok(channel) = Webhook::new(
                sub.channel.clone(),
                sub.url.clone(),
                sub.signer(),
                Payload::Thin,
                endpoint.clone(),
                *clock,
            ) else {
                continue;
            };
            let entry = driver.deliver(&channel, message).await?;
            outcomes.push((sub.channel, entry));
        }
        Ok(outcomes)
    }
}

struct SubscriptionRunEndObserver {
    store: SubscriptionStore,
    rate_limit: OnceLock<Arc<RateLimit>>,
}

impl RunEndHandler for SubscriptionRunEndObserver {
    fn has_work(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { !self.store.is_empty().await })
    }

    fn on_run_end<'a>(
        &'a self,
        context: &'a RunEndedContext<'a>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if self.store.is_empty().await {
                return;
            }
            let report = context.report;
            let axes = RunAxes {
                run_id: report.run_id.clone(),
                lifecycle: report.lifecycle,
                verdict: Some(report.verdict.clone()),
                completeness: Some(report.completeness),
            };
            let events = NotificationEvent::of_run_end(&axes);
            if events.is_empty() {
                return;
            }
            let clock = context.engine.clock();
            let snapshot = context.engine.settings().snapshot().await;
            let resolved = match snapshot.resolved() {
                Ok(resolved) => resolved,
                Err(error) => {
                    tracing::warn!(%error, run_id = %report.run_id, "cannot resolve settings for notification delivery");
                    return;
                }
            };
            let endpoint =
                NotificationEndpoint::of(resolved, clock, context.engine.blocking_threads());
            let rate_limit = self
                .rate_limit
                .get_or_init(|| Arc::new(RateLimit::new(clock)));
            for event in events {
                let message = Message {
                    event_id: event_id_for_run_end(&report.run_id, &event),
                    event,
                    run_id: Some(report.run_id.clone()),
                    schedule_id: None,
                    product: context.product,
                    title: format!("Run {} {}", report.run_id.as_str(), event.as_str()),
                    text: format!(
                        "Run `{}` ended with lifecycle `{:?}` and verdict `{:?}`.",
                        report.run_id.as_str(),
                        report.lifecycle,
                        report.verdict
                    ),
                    link: None,
                    occurred_at: clock.wall(),
                };
                if let Err(error) = self
                    .store
                    .deliver(&message, context.state, &endpoint, rate_limit, &clock)
                    .await
                {
                    tracing::warn!(
                        %error,
                        run_id = %report.run_id,
                        event = event.as_str(),
                        "failed to record notification delivery for run end"
                    );
                }
            }
        })
    }
}

/// Deterministic [`EventId`] for a Run-end event so a resumed Run hits the
/// delivery ledger and never sends the same trigger twice (ADR 0027).
fn event_id_for_run_end(run_id: &RunId, event: &NotificationEvent) -> EventId {
    let sanitized: String = run_id
        .as_str()
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-') {
                char::from(byte)
            } else {
                '_'
            }
        })
        .collect();
    let candidate = format!("run_{sanitized}_{}", event.as_str());
    if candidate.len() <= EVENT_ID_MAX
        && let Ok(id) = EventId::new(candidate)
    {
        return id;
    }
    let mut hasher = Sha256::new();
    hasher.update(run_id.as_str().as_bytes());
    hasher.update(b":");
    hasher.update(event.as_str().as_bytes());
    let digest = data_encoding::HEXLOWER.encode(&hasher.finalize()[..16]);
    EventId::new(format!("run_{digest}_{}", event.as_str()))
        .expect("invariant: hashed run-end event ID fits EVENT_ID_MAX")
}

/// What the notification crate adds to an engine: the delivery ledger's run
/// file table, the `notification.subscribe` / `notification.unsubscribe`
/// handlers, and the run-end delivery observer (ADRs 0027 and 0129; issue 1403).
#[must_use]
pub fn contribution() -> Contribution {
    contribution_with_store(SubscriptionStore::new())
}

/// Same as [`contribution`], sharing an externally-held [`SubscriptionStore`].
#[must_use]
pub fn contribution_with_store(store: SubscriptionStore) -> Contribution {
    let sub_store = store.clone();
    let unsub_store = store.clone();
    let observer = RunEndObserver::new(SubscriptionRunEndObserver {
        store,
        rate_limit: OnceLock::new(),
    });
    Contribution::new("notification")
        .with_run_state_tables(crate::ledger::TABLES)
        .with_run_end_observer(observer)
        .with_handler(Handler::new(
            &NOTIFICATION_SUBSCRIBE,
            move |params, context| subscribe(sub_store.clone(), params, context),
        ))
        .with_handler(Handler::new(
            &NOTIFICATION_UNSUBSCRIBE,
            move |params, context| unsubscribe(unsub_store.clone(), params, context),
        ))
}

fn check_allowed_surface(context: &CallContext) -> Result<(), CallError> {
    match context.surface() {
        Surface::Mcp | Surface::Worker => Err(CallError::protocol(
            ProtocolFailure::MethodNotFound,
            "notification subscriptions are kept off agent surfaces",
        )),
        Surface::HttpListener
        | Surface::Cli
        | Surface::SdkTypescript
        | Surface::SdkPython
        | Surface::Rust
        | Surface::Kennel
        | Surface::Cloud
        | Surface::BrowserHost => Ok(()),
    }
}

async fn subscribe(
    store: SubscriptionStore,
    params: NotificationSubscribeParams,
    context: CallContext,
) -> Result<NotificationSubscribeResult, CallError> {
    check_allowed_surface(&context)?;
    let channel = params.channel.trim().to_owned();
    if channel.is_empty() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "notification.subscribe requires a non-empty `channel`",
        ));
    }
    let url = Url::parse(&params.url).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("invalid webhook URL `{}`: {error}", params.url),
        )
    })?;
    if url.host_str().is_none_or(str::is_empty) {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "a webhook URL must include a host",
        ));
    }

    let engine = context.engine();
    let snapshot = engine.settings().snapshot().await;
    let resolved = snapshot
        .resolved()
        .map_err(|error| CallError::protocol(ProtocolFailure::InternalError, error.to_string()))?;
    let endpoint = NotificationEndpoint::of(resolved, engine.clock(), engine.blocking_threads());
    endpoint.judge(&url).map_err(|refused| refused.error())?;

    let mut guard = store.inner.lock().await;
    if let Some(existing) = guard.get(&channel)
        && existing.url == url
        && existing.events == params.events
    {
        return Ok(NotificationSubscribeResult {
            channel,
            created: false,
            secret: None,
        });
    }
    if !guard.contains_key(&channel) && guard.len() >= store.max_subscriptions {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidRequest,
            format!(
                "notification subscription limit ({}) reached; unsubscribe an unused channel first",
                store.max_subscriptions
            ),
        ));
    }

    // Generate a 32-byte Standard Webhooks `whsec_` signing secret from the OS
    // cryptographic generator (entropy.rs: keys and tokens never come from
    // `Entropy`) and return it once.
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("OS randomness unavailable for webhook signing secret: {error}"),
        )
    })?;
    let secret_text = format!("whsec_{}", BASE64.encode(&raw));
    let secret = SecretString::from(secret_text.clone());
    // Validate invariant at construction so `Subscription::signer()` cannot fail.
    let _ = SigningKey::from_secret(&secret)
        .expect("invariant: 32-byte whsec_ key generated by the engine is valid");

    guard.insert(
        channel.clone(),
        Subscription {
            channel: channel.clone(),
            url,
            events: params.events,
            secret,
        },
    );

    Ok(NotificationSubscribeResult {
        channel,
        created: true,
        secret: Some(secret_text),
    })
}

async fn unsubscribe(
    store: SubscriptionStore,
    params: NotificationUnsubscribeParams,
    context: CallContext,
) -> Result<NotificationUnsubscribeResult, CallError> {
    check_allowed_surface(&context)?;
    let channel = params.channel.trim().to_owned();
    if channel.is_empty() {
        return Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            "notification.unsubscribe requires a non-empty `channel`",
        ));
    }
    let removed = store.inner.lock().await.remove(&channel).is_some();
    Ok(NotificationUnsubscribeResult { channel, removed })
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::notifications::NotificationEvent;

    use super::*;

    /// Covers: story:11-255
    #[test]
    fn subscription_receives_selected_events_or_attention_set() {
        let secret = SecretString::from(format!("whsec_{}", BASE64.encode(&[7u8; 32])));
        let url = Url::parse("https://hooks.example.com/trigger").expect("valid url");

        let default_sub = Subscription {
            channel: "n8n-default".to_owned(),
            url: url.clone(),
            events: None,
            secret: secret.clone(),
        };
        assert!(default_sub.receives(&NotificationEvent::RUN_FAILED));
        assert!(!default_sub.receives(&NotificationEvent::RUN_STARTED));
        let _ = default_sub.signer();

        let custom_sub = Subscription {
            channel: "n8n-custom".to_owned(),
            url,
            events: Some(vec![NotificationEvent::RUN_FINISHED]),
            secret,
        };
        assert!(custom_sub.receives(&NotificationEvent::RUN_FINISHED));
        assert!(!custom_sub.receives(&NotificationEvent::RUN_FAILED));
    }
}
