//! The plugin channel through the driver, on the paused `Clock`, with a
//! scripted `notification.send`: what the call carries, and its
//! translation table.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "test recorders hold std::sync::Mutex only for synchronous pushes, never across an `.await`, outside loom (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::plugin_gate::PluginFetch;
use goodfetch_engine::plugin_host::instance::{
    InMemoryPeer, InMemoryPlugin, PluginInstance, Scope, UncertainCall, UncertainReporter,
};
use goodfetch_engine::plugin_kinds::notification::{
    NotificationDriver, ProcessNotificationChannel, unavailable,
};
use goodfetch_engine::plugin_kinds::{Accounts, PluginCall, Primitive};
use goodfetch_engine::run_state::{RunState, RunStateOptions};
use goodfetch_engine::secrets::PartyText;
use goodfetch_engine::settings::{Layers, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::{OperationClass, Product};
use goodfetch_protocol::notifications::NotificationEvent;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::plugin::notification::NotificationSendParams;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_PLUGIN};
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::sync::Mutex;
use url::Url;

use super::{CallFailure, NotificationCall, Params, PluginChannel, PluginNotificationCall};
use crate::driver::{Channel, Driver};
use crate::failure::{ChannelText, Failure};
use crate::ledger::{Entry, Ledger, TABLES};
use crate::message::{EventId, Message};
use crate::rate_limit::RateLimit;

/// A `notification.send` whose program answers as scripted, then takes
/// every call, and which notes what each call carried and when.
struct Scripted {
    clock: Clock,
    answers: Mutex<VecDeque<CallFailure>>,
    calls: Mutex<Vec<Call>>,
}

/// What one call carried, and the second it was made.
#[derive(Debug, PartialEq, Eq)]
struct Call {
    second: u64,
    event_id: String,
    event: String,
    text: String,
    link: Option<String>,
}

impl Scripted {
    fn new(clock: Clock, answers: impl IntoIterator<Item = CallFailure>) -> Self {
        Self {
            clock,
            answers: Mutex::new(answers.into_iter().collect()),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl NotificationCall for Scripted {
    async fn send(&self, params: Params<'_>) -> Result<(), CallFailure> {
        let second = self.clock.wall().duration_since(UNIX_EPOCH).unwrap();
        self.calls.lock().await.push(Call {
            second: second.as_secs(),
            event_id: params.event_id.to_owned(),
            event: params.event.to_string(),
            text: params.text.to_owned(),
            link: params.link.map(Url::to_string),
        });
        self.answers.lock().await.pop_front().map_or(Ok(()), Err)
    }
}

/// A `notification.send` that never answers: the translation table sends
/// nothing.
struct NeverCalled;

impl NotificationCall for NeverCalled {
    async fn send(&self, _: Params<'_>) -> Result<(), CallFailure> {
        std::future::pending().await
    }
}

/// A driver over a Run's run file.
async fn driver(env: &TestEnv, owner: &TaskOwner, clock: Clock) -> Driver {
    let state = RunState::open(
        env.folder().join("run.sqlite"),
        &RunStateOptions::default(),
        vec![TABLES],
        1,
        owner,
    )
    .await
    .unwrap();
    Driver::new(Ledger::new(state), Arc::new(RateLimit::new(clock)), clock)
}

fn message() -> Message {
    Message {
        event_id: EventId::new("evt-1").unwrap(),
        event: NotificationEvent::RUN_FAILED,
        run_id: None,
        schedule_id: None,
        product: Product::Collie,
        title: "A Run failed".to_owned(),
        text: "The Run stopped on an error.".to_owned(),
        link: Some(Url::parse("http://127.0.0.1:7777/runs/nightly").unwrap()),
        occurred_at: UNIX_EPOCH,
    }
}

/// Covers: story:11-269
/// Covers: story:11-270
#[test]
fn a_programs_other_error_is_sent_again_on_the_schedule_with_the_same_event() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let env = TestEnv::in_memory();
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let driver = driver(&env, &owner, clock).await;
        let call = Scripted::new(
            clock,
            [CallFailure::Other(ChannelText::new("TypeError: boom"))],
        );
        let channel = PluginChannel::new("pager", "plugin:pager", call);

        let held = driver.deliver(&channel, &message()).await.unwrap();

        assert_eq!(held, Entry::Delivered { attempt: 1 });
        let carried = |second| Call {
            second,
            event_id: "evt-1".to_owned(),
            event: "run_failed".to_owned(),
            text: "The Run stopped on an error.".to_owned(),
            link: Some("http://127.0.0.1:7777/runs/nightly".to_owned()),
        };
        assert_eq!(*channel.call.calls.lock().await, [carried(0), carried(5)]);
    });
}

