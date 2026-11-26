//! The verification helper through its interface (issue 1391): the shared
//! Standard Webhooks vectors, which the TypeScript and Python helpers pass
//! too, and Notification payloads that the engine's signer signs.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::webhook_signing::{EndpointId, Message, Signer, SigningKey};
use goodfetch_protocol::catalog::{ErrorGroup, Product, ProtocolVersion};
use goodfetch_protocol::notifications::{
    FullNotification, NotificationData, NotificationEvent, NotificationPayload, ThinNotification,
    WebhookInvalidDetails, WebhookInvalidReason,
};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::{SignedVector, VerificationOutcome, webhook_signatures};
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use super::{VerifyWebhookOptions, verify_webhook, verify_webhook_with};
use crate::{Error, ErrorCode};

/// When the test deliveries are signed, in Unix seconds.
const SIGNED_AT: u64 = 1_767_603_600;

/// The signed vector named `name`.
fn vector(name: &str) -> SignedVector {
    webhook_signatures()
        .signed
        .into_iter()
        .find(|vector| vector.name == name)
        .unwrap()
}

/// The signer of `vector`'s keys, for its endpoint.
fn signer(vector: &SignedVector) -> Signer {
    let keys = vector
        .keys
        .iter()
        .map(|key| SigningKey::from_secret(&SecretString::from(key.clone())).unwrap())
        .collect();
    let endpoint = vector
        .endpoint
        .as_ref()
        .map(|id| EndpointId::new(id.as_str()).unwrap());
    Signer::new(keys, endpoint).unwrap()
}

/// `seconds` after the Unix epoch.
fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

/// A delivery of `body` signed by `signer` at [`SIGNED_AT`]: its headers.
fn signed(signer: &Signer, body: &[u8]) -> Vec<(String, String)> {
    signer
        .headers(
            &Message {
                id: "evt_01JDQ8X2Y5",
                timestamp: SIGNED_AT.try_into().unwrap(),
                body,
            },
            &[("content-type", "application/json")],
        )
        .unwrap()
}

/// `headers` as the helper takes them.
fn pairs(headers: &[(String, String)]) -> impl Iterator<Item = (&str, &str)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_str()))
}

/// The Run the test Notifications concern.
fn nightly() -> RunId {
    RunId::parse("nightly-prices").unwrap()
}

/// A full Notification payload of a failed Run.
fn full() -> NotificationPayload {
    NotificationPayload {
        kind: "notification.run_failed".to_owned(),
        timestamp: at(SIGNED_AT - 2),
        format: ProtocolVersion { major: 0, minor: 1 },
        data: NotificationData::Full(FullNotification {
            event_id: "evt_01JDQ8X2Y5".to_owned(),
            event: NotificationEvent::RUN_FAILED,
            run_id: Some(nightly()),
            schedule_id: None,
            product: Product::Collie,
            title: "nightly-prices failed".to_owned(),
            text: "The Run ended on an error.".to_owned(),
            link: Some("http://127.0.0.1:7777/runs/nightly-prices".to_owned()),
        }),
    }
}

/// The reason `error` names, after checking it is `webhook_invalid`.
fn reason(error: &Error) -> WebhookInvalidReason {
    assert_eq!(error.code(), ErrorCode::WEBHOOK_INVALID, "{error}");
    assert_eq!(error.group(), ErrorGroup::Program);
    assert!(!error.retryable());
    error.details::<WebhookInvalidDetails>().unwrap().reason
}

/// Covers: story:11-271, error:webhook_invalid
#[test]
fn each_vector_is_refused_for_its_reason_or_passes_its_signature() {
    for case in webhook_signatures().verifications {
        let headers = [
            ("webhook-id", case.id.as_deref()),
            ("webhook-timestamp", case.timestamp.as_deref()),
            ("webhook-signature", case.signature.as_deref()),
        ];
        let mut options = VerifyWebhookOptions::default().at(at(case.now.try_into().unwrap()));
        if let Some(endpoint) = &case.endpoint {
            options = options.endpoint(endpoint);
        }
        let error = verify_webhook_with(
            case.body.as_bytes(),
            headers
                .into_iter()
                .filter_map(|(name, value)| Some((name, value?))),
            &case.trusted,
            &options,
        )
        .unwrap_err();
        // A vector's body is no Notification payload: one whose signature
        // verifies is refused only once its body is read.
        let expected = match case.outcome {
            VerificationOutcome::Verified => WebhookInvalidReason::Malformed,
            VerificationOutcome::MissingHeader => WebhookInvalidReason::MissingHeader,
            VerificationOutcome::TimestampInvalid
            | VerificationOutcome::TimestampTooOld
            | VerificationOutcome::TimestampTooNew => WebhookInvalidReason::Timestamp,
            VerificationOutcome::NoMatchingSignature => WebhookInvalidReason::Signature,
        };
        assert_eq!(reason(&error), expected, "{}", case.name);
    }
}

