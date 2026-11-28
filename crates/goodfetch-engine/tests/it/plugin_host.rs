//! A process Plugin host's concurrency credits, deadlines, `$/cancel`,
//! late-result refusal, uncertain Effect reporting and work-item lease
//! bounding (ADRs 0003, 0024, 0029, 0033 and 0080), tested on `Clock::paused`
//! over the plugin protocol's in-memory adapter.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests run on `Clock::paused` (ADR 0059)"
)]

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::{Deadline, Enclosing};
use goodfetch_engine::home::Home;
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::plugin_gate::{OpenStreams, PluginFetch};
use goodfetch_engine::plugin_host::instance::{
    CallFailure, CallOptions, InMemoryPeer, InMemoryPlugin, InstanceHooks, PluginInstance,
    QuarantineEvent, QuarantineNotifier, Scope, UncertainCall, UncertainReporter,
};
use goodfetch_engine::plugin_host::limits::{ResourceMonitor, ResourceSample};
use goodfetch_engine::plugin_kinds::deadline_for;
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{CALL_CANCELLED, EnclosingCall};
use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::plugin::{
    PLUGIN_QUARANTINED_AUDIT, PluginKindName, PluginManifest, PluginResourceLimit,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_HOOK, TIMEOUT_PLUGIN};
use goodfetch_test_support::capturing;
use rstest::rstest;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines, ReadHalf, WriteHalf};
use tokio::time::Instant;
use tracing::Level;

/// One line at a time over an [`InMemoryPeer`], acting as the Plugin end
/// of the plugin protocol.
struct Peer {
    incoming: Lines<BufReader<ReadHalf<tokio::io::DuplexStream>>>,
    outgoing: WriteHalf<tokio::io::DuplexStream>,
}

impl Peer {
    fn new(peer: InMemoryPeer) -> Self {
        let (from_engine, _) = tokio::io::split(peer.from_engine);
        let (_, to_engine) = tokio::io::split(peer.to_engine);
        Self {
            incoming: BufReader::new(from_engine).lines(),
            outgoing: to_engine,
        }
    }

    async fn read(&mut self) -> Value {
        let line = self.incoming.next_line().await.unwrap().unwrap();
        serde_json::from_str(&line).unwrap()
    }

    /// Reads the next non-`$/ping` message, answering any interleaved `$/ping`
    /// heartbeats so long `Clock::paused` deadline tests stay focused on calls.
    async fn read_non_ping(&mut self) -> Value {
        loop {
            let msg = self.read().await;
            if msg["method"] == json!("$/ping") {
                self.send(&json!({ "jsonrpc": "2.0", "id": msg["id"], "result": {} }))
                    .await;
                continue;
            }
            return msg;
        }
    }

    async fn send(&mut self, message: &Value) {
        let mut line = serde_json::to_vec(message).unwrap();
        line.push(b'\n');
        self.outgoing.write_all(&line).await.unwrap();
        self.outgoing.flush().await.unwrap();
    }

    /// Reads the engine's `hello` and replies with the matching approved
    /// manifest fields.
    async fn accept_hello(&mut self) {
        let hello = self.read().await;
        assert_eq!(hello["method"], json!("hello"), "{hello}");
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": hello["id"],
            "result": {
                "protocol_version": "0.1",
                "name": "in-memory",
                "version": "1.0.0",
                "kinds": [{ "kind": "proxy", "capabilities": [] }],
                "network_level": "none",
            },
        }))
        .await;
    }
}

/// Records every uncertain Effect call reported by the Plugin host.
#[derive(Debug, Default)]
struct RecordedUncertain(Mutex<Vec<UncertainCall>>);

impl RecordedUncertain {
    fn snapshot(&self) -> Vec<UncertainCall> {
        self.0.lock().unwrap().clone()
    }
}

impl UncertainReporter for RecordedUncertain {
    fn report(&self, call: UncertainCall) {
        self.0.lock().unwrap().push(call);
    }
}

/// A work-item lease for testing `LeaseCheck::enclosing` and `LeaseCheck::check`.
#[derive(Debug)]
struct TestLease {
    enclosing: Enclosing,
    ended: std::sync::atomic::AtomicBool,
}

impl LeaseCheck for TestLease {
    fn check(&self) -> Result<(), LeaseEnded> {
        if self.ended.load(std::sync::atomic::Ordering::Relaxed) {
            Err(LeaseEnded::Lost)
        } else {
            Ok(())
        }
    }

    fn withheld(&self, ended: LeaseEnded, method: &str) -> CallError {
        CallError::new(
            &CALL_CANCELLED,
            format!("lease {ended}: `{method}` withheld"),
            &(),
        )
    }

    fn enclosing(&self) -> Option<Enclosing> {
        Some(self.enclosing.clone())
    }
}

