//! The `sns` channel (issue 3680), publishing to an `httpmock` stand-in of
//! Amazon SNS: one signed `Publish` call per Notification, its form
//! snapshotted and its link without credentials; a FIFO topic's
//! deduplication ID the event's, the same on a retry; a region, topic or
//! endpoint the channel cannot use failing the configuration; and SNS's
//! translation table, one case per failure it knows and one it does not.
//! No real account is used.

use std::time::Duration;

use goodfetch_engine::clock::Clock;
use goodfetch_notification::api::ApiUnusable;
use goodfetch_notification::driver::Channel as _;
use goodfetch_notification::sns::Sns;
use goodfetch_protocol::egress::EgressRule;
use httpmock::MockServer;
use rstest::rstest;
use serde_json::json;
use url::Url;
use url::form_urlencoded;

use crate::email::{
    LINK_PASSWORD, Posted, credentials, endpoint, message, runtime, settings, stand_in, summary,
};

/// A standard topic's ARN.
const TOPIC: &str = "arn:aws:sns:eu-west-1:123456789012:run-events";

/// A FIFO topic's ARN.
const FIFO_TOPIC: &str = "arn:aws:sns:eu-west-1:123456789012:run-events.fifo";

/// An endpoint URL the channel takes, unless the egress policy refuses it.
const HTTP_ENDPOINT: &str = "http://127.0.0.1:9/";

/// The secret access key requests are signed with.
const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

/// SNS's answer to a `Publish` it took.
const PUBLISHED: &str = r#"<PublishResponse xmlns="https://sns.amazonaws.com/doc/2010-03-31/"><PublishResult><MessageId>567910cd-659e-55d4-8ccb-5aaf14679dc0</MessageId></PublishResult><ResponseMetadata><RequestId>d74b8436-ae13-5ab4-a9ff-ce54dfea72a0</RequestId></ResponseMetadata></PublishResponse>"#;

/// The channel `ops` publishing to `topic` through the stand-in `server`.
fn channel(topic: &str, server: &MockServer) -> Sns {
    let clock = Clock::system();
    Sns::new(
        "ops",
        topic,
        "eu-west-1",
        Some(Url::parse(&server.base_url()).unwrap()),
        credentials("AKIDEXAMPLE", SECRET),
        endpoint(&settings(json!({}), json!({})), clock),
        clock,
    )
    .unwrap()
}

/// The form `posted` carries, a pair a line.
fn form(posted: &Posted) -> Vec<(String, String)> {
    form_urlencoded::parse(&posted.body)
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect()
}

/// The value of the field `name` of `form`, if it has one.
fn field<'form>(form: &'form [(String, String)], name: &str) -> Option<&'form str> {
    form.iter()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value.as_str())
}

/// SNS's XML error answer naming `code`.
fn error(code: &str) -> String {
    format!(
        r#"<ErrorResponse xmlns="https://sns.amazonaws.com/doc/2010-03-31/"><Error><Type>Sender</Type><Code>{code}</Code><Message>{code} happened</Message></Error><RequestId>42d59b56-7407-4c4a-be0f-4c88daeea257</RequestId></ErrorResponse>"#
    )
}

/// A Notification is one `Publish` call to the topic, signed for `sns` in
/// the channel's region: its message the `full` Notification payload, its
/// link without credentials, under the title as its subject, and no
/// deduplication ID for a standard topic.
/// Covers: setting:notifications.channels
#[test]
fn a_notification_is_one_signed_publish() {
    let (server, posted) = stand_in(200, &[("content-type", "text/xml")], PUBLISHED);
    let runtime = runtime();
    let channel = channel(TOPIC, &server);

    runtime.block_on(channel.send(&message())).unwrap();

    let posted = posted.lock().unwrap();
    let [request] = &posted[..] else {
        panic!("one request, got {posted:?}")
    };
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("POST", "/")
    );
    assert_eq!(
        request.header("content-type"),
        Some("application/x-www-form-urlencoded; charset=utf-8")
    );
    let authorization = request.header("authorization").unwrap();
    assert!(
        authorization.starts_with("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/")
            && authorization.contains("/eu-west-1/sns/aws4_request"),
        "{authorization}"
    );
    let text = String::from_utf8(request.body.clone()).unwrap();
    assert!(
        !text.contains(LINK_PASSWORD) && !text.contains(SECRET),
        "the form carries a credential: {text}"
    );
    let form = form(request);
    assert_eq!(field(&form, "MessageDeduplicationId"), None);
    let lines: Vec<String> = form
        .iter()
        .map(|(name, value)| format!("{name} = {value}"))
        .collect();
    insta::assert_snapshot!("sns_publish_form", lines.join("\n"));
}