/// Covers: story:11-271, format:notification-payload
#[rstest]
#[case::full(full())]
#[case::thin(NotificationPayload {
    data: NotificationData::Thin(ThinNotification {
        event_id: "evt_01JDQ8X2Y5".to_owned(),
        run_id: Some(nightly()),
        schedule_id: None,
        link: None,
    }),
    ..full()
})]
fn a_signed_payload_verifies_and_reads_back(#[case] payload: NotificationPayload) {
    let body = serde_json::to_vec(&payload).unwrap();
    let headers = signed(&signer(&vector("v1_published")), &body);

    let verified = verify_webhook_with(
        &body,
        pairs(&headers),
        &vector("v1_published").keys,
        &VerifyWebhookOptions::default().at(at(SIGNED_AT + 1)),
    )
    .unwrap();

    assert_eq!(verified, payload);
}

/// Covers: story:11-271
#[rstest]
#[case::the_current_secret(0)]
#[case::the_previous_secret(1)]
fn a_rotation_list_accepts_either_secret(#[case] trusted: usize) {
    let rotation = vector("rotation_two_v1_keys");
    let body = serde_json::to_vec(&full()).unwrap();
    let headers = signed(&signer(&rotation), &body);
    let options = VerifyWebhookOptions::default().at(at(SIGNED_AT));
    let unknown = "whsec_c2VjcmV0IG9mIGFub3RoZXIgY2hhbm5lbCEhISE=";

    let one = verify_webhook_with(&body, pairs(&headers), &[&rotation.keys[trusted]], &options);
    let listed = verify_webhook_with(
        &body,
        pairs(&headers),
        &[unknown, &rotation.keys[trusted]],
        &options,
    );

    assert_eq!(one.unwrap(), full());
    assert_eq!(listed.unwrap(), full());
}

/// Covers: story:11-271
#[test]
fn a_v2a_signature_verifies_with_the_public_key_for_its_endpoint() {
    let signing = vector("v2a_published");
    let public = webhook_signatures()
        .verifications
        .into_iter()
        .find(|case| case.name == "v2a_published")
        .unwrap();
    let body = serde_json::to_vec(&full()).unwrap();
    let headers = signed(&signer(&signing), &body);
    let options = VerifyWebhookOptions::default().at(at(SIGNED_AT));

    let verified = verify_webhook_with(
        &body,
        pairs(&headers),
        &public.trusted,
        &options
            .clone()
            .endpoint(signing.endpoint.as_deref().unwrap()),
    );
    let elsewhere = verify_webhook_with(
        &body,
        pairs(&headers),
        &public.trusted,
        &options.endpoint("ep_another"),
    );

    assert_eq!(verified.unwrap(), full());
    assert_eq!(
        reason(&elsewhere.unwrap_err()),
        WebhookInvalidReason::Signature
    );
}

/// Covers: story:11-271
#[rstest]
#[case::forged_body(b"{\"type\":\"notification.run_finished\"}".as_slice(), SIGNED_AT, WebhookInvalidReason::Signature)]
#[case::replayed_six_minutes_later(b"".as_slice(), SIGNED_AT + 6 * 60, WebhookInvalidReason::Timestamp)]
#[case::dated_six_minutes_ahead(b"".as_slice(), SIGNED_AT - 6 * 60, WebhookInvalidReason::Timestamp)]
fn a_forged_or_replayed_call_is_refused(
    #[case] forged: &[u8],
    #[case] now: u64,
    #[case] expected: WebhookInvalidReason,
) {
    let v1 = vector("v1_published");
    let body = serde_json::to_vec(&full()).unwrap();
    let headers = signed(&signer(&v1), &body);
    let received = if forged.is_empty() {
        body.as_slice()
    } else {
        forged
    };

    let error = verify_webhook_with(
        received,
        pairs(&headers),
        &v1.keys,
        &VerifyWebhookOptions::default().at(at(now)),
    )
    .unwrap_err();

    assert_eq!(reason(&error), expected);
}

/// Covers: story:11-271
#[test]
fn a_wider_tolerance_accepts_an_older_call() {
    let v1 = vector("v1_published");
    let body = serde_json::to_vec(&full()).unwrap();
    let headers = signed(&signer(&v1), &body);

    let verified = verify_webhook_with(
        &body,
        pairs(&headers),
        &v1.keys,
        &VerifyWebhookOptions::default()
            .at(at(SIGNED_AT + 6 * 60))
            .tolerance(Duration::from_mins(10)),
    );

    assert_eq!(verified.unwrap(), full());
}

