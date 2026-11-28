//! Engine-side Notification dispatcher (ADRs 0027, 0041, 0087 and 0129):
//! reads `notifications.channels` from the engine's resolved machine settings,
//! resolves each channel's Secret references through the engine's driver, and
//! delivers [`EngineNotification`]s through the configured built-in channels.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_engine::notification::{EngineNotification, NotificationDispatcher};
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::secrets::{Purpose, RunSecrets};
use goodfetch_engine::webhook_signing::{Signer, SigningKey};
use goodfetch_engine::{Contribution, EngineContext};
use goodfetch_protocol::notifications::{
    ChannelEmail, ChannelSettings, ChannelTarget, NOTIFICATIONS_CHANNELS, WebhookPayload,
};
use goodfetch_protocol::secret_reference::SecretReference;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::chat::Chat;
use crate::driver::Channel as _;
use crate::email::{Address, Email, Login, Mailgun, Postmark, Provider, Resend, Sendgrid, Smtp};
use crate::message::{EventId, Message};
use crate::rate_limit::RateLimit;
use crate::webhook::Webhook;

/// What `goodfetch-notification` contributes to an engine composition: its
/// [`NotificationDispatcher`], wired into Collie and Foxhound from the
/// composition root (ADR 0087).
#[must_use]
pub fn contribution() -> Contribution {
    Contribution::new("goodfetch-notification")
        .with_notification_dispatcher(Arc::new(Dispatcher::new()))
}

struct Dispatcher {
    rate_limit: std::sync::OnceLock<Arc<RateLimit>>,
}

impl Dispatcher {
    fn new() -> Self {
        Self {
            rate_limit: std::sync::OnceLock::new(),
        }
    }
}

impl NotificationDispatcher for Dispatcher {
    fn dispatch(
        &self,
        context: EngineContext,
        notification: EngineNotification,
    ) -> Pin<Box<dyn Future<Output = bool> + Send>> {
        let rate_limit = Arc::clone(
            self.rate_limit
                .get_or_init(|| Arc::new(RateLimit::new(context.clock()))),
        );
        Box::pin(async move { deliver_to_channels(&context, &rate_limit, notification).await })
    }
}

async fn deliver_to_channels(
    context: &EngineContext,
    rate_limit: &RateLimit,
    notification: EngineNotification,
) -> bool {
    let snapshot = context.settings().snapshot().await;
    let Ok(resolved) = snapshot.resolved() else {
        tracing::warn!("the machine's settings could not be resolved for Notification delivery");
        return false;
    };
    let Some(channels) = resolved.value(&NOTIFICATIONS_CHANNELS).as_object() else {
        return true;
    };
    if channels.is_empty() {
        return true;
    }
    let Ok(event_id) = EventId::new(notification.event_id) else {
        tracing::warn!("an engine Notification carried an invalid event ID");
        return false;
    };
    let link = notification
        .link
        .as_deref()
        .and_then(|raw| Url::parse(raw).ok());
    let message = Message {
        event_id,
        event: notification.event,
        run_id: notification.run_id,
        schedule_id: notification.schedule_id,
        product: notification.product,
        title: notification.title,
        text: notification.text,
        link,
        occurred_at: notification.occurred_at,
    };
    let endpoint = NotificationEndpoint::of(resolved, context.clock(), context.blocking_threads());
    let Some(secrets) = context.secret_driver().endpoint_keys(resolved) else {
        tracing::warn!("the Secret driver is not ready for Notification delivery");
        return false;
    };
    let mut all_ok = true;
    for (name, entry) in channels {
        let Ok(settings) = ChannelSettings::read(entry) else {
            tracing::warn!(
                channel = name,
                "skipping unreadable Notification channel entry"
            );
            continue;
        };
        if !settings.receives(message.event) {
            continue;
        }
        if message.run_id.is_none() && settings.runs.as_deref().is_some_and(|runs| runs != "all") {
            continue;
        }
        if !send_channel(
            name,
            &settings,
            &message,
            &endpoint,
            &secrets,
            rate_limit,
            context.clock(),
        )
        .await
        {
            all_ok = false;
        }
    }
    all_ok
}

