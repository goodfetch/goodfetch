//! The chat channels (issue 1387): Slack, Microsoft Teams, Discord and
//! Telegram, each posting to an `httpmock` stand-in of its platform: one
//! JSON `POST` per Notification, its body snapshotted, its link without
//! credentials; a destination the egress policy refuses, or that is no
//! destination of the platform, failing the configuration rather than a
//! delivery; and each platform's translation table, one case per failure
//! it knows and one it does not. No real account is used.

use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_notification::api::ApiUnusable;
use goodfetch_notification::chat::Chat;
use goodfetch_notification::driver::Channel as _;
use goodfetch_protocol::egress::EgressRule;
use httpmock::MockServer;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};
use url::Url;

use crate::email::{
    LINK_PASSWORD, Posted, endpoint, message, runtime, settings, stand_in, summary,
};

/// The Telegram bot's token, which only the API's path carries.
const BOT_TOKEN: &str = "123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11";

/// The Telegram chat the channel posts to: a supergroup's ID.
const CHAT_ID: &str = "-1001234567890";

/// The chat platforms.
#[derive(Clone, Copy, Debug)]
enum Platform {
    Slack,
    Teams,
    Discord,
    Telegram,
}

impl Platform {
    /// The path the platform's webhook, or bot's API, is posted at.
    fn path(self) -> String {
        match self {
            Self::Slack => "/services/T0001/B0001/XXXXXXXXXXXXXXXXXXXXXXXX".to_owned(),
            Self::Teams => "/workflows/0a1b2c/triggers/manual/paths/invoke".to_owned(),
            Self::Discord => "/api/webhooks/1234567890/webhook-token".to_owned(),
            Self::Telegram => format!("/bot{BOT_TOKEN}/sendMessage"),
        }
    }
}

/// The channel `ops` posting to `platform`'s stand-in `server` under the
/// Operator policy `policy`.
fn channel_under(platform: Platform, server: &MockServer, policy: Value) -> Chat {
    let clock = Clock::system();
    let endpoint = endpoint(&settings(policy, json!({})), clock);
    let base = Url::parse(&server.base_url()).unwrap();
    let url = base.join(&platform.path()).unwrap();
    match platform {
        Platform::Slack => Chat::slack("ops", &url, endpoint, clock),
        Platform::Teams => Chat::teams("ops", &url, endpoint, clock),
        Platform::Discord => Chat::discord("ops", &url, endpoint, clock),
        Platform::Telegram => Chat::telegram(
            "ops",
            CHAT_ID,
            &SecretString::from(BOT_TOKEN.to_owned()),
            Some(base),
            endpoint,
            clock,
        ),
    }
    .unwrap()
}

/// The channel `ops` posting to `platform`'s stand-in `server`.
fn channel(platform: Platform, server: &MockServer) -> Chat {
    channel_under(platform, server, json!({}))
}

/// The one request `posted` holds.
fn only(posted: &[Posted]) -> Posted {
    let [request] = posted else {
        panic!("one request, got {posted:?}")
    };
    request.clone()
}

/// The failure `platform`'s stand-in answering `status`, with `headers`
/// and `body`, maps to.
fn answered(
    platform: Platform,
    status: u16,
    headers: &[(&str, &str)],
    body: &str,
) -> (&'static str, bool, Option<Duration>) {
    let (server, _) = stand_in(status, headers, body);
    let runtime = runtime();
    let channel = channel(platform, &server);
    let failure = runtime.block_on(channel.send(&message())).unwrap_err();
    summary(&channel.map_failure(failure))
}

