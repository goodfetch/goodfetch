//! The live heap of one engine as a Daemon holds it for its life (#1768;
//! ADR 0134's soak note). Once warm, each Run started and finished leaves
//! no byte more in it; and the event stream's window of its last lossless
//! events (ADR 0031), full at the Daemon's 4,096, holds under 1 MB of it
//! (#1782). The engine runs in this test's process over the in-memory
//! network, on the tests' paused runtime with its blocking work inline, so
//! each Run allocates the same; this test binary's global allocator is the
//! executables' count, `goodfetch_engine::heap::Counting`, around the
//! system's, and `heap::live()` reads it. For the Runs, the window is set
//! to 16, eight Runs' worth, so it is full long before the warm-up ends;
//! the Daemon's 4,096 fill after about 2,000 Runs, which the Daemon soak
//! cannot reach, so the window's own test fills it with the events a
//! stand-in operation publishes. The tests declare no Feature: they are
//! spec #213's own checks of the runs registry and the event stream, as the
//! soak is spec #327's.

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::events::EventsOptions;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{
    self, ConnectionReader, ConnectionWriter, InMemoryDialer, Network,
};
use goodfetch_engine::{
    CallContext, CallError, Channel, Composition, Contribution, Dependencies, Engine, Handler,
    Identity,
};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::run::{Completeness, Lifecycle, RUN_AXES_CHANGED, RunAxes, RunId, Verdict};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, Lines};

/// The Runs before the live heap is read: the event stream's window, the
/// Run index's and the run files' first allocations are all made by then.
const WARM: usize = 40;

/// The Runs after each of which the live heap is read.
const MEASURED: usize = 100;

/// The steepest a line fitted to those readings may climb, in bytes a Run.
/// A Run's own allocations are freed by its end, so only what the engine
/// keeps of each ended Run climbs it: the axes and the registry entry each
/// held before #1768's fix climbed it by 394 bytes a Run, and the fixed
/// engine by none.
const MOST_GROWTH_A_RUN: i128 = 16;

/// The window of lossless events the engine keeps while Runs end: each Run
/// adds two, its start's and its end's `run.axes_changed`.
const RETAINED: usize = 16;

/// The most the Daemon's full window may hold of the live heap (#1782):
/// 1 MB.
const MOST_WINDOW_BYTES: u64 = 1_000_000;

/// The stand-in operation that publishes `events` `run.axes_changed`, each
/// of an ended Run numbered from `from`, as a Daemon's ended Runs do.
const PUBLISH_AXES: Operation = Operation {
    name: "test.publish_axes",
    description: "Publishes ended Runs' axes, as many as asked.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

/// One Client's Connection to the engine.
struct Client {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
}

impl Client {
    /// Connects through `dialer` and says hello.
    async fn connected(dialer: &InMemoryDialer) -> Self {
        let (reader, writer) = dialer.connect().await.unwrap().split();
        let mut client = Self {
            lines: BufReader::new(reader).lines(),
            writer,
        };
        let hello = client
            .call(
                0,
                "hello",
                json!({
                    "protocol_version": PROTOCOL_VERSION.to_string(),
                    "surface": "sdk_typescript",
                    "needs": "collie",
                    "product_version": "0.0.0",
                }),
            )
            .await;
        assert!(hello.get("result").is_some(), "{hello}");
        client
    }

    /// Sends the call `method` with `params`, and answers its answer.
    async fn call(&mut self, id: u64, method: &str, params: Value) -> Value {
        let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.writer
            .write_all(format!("{request}\n").as_bytes())
            .await
            .unwrap();
        loop {
            let line = self.lines.next_line().await.unwrap().unwrap();
            let message: Value = serde_json::from_str(&line).unwrap();
            if message["id"] == json!(id) {
                return message;
            }
        }
    }
}

/// An engine of `composition`, keeping `retained` lossless events.
fn engine(composition: &Composition, retained: usize) -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").unwrap(),
        channel: Channel::CratesIo,
    };
    Engine::new(identity, composition)
        .unwrap()
        .with_events(EventsOptions {
            queue: 64,
            retained,
        })
}

/// What the engine runs on in `env`, made on the runtime it runs on, as
/// its paused Clock is.
#[expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (TESTING.md, Time)"
)]
fn dependencies(env: &TestEnv) -> Dependencies {
    Dependencies {
        environment: std::collections::BTreeMap::new(),
        keychain: goodfetch_engine::secrets::Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::paused(env.start_time()),
        entropy: Entropy::seeded(env.seed()),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
    }
}

