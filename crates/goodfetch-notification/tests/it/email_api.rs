//! The `email` channel's HTTP API providers (issue 1389): Resend,
//! Postmark, SendGrid and Mailgun, each posted to an `httpmock` stand-in of
//! its API: one request of the message's subject and both parts, its link
//! without credentials, authenticated as the provider asks; Resend's
//! idempotency key; and each provider's translation table, one case per
//! failure it knows and one it does not. No real account is used.

use std::time::{Duration, UNIX_EPOCH};

use data_encoding::BASE64;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_notification::driver::Channel as _;
use goodfetch_notification::email::{
    Address, ApiUnusable, Email, Mailgun, Postmark, Provider, Resend, Sendgrid,
};
use goodfetch_protocol::notifications::MailgunRegion;
use httpmock::MockServer;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};
use url::{Url, form_urlencoded};

use crate::email::{
    LINK_PASSWORD, Posted, email, endpoint, message, runtime, settings, stand_in, summary,
};

/// The API key every provider is given.
const API_KEY: &str = "key-0123456789";

/// The Mailgun domain the channel sends from.
const DOMAIN: &str = "mg.example.com";

/// The providers reached over an HTTP API.
#[derive(Clone, Copy, Debug)]
enum Api {
    Resend,
    Postmark,
    Sendgrid,
    Mailgun,
}

/// A reachable endpoint on `clock`.
fn reachable(clock: Clock) -> NotificationEndpoint {
    endpoint(&settings(json!({}), json!({})), clock)
}

/// The provider `api` at `base`, sent [`API_KEY`].
fn provider(api: Api, base: Option<Url>, clock: Clock) -> Provider {
    let key = SecretString::from(API_KEY.to_owned());
    let endpoint = reachable(clock);
    match api {
        Api::Resend => Provider::Resend(Resend::new(&key, base, endpoint).unwrap()),
        Api::Postmark => Provider::Postmark(Postmark::new(&key, base, endpoint).unwrap()),
        Api::Sendgrid => Provider::Sendgrid(Sendgrid::new(&key, base, endpoint).unwrap()),
        Api::Mailgun => Provider::Mailgun(
            Mailgun::new(&key, DOMAIN, MailgunRegion::Us, base, endpoint).unwrap(),
        ),
    }
}

/// The channel `ops` posting to `server` through `api`.
fn channel(api: Api, server: &MockServer, clock: Clock) -> Email {
    let base = Url::parse(&server.base_url()).unwrap();
    email(provider(api, Some(base), clock), clock)
}

/// The one request `posted` holds.
fn only(posted: &[Posted]) -> Posted {
    let [request] = posted else {
        panic!("one request, got {posted:?}")
    };
    request.clone()
}

/// Sends the Notification through `api` to a stand-in answering `status`
/// with `body`, and gives back the outcome and the one request it got.
fn sent(api: Api, status: u16, body: &str) -> (Result<(), &'static str>, Posted) {
    let (server, posted) = stand_in(status, &[], body);
    let runtime = runtime();
    let channel = channel(api, &server, Clock::system());
    let sent = runtime
        .block_on(channel.send(&message()))
        .map_err(|failure| channel.map_failure(failure).name());
    let request = only(&posted.lock().unwrap());
    (sent, request)
}