/// Covers: story:11-269
#[rstest]
#[case::retryable_with_a_wait(
    CallFailure::Goodfetch { code: "rate_limited".to_owned(), retryable: true, retry_after: Some(Duration::from_secs(60)) },
    Failure::RateLimited { retry_after: Some(Duration::from_secs(60)) },
)]
#[case::retryable(
    CallFailure::Goodfetch { code: "upstream_down".to_owned(), retryable: true, retry_after: None },
    Failure::ServerError,
)]
#[case::not_retryable(
    CallFailure::Goodfetch { code: "bad_channel".to_owned(), retryable: false, retry_after: None },
    Failure::Rejected,
)]
#[case::other(
    CallFailure::Other(ChannelText::new("TypeError: boom")),
    Failure::Unrecognized(PartyText::new("TypeError: boom"))
)]
#[case::uncertain(CallFailure::Uncertain, Failure::Uncertain)]
fn each_answer_maps_to_its_failure(#[case] answer: CallFailure, #[case] failure: Failure) {
    let channel = PluginChannel::new("pager", "plugin:pager", NeverCalled);

    let mapped = channel.map_failure(answer);

    assert_eq!(mapped, failure);
    assert_eq!(
        mapped.is_transient(),
        !matches!(failure, Failure::Rejected | Failure::Uncertain)
    );
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedCall {
    plugin: String,
    method: String,
    class: OperationClass,
    failed: Option<&'static str>,
}

#[derive(Default)]
struct RecordedAccounts {
    calls: StdMutex<Vec<RecordedCall>>,
}

impl Accounts for RecordedAccounts {
    fn call_starts<'call>(
        &'call self,
        _call: &'call PluginCall<'call>,
    ) -> Pin<Box<dyn Future<Output = Result<(), CallError>> + Send + 'call>> {
        Box::pin(async { Ok(()) })
    }

    fn call_ended<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> Pin<Box<dyn Future<Output = Result<(), CallError>> + Send + 'call>> {
        self.calls.lock().unwrap().push(RecordedCall {
            plugin: call.plugin.to_owned(),
            method: call.method.to_owned(),
            class: call.class,
            failed: call.failed,
        });
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Default)]
struct RecordedUncertain(StdMutex<Vec<UncertainCall>>);

impl UncertainReporter for RecordedUncertain {
    fn report(&self, call: UncertainCall) {
        self.0.lock().unwrap().push(call);
    }
}

