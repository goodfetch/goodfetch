//! Conformance tests for signed Webhook Sinks (ADRs 0008, 0017 and 0026;
//! issue 1166) against a real local HTTP receiver on loopback:
//! Standard Webhooks signature verification, replay delivery-ID preservation,
//! 256 KB batch cap, thin payload shape, retry schedule, HTTP 410 pause, and
//! `setting_invalid` refusal when `webhook.signing_secret` is absent.
#![expect(
    clippy::disallowed_methods,
    reason = "integration tests bind a local HTTP receiver and run on Tokio's runtime"
)]
#![expect(
    clippy::disallowed_types,
    reason = "local loopback HTTP test receiver records deliveries in std::sync::Mutex"
)]

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::notification_endpoint::NotificationEndpoint;
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::webhook_signing::{
    ID_HEADER, Received, SIGNATURE_HEADER, Signer, SigningKey, TIMESTAMP_HEADER, Verifier,
    VerifyingKey,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::foxhound::sink::WebhookPayload;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_sink::{
    DEFAULT_MAX_BATCH_BYTES, Driver, PauseReason, RunCopy, Sinks, Status, WebhookConfig,
    WebhookSink,
};
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use secrecy::SecretString;
use serde_json::json;
use tokio::net::TcpListener;
use url::Url;

use crate::run_files::run_file;

/// Visibly fake 32-byte all-zero test key in Standard Webhooks `whsec_` format
/// (never a real or published vector).
const TEST_SIGNING_SECRET: &str = "whsec_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

#[derive(Clone, Debug)]
struct CapturedDelivery {
    id: Option<String>,
    timestamp: Option<String>,
    signature: Option<String>,
    body: Vec<u8>,
}

#[derive(Clone, Debug)]
struct ScriptedStatus {
    status: StatusCode,
    retry_after: Option<String>,
}

struct Receiver {
    address: SocketAddr,
    captured: Arc<Mutex<Vec<CapturedDelivery>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Receiver {
    async fn start(script: Vec<ScriptedStatus>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let captured = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(VecDeque::from(script)));
        let cap_clone = Arc::clone(&captured);
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let cap = Arc::clone(&cap_clone);
                let reps = Arc::clone(&replies);
                tokio::spawn(async move {
                    let service = service_fn(move |req: Request<Incoming>| {
                        let cap = Arc::clone(&cap);
                        let reps = Arc::clone(&reps);
                        async move {
                            let header = |name: &str| {
                                req.headers()
                                    .get(name)
                                    .and_then(|v| v.to_str().ok())
                                    .map(str::to_owned)
                            };
                            let id = header(ID_HEADER);
                            let timestamp = header(TIMESTAMP_HEADER);
                            let signature = header(SIGNATURE_HEADER);
                            let body = req.into_body().collect().await.unwrap().to_bytes().to_vec();
                            cap.lock().unwrap().push(CapturedDelivery {
                                id,
                                timestamp,
                                signature,
                                body,
                            });
                            let reply =
                                reps.lock().unwrap().pop_front().unwrap_or(ScriptedStatus {
                                    status: StatusCode::OK,
                                    retry_after: None,
                                });
                            let mut builder = Response::builder().status(reply.status);
                            if let Some(ra) = reply.retry_after {
                                builder = builder.header("retry-after", ra);
                            }
                            Ok::<_, hyper::Error>(
                                builder.body(Full::new(Bytes::from_static(b"ok"))).unwrap(),
                            )
                        }
                    });
                    let _ = http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        });
        Self {
            address,
            captured,
            task,
        }
    }

    fn url(&self) -> Url {
        Url::parse(&format!("http://{}/hook", self.address)).unwrap()
    }

    fn deliveries(&self) -> Vec<CapturedDelivery> {
        self.captured.lock().unwrap().clone()
    }
}

fn test_signer_and_verifier() -> (Signer, Verifier) {
    let secret = SecretString::from(TEST_SIGNING_SECRET.to_owned());
    let signing = SigningKey::from_secret(&secret).unwrap();
    let verifying = VerifyingKey::from_secret(&secret).unwrap();
    (
        Signer::new(vec![signing], None).unwrap(),
        Verifier::new(vec![verifying], None).unwrap(),
    )
}

fn local_endpoint(clock: Clock) -> NotificationEndpoint {
    let resolved = resolve(&Layers::default(), Product::Foxhound, Product::Foxhound).unwrap();
    NotificationEndpoint::of(
        &resolved,
        clock,
        TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
    )
}

fn verify_delivery(
    verifier: &Verifier,
    delivery: &CapturedDelivery,
    now_secs: i64,
) -> serde_json::Value {
    let checked = verifier
        .verify(
            &Received {
                id: delivery.id.as_deref(),
                timestamp: delivery.timestamp.as_deref(),
                signature: delivery.signature.as_deref(),
                body: &delivery.body,
            },
            now_secs,
        )
        .expect("Standard Webhooks signature must verify");
    json!({
        "run_epoch": checked.run_epoch,
        "data": checked.data,
    })
}

fn new_driver(
    receiver: &Receiver,
    clock: Clock,
    run: RunCopy,
    dataset: &str,
) -> Driver<WebhookSink> {
    let sink = WebhookSink::new(
        "hooks",
        receiver.url(),
        test_signer_and_verifier().0,
        local_endpoint(clock),
        clock,
        WebhookConfig::default(),
    )
    .unwrap();
    Driver::new("hooks", sink, run, vec![dataset.to_owned()]).with_clock(clock)
}

async fn deliver_to(
    driver: &mut Driver<WebhookSink>,
    file: &rusqlite::Connection,
    sink: &WebhookSink,
) -> Status {
    Box::pin(driver.deliver_with(file, |batch| async move {
        sink.send_batch(&batch, Duration::from_secs(10)).await
    }))
    .await
    .unwrap()
}

/// Signed webhook deliveries verify under Standard Webhooks, a replayed
/// delivery keeps the exact same delivery ID, and batches never exceed 256 KB
/// (ADRs 0008, 0017 and 0026).
///
/// Covers: story:6-296
#[tokio::test]
async fn signed_batch_verifies_replay_preserves_id_and_256kb_cap_holds() {
    let clock = Clock::system();
    let receiver = Receiver::start(Vec::new()).await;
    let (signer, verifier) = test_signer_and_verifier();
    let sink = WebhookSink::new(
        "hooks",
        receiver.url(),
        signer,
        local_endpoint(clock),
        clock,
        WebhookConfig::default(),
    )
    .unwrap();

    // 4 Records of 100 KB each (~400 KB total) must split so no POST exceeds 256 KB.
    let big = "a".repeat(100 * 1024);
    let file = run_file(&[(
        "items",
        (1..=4)
            .map(|n| json!({ "n": n, "blob": big }))
            .collect::<Vec<_>>(),
    )]);
    let run = RunCopy {
        run_id: "webhook-run".to_owned(),
        run_epoch: 3,
    };
    let mut driver = new_driver(&receiver, clock, run.clone(), "items");
    assert_eq!(
        deliver_to(&mut driver, &file, &sink).await,
        Status::Complete
    );

    let now_secs =
        i64::try_from(clock.wall().duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap();
    let deliveries = receiver.deliveries();
    assert!(
        deliveries.len() >= 2,
        "400 KB batch must split into multiple <= 256 KB requests, got {}",
        deliveries.len()
    );
    let mut seen_numbers = Vec::new();
    for delivery in &deliveries {
        assert!(
            delivery.body.len() <= DEFAULT_MAX_BATCH_BYTES,
            "webhook body {} exceeded 256 KB cap",
            delivery.body.len()
        );
        let checked = verify_delivery(&verifier, delivery, now_secs);
        assert_eq!(checked["run_epoch"], json!(3));
        assert_eq!(checked["data"]["shape"], json!("batch"));
        for rec in checked["data"]["records"].as_array().unwrap() {
            seen_numbers.push(rec["number"].as_u64().unwrap());
        }
    }
    assert_eq!(seen_numbers, vec![1, 2, 3, 4]);

    // Replaying the same batch produces the exact same `webhook-id`.
    let first_id = deliveries[0].id.clone().unwrap();
    let first_batch_count = deliveries.len();
    let mut replay_driver = new_driver(&receiver, clock, run, "items");
    assert_eq!(
        deliver_to(&mut replay_driver, &file, &sink).await,
        Status::Complete
    );
    let after_replay = receiver.deliveries();
    assert_eq!(after_replay.len(), first_batch_count * 2);
    assert_eq!(
        after_replay[first_batch_count].id.as_deref(),
        Some(first_id.as_str())
    );
}

/// Thin webhook payloads carry the record range (`from`, `through`, `count`)
/// and a `fetch_url` inside the signed Standard Webhooks envelope (ADR 0026).
///
/// Covers: story:6-296
#[tokio::test]
async fn thin_payload_carries_record_range_and_fetch_url() {
    let clock = Clock::system();
    let receiver = Receiver::start(Vec::new()).await;
    let (signer, verifier) = test_signer_and_verifier();
    let sink = WebhookSink::new(
        "hooks",
        receiver.url(),
        signer,
        local_endpoint(clock),
        clock,
        WebhookConfig {
            payload: WebhookPayload::Thin,
            max_batch_bytes: DEFAULT_MAX_BATCH_BYTES,
            fetch_base_url: "https://api.example.test/runs".to_owned(),
        },
    )
    .unwrap();
    let file = run_file(&[(
        "products",
        vec![json!({ "id": 1 }), json!({ "id": 2 }), json!({ "id": 3 })],
    )]);
    let mut driver = new_driver(
        &receiver,
        clock,
        RunCopy {
            run_id: "thin-run".to_owned(),
            run_epoch: 2,
        },
        "products",
    );
    assert_eq!(
        deliver_to(&mut driver, &file, &sink).await,
        Status::Complete
    );

    let now_secs =
        i64::try_from(clock.wall().duration_since(UNIX_EPOCH).unwrap().as_secs()).unwrap();
    let deliveries = receiver.deliveries();
    assert_eq!(deliveries.len(), 1);
    let checked = verify_delivery(&verifier, &deliveries[0], now_secs);
    assert_eq!(checked["run_epoch"], json!(2));
    assert_eq!(checked["data"]["shape"], json!("thin"));
    assert_eq!(checked["data"]["from"], json!(1));
    assert_eq!(checked["data"]["through"], json!(3));
    assert_eq!(checked["data"]["count"], json!(3));
    assert_eq!(
        checked["data"]["fetch_url"],
        json!("https://api.example.test/runs/thin-run/datasets/products/records?after=0&through=3")
    );
}

/// Webhook retries follow the Standard Webhooks schedule (honouring
/// `Retry-After`) and HTTP 410 Gone pauses the Sink (ADR 0026).
///
/// Covers: story:6-296
#[tokio::test]
async fn webhook_retries_follow_schedule_and_410_pauses_the_sink() {
    let clock = Clock::system();
    let receiver = Receiver::start(vec![
        ScriptedStatus {
            status: StatusCode::SERVICE_UNAVAILABLE,
            retry_after: Some("7".to_owned()),
        },
        ScriptedStatus {
            status: StatusCode::OK,
            retry_after: None,
        },
        ScriptedStatus {
            status: StatusCode::GONE,
            retry_after: None,
        },
    ])
    .await;
    let (signer, _) = test_signer_and_verifier();
    let sink = WebhookSink::new(
        "hooks",
        receiver.url(),
        signer,
        local_endpoint(clock),
        clock,
        WebhookConfig::default(),
    )
    .unwrap();
    let file = run_file(&[("items", vec![json!({ "id": 1 })])]);
    let mut driver = new_driver(
        &receiver,
        clock,
        RunCopy {
            run_id: "retry-webhook".to_owned(),
            run_epoch: 1,
        },
        "items",
    );

    // 1. 503 with Retry-After: 7 -> Behind with retry_in == 7 s.
    let first = deliver_to(&mut driver, &file, &sink).await;
    assert!(
        matches!(first, Status::Behind { retry_in, .. } if retry_in == Duration::from_secs(7)),
        "{first:?}"
    );

    // 2. Retry succeeds with HTTP 200 and preserves exact same webhook-id.
    assert_eq!(
        deliver_to(&mut driver, &file, &sink).await,
        Status::Complete
    );
    let deliveries = receiver.deliveries();
    assert_eq!(deliveries[0].id, deliveries[1].id);

    // 3. HTTP 410 Gone pauses the Sink immediately.
    let file_gone = run_file(&[("items", vec![json!({ "id": 99 })])]);
    let mut gone_driver = new_driver(
        &receiver,
        clock,
        RunCopy {
            run_id: "gone-webhook".to_owned(),
            run_epoch: 1,
        },
        "items",
    );
    let paused = deliver_to(&mut gone_driver, &file_gone, &sink).await;
    assert!(
        matches!(
            paused,
            Status::Paused {
                reason: PauseReason::Gone,
                ..
            }
        ),
        "{paused:?}"
    );
}

/// A webhook Sink with no `webhook.signing_secret` is refused at start with
/// `setting_invalid` naming `sinks.<name>.webhook.signing_secret`, while one
/// carrying a Secret reference is accepted (ADRs 0008 and 0026).
///
/// Covers: story:6-296
#[test]
fn webhook_without_signing_secret_is_refused_at_start_with_setting_invalid() {
    let serde_json::Value::Object(missing_secret) = json!({
        "sinks": {
            "hooks": {
                "type": "webhook",
                "destination": "env:HOOK_URL",
                "datasets": ["pages"]
            }
        }
    }) else {
        unreachable!("object");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &missing_secret, Product::Foxhound).unwrap(),
        ..Layers::default()
    };
    let resolved = resolve(&layers, Product::Foxhound, Product::Foxhound).unwrap();
    let refusal = Sinks::new()
        .check(&resolved, &[])
        .expect_err("webhook without signing_secret must be refused");
    assert_eq!(refusal.code(), "setting_invalid");
    assert!(
        refusal
            .message()
            .contains("sinks.hooks.webhook.signing_secret"),
        "{refusal}"
    );

    let serde_json::Value::Object(with_secret) = json!({
        "sinks": {
            "hooks": {
                "type": "webhook",
                "destination": "env:HOOK_URL",
                "datasets": ["pages"],
                "webhook": {
                    "signing_secret": "env:HOOK_SIGNING_KEY"
                }
            }
        }
    }) else {
        unreachable!("object");
    };
    let ok_layers = Layers {
        run: Values::read(SettingsLayer::Run, &with_secret, Product::Foxhound).unwrap(),
        ..Layers::default()
    };
    let ok_resolved = resolve(&ok_layers, Product::Foxhound, Product::Foxhound).unwrap();
    Sinks::new()
        .check(&ok_resolved, &[])
        .expect("webhook with signing_secret is accepted");
}
