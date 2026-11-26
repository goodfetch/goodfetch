//! The `webhook` channel (issue 1385): a Notification posted as a signed
//! Standard Webhooks delivery that a receiver verifies with the reference
//! verifier, carrying neither the channel's secret nor a cookie, and its
//! translation table, one case per failure the driver knows and one it
//! does not, each answered by an `httpmock` receiver or a local socket.
#![expect(
    clippy::disallowed_types,
    reason = "the receivers stand in for a webhook's, outside the engine and any loom model: their own sockets, and a standard lock on what httpmock's thread received (ADR 0060)"
)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::settings::{Layers, Policy, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::webhook_signing::{
    EndpointId, Received, Signer, SigningKey, Verifier, VerifyingKey,
};
use goodfetch_notification::driver::Channel;
use goodfetch_notification::failure::Failure;
use goodfetch_notification::message::{EventId, Message};
use goodfetch_notification::webhook::{Payload, Webhook};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{Canaries, Canary, Output, SecretKind};
use httpmock::MockServer;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, json};
use url::Url;

/// A `v1` secret of the Standard Webhooks vectors.
const V1_SECRET: &str = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";

/// A `v2a` secret of the Standard Webhooks vectors, with its endpoint.
const V2A_SECRET: &str = "whsk_4uKPjji/dhB8w6GW3YYDnIMYvXcdHrvH6lidTr4Z+EY=";
const V2A_ENDPOINT: &str = "ep_2fVZdC8Kx9TR4ZoCqHWt3NnLgRb";

/// When the reported event happened.
const OCCURRED_AT: u64 = 1_764_000_000;