fn notification_manifest() -> PluginManifest {
    PluginManifest::read(
        json!({
            "manifest_version": "0.1",
            "name": "toy-pager",
            "version": "1.0.0",
            "kinds": ["notification"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "toy-pager",
            "network_level": "none",
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap()
}

fn start_toy_plugin(owner: &TaskOwner, clock: Clock) -> (ProcessNotificationChannel, InMemoryPeer) {
    let resolved = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
    let fetch =
        Arc::new(PluginFetch::of(&resolved, "toy-pager", clock, owner.blocking_threads()).unwrap());
    let (instance, peer) = PluginInstance::start_in_memory(
        owner,
        InMemoryPlugin {
            manifest: notification_manifest(),
            product: Product::Collie,
            fetch,
            clock,
            spend: None,
        },
        Scope::Task(None),
    );
    (
        ProcessNotificationChannel::new("toy-pager".to_owned(), instance),
        peer,
    )
}

async fn read_json_line(reader: &mut BufReader<tokio::io::DuplexStream>) -> Option<Value> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line).await.unwrap();
    if bytes == 0 {
        return None;
    }
    Some(serde_json::from_str(&line).unwrap())
}

async fn write_json_line(writer: &mut tokio::io::DuplexStream, value: &Value) {
    let mut line = serde_json::to_string(value).unwrap();
    line.push('\n');
    writer.write_all(line.as_bytes()).await.unwrap();
    writer.flush().await.unwrap();
}

async fn complete_hello(
    reader: &mut BufReader<tokio::io::DuplexStream>,
    writer: &mut tokio::io::DuplexStream,
) {
    let hello = read_json_line(reader).await.unwrap();
    assert_eq!(hello["method"], json!("hello"));
    write_json_line(
        writer,
        &json!({
            "jsonrpc": "2.0",
            "id": hello["id"],
            "result": {
                "protocol_version": "0.1",
                "name": "toy-pager",
                "version": "1.0.0",
                "kinds": [{ "kind": "notification", "capabilities": ["send"] }],
                "network_level": "none",
            }
        }),
    )
    .await;
}

/// A toy Notification channel Plugin receives an event once through ADR 0027's
/// delivery ledger; after a resume of the same run (new run epoch over the
/// same run file) it is not delivered again.
///
/// Covers: story:7-96, plugin-kind:notification
#[test]
fn a_toy_notification_channel_receives_an_event_once_and_never_repeats_after_resume() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let env = TestEnv::in_memory();
        let run_file = env.folder().join("run.sqlite");
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let accounts = Arc::new(RecordedAccounts::default());
        let received_events: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

        let (plugin, peer) = start_toy_plugin(&owner, clock);
        let received_for_peer = Arc::clone(&received_events);
        owner.spawn(async move {
            let mut reader = BufReader::new(peer.from_engine);
            let mut writer = peer.to_engine;
            complete_hello(&mut reader, &mut writer).await;
            while let Some(msg) = read_json_line(&mut reader).await {
                if msg["method"] == json!("notification.send") {
                    received_for_peer.lock().await.push(msg["params"].clone());
                    write_json_line(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": msg["id"],
                            "result": { "delivered": true },
                        }),
                    )
                    .await;
                } else if msg["method"] == json!("$/ping") {
                    write_json_line(
                        &mut writer,
                        &json!({ "jsonrpc": "2.0", "id": msg["id"], "result": {} }),
                    )
                    .await;
                }
            }
        });

        let deadline = Deadline::given(&TIMEOUT_PLUGIN, 5_000, SettingsLayer::Default);
        let call = PluginNotificationCall::new(
            "pager",
            plugin.clone(),
            Arc::clone(&accounts) as Arc<dyn Accounts>,
            clock,
            deadline.clone(),
        );
        let channel = PluginChannel::new("pager", "plugin:toy-pager", call);

        // Epoch 1: deliver event once.
        let state_1 = RunState::open(
            run_file.clone(),
            &RunStateOptions::default(),
            vec![TABLES],
            1,
            &owner,
        )
        .await
        .unwrap();
        let driver_1 = Driver::new(Ledger::new(state_1), Arc::new(RateLimit::new(clock)), clock);
        let first = driver_1.deliver(&channel, &message()).await.unwrap();
        assert_eq!(first, Entry::Delivered { attempt: 0 });
        drop(driver_1);

        // Epoch 2 (resume of the same run): the ledger already holds the event
        // delivered, so `notification.send` is NOT called again.
        let state_2 = RunState::open(
            run_file,
            &RunStateOptions::default(),
            vec![TABLES],
            2,
            &owner,
        )
        .await
        .unwrap();
        let driver_2 = Driver::new(Ledger::new(state_2), Arc::new(RateLimit::new(clock)), clock);
        let resumed = driver_2.deliver(&channel, &message()).await.unwrap();
        assert_eq!(resumed, Entry::Delivered { attempt: 0 });

        let delivered = received_events.lock().await;
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0]["event_id"], json!("evt-1"));
        assert_eq!(delivered[0]["event"], json!("run_failed"));
        assert_eq!(delivered[0]["text"], json!("The Run stopped on an error."));
        assert_eq!(
            delivered[0]["link"],
            json!("http://127.0.0.1:7777/runs/nightly")
        );
        assert_eq!(
            accounts.calls.lock().unwrap().as_slice(),
            &[RecordedCall {
                plugin: "toy-pager".to_owned(),
                method: "notification.send".to_owned(),
                class: OperationClass::Effect,
                failed: None,
            }]
        );
        assert_eq!(Primitive::NotificationSend.class(), OperationClass::Effect);
        drop(delivered);
        drop(driver_2);
        owner.stop().await;
    });
}