/// Starts an in-memory instance with `concurrency` and `user` settings on
/// `clock`, returning the instance and its protocol peer.
fn start_in_memory(
    owner: &TaskOwner,
    clock: Clock,
    concurrency: u32,
    user: &Value,
) -> (PluginInstance, Peer) {
    let manifest_json = json!({
        "manifest_version": "0.1",
        "name": "in-memory",
        "version": "1.0.0",
        "kinds": ["proxy"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
        "limits": { "concurrency": concurrency },
    });
    let manifest = PluginManifest::read(manifest_json.to_string().as_bytes()).unwrap();
    let Value::Object(user) = user else {
        panic!("settings are an object");
    };
    let layers = Layers {
        user_configuration: Values::read(SettingsLayer::UserConfiguration, user, Product::Collie)
            .unwrap(),
        ..Layers::default()
    };
    let settings = resolve(&layers, Product::Collie, Product::Collie).unwrap();
    let fetch = PluginFetch::of(&settings, "in-memory", clock, owner.blocking_threads()).unwrap();
    let (instance, peer) = PluginInstance::start_in_memory(
        owner,
        InMemoryPlugin {
            manifest,
            product: Product::Collie,
            fetch: Arc::new(fetch),
            clock,
            spend: None,
        },
        Scope::Task(None),
    );
    (instance, Peer::new(peer))
}

/// A loopback server answering its first request `200` with `hello`.
async fn answering_once(owner: &TaskOwner) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    owner.spawn(async move {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        let (reader, mut writer) = stream.into_split();
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await
            && !line.is_empty()
        {}
        let _ = writer
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello")
            .await;
    });
    address
}

/// Calls past the manifest's concurrency limit wait in FIFO order for a
/// credit with nothing dropped, and a Plugin at its limit can still call
/// back into the engine without deadlocking.
///
/// Covers: story:7-32
#[tokio::test(start_paused = true)]
async fn calls_beyond_concurrency_wait_in_fifo_order_and_callbacks_never_deadlock() {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let endpoint = answering_once(&root).await;
    let user = json!({ "plugins": { "in-memory": { "settings": { "api": endpoint } } } });
    let (instance, mut peer) = start_in_memory(&root, clock, 2, &user);
    peer.accept_hello().await;

    // Launch four calls sequentially into the mailbox while concurrency is 2.
    let c1 = {
        let instance = instance.clone();
        tokio::spawn(async move { instance.call("work.1", &json!({})).await })
    };
    let req1 = peer.read().await;
    assert_eq!(req1["method"], json!("work.1"));

    let c2 = {
        let instance = instance.clone();
        tokio::spawn(async move { instance.call("work.2", &json!({})).await })
    };
    let req2 = peer.read().await;
    assert_eq!(req2["method"], json!("work.2"));

    // Concurrency limit (2) is now full; calls 3 and 4 must wait in FIFO order.
    let c3 = {
        let instance = instance.clone();
        tokio::spawn(async move { instance.call("work.3", &json!({})).await })
    };
    tokio::task::yield_now().await;
    let c4 = {
        let instance = instance.clone();
        tokio::spawn(async move { instance.call("work.4", &json!({})).await })
    };
    tokio::task::yield_now().await;

    // While at its concurrency limit with calls 3 and 4 waiting, the Plugin
    // calls back `net.fetch` naming call 1 and receives its reply (separate pool).
    peer.send(&json!({
        "jsonrpc": "2.0",
        "id": "cb-1",
        "method": "net.fetch",
        "params": {
            "call_id": req1["id"],
            "method": "GET",
            "url": format!("{endpoint}/status"),
        },
    }))
    .await;
    let cb_reply = peer.read().await;
    assert_eq!(cb_reply["id"], json!("cb-1"), "{cb_reply}");
    assert_eq!(cb_reply["result"]["status"], json!(200), "{cb_reply}");

    // Answering call 1 frees one credit -> call 3 must arrive next (FIFO).
    peer.send(&json!({ "jsonrpc": "2.0", "id": req1["id"], "result": { "n": 1 } }))
        .await;
    let req3 = peer.read().await;
    assert_eq!(req3["method"], json!("work.3"), "{req3}");

    // Answering call 2 frees the next credit -> call 4 must arrive next.
    peer.send(&json!({ "jsonrpc": "2.0", "id": req2["id"], "result": { "n": 2 } }))
        .await;
    let req4 = peer.read().await;
    assert_eq!(req4["method"], json!("work.4"), "{req4}");

    peer.send(&json!({ "jsonrpc": "2.0", "id": req3["id"], "result": { "n": 3 } }))
        .await;
    peer.send(&json!({ "jsonrpc": "2.0", "id": req4["id"], "result": { "n": 4 } }))
        .await;

    let results: Vec<Value> = vec![
        c1.await.unwrap().unwrap().read().unwrap(),
        c2.await.unwrap().unwrap().read().unwrap(),
        c3.await.unwrap().unwrap().read().unwrap(),
        c4.await.unwrap().unwrap().read().unwrap(),
    ];
    assert_eq!(
        results,
        vec![
            json!({ "n": 1 }),
            json!({ "n": 2 }),
            json!({ "n": 3 }),
            json!({ "n": 4 }),
        ]
    );
    instance.stop().await;
    root.stop().await;
}