/// A runtime of real time: on a paused clock the channel's timeout would
/// fire before a receiver on another thread answers (TESTING.md, Time).
fn runtime() -> tokio::runtime::Runtime {
    goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// The machine's settings, under an Operator `policy`.
fn settings(policy: Value) -> Resolved {
    let Value::Object(policy) = policy else {
        panic!("a policy is a table")
    };
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(SettingsLayer::OperatorPolicy, &policy, Product::Collie)
                .unwrap(),
            ..Policy::default()
        },
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

/// The Notification each test delivers.
fn message() -> Message {
    Message {
        event_id: EventId::new("evt_01JDQ8X2Y5").unwrap(),
        event: "run_failed".to_owned(),
        run_id: Some(RunId::parse("nightly-prices").unwrap()),
        schedule_id: Some("prices".to_owned()),
        product: Product::Collie,
        title: "Run failed".to_owned(),
        text: "The Run nightly-prices failed: navigation_failed.".to_owned(),
        link: Some(Url::parse("http://127.0.0.1:7777/runs/nightly-prices").unwrap()),
        occurred_at: UNIX_EPOCH + Duration::from_secs(OCCURRED_AT),
    }
}

/// A signer of the keys `secrets`, for `endpoint`.
fn signer(secrets: &[&str], endpoint: Option<&str>) -> Signer {
    Signer::new(
        secrets.iter().map(|secret| key(secret)).collect(),
        endpoint.map(|endpoint| EndpointId::new(endpoint).unwrap()),
    )
    .unwrap()
}

fn key(secret: &str) -> SigningKey {
    SigningKey::from_secret(&SecretString::from(secret.to_owned())).unwrap()
}

/// The channel posting to `url` with `signer` and `payload`, under the
/// machine's `settings`, on `clock`.
fn webhook(
    url: &str,
    signer: Signer,
    payload: Payload,
    settings: &Resolved,
    clock: Clock,
) -> Webhook {
    Webhook::new(
        "ops",
        Url::parse(url).unwrap(),
        signer,
        payload,
        NotificationEndpoint::of(
            settings,
            clock,
            TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
        ),
        clock,
    )
    .unwrap()
}

/// A delivery as the receiver got it.
#[derive(Clone, Debug)]
struct Delivered {
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Delivered {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// An `httpmock` receiver answering `status`, with `headers`, a body of
/// `body_bytes` bytes, and keeping every delivery it got.
fn receiver(
    status: u16,
    headers: &[(&str, &str)],
    body_bytes: usize,
) -> (MockServer, Arc<Mutex<Vec<Delivered>>>) {
    let server = MockServer::start();
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&delivered);
    server.mock(|when, then| {
        when.is_true(move |request| {
            kept.lock().unwrap().push(Delivered {
                headers: request.headers_vec().clone(),
                body: request.body_vec(),
            });
            true
        });
        let then = headers
            .iter()
            .fold(then.status(status), |then, (name, value)| {
                then.header(*name, *value)
            });
        then.body(vec![b'x'; body_bytes]);
    });
    (server, delivered)
}

/// What `message()` sends with `payload`, and what the receiver got.
fn deliver(signer: Signer, payload: Payload) -> (Result<(), Failure>, Vec<Delivered>, SystemTime) {
    let (server, delivered) = receiver(204, &[], 0);
    let runtime = runtime();
    let clock = Clock::system();
    let channel = webhook(
        &server.url("/hooks"),
        signer,
        payload,
        &settings(json!({})),
        clock,
    );
    let sent = runtime.block_on(channel.send(&message()));
    let delivered = delivered.lock().unwrap().clone();
    (
        sent.map_err(|failure| channel.map_failure(failure)),
        delivered,
        clock.wall(),
    )
}

/// A delivery is a Standard Webhooks body whose `webhook-id` is the event
/// ID, signed so that the reference verifier accepts it under the key's
/// own scheme, `v1` HMAC by default or `v2a` ed25519 for an endpoint.
/// Covers: story:11-269, story:11-272
#[rstest]
#[case::v1(&[V1_SECRET], None)]
#[case::v2a(&[V2A_SECRET], Some(V2A_ENDPOINT))]
#[case::rotating_v1_to_v2a(&[V1_SECRET, V2A_SECRET], Some(V2A_ENDPOINT))]
fn a_delivery_is_a_standard_webhook_the_reference_verifier_accepts(
    #[case] secrets: &[&str],
    #[case] endpoint: Option<&str>,
) {
    let (sent, delivered, sent_at) = deliver(signer(secrets, endpoint), Payload::Full);

    let [delivery] = delivered.as_slice() else {
        panic!("one delivery, got {delivered:?}")
    };
    let now = i64::try_from(sent_at.duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap();
    let verifier = Verifier::new(
        secrets
            .iter()
            .map(|secret| key(secret).verifying_key())
            .collect::<Vec<VerifyingKey>>(),
        endpoint.map(|endpoint| EndpointId::new(endpoint).unwrap()),
    )
    .unwrap();
    let verified = verifier.verify_signature(
        &Received {
            id: delivery.header("webhook-id"),
            timestamp: delivery.header("webhook-timestamp"),
            signature: delivery.header("webhook-signature"),
            body: &delivery.body,
        },
        now,
    );
    assert_eq!(
        (
            sent,
            verified,
            delivery.header("webhook-id"),
            delivery.header("content-type"),
            serde_json::from_slice::<Value>(&delivery.body).unwrap(),
        ),
        (
            Ok(()),
            Ok(()),
            Some("evt_01JDQ8X2Y5"),
            Some("application/json"),
            json!({
                "type": "notification.run_failed",
                "timestamp": "2025-11-24T16:00:00Z",
                "format": "0.1",
                "data": {
                    "event_id": "evt_01JDQ8X2Y5",
                    "event": "run_failed",
                    "run_id": "nightly-prices",
                    "schedule_id": "prices",
                    "product": "collie",
                    "title": "Run failed",
                    "text": "The Run nightly-prices failed: navigation_failed.",
                    "link": "http://127.0.0.1:7777/runs/nightly-prices",
                },
            }),
        )
    );
}

/// The thin payload Integrations' triggers take holds the IDs and the
/// link only, never the title or text.
/// Covers: story:11-269
#[test]
fn the_thin_payload_holds_the_ids_and_link_only() {
    let (sent, delivered, _) = deliver(signer(&[V1_SECRET], None), Payload::Thin);

    let [delivery] = delivered.as_slice() else {
        panic!("one delivery, got {delivered:?}")
    };
    assert_eq!(
        (
            sent,
            serde_json::from_slice::<Value>(&delivery.body).unwrap()["data"].clone()
        ),
        (
            Ok(()),
            json!({
                "event_id": "evt_01JDQ8X2Y5",
                "run_id": "nightly-prices",
                "schedule_id": "prices",
                "link": "http://127.0.0.1:7777/runs/nightly-prices",
            })
        )
    );
}

/// A delivery carries neither the channel's signing secret, in any form
/// the canary search knows, nor a cookie: it goes through the engine's own
/// client, never a Session's (ADR 0088).
/// Covers: story:11-270
#[test]
fn a_delivery_carries_no_secret_and_no_cookie() {
    let mut canaries = Canaries::new(1385);
    canaries.add(Canary::known(
        "webhook-secret",
        SecretKind::WebhookSigningSecret,
        V1_SECRET,
    ));

    let (sent, delivered, _) = deliver(signer(&[V1_SECRET], None), Payload::Full);

    let [delivery] = delivered.as_slice() else {
        panic!("one delivery, got {delivered:?}")
    };
    let headers = delivery
        .headers
        .iter()
        .map(|(name, value)| [name.as_str(), ": ", value.as_str(), "\n"].concat())
        .collect::<String>();
    let leaks = canaries
        .leaks(&[
            Output::Bytes {
                name: "headers",
                bytes: headers.as_bytes(),
            },
            Output::Bytes {
                name: "body",
                bytes: &delivery.body,
            },
        ])
        .unwrap();
    assert_eq!(
        (sent, leaks, delivery.header("cookie")),
        (Ok(()), Vec::new(), None)
    );
}

/// The failure the driver gets for a receiver's answer: its name, whether
/// it is retried, and the wait it asks for.
fn answered(
    status: u16,
    headers: &[(&str, &str)],
    body_bytes: usize,
) -> (&'static str, bool, Option<Duration>) {
    let (server, _) = receiver(status, headers, body_bytes);
    let runtime = runtime();
    let channel = webhook(
        &server.url("/hooks"),
        signer(&[V1_SECRET], None),
        Payload::Full,
        &settings(json!({})),
        Clock::system(),
    );
    let Err(failure) = runtime.block_on(channel.send(&message())) else {
        panic!("status {status} delivered")
    };
    summary(&channel.map_failure(failure))
}

fn summary(failure: &Failure) -> (&'static str, bool, Option<Duration>) {
    (
        failure.name(),
        failure.is_transient(),
        failure.retry_after(),
    )
}

/// The webhook's translation table: each answer the driver knows maps to
/// its failure, retried or not, with the wait a `Retry-After` asks for;
/// an answer it does not know is `unrecognized` and retried.
/// Covers: story:11-269
#[rstest]
#[case::request_timeout(408, &[], 0, ("timeout", true, None))]
#[case::rate_limited_with_retry_after(429, &[("retry-after", "60")], 0, ("rate_limited", true, Some(Duration::from_mins(1))))]
#[case::rate_limited_without_retry_after(429, &[], 0, ("rate_limited", true, None))]
#[case::internal_server_error(500, &[], 0, ("server_error", true, None))]
#[case::service_unavailable(503, &[], 0, ("server_error", true, None))]
#[case::unauthorized(401, &[], 0, ("unauthorized", false, None))]
#[case::forbidden(403, &[], 0, ("unauthorized", false, None))]
#[case::not_found(404, &[], 0, ("gone", false, None))]
#[case::gone(410, &[], 0, ("gone", false, None))]
#[case::bad_request(400, &[], 0, ("rejected", false, None))]
#[case::payload_too_large(413, &[], 0, ("rejected", false, None))]
#[case::unsupported_media_type(415, &[], 0, ("rejected", false, None))]
#[case::unprocessable(422, &[], 0, ("rejected", false, None))]
#[case::unknown_status(418, &[], 0, ("unrecognized", true, None))]
#[case::redirect(302, &[("location", "http://127.0.0.1:1/")], 0, ("unrecognized", true, None))]
#[case::answer_too_long(200, &[], 65 * 1024, ("unrecognized", true, None))]
fn each_answer_maps_to_its_failure(
    #[case] status: u16,
    #[case] headers: &[(&str, &str)],
    #[case] body_bytes: usize,
    #[case] expected: (&str, bool, Option<Duration>),
) {
    assert_eq!(answered(status, headers, body_bytes), expected);
}

/// A receiver nobody listens on is `unreachable`, retried.
/// Covers: story:11-269
#[test]
fn a_closed_port_is_unreachable() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let runtime = runtime();
    let channel = webhook(
        &format!("http://127.0.0.1:{port}/hooks"),
        signer(&[V1_SECRET], None),
        Payload::Full,
        &settings(json!({})),
        Clock::system(),
    );

    let failure = runtime.block_on(channel.send(&message())).unwrap_err();

    assert_eq!(
        summary(&channel.map_failure(failure)),
        ("unreachable", true, None)
    );
}

/// A receiver on a private address under `egress.no_private_endpoints` is
/// refused before any request, `rejected` and not retried.
/// Covers: story:11-269, setting:egress.no_private_endpoints
#[test]
fn a_private_receiver_refused_by_the_policy_is_rejected() {
    let (server, delivered) = receiver(204, &[], 0);
    let runtime = runtime();
    let channel = webhook(
        &server.url("/hooks"),
        signer(&[V1_SECRET], None),
        Payload::Full,
        &settings(json!({ "egress": { "no_private_endpoints": true } })),
        Clock::system(),
    );

    let failure = runtime.block_on(channel.send(&message())).unwrap_err();

    assert_eq!(
        (
            summary(&channel.map_failure(failure)),
            delivered.lock().unwrap().len()
        ),
        (("rejected", false, None), 0)
    );
}

/// A receiver that takes the delivery and never answers is a `timeout`,
/// retried, once the channel's wait has passed on the paused clock.
/// Covers: story:11-269
#[test]
fn a_receiver_that_never_answers_times_out() {
    let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = silent.local_addr().unwrap().port();

    let failure = goodfetch_test_support::runtime().block_on(async {
        #[expect(
            clippy::disallowed_methods,
            reason = "the channel's wait runs on the paused Clock, the Clock's test version (ADR 0059; TESTING.md, Time)"
        )]
        let clock = Clock::paused(UNIX_EPOCH + Duration::from_secs(OCCURRED_AT));
        let channel = webhook(
            &format!("http://127.0.0.1:{port}/hooks"),
            signer(&[V1_SECRET], None),
            Payload::Full,
            &settings(json!({})),
            clock,
        );
        let failure = channel.send(&message()).await.unwrap_err();
        summary(&channel.map_failure(failure))
    });

    drop(silent);
    assert_eq!(failure, ("timeout", true, None));
}