async fn resolve_secret(secrets: &RunSecrets, key: &str, raw_ref: &str) -> Option<SecretString> {
    let reference = SecretReference::parse(raw_ref).ok()?;
    let purpose = Purpose {
        placeholder: key,
        domain: None,
        domains: &[],
        layer: None,
        run_id: None,
    };
    match secrets.resolve(&reference, &purpose).await {
        Ok(secret) => Some(secret),
        Err(error) => {
            tracing::warn!(
                key,
                %error,
                "could not resolve Secret reference for Notification channel"
            );
            None
        }
    }
}

async fn send_channel(
    name: &str,
    settings: &ChannelSettings,
    message: &Message,
    endpoint: &NotificationEndpoint,
    secrets: &RunSecrets,
    rate_limit: &RateLimit,
    clock: goodfetch_engine::clock::Clock,
) -> bool {
    match settings.target() {
        Some(ChannelTarget::Webhook {
            url,
            secret,
            payload,
        }) => {
            send_webhook(
                name, url, secret, payload, message, endpoint, secrets, rate_limit, clock,
            )
            .await
        }
        Some(ChannelTarget::Telegram { chat_id, bot_token }) => {
            send_telegram(
                name, chat_id, bot_token, message, endpoint, secrets, rate_limit, clock,
            )
            .await
        }
        Some(ChannelTarget::Slack { url }) => {
            send_webhook_chat(
                "slack", name, url, message, endpoint, secrets, rate_limit, clock,
            )
            .await
        }
        Some(ChannelTarget::Teams { url }) => {
            send_webhook_chat(
                "teams", name, url, message, endpoint, secrets, rate_limit, clock,
            )
            .await
        }
        Some(ChannelTarget::Discord { url }) => {
            send_webhook_chat(
                "discord", name, url, message, endpoint, secrets, rate_limit, clock,
            )
            .await
        }
        Some(ChannelTarget::Email(email)) => {
            send_email(name, email, message, endpoint, secrets, rate_limit, clock).await
        }
        Some(ChannelTarget::Incomplete) => false,
        Some(ChannelTarget::Other(other)) => {
            tracing::warn!(
                channel = name,
                channel_type = other,
                "Notification channel type is not wired for scope-level Notifications yet"
            );
            true
        }
        None => true,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "sends a single webhook notification with engine endpoint, secrets, rate limit and clock"
)]
async fn send_webhook(
    name: &str,
    url_ref: &str,
    secret_ref: &str,
    payload: WebhookPayload,
    message: &Message,
    endpoint: &NotificationEndpoint,
    secrets: &RunSecrets,
    rate_limit: &RateLimit,
    clock: goodfetch_engine::clock::Clock,
) -> bool {
    let Some(url_secret) = resolve_secret(
        secrets,
        &format!("notifications.channels.{name}.url"),
        url_ref,
    )
    .await
    else {
        return false;
    };
    let Ok(url) = Url::parse(url_secret.expose_secret()) else {
        tracing::warn!(channel = name, "webhook channel URL is invalid");
        return false;
    };
    let Some(signing_secret) = resolve_secret(
        secrets,
        &format!("notifications.channels.{name}.webhook.secret"),
        secret_ref,
    )
    .await
    else {
        return false;
    };
    let Ok(key) = SigningKey::from_secret(&signing_secret) else {
        tracing::warn!(channel = name, "webhook signing key is invalid");
        return false;
    };
    let Ok(signer) = Signer::new(vec![key], None) else {
        tracing::warn!(channel = name, "webhook signer could not be built");
        return false;
    };
    let Ok(channel) = Webhook::new(name, url, signer, payload, endpoint.clone(), clock) else {
        tracing::warn!(channel = name, "webhook channel could not be constructed");
        return false;
    };
    rate_limit.wait_turn(channel.destination()).await;
    match channel.send(message).await {
        Ok(()) => true,
        Err(failure) => {
            let mapped = channel.map_failure(failure);
            tracing::warn!(
                channel = name,
                event = message.event_id.as_str(),
                failure = mapped.name(),
                "a webhook Notification was not delivered"
            );
            false
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "sends a single Telegram notification with engine endpoint, secrets, rate limit and clock"
)]
async fn send_telegram(
    name: &str,
    chat_id: &str,
    token_ref: &str,
    message: &Message,
    endpoint: &NotificationEndpoint,
    secrets: &RunSecrets,
    rate_limit: &RateLimit,
    clock: goodfetch_engine::clock::Clock,
) -> bool {
    let Some(token) = resolve_secret(
        secrets,
        &format!("notifications.channels.{name}.telegram.bot_token"),
        token_ref,
    )
    .await
    else {
        return false;
    };
    let Ok(channel) = Chat::telegram(name, chat_id, &token, None, endpoint.clone(), clock) else {
        tracing::warn!(channel = name, "chat channel could not be constructed");
        return false;
    };
    deliver_chat(name, &channel, message, rate_limit).await
}

#[expect(
    clippy::too_many_arguments,
    reason = "sends a single chat channel notification with engine endpoint, secrets, rate limit and clock"
)]
async fn send_webhook_chat(
    kind: &str,
    name: &str,
    url_ref: &str,
    message: &Message,
    endpoint: &NotificationEndpoint,
    secrets: &RunSecrets,
    rate_limit: &RateLimit,
    clock: goodfetch_engine::clock::Clock,
) -> bool {
    let Some(url_secret) = resolve_secret(
        secrets,
        &format!("notifications.channels.{name}.url"),
        url_ref,
    )
    .await
    else {
        return false;
    };
    let Ok(url) = Url::parse(url_secret.expose_secret()) else {
        tracing::warn!(channel = name, "chat channel URL is invalid");
        return false;
    };
    let channel = match kind {
        "slack" => Chat::slack(name, &url, endpoint.clone(), clock),
        "teams" => Chat::teams(name, &url, endpoint.clone(), clock),
        "discord" => Chat::discord(name, &url, endpoint.clone(), clock),
        _ => return false,
    };
    let Ok(channel) = channel else {
        tracing::warn!(channel = name, "chat channel could not be constructed");
        return false;
    };
    deliver_chat(name, &channel, message, rate_limit).await
}