/// To a FIFO topic the event ID goes as the deduplication ID and the Run's
/// ID as the message group, the same on the driver's next attempt, so SNS
/// publishes a retried event once.
/// Covers: setting:notifications.channels
#[test]
fn a_fifo_topic_gets_the_event_id_on_every_attempt() {
    let (server, posted) = stand_in(200, &[("content-type", "text/xml")], PUBLISHED);
    let runtime = runtime();
    let channel = channel(FIFO_TOPIC, &server);

    runtime.block_on(async {
        channel.send(&message()).await.unwrap();
        channel.send(&message()).await.unwrap();
    });

    let ids: Vec<(Option<String>, Option<String>)> = posted
        .lock()
        .unwrap()
        .iter()
        .map(|request| {
            let form = form(request);
            (
                field(&form, "MessageDeduplicationId").map(str::to_owned),
                field(&form, "MessageGroupId").map(str::to_owned),
            )
        })
        .collect();
    let attempt = (
        Some("evt_01JDQ8X2Y5".to_owned()),
        Some("nightly-prices".to_owned()),
    );
    assert_eq!(ids, vec![attempt.clone(), attempt]);
}

/// A title holding a line break, longer than SNS's subject takes, is one
/// line cut under 100 characters.
/// Covers: setting:notifications.channels
#[test]
fn a_long_title_is_one_line_under_100_characters() {
    let (server, posted) = stand_in(200, &[], PUBLISHED);
    let runtime = runtime();
    let channel = channel(TOPIC, &server);
    let mut message = message();
    message.title = format!("Run failed\n{}", "x".repeat(200));

    runtime.block_on(channel.send(&message)).unwrap();

    let form = form(&posted.lock().unwrap()[0]);
    let subject = field(&form, "Subject").unwrap();
    assert_eq!(subject.chars().count(), 99);
    assert!(
        subject.starts_with("Run failed x") && subject.ends_with('…'),
        "{subject}"
    );
}

/// A region or topic ARN the channel cannot use, or an endpoint the egress
/// policy refuses, fails the configuration rather than each delivery.
/// Covers: setting:notifications.channels, setting:egress.no_private_endpoints
#[rstest]
#[case::empty_region(TOPIC, "", HTTP_ENDPOINT, json!({}), ApiUnusable::Region)]
#[case::upper_case_region(TOPIC, "EU-WEST-1", HTTP_ENDPOINT, json!({}), ApiUnusable::Region)]
#[case::queue_arn("arn:aws:sqs:eu-west-1:123456789012:run-events", "eu-west-1", HTTP_ENDPOINT, json!({}), ApiUnusable::Topic)]
#[case::short_account("arn:aws:sns:eu-west-1:1234:run-events", "eu-west-1", HTTP_ENDPOINT, json!({}), ApiUnusable::Topic)]
#[case::spaced_topic("arn:aws:sns:eu-west-1:123456789012:run events", "eu-west-1", HTTP_ENDPOINT, json!({}), ApiUnusable::Topic)]
#[case::bare_fifo("arn:aws:sns:eu-west-1:123456789012:.fifo", "eu-west-1", HTTP_ENDPOINT, json!({}), ApiUnusable::Topic)]
#[case::ftp_endpoint(TOPIC, "eu-west-1", "ftp://127.0.0.1:9/", json!({}), ApiUnusable::Url)]
#[case::private_endpoint(TOPIC, "eu-west-1", HTTP_ENDPOINT, json!({ "egress": { "no_private_endpoints": true } }), ApiUnusable::Refused(EgressRule::PrivateAddress))]
fn an_unusable_channel_fails_the_configuration(
    #[case] topic: &str,
    #[case] region: &str,
    #[case] endpoint_url: &str,
    #[case] policy: serde_json::Value,
    #[case] expected: ApiUnusable,
) {
    let clock = Clock::system();
    let unusable = Sns::new(
        "ops",
        topic,
        region,
        Some(Url::parse(endpoint_url).unwrap()),
        credentials("AKIDEXAMPLE", SECRET),
        endpoint(&settings(policy, json!({})), clock),
        clock,
    )
    .unwrap_err();

    assert_eq!(unusable, expected);
}