/// Covers: story:11-271
#[test]
fn a_newer_minor_parses_and_keeps_an_unknown_event() {
    let v1 = vector("v1_published");
    let body = serde_json::to_vec(&json!({
        "type": "notification.crawl_drained",
        "timestamp": "2026-01-05T09:00:00Z",
        "format": "0.7",
        "data": {
            "event_id": "evt_01JDQ8X2Y5",
            "event": "crawl_drained",
            "run_id": "nightly-prices",
            "product": "foxhound",
            "title": "nightly-prices drained",
            "text": "The crawl drained.",
            "severity": "info",
        },
        "delivery_attempt": 2,
    }))
    .unwrap();
    let headers = signed(&signer(&v1), &body);

    let verified = verify_webhook_with(
        &body,
        pairs(&headers),
        &v1.keys,
        &VerifyWebhookOptions::default().at(at(SIGNED_AT)),
    )
    .unwrap();

    assert_eq!(verified.kind, "notification.crawl_drained");
    assert_eq!(verified.format, ProtocolVersion { major: 0, minor: 7 });
    let NotificationData::Full(data) = verified.data else {
        panic!("a full payload reads as full: {:?}", verified.data);
    };
    assert_eq!(data.event.as_str(), "crawl_drained");
    assert_eq!(data.product, Product::Foxhound);
}

/// Covers: story:11-271
#[rstest]
#[case::another_major(serde_json::to_string(&NotificationPayload {
    format: ProtocolVersion { major: 1, minor: 0 },
    ..full()
}).unwrap())]
#[case::no_payload("{\"test\": 2432232314}".to_owned())]
#[case::no_json("ok".to_owned())]
fn a_body_of_no_payload_this_sdk_reads_is_malformed(#[case] body: String) {
    let v1 = vector("v1_published");
    let headers = signed(&signer(&v1), body.as_bytes());

    let error = verify_webhook_with(
        body.as_bytes(),
        pairs(&headers),
        &v1.keys,
        &VerifyWebhookOptions::default().at(at(SIGNED_AT)),
    )
    .unwrap_err();

    assert_eq!(reason(&error), WebhookInvalidReason::Malformed);
}

/// Covers: story:11-271
#[test]
fn header_names_match_in_any_case() {
    let v1 = vector("v1_published");
    let body = serde_json::to_vec(&full()).unwrap();
    let headers: Vec<(String, String)> = signed(&signer(&v1), &body)
        .into_iter()
        .map(|(name, value)| (name.to_ascii_uppercase(), value))
        .collect();

    let verified = verify_webhook_with(
        &body,
        pairs(&headers),
        &v1.keys,
        &VerifyWebhookOptions::default().at(at(SIGNED_AT)),
    );

    assert_eq!(verified.unwrap(), full());
}

/// Covers: story:11-271
#[test]
fn the_system_clock_refuses_a_call_signed_long_ago() {
    let v1 = vector("v1_published");
    let body = serde_json::to_vec(&full()).unwrap();
    let headers = signed(&signer(&v1), &body);

    let error = verify_webhook(&body, pairs(&headers), &v1.keys).unwrap_err();

    assert_eq!(reason(&error), WebhookInvalidReason::Timestamp);
}

#[rstest]
#[case::no_secret(&[], None)]
#[case::no_prefix(&["MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw"], None)]
#[case::an_empty_secret(&["whsec_"], None)]
#[case::a_signing_key(&["whsk_VQMJMCF2TCBiITJzbZB7z+0sHSkvbmqQaNgo84ExILU="], Some("ep_1"))]
#[case::a_public_key_with_no_endpoint(&["whpk_2zPwzAhd9xyyTX6hydEXQjvUWwvpqA/DmAUXdzvZGyU="], None)]
#[case::an_endpoint_with_a_dot(&["whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw"], Some("ep.1"))]
fn a_secret_the_helper_cannot_use_is_refused_before_the_call(
    #[case] secrets: &[&str],
    #[case] endpoint: Option<&str>,
) {
    let mut options = VerifyWebhookOptions::default();
    if let Some(endpoint) = endpoint {
        options = options.endpoint(endpoint);
    }

    let error = verify_webhook_with(b"", [], secrets, &options).unwrap_err();

    assert_eq!(error.code().as_str(), "invalid_params", "{error}");
    assert_eq!(error.operation(), Some("verify_webhook"));
}