async fn deliver_chat(
    name: &str,
    channel: &Chat,
    message: &Message,
    rate_limit: &RateLimit,
) -> bool {
    rate_limit.wait_turn(channel.destination()).await;
    match channel.send(message).await {
        Ok(()) => true,
        Err(failure) => {
            let mapped = channel.map_failure(failure);
            tracing::warn!(
                channel = name,
                event = message.event_id.as_str(),
                failure = mapped.name(),
                "a chat Notification was not delivered"
            );
            false
        }
    }
}

enum BuiltEmailProvider {
    Ready(Box<Provider>),
    Skipped,
    Invalid,
}

async fn build_email_provider(
    name: &str,
    provider_kind: goodfetch_protocol::notifications::EmailProvider,
    email: &goodfetch_protocol::notifications::ChannelEmail,
    endpoint: &NotificationEndpoint,
    secrets: &RunSecrets,
) -> BuiltEmailProvider {
    match provider_kind {
        goodfetch_protocol::notifications::EmailProvider::Smtp => {
            let Some(smtp) = email.smtp.as_ref() else {
                return BuiltEmailProvider::Invalid;
            };
            let Some(host) = smtp.host.as_deref() else {
                return BuiltEmailProvider::Invalid;
            };
            let security = smtp
                .security
                .unwrap_or(goodfetch_protocol::notifications::SmtpSecurity::Starttls);
            let login = match (smtp.username.as_deref(), smtp.password.as_deref()) {
                (Some(user_ref), Some(pass_ref)) => {
                    let (Some(username), Some(password)) = (
                        resolve_secret(
                            secrets,
                            &format!("notifications.channels.{name}.email.smtp.username"),
                            user_ref,
                        )
                        .await,
                        resolve_secret(
                            secrets,
                            &format!("notifications.channels.{name}.email.smtp.password"),
                            pass_ref,
                        )
                        .await,
                    ) else {
                        return BuiltEmailProvider::Invalid;
                    };
                    Some(Login { username, password })
                }
                _ => None,
            };
            BuiltEmailProvider::Ready(Box::new(Provider::Smtp(Smtp::new(
                host,
                smtp.port,
                security,
                login,
                endpoint.clone(),
            ))))
        }
        goodfetch_protocol::notifications::EmailProvider::Resend => {
            let Some(key) = resolve_email_api_key(name, email.api_key.as_deref(), secrets).await
            else {
                return BuiltEmailProvider::Invalid;
            };
            Resend::new(&key, None, endpoint.clone()).map_or(BuiltEmailProvider::Invalid, |p| {
                BuiltEmailProvider::Ready(Box::new(Provider::Resend(p)))
            })
        }
        goodfetch_protocol::notifications::EmailProvider::Postmark => {
            let Some(key) = resolve_email_api_key(name, email.api_key.as_deref(), secrets).await
            else {
                return BuiltEmailProvider::Invalid;
            };
            Postmark::new(&key, None, endpoint.clone()).map_or(BuiltEmailProvider::Invalid, |p| {
                BuiltEmailProvider::Ready(Box::new(Provider::Postmark(p)))
            })
        }
        goodfetch_protocol::notifications::EmailProvider::Sendgrid => {
            let Some(key) = resolve_email_api_key(name, email.api_key.as_deref(), secrets).await
            else {
                return BuiltEmailProvider::Invalid;
            };
            Sendgrid::new(&key, None, endpoint.clone()).map_or(BuiltEmailProvider::Invalid, |p| {
                BuiltEmailProvider::Ready(Box::new(Provider::Sendgrid(p)))
            })
        }
        goodfetch_protocol::notifications::EmailProvider::Mailgun => {
            let Some(mailgun) = email.mailgun.as_ref() else {
                return BuiltEmailProvider::Invalid;
            };
            let Some(domain) = mailgun.domain.as_deref() else {
                return BuiltEmailProvider::Invalid;
            };
            let region = mailgun
                .region
                .unwrap_or(goodfetch_protocol::notifications::MailgunRegion::Us);
            let Some(key) = resolve_email_api_key(name, email.api_key.as_deref(), secrets).await
            else {
                return BuiltEmailProvider::Invalid;
            };
            Mailgun::new(&key, domain, region, None, endpoint.clone())
                .map_or(BuiltEmailProvider::Invalid, |p| {
                    BuiltEmailProvider::Ready(Box::new(Provider::Mailgun(p)))
                })
        }
        goodfetch_protocol::notifications::EmailProvider::Ses => {
            tracing::warn!(
                channel = name,
                "SES email channel awaits AWS credential settings (#3955)"
            );
            BuiltEmailProvider::Skipped
        }
    }
}

