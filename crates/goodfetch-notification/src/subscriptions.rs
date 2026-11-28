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
use std::sync::Arc;

use data_encoding::BASE64;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::run_state::RunState;
use goodfetch_engine::webhook_signing::{Signer, SigningKey};
use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::notifications::{
    NOTIFICATION_SUBSCRIBE, NOTIFICATION_UNSUBSCRIBE, NotificationEvent,
    NotificationSubscribeParams, NotificationSubscribeResult, NotificationUnsubscribeParams,
    NotificationUnsubscribeResult,
};
use secrecy::SecretString;
use tokio::sync::Mutex;
use url::Url;

use crate::driver::Driver;
use crate::ledger::{Entry, Ledger};
use crate::message::Message;
use crate::rate_limit::RateLimit;
use crate::webhook::{Payload, Webhook};

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
#[derive(Clone, Default, Debug)]
pub struct SubscriptionStore {
    inner: Arc<Mutex<BTreeMap<String, Subscription>>>,
}

impl SubscriptionStore {
    /// Creates an empty subscription store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

/// What the notification crate adds to an engine: the delivery ledger's run
/// file table and the `notification.subscribe` / `notification.unsubscribe`
/// handlers (ADRs 0027 and 0129; issue 1403).
#[must_use]
pub fn contribution() -> Contribution {
    contribution_with_store(SubscriptionStore::new())
}

/// Same as [`contribution`], sharing an externally-held [`SubscriptionStore`].
#[must_use]
pub fn contribution_with_store(store: SubscriptionStore) -> Contribution {
    let sub_store = store.clone();
    let unsub_store = store;
    Contribution::new("notification")
        .with_run_state_tables(crate::ledger::TABLES)
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

    // Generate a 32-byte Standard Webhooks `whsec_` signing secret from the
    // engine's entropy stream and return it once.
    let mut raw = [0u8; 32];
    for chunk in raw.as_chunks_mut::<8>().0 {
        chunk.copy_from_slice(&engine.draw_u64().to_le_bytes());
    }
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