/// Each platform's message is one JSON `POST` to its webhook, or its bot's
/// `sendMessage`, carrying the title, the text, the Product, the event and
/// the link without its credentials.
/// Covers: setting:notifications.channels
#[rstest]
#[case::slack(Platform::Slack, "slack_body")]
#[case::teams(Platform::Teams, "teams_body")]
#[case::discord(Platform::Discord, "discord_body")]
#[case::telegram(Platform::Telegram, "telegram_body")]
fn each_platform_gets_one_json_post(#[case] platform: Platform, #[case] snapshot: &str) {
    let (server, posted) = stand_in(200, &[], r#"{"ok":true}"#);
    let runtime = runtime();
    let channel = channel(platform, &server);

    runtime.block_on(channel.send(&message())).unwrap();

    let request = only(&posted.lock().unwrap());
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, platform.path());
    assert_eq!(request.header("content-type"), Some("application/json"));
    let text = String::from_utf8(request.body.clone()).unwrap();
    assert!(
        !text.contains(LINK_PASSWORD),
        "the body carries the link's password: {text}"
    );
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    insta::assert_snapshot!(snapshot, serde_json::to_string_pretty(&body).unwrap());
}

/// No channel's `Debug` carries its URL, which holds the destination's
/// credential: a Telegram bot's token, or a webhook's secret path.
/// Covers: setting:notifications.channels
#[rstest]
#[case::slack(Platform::Slack)]
#[case::telegram(Platform::Telegram)]
fn a_channel_formats_no_credential(#[case] platform: Platform) {
    let server = MockServer::start();
    let channel = channel(platform, &server);

    let formatted = format!("{channel:?}");

    assert!(!formatted.contains("XXXXXXXX"), "{formatted}");
    assert!(!formatted.contains(BOT_TOKEN), "{formatted}");
    assert!(formatted.contains("127.0.0.1"), "{formatted}");
}

/// A Slack webhook the egress policy refuses fails the channel's
/// configuration, before any delivery, naming the rule: a private address
/// under `egress.no_private_endpoints`, or a scheme other than HTTP's.
/// Covers: setting:notifications.channels, setting:egress.no_private_endpoints
#[test]
fn a_refused_slack_webhook_fails_the_configuration() {
    let clock = Clock::system();
    let refusing = endpoint(
        &settings(
            json!({ "egress": { "no_private_endpoints": true } }),
            json!({}),
        ),
        clock,
    );
    let private = Url::parse("http://127.0.0.1:9/services/T0001/B0001/XXXX").unwrap();

    let refused = Chat::slack("ops", &private, refusing, clock).unwrap_err();

    assert_eq!(refused, ApiUnusable::Refused(EgressRule::PrivateAddress));

    let reachable = endpoint(&settings(json!({}), json!({})), clock);
    let ftp = Url::parse("ftp://hooks.slack.com/services/T0001/B0001/XXXX").unwrap();
    assert_eq!(
        Chat::slack("ops", &ftp, reachable, clock).unwrap_err(),
        ApiUnusable::Url
    );
}

/// An Office 365 connector's URL, which takes no Adaptive Card and which
/// Microsoft retired, is no `teams` destination: the configuration fails.
/// Covers: setting:notifications.channels
#[rstest]
#[case::outlook("https://outlook.office.com/webhook/0a1b2c")]
#[case::webhook_office("https://contoso.webhook.office.com/webhookb2/0a1b2c")]
fn a_teams_connector_url_fails_the_configuration(#[case] url: &str) {
    let clock = Clock::system();
    let endpoint = endpoint(&settings(json!({}), json!({})), clock);

    let refused = Chat::teams("ops", &Url::parse(url).unwrap(), endpoint, clock).unwrap_err();

    assert_eq!(refused, ApiUnusable::TeamsConnector);
}

