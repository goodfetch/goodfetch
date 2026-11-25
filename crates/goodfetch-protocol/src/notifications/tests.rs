use rstest::rstest;
use serde_json::{Value, json};

use super::{CHANNEL_SHAPE, ChannelSettings, WebhookPayload};

/// Covers: story:11-262, story:11-263, setting:notifications.channels
#[test]
fn a_webhook_channel_reads_its_url_and_signing_secret_as_secret_references() {
    let entry = json!({
        "type": "webhook",
        "url": "env:HOOK_URL",
        "events": ["run_failed"],
        "runs": "tagged:nightly",
        "webhook": { "secret": "env:HOOK_SECRET", "payload": "thin" },
    });

    (CHANNEL_SHAPE.check)(&entry).unwrap();
    let read = ChannelSettings::read(&entry).unwrap();

    let webhook = read.webhook.unwrap();
    assert_eq!(
        (
            read.channel_type.as_deref(),
            read.url.as_deref(),
            read.runs.as_deref(),
            webhook.secret.as_deref(),
            webhook.payload,
        ),
        (
            Some("webhook"),
            Some("env:HOOK_URL"),
            Some("tagged:nightly"),
            Some("env:HOOK_SECRET"),
            Some(WebhookPayload::Thin),
        )
    );
}

/// A URL and every credential is a Secret reference, never the value
/// itself, and the refusal never repeats what it was given (ADR 0015).
///
/// Covers: story:11-263, setting:notifications.channels
#[rstest]
#[case::url(json!({ "type": "slack", "url": "https://hooks.slack.example/hunter2" }), "url")]
#[case::webhook_secret(
    json!({ "type": "webhook", "url": "env:HOOK_URL", "webhook": { "secret": "whsec_hunter2" } }),
    "webhook.secret"
)]
#[case::email_api_key(
    json!({ "type": "email", "email": { "provider": "resend", "api_key": "re_hunter2" } }),
    "email.api_key"
)]
#[case::smtp_username(
    json!({ "type": "email", "email": { "smtp": { "username": "hunter2@example.com" } } }),
    "email.smtp.username"
)]
#[case::smtp_password(
    json!({ "type": "email", "email": { "smtp": { "password": "hunter2" } } }),
    "email.smtp.password"
)]
#[case::telegram_bot_token(
    json!({ "type": "telegram", "telegram": { "chat_id": "1", "bot_token": "123:hunter2" } }),
    "telegram.bot_token"
)]
fn a_value_in_a_secret_references_place_is_refused_without_repeating_it(
    #[case] entry: Value,
    #[case] key: &str,
) {
    let refused = (CHANNEL_SHAPE.check)(&entry).unwrap_err();

    assert!(refused.contains(&format!("`{key}`")), "{refused}");
    assert!(!refused.contains("hunter2"), "{refused}");
}

/// Covers: story:11-262, story:11-263, setting:notifications.channels
#[rstest]
#[case::webhook_without_url(json!({ "type": "webhook", "webhook": { "secret": "env:S" } }))]
#[case::webhook_without_secret(json!({ "type": "webhook", "url": "env:U" }))]
#[case::empty_type(json!({ "type": "" }))]
#[case::runs_neither_all_nor_tagged(json!({ "type": "slack", "url": "env:U", "runs": "some" }))]
#[case::runs_tagged_without_tag(json!({ "type": "slack", "url": "env:U", "runs": "tagged:" }))]
#[case::unknown_key(json!({ "type": "slack", "url": "env:U", "channel": "#ops" }))]
#[case::unknown_payload(
    json!({ "type": "webhook", "url": "env:U", "webhook": { "secret": "env:S", "payload": "batch" } })
)]
#[case::unknown_email_provider(json!({ "type": "email", "email": { "provider": "sendmail" } }))]
fn an_entry_outside_the_shape_is_refused(#[case] entry: Value) {
    assert!((CHANNEL_SHAPE.check)(&entry).is_err(), "{entry}");
}

/// Covers: setting:notifications.channels
#[test]
fn each_example_is_a_channel_entry() {
    for example in (CHANNEL_SHAPE.examples)() {
        (CHANNEL_SHAPE.check)(&example).unwrap();
    }
}