/// A call past its deadline is abandoned with `$/cancel`, fails with
/// `deadline_exceeded` naming its kind's timeout setting, releases its
/// credit, and its late result is refused as `late_result`.
///
/// Covers: story:7-33
#[rstest]
#[case::plugin(PluginKindName::EXTRACT, "timeout.plugin", 30_000)]
#[case::hook(PluginKindName::HOOK, "timeout.hook", 10_000)]
#[case::model(PluginKindName::MODEL, "timeout.model", 60_000)]
#[tokio::test(start_paused = true)]
async fn a_call_past_its_deadline_sends_cancel_fails_with_setting_and_refuses_late_result(
    #[case] kind: PluginKindName,
    #[case] expected_key: &str,
    #[case] expected_ms: u64,
) {
    let (subscriber, log) = capturing(&[("goodfetch_engine::plugin_host", Level::WARN)]);
    let _logging = tracing::subscriber::set_default(subscriber);
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let (instance, mut peer) = start_in_memory(&root, clock, 1, &json!({}));
    peer.accept_hello().await;

    let deadline = deadline_for(&kind, None);
    assert_eq!(deadline.duration(), Duration::from_millis(expected_ms));

    let call = {
        let instance = instance.clone();
        tokio::spawn(async move {
            instance
                .call_under("work.slow", &json!({}), CallOptions::read(deadline))
                .await
        })
    };
    let req = peer.read().await;
    assert_eq!(req["method"], json!("work.slow"));

    // Advance paused clock past the kind's deadline.
    tokio::time::advance(Duration::from_millis(expected_ms + 1)).await;

    let failure = call.await.unwrap();
    let Err(CallFailure::DeadlineExceeded(error)) = failure else {
        panic!("expected DeadlineExceeded, got {failure:?}");
    };
    assert_eq!(error.code(), "deadline_exceeded");
    assert!(
        error.message().contains(expected_key),
        "message `{}` should name `{expected_key}`",
        error.message()
    );
    assert_eq!(
        error.details().map(|d| d["limit"]["key"].clone()),
        Some(json!(expected_key))
    );

    // `$/cancel` was already written before `call_under` returned.
    let cancel = peer.read_non_ping().await;
    assert_eq!(cancel["method"], json!("$/cancel"), "{cancel}");
    assert_eq!(cancel["params"]["call_id"], req["id"], "{cancel}");
    assert_eq!(cancel["params"]["deadline"], json!(true), "{cancel}");

    // Send a late result for the abandoned call, then serve a follow-up call
    // to prove the credit was released and the late result was refused.
    peer.send(&json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "stale": true } }))
        .await;
    let follow_up = {
        let instance = instance.clone();
        tokio::spawn(async move { instance.call("work.after", &json!({})).await })
    };
    let after_req = peer.read_non_ping().await;
    assert_eq!(after_req["method"], json!("work.after"));
    peer.send(&json!({ "jsonrpc": "2.0", "id": after_req["id"], "result": { "fresh": true } }))
        .await;
    let after_res: Value = follow_up.await.unwrap().unwrap().read().unwrap();
    assert_eq!(after_res, json!({ "fresh": true }));

    let written = String::from_utf8(log.written()).unwrap();
    assert!(
        written.contains("WARN") && written.contains("late_result"),
        "expected WARN late_result in log:\n{written}"
    );
    instance.stop().await;
    root.stop().await;
}

/// An abandoned call of an Effect-class method that was sent to the Plugin
/// is reported uncertain at once, whereas an Effect-class call that timed
/// out while still waiting for a credit (never sent) is not uncertain.
///
/// Covers: story:7-33
#[tokio::test(start_paused = true)]
async fn an_abandoned_effect_call_is_reported_uncertain_at_once_while_a_queued_effect_is_not() {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let (instance, mut peer) = start_in_memory(&root, clock, 1, &json!({}));
    peer.accept_hello().await;

    let reporter = Arc::new(RecordedUncertain::default());

    // Call 1 (in-flight Effect, 5 s deadline) takes the sole credit.
    let in_flight = {
        let instance = instance.clone();
        let reporter: Arc<dyn UncertainReporter> = Arc::clone(&reporter) as _;
        tokio::spawn(async move {
            instance
                .call_under(
                    "sink.write",
                    &json!({ "id": 1 }),
                    CallOptions::effect(
                        Deadline::given(&TIMEOUT_PLUGIN, 5_000, SettingsLayer::Default),
                        reporter,
                    ),
                )
                .await
        })
    };
    let req1 = peer.read().await;
    assert_eq!(req1["method"], json!("sink.write"));

    // Call 2 (queued Effect, 2 s deadline) waits for a credit and never leaves.
    let queued = {
        let instance = instance.clone();
        let reporter: Arc<dyn UncertainReporter> = Arc::clone(&reporter) as _;
        tokio::spawn(async move {
            instance
                .call_under(
                    "sink.write_queued",
                    &json!({ "id": 2 }),
                    CallOptions::effect(
                        Deadline::given(&TIMEOUT_PLUGIN, 2_000, SettingsLayer::Default),
                        reporter,
                    ),
                )
                .await
        })
    };
    tokio::task::yield_now().await;

    // Advance 2 001 ms: call 2 times out while queued -> NOT uncertain.
    tokio::time::advance(Duration::from_millis(2_001)).await;
    assert!(matches!(
        queued.await.unwrap(),
        Err(CallFailure::DeadlineExceeded(_))
    ));
    assert_eq!(reporter.snapshot(), Vec::new());

    // Advance 3 000 ms more (total 5 001 ms): call 1 times out in flight ->
    // reported uncertain at once!
    tokio::time::advance(Duration::from_millis(3_000)).await;
    assert!(matches!(
        in_flight.await.unwrap(),
        Err(CallFailure::DeadlineExceeded(_))
    ));
    let uncertain = reporter.snapshot();
    assert_eq!(uncertain.len(), 1, "{uncertain:?}");
    assert_eq!(uncertain[0].plugin, "in-memory");
    assert_eq!(uncertain[0].method, "sink.write");
    assert_eq!(uncertain[0].reason, UncertainReason::Interrupted);

    // Only call 1 received `$/cancel`; call 2 was never sent.
    let cancel = peer.read_non_ping().await;
    assert_eq!(cancel["method"], json!("$/cancel"), "{cancel}");
    assert_eq!(cancel["params"]["call_id"], req1["id"], "{cancel}");

    instance.stop().await;
    root.stop().await;
}

