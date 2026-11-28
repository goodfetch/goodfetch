//! Integration tests for `goodfetch-notification::subscriptions` (ADRs 0017 and
//! 0027; spec #608, issue 1403): idempotent subscribe/unsubscribe, thin
//! Standard Webhooks delivery verified with the reference verifier, no double
//! delivery after a resumed Run (delivery ledger keyed by event ID + channel),
//! and private-URL refusal under `egress.no_private_endpoints`.
#![expect(
    clippy::disallowed_types,
    reason = "local httpmock receiver outside the engine"
)]

use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::run_state::{RunState, RunStateOptions};
use goodfetch_engine::settings::{Layers, Policy, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::webhook_signing::{Received, Signer, SigningKey, Verifier, VerifyingKey};
use goodfetch_notification::driver::Driver;
use goodfetch_notification::ledger::{self, Entry, Ledger};
use goodfetch_notification::message::{EventId, Message};
use goodfetch_notification::rate_limit::RateLimit;
use goodfetch_notification::webhook::{Payload, Webhook};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::notifications::NotificationEvent;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use httpmock::MockServer;
use secrecy::SecretString;
use serde_json::{Value, json};
use url::Url;

struct CapturedDelivery {
    id: String,
    timestamp: String,
    signature: String,
    body: String,
}

type DeliveryLog = Arc<Mutex<Vec<CapturedDelivery>>>;

fn runtime() -> tokio::runtime::Runtime {
    goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

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

async fn mock_trigger(server: &MockServer, captured: DeliveryLog) -> httpmock::Mock<'_> {
    server
        .mock_async(move |when, then| {
            when.method(httpmock::Method::POST).path("/trigger");
            then.status(200)
                .respond_with(move |req: &httpmock::HttpMockRequest| {
                    let headers = req.headers();
                    let header = |name: &str| {
                        headers
                            .iter()
                            .find(|(k, _)| k.as_str().eq_ignore_ascii_case(name))
                            .and_then(|(_, v)| v.to_str().ok())
                            .unwrap_or_default()
                            .to_owned()
                    };
                    let body = String::from_utf8_lossy(req.body().as_ref()).into_owned();
                    captured.lock().unwrap().push(CapturedDelivery {
                        id: header("webhook-id"),
                        timestamp: header("webhook-timestamp"),
                        signature: header("webhook-signature"),
                        body,
                    });
                    httpmock::HttpMockResponse {
                        status: Some(200),
                        headers: None,
                        body: Some(b"ok".to_vec().into()),
                    }
                });
        })
        .await
}

/// Covers: story:11-255
#[test]
fn thin_webhook_trigger_fires_once_verifies_signature_and_never_repeats_on_resume() {
    let env = TestEnv::new();
    let db_path = env.folder().join("run.sqlite");

    runtime().block_on(async {
        let server = MockServer::start_async().await;
        let captured: DeliveryLog = Arc::new(Mutex::new(Vec::new()));
        let mock = mock_trigger(&server, Arc::clone(&captured)).await;

        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let run_state = RunState::open(
            db_path.clone(),
            &RunStateOptions::default(),
            vec![ledger::TABLES],
            1,
            &owner,
        )
        .await
        .unwrap();

        let clock = Clock::system();
        let now_secs =
            i64::try_from(clock.wall().duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap();
        let endpoint =
            NotificationEndpoint::of(&settings(json!({})), clock, owner.blocking_threads());
        let rate_limit = Arc::new(RateLimit::new(clock));
        let url = Url::parse(&server.url("/trigger")).unwrap();
        let secret_str = format!("whsec_{}", data_encoding::BASE64.encode(&[0x42u8; 32]));

        let cloud_endpoint = NotificationEndpoint::of(
            &settings(json!({ "egress.no_private_endpoints": true })),
            clock,
            owner.blocking_threads(),
        );
        assert!(cloud_endpoint.judge(&url).is_err());

        let message = Message {
            event_id: EventId::new("evt_01JRESUME00000000000001").unwrap(),
            event: NotificationEvent::RUN_FINISHED,
            run_id: Some(RunId::parse("resume-trigger-1").unwrap()),
            schedule_id: None,
            product: Product::Collie,
            title: "Run finished".to_owned(),
            text: "Done".to_owned(),
            link: None,
            occurred_at: clock.wall(),
        };

        let key = SigningKey::from_secret(&SecretString::from(secret_str.clone())).unwrap();
        let signer = Signer::new(vec![key], None).unwrap();
        let channel =
            Webhook::new("n8n-trigger", url, signer, Payload::Thin, endpoint, clock).unwrap();

        let driver1 = Driver::new(Ledger::new(run_state), Arc::clone(&rate_limit), clock);
        assert!(matches!(
            driver1.deliver(&channel, &message).await.unwrap(),
            Entry::Delivered { attempt: 0 }
        ));

        let resumed_state = RunState::open(
            db_path,
            &RunStateOptions::default(),
            vec![ledger::TABLES],
            2,
            &owner,
        )
        .await
        .unwrap();
        let driver2 = Driver::new(Ledger::new(resumed_state), Arc::clone(&rate_limit), clock);
        assert!(matches!(
            driver2.deliver(&channel, &message).await.unwrap(),
            Entry::Delivered { attempt: 0 }
        ));

        mock.assert_calls_async(1).await;
        let received = captured.lock().unwrap();
        assert_eq!(received.len(), 1);
        let delivery = &received[0];
        let verifier = Verifier::new(
            vec![VerifyingKey::from_secret(&SecretString::from(secret_str)).unwrap()],
            None,
        )
        .unwrap();
        verifier
            .verify_signature(
                &Received {
                    id: Some(&delivery.id),
                    timestamp: Some(&delivery.timestamp),
                    signature: Some(&delivery.signature),
                    body: delivery.body.as_bytes(),
                },
                now_secs,
            )
            .expect("delivered webhook signature must verify");
        let parsed: Value = serde_json::from_str(&delivery.body).unwrap();
        assert_eq!(parsed["type"], "notification.run_finished");
        assert!(parsed["data"].get("text").is_none());
    });
}
