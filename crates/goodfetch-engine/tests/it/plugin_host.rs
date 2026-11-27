//! A process Plugin host's concurrency credits, deadlines, `$/cancel`,
//! late-result refusal, uncertain Effect reporting and work-item lease
//! bounding (ADRs 0003, 0024, 0029, 0033 and 0080), tested on `Clock::paused`
//! over the plugin protocol's in-memory adapter.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests run on `Clock::paused` (ADR 0059)"
)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::{Deadline, Enclosing};
use goodfetch_engine::lease::{LeaseCheck, LeaseEnded};
use goodfetch_engine::plugin_gate::PluginFetch;
use goodfetch_engine::plugin_host::instance::{
    CallFailure, CallOptions, InMemoryPeer, InMemoryPlugin, PluginInstance, Scope, UncertainCall,
    UncertainReporter,
};
use goodfetch_engine::plugin_kinds::deadline_for;
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{CALL_CANCELLED, EnclosingCall};
use goodfetch_protocol::effect::UncertainReason;
use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::plugin::{PluginKindName, PluginManifest};
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
    let cancel = peer.read().await;
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
    let after_req = peer.read().await;
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
    let cancel = peer.read().await;
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
    let cancel = peer.read().await;
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