/// Serves `engine` in `env` while `talk` talks to it through a Client,
/// answering what `talk` answers.
fn served<T>(env: &TestEnv, engine: &Engine, talk: impl AsyncFnOnce(&mut Client) -> T) -> T {
    goodfetch_test_support::runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let talked = async {
            let mut client = Client::connected(&dialer).await;
            let answer = talk(&mut client).await;
            client.writer.shutdown().await.unwrap();
            answer
        };
        let (served, answer) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies(env)),
            talked
        );
        served.unwrap();
        answer
    })
}

/// The slope of the least-squares line through `heap`, the live heap after
/// each of consecutive Runs, in bytes a Run, rounded toward zero.
fn slope_of(heap: &[u64]) -> i128 {
    let count = i128::try_from(heap.len()).unwrap();
    let total: i128 = (0..count).sum();
    // Each Run's distance from the mean Run, times `count`, so that the
    // arithmetic stays in integers.
    let distance = |run: i128| count * run - total;
    let rise: i128 = (0..count)
        .zip(heap)
        .map(|(run, held)| distance(run) * i128::from(*held))
        .sum();
    let run: i128 = (0..count).map(|run| distance(run) * run).sum();
    rise / run
}

/// Starts and finishes `runs` bare Runs one after another over `client`,
/// numbered from `first`, answering the live heap after each.
async fn ended_runs(client: &mut Client, first: usize, runs: usize) -> Vec<u64> {
    let mut heap = Vec::with_capacity(runs);
    for number in first..first + runs {
        let run_id = format!("run-{number}");
        let id = u64::try_from(number).unwrap() * 2;
        let started = client
            .call(id, "run.start", json!({ "run_id": run_id }))
            .await;
        assert_eq!(started["result"]["lifecycle"], "running", "{started}");
        let finished = client
            .call(id + 1, "run.finish", json!({ "run_id": run_id }))
            .await;
        assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
        heap.push(goodfetch_engine::heap::live().unwrap());
    }
    heap
}

/// [`PUBLISH_AXES`]'s handler: each Run's axes as its end reports them,
/// under a Run ID as long as the engine generates, a ULID's 26 characters.
async fn publish_axes(params: Value, context: CallContext) -> Result<Value, CallError> {
    let from = params["from"].as_u64().unwrap();
    let events = params["events"].as_u64().unwrap();
    for number in from..from + events {
        let axes = RunAxes {
            run_id: RunId::parse(&format!("01jx{number:022}")).unwrap(),
            lifecycle: Lifecycle::Finished,
            verdict: Some(Verdict::NeedsReview),
            completeness: Some(Completeness::Unverified),
        };
        context
            .events()
            .publish(&RUN_AXES_CHANGED, Some(axes.run_id.as_str()), &axes)
            .await;
    }
    Ok(json!({}))
}

#[test]
fn runs_ended_one_after_another_leave_the_engines_live_heap_flat_once_warm() {
    let env = TestEnv::in_memory();
    let composition = Composition::default().with(goodfetch_engine::contribution());
    let heap = served(&env, &engine(&composition, RETAINED), async |client| {
        ended_runs(client, 1, WARM).await;
        ended_runs(client, WARM + 1, MEASURED).await
    });

    let slope = slope_of(&heap);
    assert!(
        slope <= MOST_GROWTH_A_RUN,
        "the engine's live heap climbed {slope} bytes a Run over {MEASURED} Runs once warm, more than {MOST_GROWTH_A_RUN}; its live heap after each: {heap:?}"
    );
}

#[test]
fn the_daemons_full_window_of_lossless_events_holds_under_a_megabyte() {
    let env = TestEnv::in_memory();
    let retained = EventsOptions::default().retained;
    let stand_in = Contribution::new("window-stand-in")
        .with_handler(Handler::new(&PUBLISH_AXES, publish_axes));
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(stand_in);
    let (empty, full) = served(&env, &engine(&composition, retained), async |client| {
        // One event first, so that the call's own first allocations are
        // made before the window is measured.
        let published = client
            .call(1, PUBLISH_AXES.name, json!({ "from": 0, "events": 1 }))
            .await;
        assert!(published.get("result").is_some(), "{published}");
        let empty = goodfetch_engine::heap::live().unwrap();
        // Twice the window, so that it is full and has let events go.
        let published = client
            .call(
                2,
                PUBLISH_AXES.name,
                json!({ "from": 1, "events": 2 * retained }),
            )
            .await;
        assert!(published.get("result").is_some(), "{published}");
        (empty, goodfetch_engine::heap::live().unwrap())
    });

    let window = full.saturating_sub(empty);
    assert!(
        window < MOST_WINDOW_BYTES,
        "the window of {retained} lossless events holds {window} bytes of the live heap, not under {MOST_WINDOW_BYTES}"
    );
}