/// Inside a work item, a call's deadline never outlives the work item's
/// lease, and a call after the lease already ended is withheld without
/// ever being sent.
///
/// Covers: story:7-33
#[tokio::test(start_paused = true)]
async fn inside_a_work_item_a_calls_deadline_never_outlives_the_lease() {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let (instance, mut peer) = start_in_memory(&root, clock, 1, &json!({}));
    peer.accept_hello().await;

    let lease_end: Instant = clock.now() + Duration::from_millis(5_000);
    let enclosing = Enclosing::new(
        &Deadline::given(&TIMEOUT_HOOK, 5_000, SettingsLayer::Default),
        EnclosingCall {
            operation: "crawl_request.handle".to_owned(),
            call_id: RequestId::Number(1),
        },
        move || lease_end,
    );
    let lease = Arc::new(TestLease {
        enclosing,
        ended: std::sync::atomic::AtomicBool::new(false),
    });

    // The call's own deadline is 30 s (`timeout.plugin`), but the lease ends at 5 s.
    let call = {
        let instance = instance.clone();
        let lease: Arc<dyn LeaseCheck> = Arc::clone(&lease) as _;
        tokio::spawn(async move {
            instance
                .call_under(
                    "work.leased",
                    &json!({}),
                    CallOptions::read(Deadline::declared(&TIMEOUT_PLUGIN)).with_lease(lease),
                )
                .await
        })
    };
    let req = peer.read().await;
    assert_eq!(req["method"], json!("work.leased"));

    // Advancing 5 001 ms (far short of 30 s) must abandon the call at the lease's end.
    tokio::time::advance(Duration::from_millis(5_001)).await;
    let failure = call.await.unwrap();
    let Err(CallFailure::DeadlineExceeded(error)) = failure else {
        panic!("expected DeadlineExceeded from enclosing lease, got {failure:?}");
    };
    assert_eq!(error.code(), "deadline_exceeded");
    assert_eq!(
        error.details().map(|d| d["ended_by"]["operation"].clone()),
        Some(json!("crawl_request.handle")),
        "{error:?}"
    );
    let cancel = peer.read_non_ping().await;
    assert_eq!(cancel["method"], json!("$/cancel"), "{cancel}");
    assert_eq!(cancel["params"]["call_id"], req["id"], "{cancel}");
    assert_eq!(cancel["params"]["deadline"], json!(true), "{cancel}");

    // Once the lease has ended, a new call is withheld immediately without sending.
    lease
        .ended
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let withheld = instance
        .call_under(
            "work.after_lease",
            &json!({}),
            CallOptions::read(Deadline::declared(&TIMEOUT_PLUGIN))
                .with_lease(Arc::clone(&lease) as _),
        )
        .await;
    assert!(matches!(withheld, Err(CallFailure::Unavailable(_))));

    instance.stop().await;
    root.stop().await;
}

/// Records every [`QuarantineEvent`] sent by a [`PluginInstance`].
#[derive(Debug, Default)]
struct RecordedQuarantines(Mutex<Vec<QuarantineEvent>>);

impl RecordedQuarantines {
    fn snapshot(&self) -> Vec<QuarantineEvent> {
        self.0.lock().unwrap().clone()
    }
}

impl QuarantineNotifier for RecordedQuarantines {
    fn notify(&self, event: QuarantineEvent) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        self.0.lock().unwrap().push(event);
        Box::pin(async {})
    }
}

/// Controllable [`ResourceMonitor`] for testing each resource limit on
/// `Clock::paused`.
#[derive(Debug, Default)]
struct ControllableMonitor(Mutex<Option<ResourceSample>>);

impl ControllableMonitor {
    fn set(&self, sample: Option<ResourceSample>) {
        *self.0.lock().unwrap() = sample;
    }
}

impl ResourceMonitor for ControllableMonitor {
    fn sample(&self) -> Option<ResourceSample> {
        *self.0.lock().unwrap()
    }
}