/// A Telegram chat that is no number nor `@username`, or a token that is
/// no bot's, fails the configuration.
/// Covers: setting:notifications.channels
#[rstest]
#[case::empty_chat("", BOT_TOKEN, ApiUnusable::ChatId)]
#[case::short_username("@ops", BOT_TOKEN, ApiUnusable::ChatId)]
#[case::spaced_chat("ops chat", BOT_TOKEN, ApiUnusable::ChatId)]
#[case::bare_dash("-", BOT_TOKEN, ApiUnusable::ChatId)]
#[case::token_without_colon(CHAT_ID, "123456ABC", ApiUnusable::Key)]
#[case::token_with_slash(CHAT_ID, "123456:ABC/../x", ApiUnusable::Key)]
fn an_unusable_telegram_chat_or_token_fails_the_configuration(
    #[case] chat_id: &str,
    #[case] token: &str,
    #[case] expected: ApiUnusable,
) {
    let clock = Clock::system();
    let endpoint = endpoint(&settings(json!({}), json!({})), clock);

    let refused = Chat::telegram(
        "ops",
        chat_id,
        &SecretString::from(token.to_owned()),
        None,
        endpoint,
        clock,
    )
    .unwrap_err();

    assert_eq!(refused, expected);
}

/// A public chat's `@username` and a user's positive ID are chats too.
/// Covers: setting:notifications.channels
#[rstest]
#[case::username("@goodfetch_ops")]
#[case::user("123456789")]
fn a_telegram_username_or_user_is_a_chat(#[case] chat_id: &str) {
    let clock = Clock::system();
    let endpoint = endpoint(&settings(json!({}), json!({})), clock);
    let token = SecretString::from(BOT_TOKEN.to_owned());

    assert!(Chat::telegram("ops", chat_id, &token, None, endpoint, clock).is_ok());
}

/// Slack's translation table: each error name its webhook answers with,
/// else its status, maps to its failure; an answer it does not know is
/// `unrecognized`.
/// Covers: setting:notifications.channels
#[rstest]
#[case::invalid_payload(400, &[], "invalid_payload", ("rejected", false, None))]
#[case::no_text(400, &[], "no_text", ("rejected", false, None))]
#[case::too_many_attachments(400, &[], "too_many_attachments", ("rejected", false, None))]
#[case::general_channel(403, &[], "posting_to_general_channel_denied", ("rejected", false, None))]
#[case::channel_not_found(404, &[], "channel_not_found", ("gone", false, None))]
#[case::channel_is_archived(410, &[], "channel_is_archived", ("gone", false, None))]
#[case::user_not_found(404, &[], "user_not_found", ("gone", false, None))]
#[case::no_service(404, &[], "no_service", ("gone", false, None))]
#[case::no_service_id(400, &[], "no_service_id", ("gone", false, None))]
#[case::no_team(400, &[], "no_team", ("gone", false, None))]
#[case::team_disabled(404, &[], "team_disabled", ("gone", false, None))]
#[case::invalid_token(403, &[], "invalid_token", ("unauthorized", false, None))]
#[case::action_prohibited(403, &[], "action_prohibited", ("unauthorized", false, None))]
#[case::rollup_error(500, &[], "rollup_error", ("server_error", true, None))]
#[case::rate_limited(429, &[("retry-after", "30")], "", ("rate_limited", true, Some(Duration::from_secs(30))))]
#[case::forbidden(403, &[], "", ("unauthorized", false, None))]
#[case::unavailable(503, &[], "", ("server_error", true, None))]
#[case::request_timeout(408, &[], "", ("timeout", true, None))]
#[case::unknown_name(400, &[], "something_new", ("unrecognized", true, None))]
#[case::unknown_status(418, &[], "", ("unrecognized", true, None))]
fn each_slack_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(answered(Platform::Slack, status, headers, body), expected);
}

