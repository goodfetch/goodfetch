//! The live heap of one engine that ends Run after Run, as a Daemon does
//! for its life (#1768; ADR 0134's soak note): once warm, each Run started
//! and finished leaves no byte more in it. The engine runs in this test's
//! process over the in-memory network, on the tests' paused runtime with
//! its blocking work inline, so each Run allocates the same; this test
//! binary's global allocator is the executables' count,
//! `goodfetch_engine::heap::Counting`, around the system's, and
//! `heap::live()` reads it after each Run. The event stream keeps its last
//! lossless events, a bounded window (ADR 0031), set here to 16, eight
//! Runs' worth, so it is full long before the warm-up ends; the Daemon's
//! 4,096 fill after about 2,000 Runs, which the Daemon soak cannot reach.
//! The test declares no Feature: it is spec #213's own check of the runs
//! registry and the event stream, as the soak is spec #327's.

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::events::EventsOptions;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter, Network};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
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

/// The window of lossless events the engine keeps: each Run adds two, its
/// start's and its end's `run.axes_changed`.
const RETAINED: usize = 16;

/// One Client's Connection to the engine.
struct Client {
    lines: Lines<BufReader<ConnectionReader>>,
    writer: ConnectionWriter,
}

impl Client {
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

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the engine runs on the paused Clock, the Clock's test version (TESTING.md, Time)"
)]
fn runs_ended_one_after_another_leave_the_engines_live_heap_flat_once_warm() {
    let env = TestEnv::in_memory();
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.0.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default().with(goodfetch_engine::contribution());
    let engine = Engine::new(identity, &composition)
        .unwrap()
        .with_events(EventsOptions {
            queue: 64,
            retained: RETAINED,
        });
    let heap = goodfetch_test_support::runtime().block_on(async {
        // The paused Clock is made on the runtime it runs on.
        let dependencies = Dependencies {
            environment: std::collections::BTreeMap::new(),
            keychain: goodfetch_engine::secrets::Keychain::absent(),
            home: Home::new(env.home().to_owned()),
            clock: Clock::paused(env.start_time()),
            entropy: Entropy::seeded(env.seed()),
            blocking: BlockingPolicy::Inline,
            run_state_location: RunStateLocation::PerMajor,
            run_state: RunStateOptions::default(),
            politeness: goodfetch_engine::politeness::PolitenessOptions::default(),
        };

        let (network, dialer) = transport::in_memory();
        let talk = async {
            let (reader, writer) = dialer.connect().await.unwrap().split();
            let mut client = Client {
                lines: BufReader::new(reader).lines(),
                writer,
            };
            let hello = client
                .call(
                    0,
                    "hello",
                    json!({
                        "protocol_version": goodfetch_protocol::catalog::PROTOCOL_VERSION.to_string(),
                        "surface": "sdk_typescript",
                        "needs": "collie",
                        "product_version": "0.0.0",
                    }),
                )
                .await;
            assert!(hello.get("result").is_some(), "{hello}");
            ended_runs(&mut client, 1, WARM).await;
            let heap = ended_runs(&mut client, WARM + 1, MEASURED).await;
            client.writer.shutdown().await.unwrap();
            heap
        };
        let (served, heap) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            talk
        );
        served.unwrap();
        heap
    });

    let slope = slope_of(&heap);
    assert!(
        slope <= MOST_GROWTH_A_RUN,
        "the engine's live heap climbed {slope} bytes a Run over {MEASURED} Runs once warm, more than {MOST_GROWTH_A_RUN}; its live heap after each: {heap:?}"
    );
}