/// Starts a restartable in-memory instance with custom manifest `limits`,
/// `scope` and `hooks`.
fn start_restarting(
    owner: &TaskOwner,
    clock: Clock,
    limits: &Value,
    scope: Scope,
    hooks: InstanceHooks,
) -> (
    PluginInstance,
    tokio::sync::mpsc::UnboundedReceiver<InMemoryPeer>,
) {
    let manifest_json = json!({
        "manifest_version": "0.1",
        "name": "in-memory",
        "version": "1.0.0",
        "kinds": ["proxy"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "network_level": "none",
        "limits": limits,
    });
    let manifest = PluginManifest::read(manifest_json.to_string().as_bytes()).unwrap();
    let settings = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
    let fetch = PluginFetch::of(&settings, "in-memory", clock, owner.blocking_threads()).unwrap();
    PluginInstance::start_in_memory_restarting(
        owner,
        InMemoryPlugin {
            manifest,
            product: Product::Collie,
            fetch: Arc::new(fetch),
            clock,
            spend: None,
        },
        scope,
        hooks,
    )
}

async fn next_peer(rx: &mut tokio::sync::mpsc::UnboundedReceiver<InMemoryPeer>) -> Peer {
    Peer::new(rx.recv().await.expect("expected next in-memory peer"))
}

/// A crashed Plugin fails its in-flight read with retryable `plugin_crashed`,
/// retries reads under their lease after backoff + `hello_ok`, and reports an
/// unchecked in-flight Effect uncertain without retrying it blindly.
///
/// Covers: story:7-34
#[tokio::test(start_paused = true)]
async fn crash_fails_read_retried_under_lease_and_reports_unchecked_effect_uncertain() {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let (instance, mut peers) = start_restarting(
        &root,
        clock,
        &json!({ "concurrency": 3 }),
        Scope::Task(None),
        InstanceHooks::default(),
    );
    let mut first_peer = next_peer(&mut peers).await;
    first_peer.accept_hello().await;

    let lease_end = clock.now() + Duration::from_secs(30);
    let lease: Arc<dyn LeaseCheck> = Arc::new(TestLease {
        enclosing: Enclosing::new(
            &Deadline::given(&TIMEOUT_PLUGIN, 30_000, SettingsLayer::Default),
            EnclosingCall {
                operation: "work.item".to_owned(),
                call_id: RequestId::Number(1),
            },
            move || lease_end,
        ),
        ended: std::sync::atomic::AtomicBool::new(false),
    });
    let uncertain = Arc::new(RecordedUncertain::default());

    let direct_read = {
        let instance = instance.clone();
        tokio::spawn(async move {
            instance
                .call_under(
                    "work.direct_read",
                    &json!({}),
                    CallOptions::read(Deadline::declared(&TIMEOUT_PLUGIN)),
                )
                .await
        })
    };
    let req_direct = first_peer.read().await;
    assert_eq!(req_direct["method"], json!("work.direct_read"));

    let leased_read = {
        let instance = instance.clone();
        let lease = Arc::clone(&lease);
        tokio::spawn(async move {
            instance
                .call_retrying(
                    "work.leased_read",
                    &json!({}),
                    CallOptions::read(Deadline::declared(&TIMEOUT_PLUGIN)).with_lease(lease),
                )
                .await
        })
    };
    let req_leased = first_peer.read().await;
    assert_eq!(req_leased["method"], json!("work.leased_read"));

    let effect_call = {
        let instance = instance.clone();
        let uncertain = Arc::clone(&uncertain);
        tokio::spawn(async move {
            instance
                .call_retrying(
                    "work.unchecked_effect",
                    &json!({}),
                    CallOptions::effect(Deadline::declared(&TIMEOUT_PLUGIN), uncertain),
                )
                .await
        })
    };
    let req_effect = first_peer.read().await;
    assert_eq!(req_effect["method"], json!("work.unchecked_effect"));

    // Crash peer 1 while all three calls are in flight.
    drop(first_peer);

    let Err(CallFailure::Unavailable(direct_err)) = direct_read.await.unwrap() else {
        panic!("direct read must fail with plugin_crashed");
    };
    assert_eq!(direct_err.code(), "plugin_crashed");
    assert!(direct_err.retryable());

    let Err(CallFailure::Unavailable(effect_err)) = effect_call.await.unwrap() else {
        panic!("unchecked Effect must fail on crash without blind retry");
    };
    assert_eq!(effect_err.code(), "plugin_crashed");
    let reported = uncertain.snapshot();
    assert_eq!(reported.len(), 1, "{reported:?}");
    assert_eq!(reported[0].method, "work.unchecked_effect");
    assert_eq!(reported[0].reason, UncertainReason::Interrupted);

    // Advance 1 s restart backoff: peer 2 starts, accepts `hello`, receives the
    // retried leased read (and never the unchecked Effect).
    tokio::time::advance(Duration::from_secs(1)).await;
    let mut second_peer = next_peer(&mut peers).await;
    second_peer.accept_hello().await;
    let retried = second_peer.read().await;
    assert_eq!(retried["method"], json!("work.leased_read"));
    second_peer
        .send(&json!({
            "jsonrpc": "2.0",
            "id": retried["id"],
            "result": { "retried": true },
        }))
        .await;

    let retried_ok: Value = leased_read.await.unwrap().unwrap().read().unwrap();
    assert_eq!(retried_ok, json!({ "retried": true }));

    instance.stop().await;
    root.stop().await;
}

/// Restart waits follow 1 s, 2 s, 4 s, 8 s, 16 s, 30 s, 30 s on `Clock::paused`
/// when crashes stay outside the 10-minute quarantine window.
///
/// Covers: story:7-34
#[tokio::test(start_paused = true)]
async fn backoff_doubles_from_one_second_to_thirty_seconds_on_paused_clock() {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let (backoff_instance, mut backoff_peers) = start_restarting(
        &root,
        clock,
        &json!({ "concurrency": 2 }),
        Scope::Task(None),
        InstanceHooks::default(),
    );
    let mut peer = next_peer(&mut backoff_peers).await;
    peer.accept_hello().await;
    for expected_secs in [1_u64, 2, 4, 8, 16, 30, 30] {
        for _ in 0..132 {
            tokio::time::advance(Duration::from_secs(5)).await;
            let ping = peer.read().await;
            assert_eq!(ping["method"], json!("$/ping"));
            peer.send(&json!({ "jsonrpc": "2.0", "id": ping["id"], "result": {} }))
                .await;
        }
        drop(peer);
        tokio::time::advance(Duration::from_millis(expected_secs * 1000 - 1)).await;
        tokio::task::yield_now().await;
        assert!(
            backoff_peers.try_recv().is_err(),
            "replacement peer must not start before {expected_secs} s backoff"
        );
        tokio::time::advance(Duration::from_millis(1)).await;
        peer = next_peer(&mut backoff_peers).await;
        peer.accept_hello().await;
    }
    backoff_instance.stop().await;
    root.stop().await;
}

/// Three crashes within 10 minutes quarantine the Plugin, fail queued and
/// later calls with `plugin_quarantined`, record the quarantine in the Audit
/// log, and send exactly one Notification.
///
/// Covers: story:7-35
#[tokio::test(start_paused = true)]
async fn third_crash_in_ten_minutes_quarantines_with_audit_and_single_notification() {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let home_dir = tempfile::tempdir().unwrap();
    let home = Home::new(home_dir.path().to_path_buf());
    let audit = AuditLog::of(&home, None, Arc::new(root.child()));
    let notifications = Arc::new(RecordedQuarantines::default());
    let run_id = RunId::parse("quarantine-engine-test").unwrap();
    let (instance, mut peers) = start_restarting(
        &root,
        clock,
        &json!({ "concurrency": 1 }),
        Scope::Run(run_id.clone()),
        InstanceHooks {
            audit: Some(audit.clone()),
            quarantine_notifier: Some(Arc::clone(&notifications) as _),
            ..InstanceHooks::default()
        },
    );

    let mut p1 = next_peer(&mut peers).await;
    p1.accept_hello().await;
    drop(p1);
    tokio::time::advance(Duration::from_secs(1)).await;

    let mut p2 = next_peer(&mut peers).await;
    p2.accept_hello().await;
    drop(p2);
    tokio::time::advance(Duration::from_secs(2)).await;

    let mut p3 = next_peer(&mut peers).await;
    p3.accept_hello().await;
    let in_flight = {
        let instance = instance.clone();
        tokio::spawn(async move { instance.call("work.in_flight", &json!({})).await })
    };
    let _ = p3.read().await;
    let queued = {
        let instance = instance.clone();
        tokio::spawn(async move { instance.call("work.queued", &json!({})).await })
    };
    tokio::task::yield_now().await;
    drop(p3);

    let Err(CallFailure::Unavailable(in_flight_err)) = in_flight.await.unwrap() else {
        panic!("in-flight call must fail on crash");
    };
    assert_eq!(in_flight_err.code(), "plugin_crashed");

    let Err(CallFailure::Unavailable(queued_err)) = queued.await.unwrap() else {
        panic!("queued call must fail with plugin_quarantined");
    };
    assert_eq!(queued_err.code(), "plugin_quarantined");
    assert!(!queued_err.retryable());

    for method in ["work.later_1", "work.later_2"] {
        let Err(CallFailure::Unavailable(err)) = instance.call(method, &json!({})).await else {
            panic!("later call on quarantined Plugin must fail");
        };
        assert_eq!(err.code(), "plugin_quarantined");
    }
    tokio::time::advance(Duration::from_mins(1)).await;
    assert!(
        peers.try_recv().is_err(),
        "quarantined Plugin must not restart"
    );

    let sent = notifications.snapshot();
    assert_eq!(sent.len(), 1, "expected 1 Notification: {sent:?}");
    assert_eq!(sent[0].plugin, "in-memory");
    assert_eq!(sent[0].run_id.as_ref(), Some(&run_id));
    assert_eq!(sent[0].crashes, 3);

    let exported = audit.export().unwrap();
    let quarantine_entries: Vec<_> = exported
        .iter()
        .filter(|e| e.entry.kind == PLUGIN_QUARANTINED_AUDIT.name)
        .collect();
    assert_eq!(quarantine_entries.len(), 1, "{exported:?}");
    assert_eq!(
        quarantine_entries[0].entry.details["plugin"],
        json!("in-memory")
    );
    assert_eq!(quarantine_entries[0].entry.details["crashes"], json!(3));

    instance.stop().await;
    root.stop().await;
}

/// Three unanswered `$/ping` heartbeats restart the instance after backoff.
///
/// Covers: story:7-36
#[tokio::test(start_paused = true)]
async fn three_unanswered_pings_restart_instance() {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let (hb_instance, mut hb_peers) = start_restarting(
        &root,
        clock,
        &json!({ "concurrency": 2 }),
        Scope::Task(None),
        InstanceHooks::default(),
    );
    let mut p1 = next_peer(&mut hb_peers).await;
    p1.accept_hello().await;
    for expected_ping in 1..=3 {
        tokio::time::advance(Duration::from_secs(5)).await;
        let ping = p1.read().await;
        assert_eq!(
            ping["method"],
            json!("$/ping"),
            "ping #{expected_ping}: {ping}"
        );
    }
    tokio::time::advance(Duration::from_secs(5)).await;
    tokio::task::yield_now().await;
    assert!(hb_peers.try_recv().is_err(), "must wait 1 s backoff");
    tokio::time::advance(Duration::from_secs(1)).await;
    let mut p2 = next_peer(&mut hb_peers).await;
    p2.accept_hello().await;
    hb_instance.stop().await;
    root.stop().await;
}

/// Holding more abandoned calls than concurrency, or abandoning a call while a
/// proxied stream is open, restarts the instance under the crash rules.
///
/// Covers: story:7-36
#[tokio::test(start_paused = true)]
async fn abandoned_overflow_and_open_proxied_stream_restart_instance() {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);

    let (ab_instance, mut ab_peers) = start_restarting(
        &root,
        clock,
        &json!({ "concurrency": 2 }),
        Scope::Task(None),
        InstanceHooks::default(),
    );
    let mut p1 = next_peer(&mut ab_peers).await;
    p1.accept_hello().await;
    for i in 1..=3 {
        let inst = ab_instance.clone();
        let handle = tokio::spawn(async move {
            inst.call_under(
                "work.hang",
                &json!({ "n": i }),
                CallOptions::read(Deadline::given(
                    &TIMEOUT_PLUGIN,
                    500,
                    SettingsLayer::Default,
                )),
            )
            .await
        });
        let req = p1.read().await;
        assert_eq!(req["method"], json!("work.hang"));
        tokio::time::advance(Duration::from_millis(501)).await;
        assert!(matches!(
            handle.await.unwrap(),
            Err(CallFailure::DeadlineExceeded(_))
        ));
        let cancel = p1.read().await;
        assert_eq!(cancel["method"], json!("$/cancel"));
        if i < 3 {
            assert!(
                ab_peers.try_recv().is_err(),
                "must not restart at {i} <= 2 abandoned"
            );
        }
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    let mut p2 = next_peer(&mut ab_peers).await;
    p2.accept_hello().await;
    ab_instance.stop().await;

    let open_streams = OpenStreams::new(1);
    let (ps_instance, mut ps_peers) = start_restarting(
        &root,
        clock,
        &json!({ "concurrency": 4 }),
        Scope::Task(None),
        InstanceHooks {
            proxied_streams: Some(open_streams),
            ..InstanceHooks::default()
        },
    );
    let mut p1 = next_peer(&mut ps_peers).await;
    p1.accept_hello().await;
    let inst = ps_instance.clone();
    let call = tokio::spawn(async move {
        inst.call_under(
            "work.streaming",
            &json!({}),
            CallOptions::read(Deadline::given(
                &TIMEOUT_PLUGIN,
                500,
                SettingsLayer::Default,
            )),
        )
        .await
    });
    let _ = p1.read().await;
    tokio::time::advance(Duration::from_millis(501)).await;
    assert!(matches!(
        call.await.unwrap(),
        Err(CallFailure::DeadlineExceeded(_))
    ));
    tokio::time::advance(Duration::from_secs(1)).await;
    let mut p2 = next_peer(&mut ps_peers).await;
    p2.accept_hello().await;
    ps_instance.stop().await;
    root.stop().await;
}

/// Each resource limit (`memory`, `cpu_time`, `processes`, `open_files`) kills
/// the instance with non-retryable `plugin_limit_exceeded` under the crash
/// rules, restarting after backoff.
///
/// Covers: story:7-37, error:plugin_limit_exceeded
#[rstest]
#[case::memory(
    PluginResourceLimit::Memory,
    ResourceSample { memory_bytes: 10_000_001, cpu_time_secs: 0, processes: 1, open_files: 4 },
    10_000_000_u64,
)]
#[case::cpu_time(
    PluginResourceLimit::CpuTime,
    ResourceSample { memory_bytes: 1024, cpu_time_secs: 10, processes: 1, open_files: 4 },
    10_u64,
)]
#[case::processes(
    PluginResourceLimit::Processes,
    ResourceSample { memory_bytes: 1024, cpu_time_secs: 0, processes: 3, open_files: 4 },
    2_u64,
)]
#[case::open_files(
    PluginResourceLimit::OpenFiles,
    ResourceSample { memory_bytes: 1024, cpu_time_secs: 0, processes: 1, open_files: 16 },
    16_u64,
)]
#[tokio::test(start_paused = true)]
async fn each_resource_limit_kills_with_plugin_limit_exceeded_under_crash_rules(
    #[case] expected_resource: PluginResourceLimit,
    #[case] sample: ResourceSample,
    #[case] expected_limit: u64,
) {
    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let limits = json!({
        "memory": 10_000_000_u64,
        "cpu_time": 10_u64,
        "processes": 2_u32,
        "open_files": 16_u32,
        "concurrency": 2_u32,
    });
    let monitor = Arc::new(ControllableMonitor::default());
    let (instance, mut peers) = start_restarting(
        &root,
        clock,
        &limits,
        Scope::Task(None),
        InstanceHooks {
            resource_monitor: Some(Arc::clone(&monitor) as _),
            ..InstanceHooks::default()
        },
    );
    let mut p1 = next_peer(&mut peers).await;
    p1.accept_hello().await;

    let call = {
        let instance = instance.clone();
        tokio::spawn(async move {
            instance
                .call_retrying(
                    "work.heavy",
                    &json!({}),
                    CallOptions::read(Deadline::declared(&TIMEOUT_PLUGIN)),
                )
                .await
        })
    };
    let _ = p1.read().await;
    monitor.set(Some(sample));
    tokio::time::advance(Duration::from_millis(60)).await;

    let Err(CallFailure::Unavailable(error)) = call.await.unwrap() else {
        panic!("expected plugin_limit_exceeded for {expected_resource:?}");
    };
    assert_eq!(error.code(), "plugin_limit_exceeded");
    assert!(
        !error.retryable(),
        "{expected_resource:?} must not be retryable"
    );
    assert_eq!(
        error.details().map(|d| d["resource"].clone()),
        Some(json!(expected_resource.as_str()))
    );
    assert_eq!(
        error.details().map(|d| d["limit"].clone()),
        Some(json!(expected_limit))
    );

    monitor.set(None);
    tokio::time::advance(Duration::from_secs(1)).await;
    let mut p2 = next_peer(&mut peers).await;
    p2.accept_hello().await;
    instance.stop().await;
    root.stop().await;
}

