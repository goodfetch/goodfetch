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

/// Composes a real engine with `goodfetch_notification::contribution()`,
/// subscribes a webhook over RPC, runs a Run to completion, and proves:
/// 1. `notification.subscribe` generates distinct OS-random `whsec_` secrets.
/// 2. Ending a Run automatically delivers a signed thin webhook through
///    `SubscriptionStore::deliver()` without any manual driver call.
/// 3. `SubscriptionStore` enforces its bounded capacity (`with_capacity`).
///
/// Covers: story:11-255
#[test]
fn engine_run_end_delivers_subscribed_webhook_and_enforces_capacity() {
    use goodfetch_engine::entropy::Entropy;
    use goodfetch_engine::home::{Home, RunStateLocation};
    use goodfetch_engine::politeness::PolitenessOptions;
    use goodfetch_engine::secrets::Keychain;
    use goodfetch_engine::transport::{self, Network};
    use goodfetch_engine::{Composition, Dependencies, Engine, Identity};
    use goodfetch_notification::subscriptions::{SubscriptionStore, contribution_with_store};
    use goodfetch_protocol::connection::ProductVersion;
    use std::collections::BTreeMap;

    let env = TestEnv::new();
    runtime().block_on(async {
        let server = MockServer::start_async().await;
        let captured: DeliveryLog = Arc::new(Mutex::new(Vec::new()));
        let mock = mock_trigger(&server, Arc::clone(&captured)).await;

        let store = SubscriptionStore::with_capacity(2);
        let composition = Composition::default()
            .with(goodfetch_engine::contribution())
            .with(contribution_with_store(store.clone()));
        let engine = Engine::new(
            Identity {
                product: Product::Collie,
                product_version: ProductVersion::parse("0.1.0").unwrap(),
                channel: goodfetch_engine::Channel::CratesIo,
            },
            &composition,
        )
        .unwrap();

        let (network, dialer) = transport::in_memory();
        let deps = Dependencies {
            home: Home::new(env.folder().to_path_buf()),
            clock: Clock::system(),
            entropy: Entropy::from_os().unwrap(),
            blocking: BlockingPolicy::Inline,
            run_state_location: RunStateLocation::default(),
            run_state: RunStateOptions::default(),
            politeness: PolitenessOptions::default(),
            environment: BTreeMap::new(),
            keychain: Keychain::absent(),
        };

        let engine_task = tokio::task::LocalSet::new();
        let webhook_url = server.url("/trigger");
        let now_secs = i64::try_from(
            Clock::system()
                .wall()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        )
        .unwrap();

        let (secret1, secret2) = Box::pin(engine_task.run_until(async move {
            let server_fut = Box::pin(engine.run_attached(Network::InMemory(network), deps));
            let client_fut = Box::pin(async move {
                let conn = dialer.connect().await.unwrap();
                drive_subscription_and_run_client(conn, webhook_url).await
            });
            let (engine_res, secrets) = tokio::join!(server_fut, client_fut);
            engine_res.unwrap();
            secrets
        }))
        .await;

        assert_ne!(secret1, secret2);
        mock.assert_calls_async(1).await;
        let received = captured.lock().unwrap();
        assert_eq!(received.len(), 1);
        let delivery = &received[0];
        let verifier = Verifier::new(
            vec![VerifyingKey::from_secret(&SecretString::from(secret1)).unwrap()],
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
            .expect("engine-delivered webhook signature must verify");
        let parsed: Value = serde_json::from_str(&delivery.body).unwrap();
        assert_eq!(parsed["type"], "notification.run_finished");
        assert_eq!(parsed["data"]["run_id"], "e2e-webhook-run-1");
        assert!(parsed["data"].get("text").is_none());
    });
}

async fn send_jsonrpc(
    reader: &mut tokio::io::BufReader<impl tokio::io::AsyncRead + Unpin>,
    write: &mut (impl tokio::io::AsyncWrite + Unpin),
    req: Value,
) -> Value {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let mut line = serde_json::to_vec(&req).unwrap();
    line.push(b'\n');
    write.write_all(&line).await.unwrap();
    write.flush().await.unwrap();
    let mut resp = String::new();
    reader.read_line(&mut resp).await.unwrap();
    serde_json::from_str(&resp).unwrap()
}

#[expect(
    clippy::too_many_lines,
    reason = "end-to-end JSON-RPC scenario testing subscribe capacity, idempotency, and run-end delivery"
)]
async fn drive_subscription_and_run_client(
    conn: goodfetch_engine::transport::Connection,
    webhook_url: String,
) -> (String, String) {
    use tokio::io::BufReader;

    let (read, mut write) = conn.split();
    let mut reader = BufReader::new(read);

    let hello = send_jsonrpc(
        &mut reader,
        &mut write,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "hello",
            "params": {
                "protocol_version": "0.1",
                "surface": "http_listener",
                "needs": "collie",
                "product_version": "0.1.0"
            }
        }),
    )
    .await;
    assert!(hello.get("result").is_some(), "hello failed: {hello}");

    let sub1 = send_jsonrpc(
        &mut reader,
        &mut write,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "notification.subscribe",
            "params": {
                "channel": "trigger-1",
                "url": webhook_url,
                "events": ["run_finished"]
            }
        }),
    )
    .await;
    let secret1 = sub1["result"]["secret"].as_str().unwrap().to_owned();
    assert!(secret1.starts_with("whsec_"));

    let sub2 = send_jsonrpc(
        &mut reader,
        &mut write,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "notification.subscribe",
            "params": {
                "channel": "trigger-2",
                "url": "https://hooks.example.com/other",
                "events": ["run_failed"]
            }
        }),
    )
    .await;
    let secret2 = sub2["result"]["secret"].as_str().unwrap().to_owned();
    assert!(secret2.starts_with("whsec_"));
    assert_ne!(
        secret1, secret2,
        "each subscription must get a distinct OS-random secret"
    );

    let sub3 = send_jsonrpc(
        &mut reader,
        &mut write,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "notification.subscribe",
            "params": {
                "channel": "trigger-3",
                "url": "https://hooks.example.com/third",
                "events": ["run_failed"]
            }
        }),
    )
    .await;
    assert!(
        sub3.get("error").is_some(),
        "subscription past capacity must be refused: {sub3}"
    );

    let sub1_again = send_jsonrpc(
        &mut reader,
        &mut write,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "notification.subscribe",
            "params": {
                "channel": "trigger-1",
                "url": webhook_url,
                "events": ["run_finished"]
            }
        }),
    )
    .await;
    assert_eq!(sub1_again["result"]["created"], false);
    assert!(sub1_again["result"].get("secret").is_none());

    let started = send_jsonrpc(
        &mut reader,
        &mut write,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "run.start",
            "params": { "run_id": "e2e-webhook-run-1" }
        }),
    )
    .await;
    assert!(
        started.get("result").is_some(),
        "run.start failed: {started}"
    );

    let finished = send_jsonrpc(
        &mut reader,
        &mut write,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "run.finish",
            "params": { "run_id": "e2e-webhook-run-1" }
        }),
    )
    .await;
    assert!(
        finished.get("result").is_some(),
        "run.finish failed: {finished}"
    );

    drop(write);
    drop(reader);
    (secret1, secret2)
}