/// An email through Resend is one `POST /emails` of JSON, its API key a
/// bearer token, carrying both parts and its link without credentials.
/// Covers: setting:notifications.channels
#[test]
fn an_email_through_resend_is_one_json_post() {
    let (sent, request) = sent(Api::Resend, 200, r#"{"id":"49a3999c"}"#);

    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let text = String::from_utf8_lossy(&request.body).into_owned();
    assert_eq!(
        (
            sent,
            request.method.as_str(),
            request.path.as_str(),
            request.header("authorization"),
            request.header("content-type"),
            request.header("user-agent"),
            body["from"].clone(),
            body["to"].clone(),
            body["subject"].clone(),
            body["text"]
                .as_str()
                .unwrap_or_default()
                .contains("nightly-prices"),
            body["html"].as_str().unwrap_or_default().contains("<html"),
            text.contains(LINK_PASSWORD),
        ),
        (
            Ok(()),
            "POST",
            "/emails",
            Some("Bearer key-0123456789"),
            Some("application/json"),
            Some("goodfetch-notification"),
            json!("goodfetch@example.com"),
            json!(["ops@example.com", "oncall@example.com"]),
            json!("Run failed"),
            true,
            true,
            false,
        )
    );
}

/// An email through Postmark is one `POST /email` of JSON on the
/// `outbound` stream, its server token in its own header.
/// Covers: setting:notifications.channels
#[test]
fn an_email_through_postmark_is_one_json_post() {
    let (sent, request) = sent(Api::Postmark, 200, r#"{"ErrorCode":0,"Message":"OK"}"#);

    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let text = String::from_utf8_lossy(&request.body).into_owned();
    assert_eq!(
        (
            sent,
            request.path.as_str(),
            request.header("x-postmark-server-token"),
            request.header("authorization"),
            request.header("accept"),
            body["From"].clone(),
            body["To"].clone(),
            body["Subject"].clone(),
            body["MessageStream"].clone(),
            body["TextBody"]
                .as_str()
                .unwrap_or_default()
                .contains("nightly-prices"),
            body["HtmlBody"]
                .as_str()
                .unwrap_or_default()
                .contains("<html"),
            text.contains(LINK_PASSWORD),
        ),
        (
            Ok(()),
            "/email",
            Some("key-0123456789"),
            None,
            Some("application/json"),
            json!("goodfetch@example.com"),
            json!("ops@example.com, oncall@example.com"),
            json!("Run failed"),
            json!("outbound"),
            true,
            true,
            false,
        )
    );
}

/// An email through SendGrid is one `POST /v3/mail/send` of JSON, its
/// recipients in one personalization, its parts plain text then HTML.
/// Covers: setting:notifications.channels
#[test]
fn an_email_through_sendgrid_is_one_json_post() {
    let (sent, request) = sent(Api::Sendgrid, 202, "");

    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let text = String::from_utf8_lossy(&request.body).into_owned();
    assert_eq!(
        (
            sent,
            request.path.as_str(),
            request.header("authorization"),
            body["personalizations"].clone(),
            body["from"].clone(),
            body["subject"].clone(),
            body["content"][0]["type"].clone(),
            body["content"][1]["type"].clone(),
            text.contains(LINK_PASSWORD),
        ),
        (
            Ok(()),
            "/v3/mail/send",
            Some("Bearer key-0123456789"),
            json!([{ "to": [{ "email": "ops@example.com" }, { "email": "oncall@example.com" }] }]),
            json!({ "email": "goodfetch@example.com" }),
            json!("Run failed"),
            json!("text/plain"),
            json!("text/html"),
            false,
        )
    );
}

/// An email through Mailgun is one `POST /v3/<domain>/messages` of a
/// form, a `to` for each recipient, authenticated as `api` with the key.
/// Covers: setting:notifications.channels
#[test]
fn an_email_through_mailgun_is_one_form_post() {
    let (sent, request) = sent(Api::Mailgun, 200, r#"{"id":"<1@mg>","message":"Queued"}"#);

    let fields = form_urlencoded::parse(&request.body)
        .into_owned()
        .collect::<Vec<_>>();
    let field = |name: &str| {
        fields
            .iter()
            .filter(|(field, _)| field == name)
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
    };
    let login = format!("Basic {}", BASE64.encode(b"api:key-0123456789"));
    assert_eq!(
        (
            sent,
            request.path.as_str(),
            request.header("authorization"),
            request.header("content-type"),
            field("from"),
            field("to"),
            field("subject"),
            field("text").concat().contains("nightly-prices"),
            field("html").concat().contains("<html"),
            fields
                .iter()
                .any(|(_, value)| value.contains(LINK_PASSWORD)),
        ),
        (
            Ok(()),
            "/v3/mg.example.com/messages",
            Some(login.as_str()),
            Some("application/x-www-form-urlencoded"),
            vec!["goodfetch@example.com"],
            vec!["ops@example.com", "oncall@example.com"],
            vec!["Run failed"],
            true,
            true,
            false,
        )
    );
}

/// Every attempt of one Notification on one channel carries the same
/// idempotency key, so a stand-in that, as Resend does, sends one message
/// per key sends a retried Notification once; the same Notification on
/// another channel carries another key, and is sent too.
/// Covers: setting:notifications.channels
#[test]
fn resend_attempts_of_one_notification_share_an_idempotency_key() {
    let (server, posted) = stand_in(200, &[], "{}");
    let runtime = runtime();
    let clock = Clock::system();
    let ops = channel(Api::Resend, &server, clock);
    let base = Url::parse(&server.base_url()).unwrap();
    let other = Email::new(
        "oncall",
        Address::parse("goodfetch@example.com").unwrap(),
        vec![Address::parse("ops@example.com").unwrap()],
        provider(Api::Resend, Some(base), clock),
        clock,
    )
    .unwrap();

    runtime.block_on(async {
        ops.send(&message()).await.unwrap();
        ops.send(&message()).await.unwrap();
        other.send(&message()).await.unwrap();
    });

    let keys = posted
        .lock()
        .unwrap()
        .iter()
        .map(|request| {
            request
                .header("idempotency-key")
                .unwrap_or_default()
                .to_owned()
        })
        .collect::<Vec<_>>();
    let [first, retried, elsewhere] = keys.as_slice() else {
        panic!("three requests, got {keys:?}")
    };
    let messages = keys.iter().collect::<std::collections::BTreeSet<_>>().len();
    let digits = first.strip_prefix("evt_01JDQ8X2Y5.").unwrap_or_default();
    assert_eq!(
        (
            first == retried,
            first == elsewhere,
            messages,
            digits.len(),
            digits.chars().all(|digit| digit.is_ascii_hexdigit()),
        ),
        (true, false, 2, 32, true)
    );
}

/// Mailgun's region picks its API's host when no URL is given, and a
/// domain that would leave its path is refused.
/// Covers: setting:notifications.channels
#[rstest]
#[case::us(MailgunRegion::Us, DOMAIN, Ok("api.mailgun.net"))]
#[case::eu(MailgunRegion::Eu, DOMAIN, Ok("api.eu.mailgun.net"))]
#[case::path_in_domain(MailgunRegion::Us, "mg.example.com/../x", Err(ApiUnusable::Domain))]
#[case::empty_domain(MailgunRegion::Eu, "", Err(ApiUnusable::Domain))]
fn mailgun_reaches_its_region(
    #[case] region: MailgunRegion,
    #[case] domain: &str,
    #[case] expected: Result<&str, ApiUnusable>,
) {
    let clock = Clock::system();
    let made = Mailgun::new(
        &SecretString::from(API_KEY.to_owned()),
        domain,
        region,
        None,
        reachable(clock),
    )
    .map(|mailgun| email(Provider::Mailgun(mailgun), clock));

    assert_eq!(
        made.as_ref()
            .map(goodfetch_notification::driver::Channel::destination)
            .map_err(|unusable| *unusable),
        expected
    );
}

/// The failure `api`'s stand-in answering `status`, with `headers` and
/// `body`, maps to.
fn answered(
    api: Api,
    status: u16,
    headers: &[(&str, &str)],
    body: &str,
) -> (&'static str, bool, Option<Duration>) {
    let (server, _) = stand_in(status, headers, body);
    let runtime = runtime();
    let channel = channel(api, &server, Clock::system());
    let failure = runtime.block_on(channel.send(&message())).unwrap_err();
    summary(&channel.map_failure(failure))
}

/// Resend's translation table: each answer it knows, by the error `name`
/// in its body or by its status, maps to its failure, with the wait a
/// `Retry-After` asks for; an answer it does not know is `unrecognized`.
/// Covers: setting:notifications.channels
#[rstest]
#[case::rate_limited(429, &[("retry-after", "60")], r#"{"statusCode":429,"name":"rate_limit_exceeded"}"#, ("rate_limited", true, Some(Duration::from_mins(1))))]
#[case::daily_quota(429, &[], r#"{"name":"daily_quota_exceeded"}"#, ("rate_limited", true, None))]
#[case::concurrent_key(409, &[], r#"{"name":"concurrent_idempotent_requests"}"#, ("rate_limited", true, None))]
#[case::missing_key(401, &[], r#"{"name":"missing_api_key"}"#, ("unauthorized", false, None))]
#[case::invalid_key(403, &[], r#"{"name":"invalid_api_key"}"#, ("unauthorized", false, None))]
#[case::restricted_key(401, &[], r#"{"name":"restricted_api_key"}"#, ("unauthorized", false, None))]
#[case::unauthorized_validation(401, &[], r#"{"name":"validation_error"}"#, ("unauthorized", false, None))]
#[case::validation(422, &[], r#"{"name":"validation_error"}"#, ("rejected", false, None))]
#[case::invalid_from(422, &[], r#"{"name":"invalid_from_address"}"#, ("rejected", false, None))]
#[case::key_reused(409, &[], r#"{"name":"invalid_idempotent_request"}"#, ("rejected", false, None))]
#[case::not_found(404, &[], r#"{"name":"not_found"}"#, ("rejected", false, None))]
#[case::application_error(500, &[], r#"{"name":"application_error"}"#, ("server_error", true, None))]
#[case::unavailable(503, &[], "", ("server_error", true, None))]
#[case::unknown_name(422, &[], r#"{"name":"something_new"}"#, ("unrecognized", true, None))]
#[case::unknown_status(418, &[], "", ("unrecognized", true, None))]
fn each_resend_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(answered(Api::Resend, status, headers, body), expected);
}

/// Postmark's translation table: each answer it knows, by the `ErrorCode`
/// in its body or by its status, maps to its failure; an answer it does
/// not know is `unrecognized`.
/// Covers: setting:notifications.channels
#[rstest]
#[case::bad_token(401, &[], r#"{"ErrorCode":10,"Message":"Bad or missing API token"}"#, ("unauthorized", false, None))]
#[case::pending_approval(422, &[], r#"{"ErrorCode":412}"#, ("unauthorized", false, None))]
#[case::not_approved(422, &[], r#"{"ErrorCode":413}"#, ("unauthorized", false, None))]
#[case::inactive_recipient(422, &[], r#"{"ErrorCode":406}"#, ("gone", false, None))]
#[case::invalid_email(422, &[], r#"{"ErrorCode":300}"#, ("rejected", false, None))]
#[case::invalid_json(422, &[], r#"{"ErrorCode":402}"#, ("rejected", false, None))]
#[case::unknown_stream(422, &[], r#"{"ErrorCode":1235}"#, ("rejected", false, None))]
#[case::address_refused(422, &[], r#"{"ErrorCode":1480}"#, ("rejected", false, None))]
#[case::too_large(413, &[], "", ("rejected", false, None))]
#[case::rate_limited(429, &[("retry-after", "60")], "", ("rate_limited", true, Some(Duration::from_mins(1))))]
#[case::unexpected(500, &[], r#"{"ErrorCode":101}"#, ("server_error", true, None))]
#[case::maintenance(503, &[], r#"{"ErrorCode":100}"#, ("server_error", true, None))]
#[case::unknown_code(422, &[], r#"{"ErrorCode":9999}"#, ("unrecognized", true, None))]
#[case::unknown_status(418, &[], "", ("unrecognized", true, None))]
fn each_postmark_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(answered(Api::Postmark, status, headers, body), expected);
}

/// SendGrid's translation table: each status it knows maps to its
/// failure, a rate limit waiting until its `X-RateLimit-Reset`, a reset
/// already past asking for no wait; a status it does not know is
/// `unrecognized`.
/// Covers: setting:notifications.channels
#[rstest]
#[case::bad_request(400, &[], r#"{"errors":[{"message":"bad"}]}"#, ("rejected", false, None))]
#[case::unauthorized(401, &[], "", ("unauthorized", false, None))]
#[case::sender_unverified(403, &[], "", ("unauthorized", false, None))]
#[case::not_found(404, &[], "", ("rejected", false, None))]
#[case::too_large(413, &[], "", ("rejected", false, None))]
#[case::reset_past(429, &[("x-ratelimit-reset", "1")], "", ("rate_limited", true, Some(Duration::ZERO)))]
#[case::retry_after(429, &[("retry-after", "60")], "", ("rate_limited", true, Some(Duration::from_mins(1))))]
#[case::server_error(500, &[], "", ("server_error", true, None))]
#[case::unknown_status(418, &[], "", ("unrecognized", true, None))]
fn each_sendgrid_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(answered(Api::Sendgrid, status, headers, body), expected);
}