async fn accept_hello_with_level(peer: &mut Peer, name: &str, level: &str) {
    let hello = peer.read().await;
    peer.send(&json!({
        "jsonrpc": "2.0",
        "id": hello["id"],
        "result": {
            "protocol_version": "0.1",
            "name": name,
            "version": "1.0.0",
            "kinds": [{ "kind": "proxy", "capabilities": [] }],
            "network_level": level,
        }
    }))
    .await;
}

/// Before the Windows filter broker service stops, the engine stops every
/// dependent (`proxy_tcp` / `direct`) Plugin instance under the crash rules
/// (`plugin_crashed`), while a `none` instance that does not depend on the
/// broker keeps running (ADR 0033, issue 879).
///
/// Covers: story:7-112
#[tokio::test(start_paused = true)]
async fn filter_broker_stop_ends_dependent_instances_under_crash_rules_while_none_keeps_running() {
    use tokio_util::sync::CancellationToken;

    let clock = Clock::paused(SystemTime::UNIX_EPOCH);
    let root = TaskOwner::root(BlockingPolicy::Threads);
    let broker_stopping = CancellationToken::new();

    let start_level = |level: &str| {
        let manifest_json = json!({
            "manifest_version": "0.1",
            "name": format!("plugin-{level}"),
            "version": "1.0.0",
            "kinds": ["proxy"],
            "protocol": { "min": "0.1", "max": "0.1" },
            "entrypoint": "plugin",
            "network_level": level,
        });
        let manifest = PluginManifest::read(manifest_json.to_string().as_bytes()).unwrap();
        let settings = resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap();
        let fetch =
            PluginFetch::of(&settings, &manifest.name, clock, root.blocking_threads()).unwrap();
        PluginInstance::start_in_memory_restarting(
            &root,
            InMemoryPlugin {
                manifest,
                product: Product::Collie,
                fetch: Arc::new(fetch),
                clock,
                spend: None,
            },
            Scope::Task(None),
            InstanceHooks {
                broker_stopping: Some(broker_stopping.clone()),
                ..InstanceHooks::default()
            },
        )
    };

    let (proxy_instance, mut proxy_peers) = start_level("proxy_tcp");
    let (none_instance, mut none_peers) = start_level("none");
    let mut proxy_peer = next_peer(&mut proxy_peers).await;
    let mut none_peer = next_peer(&mut none_peers).await;

    accept_hello_with_level(&mut proxy_peer, "plugin-proxy_tcp", "proxy_tcp").await;
    accept_hello_with_level(&mut none_peer, "plugin-none", "none").await;

    let spawn_work = |inst: PluginInstance| {
        tokio::spawn(async move {
            inst.call_under(
                "work.run",
                &json!({}),
                CallOptions::read(Deadline::declared(&TIMEOUT_PLUGIN)),
            )
            .await
        })
    };
    let proxy_call = spawn_work(proxy_instance.clone());
    let none_call = spawn_work(none_instance.clone());

    let _proxy_req = proxy_peer.read().await;
    let none_req = none_peer.read().await;

    broker_stopping.cancel();

    // `proxy_tcp` instance immediately ends under crash rules (`plugin_crashed`).
    let Err(CallFailure::Unavailable(err)) = proxy_call.await.unwrap() else {
        panic!("expected proxy_tcp call to fail with plugin_crashed when broker stops");
    };
    assert_eq!(err.code(), "plugin_crashed");
    assert!(err.retryable());

    // `none` instance is NOT dependent on the filter broker and answers normally.
    none_peer
        .send(&json!({
            "jsonrpc": "2.0",
            "id": none_req["id"],
            "result": { "ok": true }
        }))
        .await;
    assert!(none_call.await.unwrap().is_ok());

    proxy_instance.stop().await;
    none_instance.stop().await;
    root.stop().await;
}