async fn send_email(
    name: &str,
    email: &ChannelEmail,
    message: &Message,
    endpoint: &NotificationEndpoint,
    secrets: &RunSecrets,
    rate_limit: &RateLimit,
    clock: goodfetch_engine::clock::Clock,
) -> bool {
    let (Some(provider_kind), Some(from_raw), Some(to_raw)) =
        (email.provider, email.from.as_deref(), email.to.as_ref())
    else {
        return false;
    };
    let Ok(from) = Address::parse(from_raw) else {
        return false;
    };
    let Ok(to): Result<Vec<Address>, _> = to_raw.iter().map(|addr| Address::parse(addr)).collect()
    else {
        return false;
    };
    let provider = match build_email_provider(name, provider_kind, email, endpoint, secrets).await {
        BuiltEmailProvider::Ready(provider) => *provider,
        BuiltEmailProvider::Skipped => return true,
        BuiltEmailProvider::Invalid => return false,
    };
    let Ok(channel) = Email::new(name, from, to, provider, clock) else {
        return false;
    };
    rate_limit.wait_turn(channel.destination()).await;
    match channel.send(message).await {
        Ok(()) => true,
        Err(failure) => {
            let mapped = channel.map_failure(failure);
            tracing::warn!(
                channel = name,
                event = message.event_id.as_str(),
                failure = mapped.name(),
                "an email Notification was not delivered"
            );
            false
        }
    }
}

async fn resolve_email_api_key(
    name: &str,
    api_key: Option<&str>,
    secrets: &RunSecrets,
) -> Option<SecretString> {
    resolve_secret(
        secrets,
        &format!("notifications.channels.{name}.email.api_key"),
        api_key?,
    )
    .await
}