/// A SendGrid rate limit waits until its `X-RateLimit-Reset`, in seconds
/// since the epoch, from when the answer arrived.
/// Covers: setting:notifications.channels
#[test]
fn a_sendgrid_rate_limit_waits_until_its_reset() {
    let clock = Clock::system();
    let now = clock.wall().duration_since(UNIX_EPOCH).unwrap();
    let reset = now.as_secs() + 600;

    let before = clock.wall();
    let (name, transient, wait) = answered(
        Api::Sendgrid,
        429,
        &[("x-ratelimit-reset", &reset.to_string())],
        "",
    );
    let after = clock.wall();

    let reset = UNIX_EPOCH + Duration::from_secs(reset);
    let longest = reset.duration_since(before).unwrap();
    let shortest = reset.duration_since(after).unwrap();
    assert_eq!(
        (
            name,
            transient,
            wait.is_some_and(|wait| shortest <= wait && wait <= longest),
        ),
        ("rate_limited", true, true)
    );
}

/// Mailgun's translation table: each status it knows maps to its failure,
/// its own `420` and `429` rate limits included; a status it does not
/// know is `unrecognized`.
/// Covers: setting:notifications.channels
#[rstest]
#[case::bad_request(400, &[], r#"{"message":"'from' parameter is missing"}"#, ("rejected", false, None))]
#[case::unauthorized(401, &[], "Forbidden", ("unauthorized", false, None))]
#[case::forbidden(403, &[], "", ("unauthorized", false, None))]
#[case::unknown_domain(404, &[], r#"{"message":"Domain not found"}"#, ("rejected", false, None))]
#[case::too_large(413, &[], "", ("rejected", false, None))]
#[case::rate_limited_420(420, &[], "", ("rate_limited", true, None))]
#[case::rate_limited_429(429, &[("retry-after", "60")], "", ("rate_limited", true, Some(Duration::from_mins(1))))]
#[case::server_error(500, &[], "", ("server_error", true, None))]
#[case::unknown_status(418, &[], "", ("unrecognized", true, None))]
fn each_mailgun_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(answered(Api::Mailgun, status, headers, body), expected);
}

/// A provider whose API refuses the connection is `unreachable`, retried.
/// Covers: setting:notifications.channels
#[test]
#[expect(
    clippy::disallowed_types,
    reason = "the test only takes a free local port and closes it, so the API it stands for is unreached; no engine socket (ADR 0060)"
)]
fn an_unreached_api_is_unreachable() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = Url::parse(&format!("http://{}", closed.local_addr().unwrap())).unwrap();
    drop(closed);
    let runtime = runtime();
    let clock = Clock::system();
    let channel = email(provider(Api::Sendgrid, Some(base), clock), clock);

    let failure = runtime.block_on(channel.send(&message())).unwrap_err();

    assert_eq!(
        summary(&channel.map_failure(failure)),
        ("unreachable", true, None)
    );
}