/// The SNS translation table: each answer the driver knows, by the error
/// code SNS names in its XML or by its status, maps to its failure,
/// retried or not, with the wait a `Retry-After` asks for; an answer it
/// does not know is `unrecognized` and retried.
/// Covers: setting:notifications.channels
#[rstest]
#[case::authorization_error(403, &[], &error("AuthorizationError"), ("unauthorized", false, None))]
#[case::invalid_security(403, &[], &error("InvalidSecurity"), ("unauthorized", false, None))]
#[case::signature_mismatch(403, &[], &error("SignatureDoesNotMatch"), ("unauthorized", false, None))]
#[case::unknown_access_key(403, &[], &error("InvalidClientTokenId"), ("unauthorized", false, None))]
#[case::expired_token(403, &[], &error("ExpiredToken"), ("unauthorized", false, None))]
#[case::access_denied(403, &[], &error("AccessDenied"), ("unauthorized", false, None))]
#[case::incomplete_signature(400, &[], &error("IncompleteSignature"), ("unauthorized", false, None))]
#[case::missing_token(403, &[], &error("MissingAuthenticationToken"), ("unauthorized", false, None))]
#[case::request_expired(400, &[], &error("RequestExpired"), ("unauthorized", false, None))]
#[case::kms_access_denied(400, &[], &error("KMSAccessDenied"), ("unauthorized", false, None))]
#[case::kms_opt_in_required(403, &[], &error("KMSOptInRequired"), ("unauthorized", false, None))]
#[case::not_found(404, &[], &error("NotFound"), ("gone", false, None))]
#[case::endpoint_disabled(400, &[], &error("EndpointDisabled"), ("gone", false, None))]
#[case::invalid_parameter(400, &[], &error("InvalidParameter"), ("rejected", false, None))]
#[case::parameter_value_invalid(400, &[], &error("ParameterValueInvalid"), ("rejected", false, None))]
#[case::invalid_parameter_value(400, &[], &error("InvalidParameterValue"), ("rejected", false, None))]
#[case::validation_error(400, &[], &error("ValidationError"), ("rejected", false, None))]
#[case::kms_disabled(400, &[], &error("KMSDisabled"), ("rejected", false, None))]
#[case::kms_invalid_state(400, &[], &error("KMSInvalidState"), ("rejected", false, None))]
#[case::kms_not_found(400, &[], &error("KMSNotFound"), ("rejected", false, None))]
#[case::throttled(400, &[("retry-after", "60")], &error("Throttled"), ("rate_limited", true, Some(Duration::from_mins(1))))]
#[case::throttling(400, &[], &error("Throttling"), ("rate_limited", true, None))]
#[case::kms_throttling(400, &[], &error("KMSThrottling"), ("rate_limited", true, None))]
#[case::throttling_exception(400, &[], &error("ThrottlingException"), ("rate_limited", true, None))]
#[case::internal_error(500, &[], &error("InternalError"), ("server_error", true, None))]
#[case::internal_failure(500, &[], &error("InternalFailure"), ("server_error", true, None))]
#[case::service_unavailable_code(503, &[], &error("ServiceUnavailable"), ("server_error", true, None))]
#[case::code_in_header(400, &[("x-amzn-ErrorType", "InvalidParameter:http://internal.amazon.com/")], "", ("rejected", false, None))]
#[case::service_unavailable(503, &[], "", ("server_error", true, None))]
#[case::forbidden_status(403, &[], "", ("unauthorized", false, None))]
#[case::unauthorized_status(401, &[], "", ("unauthorized", false, None))]
#[case::request_timeout_status(408, &[], "", ("timeout", true, None))]
#[case::too_many_requests_status(429, &[], "", ("rate_limited", true, None))]
#[case::unknown_code(400, &[], &error("SomethingNew"), ("unrecognized", true, None))]
#[case::unknown_status(418, &[], "", ("unrecognized", true, None))]
fn each_sns_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body: &str,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    let (server, _) = stand_in(status, headers, body);
    let runtime = runtime();
    let channel = channel(TOPIC, &server);

    let failure = runtime.block_on(channel.send(&message())).unwrap_err();

    assert_eq!(summary(&channel.map_failure(failure)), expected);
}

/// An access key no HTTP header may carry is never sent: `unauthorized`,
/// not retried.
/// Covers: setting:notifications.channels
#[test]
fn an_unsignable_access_key_is_unauthorized() {
    let (server, posted) = stand_in(200, &[], PUBLISHED);
    let runtime = runtime();
    let clock = Clock::system();
    let channel = Sns::new(
        "ops",
        TOPIC,
        "eu-west-1",
        Some(Url::parse(&server.base_url()).unwrap()),
        credentials("AKID\nEXAMPLE", SECRET),
        endpoint(&settings(json!({}), json!({})), clock),
        clock,
    )
    .unwrap();

    let failure = runtime.block_on(channel.send(&message())).unwrap_err();

    assert_eq!(
        (
            summary(&channel.map_failure(failure)),
            posted.lock().unwrap().len()
        ),
        (("unauthorized", false, None), 0)
    );
}