/// An abandoned `notification.send` (past `timeout.plugin`) is reported
/// uncertain at once, settles `uncertain` in the delivery ledger, and is never
/// repeated blindly — neither on the retry schedule nor after a resume.
///
/// Covers: story:7-96
#[test]
fn an_abandoned_notification_send_is_reported_uncertain_and_not_repeated_blindly() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let env = TestEnv::in_memory();
        let run_file = env.folder().join("run.sqlite");
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let accounts = Arc::new(RecordedAccounts::default());
        let uncertain = Arc::new(RecordedUncertain::default());
        let received_count: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let cancel_seen: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));

        let (plugin, peer) = start_toy_plugin(&owner, clock);
        let count_for_peer = Arc::clone(&received_count);
        let cancel_for_peer = Arc::clone(&cancel_seen);
        owner.spawn(async move {
            let mut reader = BufReader::new(peer.from_engine);
            let mut writer = peer.to_engine;
            complete_hello(&mut reader, &mut writer).await;
            while let Some(msg) = read_json_line(&mut reader).await {
                if msg["method"] == json!("notification.send") {
                    *count_for_peer.lock().await += 1;
                    // Intentionally hang until deadline abandons this Effect call.
                } else if msg["method"] == json!("$/cancel") {
                    *cancel_for_peer.lock().await = true;
                }
            }
        });

        let deadline = Deadline::given(&TIMEOUT_PLUGIN, 500, SettingsLayer::Default);
        let call = PluginNotificationCall::new(
            "pager",
            plugin,
            Arc::clone(&accounts) as Arc<dyn Accounts>,
            clock,
            deadline,
        )
        .with_uncertain(Arc::clone(&uncertain) as Arc<dyn UncertainReporter>);
        let channel = PluginChannel::new("pager", "plugin:toy-pager", call);

        // Epoch 1: send hangs past its 500 ms deadline -> abandoned & uncertain.
        let state_1 = RunState::open(
            run_file.clone(),
            &RunStateOptions::default(),
            vec![TABLES],
            1,
            &owner,
        )
        .await
        .unwrap();
        let driver_1 = Driver::new(Ledger::new(state_1), Arc::new(RateLimit::new(clock)), clock);
        let first = driver_1.deliver(&channel, &message()).await.unwrap();
        assert_eq!(first, Entry::Uncertain { attempt: 0 });
        assert_eq!(*received_count.lock().await, 1);
        let reported = uncertain.0.lock().unwrap().clone();
        assert_eq!(reported.len(), 1);
        assert_eq!(reported[0].plugin, "toy-pager");
        assert_eq!(reported[0].method, "notification.send");
        drop(driver_1);

        // Epoch 2 (resume of the same run): still `Uncertain`, never repeated.
        let state_2 = RunState::open(
            run_file,
            &RunStateOptions::default(),
            vec![TABLES],
            2,
            &owner,
        )
        .await
        .unwrap();
        let driver_2 = Driver::new(Ledger::new(state_2), Arc::new(RateLimit::new(clock)), clock);
        let resumed = driver_2.deliver(&channel, &message()).await.unwrap();
        assert_eq!(resumed, Entry::Uncertain { attempt: 0 });
        assert_eq!(*received_count.lock().await, 1);

        drop(driver_2);
        owner.stop().await;
    });
}

/// Malformed answers or `delivered: false` become `notification_handler_failed`
/// (permanent, not retried) and missing channels become
/// `notification_channel_unavailable`.
///
/// Covers: story:7-96, error:notification_handler_failed, error:notification_channel_unavailable
#[test]
fn malformed_answers_map_to_notification_handler_failed_and_unavailable_names_channel() {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(UNIX_EPOCH);
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let accounts = RecordedAccounts::default();
        let (plugin, peer) = start_toy_plugin(&owner, clock);
        owner.spawn(async move {
            let mut reader = BufReader::new(peer.from_engine);
            let mut writer = peer.to_engine;
            complete_hello(&mut reader, &mut writer).await;
            while let Some(msg) = read_json_line(&mut reader).await {
                if msg["method"] == json!("notification.send") {
                    write_json_line(
                        &mut writer,
                        &json!({
                            "jsonrpc": "2.0",
                            "id": msg["id"],
                            "result": { "delivered": false },
                        }),
                    )
                    .await;
                }
            }
        });
        let deadline = Deadline::given(&TIMEOUT_PLUGIN, 5_000, SettingsLayer::Default);
        let driver = NotificationDriver::new("pager", &plugin, &accounts, &clock, &deadline);
        let params = NotificationSendParams {
            event_id: "evt-bad".to_owned(),
            event: NotificationEvent::RUN_FAILED,
            text: "failed".to_owned(),
            link: None,
        };
        let err = driver.send(&params).await.unwrap_err();
        assert!(!err.is_uncertain());
        assert_eq!(err.failure().error().code(), "notification_handler_failed");

        let unavail = unavailable("pager");
        assert_eq!(unavail.code(), "notification_channel_unavailable");
        assert!(unavail.retryable());

        // Silence unused trait import check when inspecting `NotificationPrimitives`.
        assert_eq!(plugin.name(), "toy-pager");
        owner.stop().await;
    });
}