/// Microsoft Teams' translation table: each error code its Workflows
/// webhook answers with, else its status, maps to its failure; an answer
/// it does not know is `unrecognized`.
/// Covers: setting:notifications.channels
#[rstest]
#[case::schema_mismatch(400, &[], r#"{"error":{"code":"TriggerInputSchemaMismatch"}}"#, ("rejected", false, None))]
#[case::not_enabled(400, &[], r#"{"error":{"code":"WorkflowTriggerIsNotEnabled"}}"#, ("gone", false, None))]
#[case::not_found(404, &[], r#"{"error":{"code":"WorkflowNotFound"}}"#, ("gone", false, None))]
#[case::authorization(401, &[], r#"{"error":{"code":"DirectApiAuthorizationRequired"}}"#, ("unauthorized", false, None))]
#[case::bad_request(400, &[], "", ("rejected", false, None))]
#[case::missing(404, &[], "", ("gone", false, None))]
#[case::gone(410, &[], "", ("gone", false, None))]
#[case::forbidden(403, &[], "", ("unauthorized", false, None))]
#[case::rate_limited(429, &[("retry-after", "30")], "", ("rate_limited", true, Some(Duration::from_secs(30))))]
#[case::unavailable(502, &[], "", ("server_error", true, None))]
#[case::request_timeout(408, &[], "", ("timeout", true, None))]
#[case::unknown_status(418, &[], r#"{"error":{"code":"SomethingNew"}}"#, ("unrecognized", true, None))]
fn each_teams_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(answered(Platform::Teams, status, headers, body), expected);
}

/// Discord's translation table: each JSON error code its webhook answers
/// with, else its status, maps to its failure, a rate limit waiting the
/// `retry_after` its body gives in seconds; an answer it does not know is
/// `unrecognized`.
/// Covers: setting:notifications.channels
#[rstest]
#[case::unknown_webhook(404, &[], r#"{"code":10015,"message":"Unknown Webhook"}"#, ("gone", false, None))]
#[case::invalid_token(401, &[], r#"{"code":50027}"#, ("unauthorized", false, None))]
#[case::missing_permissions(403, &[], r#"{"code":50013}"#, ("unauthorized", false, None))]
#[case::empty_message(400, &[], r#"{"code":50006}"#, ("rejected", false, None))]
#[case::invalid_form_body(400, &[], r#"{"code":50035,"errors":{}}"#, ("rejected", false, None))]
#[case::rate_limited_body(429, &[("retry-after", "9")], r#"{"retry_after":1.5,"global":false}"#, ("rate_limited", true, Some(Duration::from_millis(1500))))]
#[case::rate_limited_header(429, &[("retry-after", "9")], "", ("rate_limited", true, Some(Duration::from_secs(9))))]
#[case::unavailable(503, &[], "", ("server_error", true, None))]
#[case::unknown_code(400, &[], r#"{"code":99999}"#, ("unrecognized", true, None))]
#[case::unknown_status(418, &[], "", ("unrecognized", true, None))]
fn each_discord_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(answered(Platform::Discord, status, headers, body), expected);
}

/// Telegram's translation table: a group that moved to a supergroup is
/// gone, then each status its Bot API answers with maps to its failure, a
/// rate limit waiting the `retry_after` of its `parameters`; an answer it
/// does not know is `unrecognized`.
/// Covers: setting:notifications.channels
#[rstest]
#[case::migrated(400, &[], r#"{"ok":false,"error_code":400,"parameters":{"migrate_to_chat_id":-1009876543210}}"#, ("gone", false, None))]
#[case::bad_request(400, &[], r#"{"ok":false,"error_code":400,"description":"Bad Request: chat not found"}"#, ("rejected", false, None))]
#[case::unauthorized(401, &[], r#"{"ok":false,"error_code":401}"#, ("unauthorized", false, None))]
#[case::unknown_token(404, &[], r#"{"ok":false,"error_code":404}"#, ("unauthorized", false, None))]
#[case::blocked(403, &[], r#"{"ok":false,"error_code":403,"description":"Forbidden: bot was blocked by the user"}"#, ("gone", false, None))]
#[case::rate_limited(429, &[], r#"{"ok":false,"error_code":429,"parameters":{"retry_after":7}}"#, ("rate_limited", true, Some(Duration::from_secs(7))))]
#[case::unavailable(502, &[], "", ("server_error", true, None))]
#[case::unknown_status(418, &[], r#"{"ok":false,"error_code":418}"#, ("unrecognized", true, None))]
fn each_telegram_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(
        answered(Platform::Telegram, status, headers, body),
        expected
    );
}
